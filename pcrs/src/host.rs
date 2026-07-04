//! host — 后台控制器(对应 pc/host.py)。封装 SerialLink + cc_source + 选择器 + 事件总线 + 日志。
//!
//! - start()/stop()/reconnect();连接态监听(托盘据此变色);get_status()(配置界面快照)。
//! - 屏状态机 _apply_screen:loading(无活会话) / terminal(有会话+合盖) / dashboard(有会话+开盖)。
//! - 文本流 worker:合盖期间跟随 pinned 会话末尾内容,clear 重画(出场恒定行数)。
//! - 事件总线:盒盖 LID_* / BOOT BUTTON_NEXT → 重算屏 / 切实例。
use crate::events::{Event, EventBus, EventSource, EventType};
use crate::instance_select::InstanceSelector;
use crate::lid_watch::LidEventSource;
use crate::serial_link::{SerialClient, SerialLink, SerialShared};
#[cfg(windows)]
use crate::sysmon::SysMonitor;
use crate::{cc_source, config, datasource, log, sessions, transcript, util};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

// 终端任何场景都只显示末尾约这么多「设备行」(换行后计):填满首屏7行 + 再滚出7行(7+7)
const TERM_DEVICE_LINES: i64 = 14;

// ---------- 唤醒事件(对齐 Python threading.Event:set/clear/wait(timeout))----------
struct WakeEvent {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl WakeEvent {
    fn new() -> Self {
        WakeEvent {
            flag: Mutex::new(false),
            cv: Condvar::new(),
        }
    }
    fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }
    fn clear(&self) {
        *self.flag.lock().unwrap() = false;
    }
    /// 已 set 立即返回;否则阻塞至 set 或超时(对齐 Event.wait(timeout))。
    fn wait_timeout(&self, dur: Duration) {
        let g = self.flag.lock().unwrap();
        if *g {
            return;
        }
        let _ = self.cv.wait_timeout(g, dur);
    }
}

// ---------- 文本流 worker 控制 ----------
#[derive(Default)]
struct TextCtl {
    stop: Option<Arc<AtomicBool>>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ConnState {
    connected: bool,
    port: Option<String>,
}

// 仅 Send(不需 Sync):调用恒在 listeners 锁内串行,且 EventLoopProxy(阶段6 托盘变色)只 Send
type Listener = Box<dyn Fn(bool, Option<String>) + Send>;

/// 跨线程共享状态 + SerialClient 实现(serial 线程 / 事件线程 / 文本 worker 都经它)。
pub struct HostShared {
    serial: Arc<SerialShared>,
    selector: Arc<InstanceSelector>,
    bus: Arc<EventBus>,
    conn: Mutex<ConnState>,
    last_send_ts: Mutex<Option<i64>>,
    listeners: Mutex<Vec<Listener>>,
    lid_closed: AtomicBool,
    current_is_system: AtomicBool, // 当前选中=system 伪实例(=显示 system 屏);否则显示 CC 实例
    current_is_loading: AtomicBool, // 系统屏关闭且无 CC → 等待屏(loading),无 data 帧
    #[cfg(windows)]
    sysmon: Mutex<SysMonitor>, // 系统监控采样器(仅 serial 线程经此采,Mutex 仅为内部可变)
    #[cfg(windows)]
    sysmon_ui: Mutex<SysMonitor>, // UI 专用采样器(get_sysmon;与上者分开,避免污染 CPU% 差分)
    screen: Mutex<Option<String>>, // 当前已下发的目标布局
    text_lock: Mutex<()>,          // 串行化 apply_screen + 起停文本流
    text_ctl: Mutex<TextCtl>,
    text_wake: Arc<WakeEvent>,
}

impl HostShared {
    fn connected(&self) -> bool {
        self.conn.lock().unwrap().connected
    }

