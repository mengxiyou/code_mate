//! 组 system 屏 data 帧(对位 cc_source.rs)。CPU/RAM/VRAM/磁盘活动 + 主机名。
//! 仅 Windows(系统指标走 windows-sys 裸 API);磁盘 bytes/s 归一到 0..255 供设备 LED。
use crate::config;
use crate::instance_select::{unified_ids, SYSTEM_SID};
use crate::sysmon::SysMonitor;
use crate::ui_theme;
use crate::util;
use serde_json::{json, Map, Value};

/// 一位小数,避免设备端抖动(与百分比显示一致)。
fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// 磁盘 bytes/s → 0..255 LED 级别:空闲阈值以下=0,log 缩放到上限封顶。
fn disk_level(bps: f64) -> u8 {
    let floor = 256.0 * 1024.0; // ≤256KB/s 视为空闲
    if bps <= floor {
        return 0;
    }
    let cap = 200.0 * 1024.0 * 1024.0; // ~200MB/s 封顶
    let x = (bps.min(cap) / floor).ln() / (cap / floor).ln(); // 0..1
    (x * 255.0).round().clamp(0.0, 255.0) as u8
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string())
}

/// 采一帧系统数据 → screen=system 的 data 帧。系统数据本地实时采,恒新鲜。
/// cc_instances:用于算统一会话指示(system 占最后一位的 idx/cnt + 身份色 dot)。
pub fn build_system_frame(
    mon: &mut SysMonitor,
    cc_instances: &[Map<String, Value>],
    init: bool,
    now: i64,
) -> Map<String, Value> {
    let s = mon.sample();

    let mut payload = Map::new();
    payload.insert("provider".into(), Value::from("System"));
    payload.insert("source".into(), Value::from("System"));
    payload.insert("brand".into(), Value::from("System"));
    payload.insert("theme".into(), ui_theme::system());
    payload.insert("cpu".into(), json!({ "used_pct": r1(s.cpu_pct) }));
    // CPU 副读数:体感助手(WinRing0)在跑 → 温度;否则 → 当前频率(GHz)
    let cpu_sub = match s.cpu_temp {
        Some(t) if config::temp_fahrenheit() => format!("{}°F", (t * 9.0 / 5.0 + 32.0).round() as i64),
        Some(t) => format!("{}°C", t.round() as i64),
        None if s.cpu_ghz > 0.0 => format!("{:.1} GHz", s.cpu_ghz),
        None => String::new(),
    };
    if !cpu_sub.is_empty() {
        payload.insert("cpu_sub".into(), Value::from(cpu_sub));
    }
    payload.insert(
        "ram".into(),
        json!({ "used_pct": r1(s.ram_pct), "used_mb": s.ram_used_mb, "total_mb": s.ram_total_mb }),
    );
    if s.vram_ok {
        payload.insert(
            "vram".into(),
            json!({ "used_pct": r1(s.vram_pct), "used_mb": s.vram_used_mb, "total_mb": s.vram_total_mb }),
        );
    } // 缺失整体省略 → 设备显 --
    payload.insert("disk".into(), Value::from(disk_level(s.disk_bps)));
    payload.insert("host".into(), Value::from(hostname()));

    // 网络:只显示公网 IP(在线缓存);还没抓到 / 离线时回退局域网 IP(离线),保证不空
    let net = {
        let p = crate::netinfo::public_ip();
        if p.is_empty() { crate::netinfo::local_ip() } else { p }
    };
    if !net.is_empty() {
        payload.insert("net".into(), Value::from(net));
    }

    // 统一会话指示(与 dashboard 同一套):system 是统一列表里的一个 session(末位)
    let unified = unified_ids(cc_instances);
    let dot = (util::crc32(SYSTEM_SID.as_bytes()) & 0xFFFF) as i64;
    let idx = unified.iter().position(|x| x == SYSTEM_SID).map(|p| p + 1).unwrap_or(unified.len());
    payload.insert("dot".into(), Value::from(dot));
    payload.insert("idx".into(), Value::from(idx as i64));
    payload.insert("cnt".into(), Value::from(unified.len() as i64));

    let mut frame = Map::new();
    frame.insert("t".into(), Value::from("data"));
    frame.insert("screen".into(), Value::from("system"));
    frame.insert("ts".into(), Value::from(now));
    frame.insert("fresh".into(), Value::Bool(true));
    frame.insert("stale_sec".into(), Value::from(0));
    frame.insert("init".into(), Value::Bool(init));
    frame.insert("payload".into(), Value::Object(payload));
    frame
}
