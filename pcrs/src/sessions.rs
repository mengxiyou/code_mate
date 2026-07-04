//! 按 CC 会话落盘的用量 / 工作态(对应 Python pc/sessions.py)。
//! 两个文件:`<sid>.json`(用量,statusline 写)、`<sid>.run.json`(工作态,activity 写)。
//! 写入一律**原子**(临时文件 + rename);输出 JSON 紧凑(无空格),与 Python 对齐,host 才认。
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn sessions_dir() -> PathBuf {
    home().join(".claude").join("code_mate").join("sessions")
}

/// 把不安全字符替换成 `_`、截到 80 字节,空则 "unknown"(对齐 pc/sessions._safe_sid)
fn safe_sid(sid: &str) -> String {
    let cleaned: String = sid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' })
        .take(80)
        .collect();
    if cleaned.is_empty() { "unknown".to_string() } else { cleaned }
}

pub fn session_path(sid: &str) -> PathBuf {
    sessions_dir().join(format!("{}.json", safe_sid(sid)))
}

pub fn run_path(sid: &str) -> PathBuf {
    sessions_dir().join(format!("{}.run.json", safe_sid(sid)))
}

fn read_obj_opt(path: &PathBuf) -> Option<Map<String, Value>> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| if let Value::Object(m) = v { Some(m) } else { None })
}

fn read_obj(path: &PathBuf) -> Map<String, Value> {
    read_obj_opt(path).unwrap_or_default()
}

/// 读用量文件 → Some(dict);读不到/坏文件 → None(对齐 pc/sessions.load)。
pub fn load(sid: &str) -> Option<Map<String, Value>> {
    read_obj_opt(&session_path(sid))
}

/// 读工作态文件 → Some(dict);读不到 → None。
pub fn load_run(sid: &str) -> Option<Map<String, Value>> {
    read_obj_opt(&run_path(sid))
}

/// 列出所有用量文件内容(`<sid>.json`,排除 `<sid>.run.json`;跳过坏文件)。
pub fn list_usage() -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fname.ends_with(".run.json") || !fname.ends_with(".json") {
                continue;
            }
            if let Some(m) = read_obj_opt(&p) {
                out.push(m);
            }
        }
    }
    out
}

fn atomic_write(path: &PathBuf, obj: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string(obj).unwrap_or_default();
    let tmp = path.with_file_name(format!(".sess-{}.tmp", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 读改写:合并 patch + 置 session_id/name/activity_at,原子写。绝不 panic。
pub fn update(path: &PathBuf, sid: &str, patch: Map<String, Value>, name: Option<&str>, now: i64) {
    let mut cur = read_obj(path);
    cur.insert("session_id".to_string(), Value::String(sid.to_string()));
    if let Some(n) = name {
        if !n.is_empty() {
            cur.insert("name".to_string(), Value::String(n.to_string()));
        }
    }
    for (k, v) in patch {
        cur.insert(k, v);
    }
    cur.insert("activity_at".to_string(), Value::from(now));
    let _ = atomic_write(path, &cur);
}

/// SessionEnd 关窗:删用量 + 工作态文件(缺失静默忽略)。
pub fn remove(sid: &str) {
    let _ = fs::remove_file(session_path(sid));
    let _ = fs::remove_file(run_path(sid));
}

/// 删除 activity_at 超过 max_age_sec 的会话文件(用量 + 工作态都清;缺字段默认 0 → 视为可清理)。
/// 对齐 pc/sessions.prune:遍历所有 `*.json`(含 `.run.json`),按 activity_at(次 running_at)判龄。
pub fn prune(max_age_sec: i64, now: i64) {
    if let Ok(rd) = fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !fname.ends_with(".json") {
                continue;
            }
            if let Some(d) = read_obj_opt(&p) {
                let ts = d
                    .get("activity_at")
                    .or_else(|| d.get("running_at"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if now - ts > max_age_sec {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }
}