    /// 主流程统一驱动屏:loading / terminal / dashboard;变化才发 cfg + 起停文本流。
    fn apply_screen(&self) {
        let _g = self.text_lock.lock().unwrap();
        if !self.connected() {
            return; // 未连不发 cfg(连上后由 provide_frame 按真实态驱动)
        }
        let target = if self.current_is_loading.load(Ordering::SeqCst) {
            "loading" // 系统屏关闭 + 无 CC 会话 → 等待屏
        } else if self.current_is_system.load(Ordering::SeqCst) {
            "system" // 无 CC 会话 / BOOT 选中 system(取代旧 loading「No Session Available」)
        } else if self.lid_closed.load(Ordering::SeqCst) {
            "terminal"
        } else {
            "dashboard"
        };
        {
            let mut scr = self.screen.lock().unwrap();
            if scr.as_deref() == Some(target) {
                return;
            }
            *scr = Some(target.to_string());
        }
        match target {
            "loading" => {
                self.serial.set_data_interval(3.0);
                self.serial
                    .send_frame(json!({"t":"cfg","screen":"loading","msg":"No Session Available"}));
                self.stop_text_stream();
            }
            "system" => {
                self.serial.set_data_interval(1.0); // 系统指标 + 磁盘活动:1s 刷新更跟手
                self.serial.send_frame(json!({"t":"cfg","screen":"system"}));
                self.stop_text_stream();
            }
            "terminal" => {
                self.serial.set_data_interval(3.0);
                self.serial
                    .send_frame(json!({"t":"cfg","screen":"terminal"}));
                self.start_text_stream();
            }
            _ => {
                self.serial.set_data_interval(3.0);
                self.serial
                    .send_frame(json!({"t":"cfg","screen":"dashboard"}));
                self.stop_text_stream();
            }
        }
        log::info(format!("screen → {}", target));
    }

    /// 采一帧系统监控数据(Windows;非 Windows 回最小空 system 帧,字段缺失设备显 --)。
    #[cfg(windows)]
    fn build_system_frame_now(
        &self,
        instances: &[Map<String, Value>],
        init: bool,
        now: i64,
    ) -> Map<String, Value> {
        crate::sys_source::build_system_frame(
            &mut self.sysmon.lock().unwrap(),
            instances,
            init,
            now,
        )
    }
    #[cfg(not(windows))]
    fn build_system_frame_now(
        &self,
        _instances: &[Map<String, Value>],
        init: bool,
        now: i64,
    ) -> Map<String, Value> {
        let mut f = Map::new();
        f.insert("t".into(), Value::from("data"));
        f.insert("screen".into(), Value::from("system"));
        f.insert("ts".into(), Value::from(now));
        f.insert("fresh".into(), Value::Bool(true));
        f.insert("stale_sec".into(), Value::from(0));
        f.insert("init".into(), Value::Bool(init));
        f.insert("payload".into(), Value::Object(Map::new()));
        f
    }

    /// 事件总线订阅(逻辑事件 → 动作)。
    fn on_event(&self, ev: &Event) {
        match ev.kind {
            EventType::LidClosed => self.on_lid(true),
            EventType::LidOpened => self.on_lid(false),
            EventType::ButtonNext => {
                // layout-agnostic:只重绑数据源实例 + 唤醒文本 worker
                self.selector.cycle(&datasource::list_instances());
                self.text_wake.set();
            }
        }
    }

    fn on_lid(&self, closed: bool) {
        self.lid_closed.store(closed, Ordering::SeqCst);
        self.apply_screen();
    }

    /// 起文本流 worker(调用方须持 text_lock)。
    fn start_text_stream(&self) {
        self.stop_text_stream();
        let stop = Arc::new(AtomicBool::new(false));
        let selector = self.selector.clone();
        let serial = self.serial.clone();
        let wake = self.text_wake.clone();
        let stop_c = stop.clone();
        let handle = std::thread::Builder::new()
            .name("text_stream".into())
            .spawn(move || text_stream_worker(selector, serial, wake, stop_c))
            .ok();
        let mut ctl = self.text_ctl.lock().unwrap();
        ctl.stop = Some(stop);
        ctl.handle = handle;
    }

