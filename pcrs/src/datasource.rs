//! 数据源(对应 pc/datasource.py)。Claude Code + Codex 本地会话 + system 统一进入选择器。
use crate::{cc_registry, codex_source, sessions, ui_theme, util};
use serde_json::{Map, Value};

pub const RUNNING_STUCK_SEC: i64 = 900; // running=true 但 running_at 超此秒 → 视为否
pub const SESSION_MAX_BYTES: usize = 60; // 设备 session[64] 缓冲:UTF-8 截到 ≤60 字节
pub const LIVE_SEC: i64 = 60; // 回退路径:activity_at 超此秒视为已关窗
const INSTANCE_KEYS: [&str; 3] = ["model", "mode", "context"];
const SHARED_KEYS: [&str; 2] = ["five_hour", "seven_day"];

pub fn sid_of(m: &Map<String, Value>) -> String {
    m.get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn provider_sid_of(m: &Map<String, Value>) -> String {
    m.get("provider_sid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn namespaced(provider: &str, sid: &str) -> String {
    if sid.contains(':') {
        sid.to_string()
    } else {
        format!("{}:{}", provider, sid)
    }
}

fn normalize_claude(mut inst: Map<String, Value>, raw_sid: &str) -> Map<String, Value> {
    let sid = namespaced("claude", raw_sid);
    inst.insert("session_id".to_string(), Value::String(sid));
    inst.insert(
        "provider_sid".to_string(),
        Value::String(raw_sid.to_string()),
    );
    inst.insert(
        "display_sid".to_string(),
        Value::String(raw_sid.chars().take(12).collect()),
    );
    inst.insert(
        "provider".to_string(),
        Value::String("ClaudeCode".to_string()),
    );
    inst.insert(
        "source".to_string(),
        Value::String("Claude Code".to_string()),
    );
    inst.insert(
        "brand".to_string(),
        Value::String("Claude Code".to_string()),
    );
    inst
}

fn normalize_snapshot(mut inst: Map<String, Value>) -> Map<String, Value> {
    let sid = sid_of(&inst);
    if sid.starts_with("codex:") {
        inst.entry("provider")
            .or_insert(Value::String("Codex".into()));
        inst.entry("source")
            .or_insert(Value::String("Codex".into()));
        inst.entry("brand").or_insert(Value::String("Codex".into()));
        inst
    } else {
        normalize_claude(inst, &sid)
    }
}

/// 按 UTF-8 字符边界截到 ≤max_bytes 字节(尾部不完整字节丢弃)。
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 枚举当前打开的会话实例。优先 CC 注册表(pid 存活 + 标题);不可用则回退近期活跃。
pub fn list_instances() -> Vec<Map<String, Value>> {
    let mut out = match cc_registry::live_sessions() {
        Some(reg) => {
            let mut out: Vec<Map<String, Value>> = Vec::new();
            for r in reg {
                let mut inst = sessions::load(&r.session_id).unwrap_or_default();
                inst = normalize_claude(inst, &r.session_id);
                let name = r
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| util::name_from_cwd(r.cwd.as_deref().unwrap_or("")));
                inst.insert("name".to_string(), Value::String(name));
                out.push(inst);
            }
            out.sort_by_key(sid_of); // 稳定序:idx 与 BOOT cycle 一致
            out
        }
        None => list_recent(),
    };
    out.extend(codex_source::list_instances());
    out.sort_by(|a, b| {
        let aa = a.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0);
        let ba = b.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0);
        ba.cmp(&aa).then_with(|| sid_of(a).cmp(&sid_of(b)))
    });
    out
}

/// 回退:注册表不可用 → 按 activity_at 在 LIVE_SEC 内视为活实例。
fn list_recent() -> Vec<Map<String, Value>> {
    let now = util::now_secs();
    let mut out: Vec<Map<String, Value>> = sessions::list_usage()
        .into_iter()
        .map(normalize_snapshot)
        .filter(|u| {
            let aat = u.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0);
            aat > 0 && (now - aat) <= LIVE_SEC
        })
        .collect();
    out.sort_by_key(sid_of);
    out
}

