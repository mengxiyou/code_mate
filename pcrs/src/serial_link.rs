//! serial_link — 串口发现 / 握手 / 定时下发 / 断线自愈(独立线程)。对应 pc/serial_link.py。
//!
//! - 遍历串口(开口后立即 dtr/rts=false,避免 DTR 跳变触发 ESP32 自动复位;serialport-rs 默认
//!   DCB 已 DISABLE,再显式置牢),发 `{"t":"hello"}`,收到 `{"t":"id","name":"code_mate"}` 才锁定
//!   该口(不依赖 VID/PID,仅把 Espressif 303A 口排前面加速)。
//! - 锁定后每 DATA_INTERVAL 发一帧 `data`、每 PING_INTERVAL 发 `ping`;工作态翻转 / init 即时下发。
//! - 任何读写异常 / 死链 → 关口、回发现态自动重连。协议见 CLAUDE.md §5。
use crate::log;
use serde_json::Value;
use serialport::{available_ports, ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const DEVICE_NAME: &str = "code_mate";
const BAUD: u32 = 115200;
const DATA_INTERVAL: f64 = 3.0; // data 心跳节奏(工作态变化即时下发;此为兜底心跳,< 30s STALE)
const PING_INTERVAL: f64 = 15.0; // 心跳探活
const LIVENESS_TIMEOUT: f64 = 45.0; // 连续无 RX(含 pong)视为死链 → 重连
const HANDSHAKE_WINDOW: Duration = Duration::from_millis(600); // 每口等 id 的窗口
const PORT_TIMEOUT: Duration = Duration::from_millis(200); // 读/写超时(读循环只在有数据时读,不阻塞)
                                                           // 新出现的 303A 口在 GRACE_303A 内**每轮都探**(够本设备启动 + 握手连上);过宽限期后每 SLOW_303A
                                                           // 才探一次,其余时间放手该口 → 别的 ESP32 设备的串口空出来,能正常烧录/开串口。
const GRACE_303A: Duration = Duration::from_secs(12);
const SLOW_303A: Duration = Duration::from_secs(30);
const HELLO: &[u8] = b"{\"t\":\"hello\",\"v\":1}\n";
const PING: &[u8] = b"{\"t\":\"ping\"}\n";
const ESPRESSIF_VID: u16 = 0x303A;

/// host 实现:serial 线程通过它拉帧 / 回报连接态 / 通知已发 / 上报按键。
pub trait SerialClient: Send + Sync {
    /// 拉一帧 data(dashboard);无活会话返回 None。
    fn provide_frame(&self) -> Option<Value>;
    /// 连接态变化(托盘变色 / 屏复位)。
    fn on_state(&self, connected: bool, port: Option<&str>);
    /// 每成功发一帧 data 调一次(UI 记最近同步 + 清 init 闩锁)。
    fn on_data_sent(&self);
    /// 收到设备上行 btn(BOOT 切实例)。
    fn on_button(&self, action: &str);
}

/// 跨线程共享:停止/重连标志 + 即时下发队列 + 连接信息(供 host 查询)。
pub struct SerialShared {
    stop: AtomicBool,
    reconnect: AtomicBool,
    connected: AtomicBool,
    out_q: Mutex<VecDeque<Value>>,
    port: Mutex<Option<String>>,
    fw: Mutex<Option<String>>,
    data_interval: Mutex<f64>,
}

impl SerialShared {
    fn new() -> Self {
        SerialShared {
            stop: AtomicBool::new(false),
            reconnect: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            out_q: Mutex::new(VecDeque::new()),
            port: Mutex::new(None),
            fw: Mutex::new(None),
            data_interval: Mutex::new(DATA_INTERVAL),
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// 请求重连:断开当前口、重新发现(用户在 UI 点「重新连接」)。
    pub fn reconnect(&self) {
        self.reconnect.store(true, Ordering::SeqCst);
    }

    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn port(&self) -> Option<String> {
        self.port.lock().unwrap().clone()
    }

    pub fn fw(&self) -> Option<String> {
        self.fw.lock().unwrap().clone()
    }

    /// 运行中改下发间隔(配置即时生效)。
    pub fn set_data_interval(&self, seconds: f64) {
        if seconds.is_finite() {
            *self.data_interval.lock().unwrap() = seconds.max(1.0);
        }
    }

    /// 线程安全排入一帧即时下发(cfg 切屏 / text 文本);积压过多丢弃,避免未连时堆爆。
    pub fn send_frame(&self, d: Value) {
        let mut q = self.out_q.lock().unwrap();
        if q.len() > 64 {
            return;
        }
        q.push_back(d);
    }
}

pub struct SerialLink {
    shared: Arc<SerialShared>,
}

impl Default for SerialLink {
    fn default() -> Self {
        Self::new()
    }
}

impl SerialLink {
    pub fn new() -> Self {
        SerialLink {
            shared: Arc::new(SerialShared::new()),
        }
    }

    pub fn shared(&self) -> Arc<SerialShared> {
        self.shared.clone()
    }

    /// 启动 serial 线程(发现/握手/服务循环),返回 JoinHandle 供 host stop/join。
    pub fn start(&self, client: Arc<dyn SerialClient>) -> JoinHandle<()> {
        let shared = self.shared.clone();
        thread::Builder::new()
            .name("serial_link".into())
            .spawn(move || Runner::new(shared, client).run())
            .expect("spawn serial_link thread")
    }
}

/// payload.cc_running(供「变化即发」判断)。
fn frame_running(frame: &Value) -> Option<bool> {
    frame.get("payload")?.get("cc_running")?.as_bool()
}

fn frame_init(frame: &Value) -> bool {
    frame.get("init").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn is_espressif(info: &SerialPortInfo) -> bool {
    matches!(&info.port_type, SerialPortType::UsbPort(u) if u.vid == ESPRESSIF_VID)
}

/// 以 DTR=true 打开端口。该 S3 HWCDC 调试口在 DTR=false 打开时会被拉进
/// DOWNLOAD(USB/UART0),所以不能用 false 作为默认握手状态。
fn open_port(name: &str) -> serialport::Result<Box<dyn SerialPort>> {
    let mut sp = serialport::new(name, BAUD)
        .timeout(PORT_TIMEOUT)
        .dtr_on_open(true)
        .open()?;
    let _ = sp.write_data_terminal_ready(true);
    let _ = sp.write_request_to_send(false);
    Ok(sp)
}

/// 单个 303A 口的探测节流:宽限期(刚出现)内每轮探,过后慢重试,放手让别的 ESP32 设备能用串口。
struct Probe303a {
    first_seen: Instant,
    last_try: Instant,
}

/// 线程本地的发现/服务循环状态(sp 等不跨线程共享,只 Runner 自己持有)。
struct Runner {
    shared: Arc<SerialShared>,
    client: Arc<dyn SerialClient>,
    sp: Option<Box<dyn SerialPort>>,
    port_name: Option<String>,
    last_scan_log: Option<Instant>, // 节流「搜索中」日志 + 303A 握手失败日志(8s)
    tried_others: HashSet<String>,  // 已握手失败的非 303A 口:只在新出现时试一次
    probe_303a: HashMap<String, Probe303a>, // 每个 303A 口的探测节流(宽限期 + 慢重试)
    rxbuf: Vec<u8>,                 // 上行帧拼行缓冲(btn / pong)
}

impl Runner {
    fn new(shared: Arc<SerialShared>, client: Arc<dyn SerialClient>) -> Self {
        Runner {
            shared,
            client,
            sp: None,
            port_name: None,
            last_scan_log: None,
            tried_others: HashSet::new(),
            probe_303a: HashMap::new(),
            rxbuf: Vec::new(),
        }
    }

    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    fn close(&mut self) {
        self.sp = None; // Box<dyn SerialPort> drop → 关口
        self.port_name = None;
        *self.shared.port.lock().unwrap() = None;
        *self.shared.fw.lock().unwrap() = None;
        self.shared.connected.store(false, Ordering::SeqCst);
        self.client.on_state(false, None);
    }

    fn write_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.sp
            .as_mut()
            .expect("sp present in service")
            .write_all(bytes)
    }

    fn write_frame(&mut self, v: &Value) -> std::io::Result<()> {
        // serde_json::to_string 不转义非 ASCII(对齐 json.dumps(ensure_ascii=False));设备解析 JSON,空格无关
        let mut s = serde_json::to_string(v).unwrap_or_default();
        s.push('\n');
        self.write_raw(s.as_bytes())
    }

    /// 在已开端口上发 hello、等 id 窗口内匹配:命中→Ok(Some(fw)),超时→Ok(None),IO 错→Err。
    fn hello_probe(sp: &mut Box<dyn SerialPort>) -> std::io::Result<Option<Option<String>>> {
        let _ = sp.clear(ClearBuffer::Input);
        sp.write_all(HELLO)?;
        let deadline = Instant::now() + HANDSHAKE_WINDOW;
        let mut buf: Vec<u8> = Vec::new();
        let mut rb = [0u8; 256];
        while Instant::now() < deadline {
            match sp.read(&mut rb) {
                Ok(n) if n > 0 => buf.extend_from_slice(&rb[..n]),
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = &line[..line.len() - 1];
                if let Ok(obj) = serde_json::from_slice::<Value>(line) {
                    if obj.get("t").and_then(|v| v.as_str()) == Some("id")
                        && obj.get("name").and_then(|v| v.as_str()) == Some(DEVICE_NAME)
                    {
                        return Ok(Some(
                            obj.get("fw")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        ));
                    }
                }
            }
        }
        Ok(None)
    }

    /// 打开端口 → 发 hello → 匹配 id。返回 (ok, err_str)。成功则锁定该口。
    /// 两段握手(阶段10):先保持 DTR=true 直接 hello;不回则在 DTR=true 下 RTS 复位脉冲,
    /// 等 app 重启后再 hello。DTR=false 会让本板 HWCDC 进入下载模式,不能用于探测。
    fn try_handshake(&mut self, name: &str) -> (bool, Option<String>) {
        let mut sp = match open_port(name) {
            Ok(p) => p,
            Err(e) => return (false, Some(format!("open: {}", e))),
        };
        let mut matched_fw: Option<Option<String>> = None; // Some(fw) = 命中
        for reset in [false, true] {
            let _ = sp.write_data_terminal_ready(true);
            let _ = sp.write_request_to_send(false);
            if reset {
                let _ = sp.write_request_to_send(true);
                thread::sleep(Duration::from_millis(150));
                let _ = sp.write_request_to_send(false);
                thread::sleep(Duration::from_millis(900));
            }
            match Self::hello_probe(&mut sp) {
                Ok(Some(fw)) => {
                    matched_fw = Some(fw);
                    break;
                }
                Ok(None) => {} // 该段超时 → 试下一段(复位后再探)
                Err(e) => return (false, Some(format!("io: {}", e))),
            }
        }
        if let Some(fw) = matched_fw {
            *self.shared.fw.lock().unwrap() = fw;
            *self.shared.port.lock().unwrap() = Some(name.to_string());
            self.shared.connected.store(true, Ordering::SeqCst);
            self.sp = Some(sp);
            self.port_name = Some(name.to_string());
            self.client.on_state(true, Some(name));
            return (true, None);
        }
        (false, Some("no id response".into()))
    }

    /// 扫串口找设备:**优先且基本只扫 303A 口**(插上后 Windows 要 1-2s 才枚举出设备口,期间逐个试
    /// 无关口会白等;只扫 303A → 枚举间隙几乎零成本)。返回 saw_esp(是否看到 303A 口)。
    fn discover(&mut self) -> bool {
        let ports = match available_ports() {
            Ok(p) => p,
            Err(e) => {
                log::warn(format!("available_ports() failed: {}", e));
                return false;
            }
        };
        let esp: Vec<&SerialPortInfo> = ports.iter().filter(|p| is_espressif(p)).collect();
        let others: Vec<&SerialPortInfo> = ports.iter().filter(|p| !is_espressif(p)).collect();
        let verbose = self
            .last_scan_log
            .is_none_or(|t| t.elapsed().as_secs_f64() > 8.0);
        if verbose {
            self.last_scan_log = Some(Instant::now());
            let names: Vec<&str> = esp.iter().map(|p| p.port_name.as_str()).collect();
            log::info(format!(
                "discover: 303A={:?} others={}",
                names,
                others.len()
            ));
        }
        // 消失的 303A 口移出(拔掉/重插 → 重新宽限);本设备的 de-enum 重枚举也借此拿到新宽限。
        let now = Instant::now();
        let cur_esp: HashSet<String> = esp.iter().map(|p| p.port_name.clone()).collect();
        self.probe_303a.retain(|d, _| cur_esp.contains(d));
        // 探 303A 口:刚出现的口在宽限期内每轮都探(本设备无论先后插都能及时连上);过宽限期后
        // 每 SLOW_303A 才探一次,余下时间放手该口 → 别的 ESP32 设备能正常烧录/开串口。
        for p in &esp {
            if self.stopping() {
                return false;
            }
            let go = {
                let st = self
                    .probe_303a
                    .entry(p.port_name.clone())
                    .or_insert_with(|| Probe303a {
                        first_seen: now,
                        last_try: now,
                    });
                let go = now.duration_since(st.first_seen) < GRACE_303A
                    || now.duration_since(st.last_try) >= SLOW_303A;
                if go {
                    st.last_try = now;
                }
                go
            };
            if !go {
                continue; // 过宽限期、且没到慢重试点 → 放手该口
            }
            let (ok, err) = self.try_handshake(&p.port_name);
            if ok {
                self.probe_303a.remove(&p.port_name);
                return false;
            }
            if verbose {
                if let Some(e) = err {
                    log::info(format!("  303A {} 未握手: {}", p.port_name, e));
                }
            }
        }
        // 安全网:非 303A 口只在「新出现」时试一次(消失的口移出 → 再插回当新口重试)
        let cur_others: HashSet<String> = others.iter().map(|p| p.port_name.clone()).collect();
        self.tried_others.retain(|d| cur_others.contains(d));
        if esp.is_empty() {
            for p in &others {
                if self.stopping() {
                    return false;
                }
                if self.tried_others.contains(&p.port_name) {
                    continue;
                }
                self.tried_others.insert(p.port_name.clone());
                if self.try_handshake(&p.port_name).0 {
                    return false;
                }
            }
        }
        !esp.is_empty()
    }

    /// 已连接:按节奏发 data + ping;非阻塞排空 RX;异常或死链即断开返回。
    fn service(&mut self) {
        let mut last_data: Option<Instant> = None; // None → 进入即立刻发首帧
        let mut last_ping = Instant::now();
        let mut last_rx = Instant::now();
        let mut last_probe: Option<Instant> = None;
        let mut last_running: Option<bool> = None;
        self.shared.reconnect.store(false, Ordering::SeqCst);

        while !self.stopping() && self.sp.is_some() {
            if self.shared.reconnect.swap(false, Ordering::SeqCst) {
                log::info("link dropped: 用户请求重连");
                self.close();
                return;
            }
            let now = Instant::now();
            let di = *self.shared.data_interval.lock().unwrap();
            let due = last_data.is_none_or(|t| (now - t).as_secs_f64() >= di);
            // 每 ~0.4s 探一次工作态;翻转(running 变化)或 init 即时下发 → LED 在提交/思考时就亮
            let probe = last_probe.is_none_or(|t| (now - t).as_secs_f64() >= 0.4);
            let mut send = due;
            let mut frame: Option<Value> = None;
            if due || probe {
                frame = self.client.provide_frame();
                last_probe = Some(now);
                if let Some(f) = &frame {
                    if let Some(r) = frame_running(f) {
                        if Some(r) != last_running {
                            send = true;
                        }
                    }
                    if frame_init(f) {
                        send = true;
                    }
                }
            }
            // 先发外部即时帧(cfg 切屏 / text 文本),再发 data → 连上瞬间 cfg(text) 先到,
            // 设备直接切文本屏不闪仪表盘。帧间小停顿:防突发多帧冲爆设备 RX(HWCDC 满时静默丢字节)。
            loop {
                let extra = self.shared.out_q.lock().unwrap().pop_front();
                let Some(extra) = extra else { break };
                if let Err(e) = self.write_frame(&extra) {
                    log::info(format!("link dropped: IO 错误 {}", e));
                    self.close();
                    return;
                }
                thread::sleep(Duration::from_millis(15));
            }
            if send {
                if let Some(f) = frame.clone() {
                    if let Err(e) = self.write_frame(&f) {
                        log::info(format!("link dropped: IO 错误 {}", e));
                        self.close();
                        return;
                    }
                    if let Some(r) = frame_running(&f) {
                        last_running = Some(r);
                    }
                    self.client.on_data_sent();
                }
                last_data = Some(now);
            }
            if (now - last_ping).as_secs_f64() >= PING_INTERVAL {
                if let Err(e) = self.write_raw(PING) {
                    log::info(format!("link dropped: IO 错误 {}", e));
                    self.close();
                    return;
                }
                last_ping = now;
            }
            // 排空 RX 并解析上行帧(btn 切实例 / pong)
            let avail = self
                .sp
                .as_mut()
                .expect("sp present")
                .bytes_to_read()
                .unwrap_or(0);
            if avail > 0 {
                let mut tmp = vec![0u8; avail as usize];
                match self.sp.as_mut().expect("sp present").read(&mut tmp) {
                    Ok(n) => {
                        self.rxbuf.extend_from_slice(&tmp[..n]);
                        last_rx = now;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => {
                        log::info(format!("link dropped: IO 错误 {}", e));
                        self.close();
                        return;
                    }
                }
                if let Err(e) = self.drain_rx(&mut last_data, &mut last_running) {
                    log::info(format!("link dropped: IO 错误 {}", e));
                    self.close();
                    return;
                }
                if self.rxbuf.len() > 4096 {
                    self.rxbuf.clear(); // 异常堆积保护(无换行的垃圾)
                }
            }
            if (now - last_rx).as_secs_f64() > LIVENESS_TIMEOUT {
                log::info(format!(
                    "link dropped: 死链超时({:.0}s 无 RX)",
                    LIVENESS_TIMEOUT
                ));
                self.close();
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// 逐行解析 rxbuf:遇 btn → on_button(同步切实例)+ 即时下发新实例帧(init,仪表盘秒切)。
    fn drain_rx(
        &mut self,
        last_data: &mut Option<Instant>,
        last_running: &mut Option<bool>,
    ) -> std::io::Result<()> {
        while let Some(pos) = self.rxbuf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.rxbuf.drain(..=pos).collect();
            let line = &line[..line.len() - 1];
            if line.iter().all(|b| b.is_ascii_whitespace()) {
                continue;
            }
            let obj: Value = match serde_json::from_slice(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if obj.get("t").and_then(|v| v.as_str()) != Some("btn") {
                continue; // pong 等其它上行帧:已刷新 last_rx,无需额外处理
            }
            let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("next");
            self.client.on_button(action);
            // on_button 已**同步** cycle;provide_frame 同时让 apply_screen 把切屏 cfg 入队。
            let nf = self.client.provide_frame();
            // ⚠️ 先发 cfg(切屏)再发 data:BOOT 跨屏(CC↔system)时设备先切到目标布局,再收 init
            //    data → 当前布局==目标 → 立即揭黑遮罩(否则要等 2s 失败保护超时,切屏发慢)。
            loop {
                let extra = self.shared.out_q.lock().unwrap().pop_front();
                let Some(extra) = extra else { break };
                self.write_frame(&extra)?;
                thread::sleep(Duration::from_millis(15));
            }
            if let Some(nf) = nf {
                self.write_frame(&nf)?;
                *last_data = Some(Instant::now());
                if let Some(r) = frame_running(&nf) {
                    *last_running = Some(r);
                }
                self.client.on_data_sent();
            }
        }
        Ok(())
    }

    fn run(&mut self) {
        // 发现/服务循环:IO 错误已逐处理(Result),拔插 churn 不会让线程死
        // (Rust 无 Python 那种 comports()/close() 抛异常;available_ports 返 Result、close 仅 drop)
        while !self.stopping() {
            if self.sp.is_none() {
                let saw_esp = self.discover();
                if self.sp.is_none() {
                    // 303A 在场(多半在 boot)→ 0.3s 快重试;否则 0.6s 轻量轮询枚举
                    thread::sleep(Duration::from_millis(if saw_esp { 300 } else { 600 }));
                    continue;
                }
            }
            self.service();
        }
    }
}