    /// 停文本流 worker(调用方须持 text_lock):置 stop + 唤醒,丢引用(线程自退,不 join)。
    fn stop_text_stream(&self) {
        let mut ctl = self.text_ctl.lock().unwrap();
        if let Some(stop) = ctl.stop.take() {
            stop.store(true, Ordering::SeqCst);
            self.text_wake.set(); // 唤醒 worker 尽快看到 stop
        }
        ctl.handle = None;
    }

    // ---- 供配置界面(阶段6)经 Arc<HostShared> 调用 ----

    /// 配置界面状态快照:连接 + 固件 + 快照新鲜度 + 模型/模式/会话。
    pub fn status_json(&self) -> Value {
        let (connected, port) = {
            let c = self.conn.lock().unwrap();
            (c.connected, c.port.clone())
        };
        let last = *self.last_send_ts.lock().unwrap();
        let now = util::now_secs();
        let frame = cc_source::build_data_frame(&self.selector, now);
        let (present, fresh, age, model, mode, session) = match &frame {
            Some(f) => {
                let p = f.get("payload");
                (
                    true,
                    f.get("fresh").cloned().unwrap_or(Value::Null),
                    f.get("stale_sec").cloned().unwrap_or(Value::Null),
                    p.and_then(|p| p.get("model"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    p.and_then(|p| p.get("mode"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    p.and_then(|p| p.get("session"))
                        .cloned()
                        .unwrap_or(Value::Null),
                )
            }
            None => (
                false,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
            ),
        };
        json!({
            "connected": connected,
            "port": port,
            "fw": self.serial.fw(),
            "last_send_ts": last,
            "last_send_age": last.map(|l| now - l),
            "snapshot_present": present,
            "snapshot_fresh": fresh,
            "snapshot_age": age,
            "model": model,
            "mode": mode,
            "session": session,
        })
    }

    /// 切 auto/manual:存配置 + 即时应用到选择器。
    pub fn set_instance_mode_persist(&self, mode: &str) {
        let mut cfg = config::load();
        if mode == "auto" || mode == "manual" {
            cfg.instance_mode = mode.to_string();
        }
        config::save(&cfg);
        self.selector.set_mode(&cfg.instance_mode);
        log::info(format!("instance_mode = {}", cfg.instance_mode));
    }

    pub fn instance_mode(&self) -> String {
        config::load().instance_mode
    }

    pub fn request_reconnect(&self) {
        log::info("reconnect requested");
        self.serial.reconnect();
    }

    /// 配置界面「STATUS」页:当前打开的 Agent 实例列表 + Claude 账号级公共 5h/周(只读)。
    pub fn instances_json(&self) -> Value {
        let now = util::now_secs();
        let instances = datasource::list_instances();
        let list: Vec<Value> = instances
            .iter()
            .map(|inst| {
                let mut p = datasource::dashboard_payload(inst, now); // model/mode/context/cc_running/session/provider
                if let Some(sid) = inst.get("session_id") {
                    p.insert("session_id".to_string(), sid.clone());
                }
                if let Some(at) = inst.get("activity_at") {
                    p.insert("activity_at".to_string(), at.clone());
                }
                Value::Object(p)
            })
            .collect();
        // 账号级公共数据(5h/周):取限流最新一份,STATUS 顶部 ACCOUNT 显示一次
        let shared = datasource::shared_payload(&instances);
        json!({ "ok": true, "instances": list, "shared": Value::Object(shared) })
    }

    /// 配置界面「状态」页:PC 系统监控只读快照(UI 专用采样器,不污染设备侧 CPU% 差分)。
    #[cfg(windows)]
    pub fn sysmon_json(&self) -> Value {
        let s = self.sysmon_ui.lock().unwrap().sample();
        let net = {
            let p = crate::netinfo::public_ip();
            if p.is_empty() {
                crate::netinfo::local_ip()
            } else {
                p
            }
        };
        let r1 = |x: f64| (x * 10.0).round() / 10.0;
        json!({
            "ok": true,
            "cpu_pct": r1(s.cpu_pct),
            "cpu_temp": s.cpu_temp,
            "cpu_ghz": r1(s.cpu_ghz),
            "ram_used_mb": s.ram_used_mb, "ram_total_mb": s.ram_total_mb, "ram_pct": r1(s.ram_pct),
            "vram_ok": s.vram_ok, "vram_used_mb": s.vram_used_mb,
            "vram_total_mb": s.vram_total_mb, "vram_pct": r1(s.vram_pct),
            "disk_bps": s.disk_bps,
            "host": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string()),
            "net": net,
            "provider": "System",
            "source": "System",
            "brand": "System",
            "theme": crate::ui_theme::system(),
        })
    }
    #[cfg(not(windows))]
    pub fn sysmon_json(&self) -> Value {
        json!({ "ok": false })
    }
}

// HostShared 即 SerialClient:serial 线程经它拉帧 / 回报状态。
impl SerialClient for HostShared {
    fn provide_frame(&self) -> Option<Value> {
        let now = util::now_secs();
        cc_source::maybe_prune(now);
        let instances = datasource::list_instances();
        // resolve:Some(CC 实例)→ dashboard 帧;None(选中 system / 无 CC)→ system 帧
        let (sess, init) = self.selector.resolve(&instances);
        let out = match sess {
            Some(cc) => {
                self.current_is_system.store(false, Ordering::SeqCst);
                self.current_is_loading.store(false, Ordering::SeqCst);
                Some(cc_source::build_dashboard_frame(&cc, &instances, init, now))
            }
            None if config::system_screen() => {
                self.current_is_system.store(true, Ordering::SeqCst);
                self.current_is_loading.store(false, Ordering::SeqCst);
                Some(self.build_system_frame_now(&instances, init, now))
            }
            None => {
                // 系统屏关闭 + 无 CC → 等待屏(loading):不发 data,cfg 由 apply_screen 入队
                self.current_is_system.store(false, Ordering::SeqCst);
                self.current_is_loading.store(true, Ordering::SeqCst);
                None
            }
        };
        self.apply_screen();
        out.map(Value::Object)
    }

    fn on_state(&self, connected: bool, port: Option<&str>) {
        {
            let mut c = self.conn.lock().unwrap();
            c.connected = connected;
            c.port = port.map(|s| s.to_string());
        }
        if connected {
            let p = port.unwrap_or("");
            let fws = self
                .serial
                .fw()
                .map(|f| format!(" fw={}", f))
                .unwrap_or_default();
            log::info(format!("device connected {}{}", p, fws));
        } else {
            log::info("device disconnected");
        }
        // 设备(重)连上后默认停在 loading:只清 _screen,让随后的 provide_frame 按真实会话态发 cfg
        if connected {
            *self.screen.lock().unwrap() = None;
        }
        let port_s = port.map(|s| s.to_string());
        for fn_ in self.listeners.lock().unwrap().iter() {
            fn_(connected, port_s.clone());
        }
    }

    fn on_data_sent(&self) {
        *self.last_send_ts.lock().unwrap() = Some(util::now_secs());
        self.selector.mark_sent(); // 帧真正发出 → 清 init 闩锁
    }

    fn on_button(&self, action: &str) {
        self.bus
            .publish(Event::new(EventType::ButtonNext, Some(action.to_string())));
    }
}

// ---------- 文本流 worker(自由函数,只捕获所需 Arc,不持整个 HostShared)----------
fn get_str<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(|v| v.as_str())
}

fn get_i64(m: &Map<String, Value>, k: &str) -> i64 {
    m.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
}

/// 选会话 → (transcript_path, session_id);无则 (None, None)。优先跟随 pinned 实例,
/// pinned 无转录则回退「running 优先、其次最近活跃」。
fn active_transcript(selector: &InstanceSelector) -> (Option<String>, Option<String>) {
    let instances = datasource::list_instances();
    if let Some(pinned) = selector.pinned() {
        for u in &instances {
            if get_str(u, "session_id") == Some(pinned.as_str()) {
                let raw = get_str(u, "provider_sid").unwrap_or("");
                let run = sessions::load_run(&pinned)
                    .or_else(|| {
                        if raw.is_empty() {
                            None
                        } else {
                            sessions::load_run(raw)
                        }
                    })
                    .unwrap_or_default();
                let tp = get_str(u, "transcript_path")
                    .or_else(|| get_str(&run, "transcript_path"))
                    .map(|s| s.to_string());
                if tp.is_some() {
                    return (tp, Some(pinned));
                }
                break;
            }
        }
    }
    let mut best_key: Option<(i64, i64)> = None;
    let mut best: Option<(Option<String>, Option<String>)> = None; // (sid, tp)
    for u in &instances {
        let sid = get_str(u, "session_id").map(|s| s.to_string());
        let sid_s = sid.as_deref().unwrap_or("");
        let raw = get_str(u, "provider_sid").unwrap_or("");
        let run = sessions::load_run(sid_s)
            .or_else(|| {
                if raw.is_empty() {
                    None
                } else {
                    sessions::load_run(raw)
                }
            })
            .unwrap_or_default();
        let at = get_i64(u, "activity_at")
            .max(get_i64(&run, "activity_at"))
            .max(get_i64(&run, "running_at"));
        let running = run
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let key = (if running { 1 } else { 0 }, at);
        if best_key.is_none_or(|bk| key > bk) {
            best_key = Some(key);
            let tp = get_str(u, "transcript_path")
                .or_else(|| get_str(&run, "transcript_path"))
                .map(|s| s.to_string());
            best = Some((sid, tp));
        }
    }
    match best {
        Some((sid, tp)) => (tp, sid),
        None => (None, None),
    }
}

fn wrap_text(payload: Value) -> Value {
    json!({"t":"data","screen":"terminal","ts": util::now_secs(), "payload": payload})
}

fn send_placeholder(serial: &SerialShared, msg: &str) {
    serial.send_frame(wrap_text(json!({"runs":[{"s":"h","t":msg}],"clear":true})));
}

/// 读跟随会话最近正文 → 末尾约 TERM_DEVICE_LINES 个设备行;无正文返回 None。
fn compute_tail(tp: &str) -> Option<String> {
    let (blocks, _) = transcript::read_assistant_texts(tp, None, Some(3));
    let joined = blocks
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = transcript::collapse_blanks(joined.trim_end_matches('\n'));
    if text.is_empty() {
        return None;
    }
    Some(transcript::tail_by_device_lines(&text, TERM_DEVICE_LINES))
}

fn send_tail(tail: &str, serial: &SerialShared, stop: &AtomicBool) {
    for fr in transcript::text_to_frames(tail, true) {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        serial.send_frame(wrap_text(fr.to_payload()));
    }
}

fn text_stream_worker(
    selector: Arc<InstanceSelector>,
    serial: Arc<SerialShared>,
    wake: Arc<WakeEvent>,
    stop: Arc<AtomicBool>,
) {
    // 等转录路径(CC 可能还没触发过钩子)
    let mut waited = false;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let (tp, _) = active_transcript(&selector);
        if tp.is_some() {
            break;
        }
        if !waited {
            send_placeholder(&serial, "等待 Agent 回复...\n");
            waited = true;
        }
        wake.wait_timeout(Duration::from_secs(1));
        wake.clear();
    }
    if stop.load(Ordering::SeqCst) {
        return;
    }

    // 跟随当前 pinned 会话:会话变 / 末尾内容变即 clear 重画(设备从底部逐行滚入)
    let mut last_key: Option<(Option<String>, Option<String>)> = None; // (sid, tail)
    let mut last_mtime: Option<SystemTime> = None;
    while !stop.load(Ordering::SeqCst) {
        let (tp, sid) = active_transcript(&selector);
        if let Some(tp) = tp.as_deref() {
            let mtime = std::fs::metadata(tp).and_then(|m| m.modified()).ok();
            // 会话没变且文件没动 → 跳过(空闲不重读不重画)
            let skip = last_key.as_ref().is_some_and(|lk| lk.0 == sid) && mtime == last_mtime;
            if !skip {
                let tail = compute_tail(tp);
                let key = (sid.clone(), tail.clone());
                if last_key.as_ref() != Some(&key) {
                    match &tail {
                        None => send_placeholder(&serial, "(暂无回复)\n"),
                        Some(t) => send_tail(t, &serial, &stop),
                    }
                    last_key = Some(key);
                }
                last_mtime = mtime;
            }
        }
        wake.wait_timeout(Duration::from_millis(1500));
        wake.clear();
    }
}

// ---------- Windows 退出省电限速 ----------
#[cfg(windows)]
fn disable_power_throttling() {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
        PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE,
    };
    let st = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0, // 0 = 关闭限速(全速)
    };
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &st as *const _ as *const c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn disable_power_throttling() {}

// ---------- Host 句柄 ----------
pub struct Host {
    shared: Arc<HostShared>,
    link: SerialLink,
    lid: LidEventSource,
    serial_join: Option<JoinHandle<()>>,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    pub fn new() -> Self {
        let cfg = config::load();
        let link = SerialLink::new();
        let serial = link.shared();
        let selector = Arc::new(InstanceSelector::new(&cfg.instance_mode));
        let bus = Arc::new(EventBus::new());
        let shared = Arc::new(HostShared {
            serial,
            selector,
            bus: bus.clone(),
            conn: Mutex::new(ConnState::default()),
            last_send_ts: Mutex::new(None),
            listeners: Mutex::new(Vec::new()),
            lid_closed: AtomicBool::new(false),
            current_is_system: AtomicBool::new(true), // 首帧前默认 system(无会话)
            current_is_loading: AtomicBool::new(false),
            #[cfg(windows)]
            sysmon: Mutex::new(SysMonitor::new()),
            #[cfg(windows)]
            sysmon_ui: Mutex::new(SysMonitor::new()),
            screen: Mutex::new(None),
            text_lock: Mutex::new(()),
            text_ctl: Mutex::new(TextCtl::default()),
            text_wake: Arc::new(WakeEvent::new()),
        });
        // 订阅事件总线:LID_* / BUTTON_NEXT → host 动作
        {
            let hs = shared.clone();
            bus.subscribe(Arc::new(move |ev: &Event| hs.on_event(ev)));
        }
        let lid = LidEventSource::new(bus.clone());
        Host {
            shared,
            link,
            lid,
            serial_join: None,
        }
    }