/// 实例级视图模型:model/mode/context + cc_running + session(标题)。
pub fn dashboard_payload(inst: &Map<String, Value>, now: i64) -> Map<String, Value> {
    let mut payload = Map::new();
    for k in INSTANCE_KEYS {
        if let Some(v) = inst.get(k) {
            payload.insert(k.to_string(), v.clone());
        }
    }
    for k in ["provider", "provider_sid", "display_sid", "source", "brand"] {
        if let Some(v) = inst.get(k) {
            payload.insert(k.to_string(), v.clone());
        }
    }
    let provider = inst
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("ClaudeCode");
    payload.insert("theme".into(), ui_theme::for_provider(provider));
    // 工作态:读 .run.json,running 且未卡死
    let sid = sid_of(inst);
    let raw = provider_sid_of(inst);
    let run = sessions::load_run(&sid).or_else(|| {
        if raw.is_empty() {
            None
        } else {
            sessions::load_run(&raw)
        }
    });
    let running = inst
        .get("running")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            run.as_ref()
                .and_then(|r| r.get("running"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);
    let rat = inst
        .get("running_at")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            run.as_ref()
                .and_then(|r| r.get("running_at"))
                .and_then(|v| v.as_i64())
        })
        .unwrap_or(0);
    payload.insert(
        "cc_running".to_string(),
        Value::Bool(running && (now - rat) < RUNNING_STUCK_SEC),
    );
    // session 标题(按 UTF-8 字符截到设备缓冲容得下)
    if let Some(name) = inst.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            payload.insert(
                "session".to_string(),
                Value::String(truncate_utf8(name, SESSION_MAX_BYTES)),
            );
        }
    }
    payload
}

/// 指定 provider 的账号级公共数据(5h/周):取**限流最新**(five_hour.resets_at 为主、captured_at 次)的一份。
/// ⚠️ 不能只按 captured_at 选——空闲终端可能写出「新写入时间 + 陈限流」而横跳。
pub fn provider_shared_payload(
    instances: &[Map<String, Value>],
    provider: &str,
) -> Map<String, Value> {
    fn rank(u: &Map<String, Value>) -> (i64, i64) {
        let fr = u
            .get("five_hour")
            .and_then(|v| v.get("resets_at"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cap = u.get("captured_at").and_then(|v| v.as_i64()).unwrap_or(0);
        (fr, cap)
    }
    // 取第一个严格最大者(对齐 Python max 的「首个最大」语义)
    let mut best: Option<&Map<String, Value>> = None;
    let mut best_key = (i64::MIN, i64::MIN);
    for u in instances {
        if u.get("provider").and_then(|v| v.as_str()) != Some(provider) {
            continue;
        }
        if !SHARED_KEYS.iter().any(|k| u.contains_key(*k)) {
            continue;
        }
        let k = rank(u);
        if best.is_none() || k > best_key {
            best = Some(u);
            best_key = k;
        }
    }
    let mut out = Map::new();
    if let Some(b) = best {
        for k in SHARED_KEYS {
            if let Some(v) = b.get(k) {
                out.insert(k.to_string(), v.clone());
            }
        }
    }
    out
}

/// Back-compat for the STATUS account block: it represents Claude Code limits.
pub fn shared_payload(instances: &[Map<String, Value>]) -> Map<String, Value> {
    provider_shared_payload(instances, "ClaudeCode")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(provider: &str, pct: f64, reset: i64, captured: i64) -> Map<String, Value> {
        let mut win = Map::new();
        win.insert("used_pct".into(), Value::from(pct));
        win.insert("resets_at".into(), Value::from(reset));

        let mut m = Map::new();
        m.insert("provider".into(), Value::String(provider.to_string()));
        m.insert("captured_at".into(), Value::from(captured));
        m.insert("five_hour".into(), Value::Object(win));
        m
    }

    #[test]
    fn shared_payload_is_provider_scoped() {
        let instances = vec![
            inst("Codex", 90.0, 300, 30),
            inst("ClaudeCode", 10.0, 200, 20),
        ];

        assert_eq!(
            shared_payload(&instances)
                .get("five_hour")
                .and_then(|v| v.get("used_pct"))
                .and_then(|v| v.as_f64()),
            Some(10.0)
        );
        assert_eq!(
            provider_shared_payload(&instances, "Codex")
                .get("five_hour")
                .and_then(|v| v.get("used_pct"))
                .and_then(|v| v.as_f64()),
            Some(90.0)
        );
    }
}
