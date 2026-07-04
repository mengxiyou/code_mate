//! 阶段12:本机网络信息 —— 局域网 IP(离线,UdpSocket 路由探测)+ 公网 IP(在线,多源 HTTP)。
//! 公网 IP 在后台线程抓取并缓存:**抓到后半小时刷新,没抓到(离线/VPN 未就绪/限流)则 20s 快速重试**
//! (修复:原先首次失败要等 30 分钟,VPN 开机时序下会长时间退回显示本地 IP)。多源兜底:
//! ip-api.com(JSON)→ icanhazip.com / ifconfig.me(纯文本),任一成功即可,全 HTTP、无 TLS 依赖。
//! 全用 std::net(UdpSocket/TcpStream + serde_json),无新依赖、无 FFI。
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

static PUB_IP: Mutex<String> = Mutex::new(String::new());
static STARTED: AtomicBool = AtomicBool::new(false);

const REFRESH_OK: Duration = Duration::from_secs(1800); // 抓到后:公网 IP 很少变,半小时刷新
const RETRY_FAIL: Duration = Duration::from_secs(20); // 没抓到:快速重试(VPN 未就绪/离线/限流)
const HTTP_TIMEOUT: Duration = Duration::from_secs(6);

/// 局域网 IP(离线):连一个 UDP「目标」(不发包,只让 OS 按路由选出本地源 IP)。无网返回空串。
/// ⚠️ 开了 VPN/Tailscale 时这里返回的是隧道接口地址(如 100.64.x);仅作公网抓取失败时的兜底显示。
pub fn local_ip() -> String {
    let pick = || -> Option<String> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect("8.8.8.8:80").ok()?; // UDP connect 不发包,仅决定出站接口
        Some(s.local_addr().ok()?.ip().to_string())
    };
    pick().unwrap_or_default()
}

/// 公网 IP(缓存;空 = 还没抓到 / 离线)。
pub fn public_ip() -> String {
    PUB_IP.lock().unwrap().clone()
}

/// 启动后台抓取线程(host 启动时调;幂等):多源查公网 IP,缓存;抓到 30min 刷、没抓到 20s 重试。
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return; // 已启动
    }
    std::thread::Builder::new()
        .name("netinfo".into())
        .spawn(|| loop {
            let got = fetch_public_ip();
            let ok = got.is_some();
            if let Some(ip) = got {
                *PUB_IP.lock().unwrap() = ip;
            }
            std::thread::sleep(if ok { REFRESH_OK } else { RETRY_FAIL });
        })
        .ok();
}

/// 多源兜底抓公网 IP(任一成功即返回):ip-api.com(JSON)→ icanhazip.com / ifconfig.me(纯文本)。
fn fetch_public_ip() -> Option<String> {
    http_get("ip-api.com", "/json/?fields=status,query")
        .and_then(|b| parse_ipapi(&b))
        .or_else(|| http_get("icanhazip.com", "/").and_then(|b| parse_plain_ip(&b)))
        .or_else(|| http_get("ifconfig.me", "/ip").and_then(|b| parse_plain_ip(&b)))
}

/// 裸 HTTP/1.0 GET(Connection: close)→ 响应 body(去头);超时/离线/无 body → None。
fn http_get(host: &str, path: &str) -> Option<String> {
    let addr = format!("{host}:80").to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT).ok()?;
    let _ = s.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = s.set_write_timeout(Some(HTTP_TIMEOUT));
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: code_mate/1.0\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).ok()?;
    resp.split_once("\r\n\r\n").map(|(_, body)| body.to_string())
}

/// ip-api.com JSON:{"status":"success","query":"<ip>"}。
fn parse_ipapi(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if v.get("status").and_then(|x| x.as_str()) != Some("success") {
        return None;
    }
    valid_ip(v.get("query").and_then(|x| x.as_str())?)
}

/// 纯文本 IP 服务(icanhazip / ifconfig.me):trim 后校验是合法 IP(挡错误页/HTML)。
fn parse_plain_ip(body: &str) -> Option<String> {
    valid_ip(body.trim())
}

/// 校验是合法 IPv4/IPv6 字面量,合法则原样返回。
fn valid_ip(s: &str) -> Option<String> {
    s.parse::<IpAddr>().ok().map(|_| s.to_string())
}