    pub fn start(&mut self) {
        disable_power_throttling(); // 合盖低功耗下也保持全速:发现/下发不被降频
        let join = self.link.start(self.shared.clone());
        self.serial_join = Some(join);
        self.lid.start();
        crate::netinfo::start(); // 后台抓公网归属(system 屏左下显示 IP + 归属)
        log::info("host started (变化推送 + 3s 心跳;盒盖→文本屏)");
    }

    pub fn stop(&mut self) {
        log::info("host stopping");
        {
            let _g = self.shared.text_lock.lock().unwrap();
            self.shared.stop_text_stream();
        }
        self.lid.stop();
        self.shared.serial.stop();
        if let Some(j) = self.serial_join.take() {
            let _ = j.join();
        }
    }

    pub fn reconnect(&self) {
        self.shared.request_reconnect();
    }

    /// 供配置界面(阶段6)拿共享态,经它调 status_json/set_instance_mode_persist/request_reconnect。
    pub fn shared(&self) -> Arc<HostShared> {
        self.shared.clone()
    }

    /// 注册连接状态监听;注册时立即推一次当前状态。
    pub fn add_listener(&self, fn_: Listener) {
        let (connected, port) = {
            let c = self.shared.conn.lock().unwrap();
            (c.connected, c.port.clone())
        };
        fn_(connected, port);
        self.shared.listeners.lock().unwrap().push(fn_);
    }

    /// 配置界面切 auto/manual:存配置 + 即时应用到选择器。
    pub fn set_instance_mode(&self, mode: &str) {
        self.shared.set_instance_mode_persist(mode);
    }

    pub fn connected(&self) -> bool {
        self.shared.connected()
    }

    /// 给配置界面的状态快照(委托 HostShared::status_json)。
    pub fn get_status(&self) -> Value {
        self.shared.status_json()
    }
}
