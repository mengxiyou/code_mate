//! Codex local session source. Reads `$CODEX_HOME/sessions/**/*.jsonl` (default
//! `~/.codex/sessions`) and exposes the same loose JSON map shape used by the
//! Claude source.
use crate::{sessions, util};
use serde_json::{Map, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub const PROVIDER: &str = "Codex";
pub const NS: &str = "codex:";
const RECENT_SEC: i64 = 24 * 3600;

/// Codex 会话没有注册表/pid 可判活(JSONL 的 session_meta 无 pid 字段),单靠
/// RECENT_SEC 活动窗口会让「已关终端」的会话挂满一天(幽灵实例)。粗粒度兜底:
/// 本机一个 codex 进程都没有 → 所有 Codex 会话都不算活;有进程时仍走窗口
/// (进程↔会话无法精确配对,「开着 A 关了 B」的混合场景容忍窗口误差)。
#[cfg(windows)]
fn any_codex_process() -> bool {
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    // provide_frame ~0.4s 一 tick,进程快照缓存 3s 足够新鲜
    static G_SEEN: AtomicBool = AtomicBool::new(true);
    static G_CHECKED_AT: AtomicI64 = AtomicI64::new(0);
    let now = util::now_secs();
    if now - G_CHECKED_AT.load(Ordering::Relaxed) < 3 {
        return G_SEEN.load(Ordering::Relaxed);
    }
    let mut found = false;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return true; // 枚举失败宁可误留,不误杀
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase();
                if name.starts_with("codex") {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    G_SEEN.store(found, Ordering::Relaxed);
    G_CHECKED_AT.store(now, Ordering::Relaxed);
    found
}

#[cfg(not(windows))]
fn any_codex_process() -> bool {
    true
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::home().join(".codex"))
}

pub fn hooks_path() -> PathBuf {
    codex_home().join("hooks.json")
}

fn sessions_root() -> PathBuf {
    codex_home().join("sessions")
}

fn ns_sid(raw: &str) -> String {
    if raw.starts_with(NS) {
        raw.to_string()
    } else {
        format!("{}{}", NS, raw)
    }
}

fn short_sid(s: &str) -> String {
    if s.len() > 12 {
        s[..12].to_string()
    } else {
        s.to_string()
    }
}

fn name_from_cwd(cwd: &str) -> String {
    util::name_from_cwd(cwd)
}

fn as_i64_any(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|x| i64::try_from(x).ok()))
        .or_else(|| v.as_f64().map(|x| x as i64))
}

fn as_f64_any(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|x| x as f64))
        .or_else(|| v.as_u64().map(|x| x as f64))
}

fn get_path<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    Some(cur)
}

fn first_str<'a>(v: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    for path in paths {
        let mut cur = v;
        let mut ok = true;
        for k in *path {
            match cur.get(*k) {
                Some(n) => cur = n,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(s) = cur.as_str().filter(|s| !s.is_empty()) {
                return Some(s);
            }
        }
    }
    None
}

fn collect_jsonl(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn file_mtime_secs(path: &Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(stem.rsplit('-').next().unwrap_or(stem).to_string())
}

fn text_from_content(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    b.get("text")
                        .or_else(|| b.get("input_text"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn read_token_count(payload: &Value) -> Option<(i64, Option<i64>)> {
    let used = [
        &["info", "last_token_usage", "total_tokens"][..],
        &["last_token_usage", "total_tokens"],
        &["total_token_count"],
        &["token_count"],
        &["total_tokens"],
        &["tokens"],
        &["used_tokens"],
        &["input_tokens"],
        &["info", "total_token_usage", "total_tokens"],
        &["total_token_usage", "total_tokens"],
    ]
    .iter()
    .find_map(|p| get_path(payload, p).and_then(as_i64_any));
    let max = [
        &["info", "model_context_window"][..],
        &["model_context_window"],
        &["context_window"],
        &["max_tokens"],
        &["context_window_size"],
    ]
    .iter()
    .find_map(|p| get_path(payload, p).and_then(as_i64_any));
    used.map(|u| (u, max))
}

fn read_rate_window(win: &Value) -> Option<Value> {
    if win.is_null() {
        return None;
    }
    let mut out = Map::new();
    if let Some(p) = win
        .get("used_percent")
        .or_else(|| win.get("used_percentage"))
        .and_then(as_f64_any)
    {
        out.insert("used_pct".into(), Value::from(p));
    }
    if let Some(r) = win.get("resets_at").and_then(as_i64_any) {
        out.insert("resets_at".into(), Value::from(r));
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn read_rate_limits(payload: &Value) -> (Option<Value>, Option<Value>) {
    let rl = match payload.get("rate_limits") {
        Some(v) if v.is_object() => v,
        _ => return (None, None),
    };
    let primary = rl.get("primary").and_then(read_rate_window);
    let secondary = rl.get("secondary").and_then(read_rate_window);

    // Current Codex JSONL uses primary=5h and secondary=7d. Keep a window-size
    // fallback so the mapping remains correct if names are reordered later.
    let mut five_hour = None;
    let mut seven_day = None;
    for (key, parsed) in [("primary", primary), ("secondary", secondary)] {
        let Some(win) = rl.get(key) else { continue };
        let minutes = win.get("window_minutes").and_then(as_i64_any);
        match (minutes, key) {
            (Some(300), _) | (None, "primary") => five_hour = parsed,
            (Some(10080), _) | (None, "secondary") => seven_day = parsed,
            _ => {}
        }
    }
    (five_hour, seven_day)
}

fn activity_at(m: &Map<String, Value>) -> i64 {
    m.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0)
}

fn project_key(m: &Map<String, Value>) -> String {
    m.get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            m.get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            m.get("session_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn reduce_recent_by_project(
    instances: Vec<Map<String, Value>>,
    now: i64,
) -> Vec<Map<String, Value>> {
    let mut grouped: std::collections::HashMap<String, Map<String, Value>> =
        std::collections::HashMap::new();
    for inst in instances {
        let at = activity_at(&inst);
        if at <= 0 || now - at > RECENT_SEC {
            continue;
        }
        let key = project_key(&inst);
        let replace = grouped
            .get(&key)
            .is_none_or(|cur| activity_at(&inst) > activity_at(cur));
        if replace {
            grouped.insert(key, inst);
        }
    }
    grouped.into_values().collect()
}

pub fn parse_jsonl_file(path: &Path) -> Option<Map<String, Value>> {
    let file = fs::File::open(path).ok()?;
    let mut out = Map::new();
    let mut provider_sid = session_id_from_path(path).unwrap_or_else(|| "unknown".to_string());
    let mut cwd = String::new();
    let mut model = String::new();
    let mut mode = String::new();
    let mut ctx_max: Option<i64> = None;
    let mut ctx_used: Option<i64> = None;
    let mut five_hour: Option<Value> = None;
    let mut seven_day: Option<Value> = None;
    let mut last_assistant = String::new();
    let mut activity_at = file_mtime_secs(path);
    let mut running = false;
    let mut running_at = 0;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let typ = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = v.get("payload").unwrap_or(&Value::Null);
        match typ {
            "session_meta" => {
                if let Some(s) = first_str(payload, &[&["session_id"], &["id"]]) {
                    provider_sid = s.to_string();
                }
                if let Some(c) = first_str(payload, &[&["cwd"], &["workspace", "project_dir"]]) {
                    cwd = c.to_string();
                }
                if let Some(m) = first_str(payload, &[&["model"], &["model_slug"]]) {
                    model = m.to_string();
                }
            }
            "turn_context" => {
                if let Some(c) = first_str(payload, &[&["cwd"]]) {
                    cwd = c.to_string();
                }
                if let Some(m) = first_str(
                    payload,
                    &[&["model"], &["collaboration_mode", "settings", "model"]],
                ) {
                    model = m.to_string();
                }
                if let Some(e) = first_str(
                    payload,
                    &[&["collaboration_mode", "settings", "reasoning_effort"]],
                ) {
                    mode = e.to_uppercase();
                } else if let Some(m) = first_str(payload, &[&["collaboration_mode", "mode"]]) {
                    mode = m.to_uppercase();
                }
            }
            "event_msg" => {
                let event_type = payload.get("type").and_then(|v| v.as_str());
                if event_type == Some("task_started") {
                    if let Some(w) = payload.get("model_context_window").and_then(as_i64_any) {
                        ctx_max = Some(w);
                    }
                    if let Some(t) = payload.get("started_at").and_then(as_i64_any) {
                        activity_at = activity_at.max(t);
                        running_at = t;
                    }
                    running = true;
                    if let Some(m) = first_str(payload, &[&["collaboration_mode_kind"]]) {
                        mode = m.to_uppercase();
                    }
                } else if event_type == Some("task_complete") {
                    running = false;
                    running_at = activity_at.max(file_mtime_secs(path));
                } else if event_type == Some("token_count") {
                    if let Some((u, m)) = read_token_count(payload) {
                        ctx_used = Some(u);
                        ctx_max = m.or(ctx_max);
                    }
                    let (fh, sd) = read_rate_limits(payload);
                    if fh.is_some() {
                        five_hour = fh;
                    }
                    if sd.is_some() {
                        seven_day = sd;
                    }
                }
            }
            "token_count" => {
                if let Some((u, m)) = read_token_count(payload) {
                    ctx_used = Some(u);
                    ctx_max = m.or(ctx_max);
                }
                let (fh, sd) = read_rate_limits(payload);
                if fh.is_some() {
                    five_hour = fh;
                }
                if sd.is_some() {
                    seven_day = sd;
                }
            }
            "response_item" => {
                if payload.get("type").and_then(|v| v.as_str()) == Some("message")
                    && payload.get("role").and_then(|v| v.as_str()) == Some("assistant")
                {
                    let txt = text_from_content(payload.get("content").unwrap_or(&Value::Null));
                    if !txt.trim().is_empty() {
                        last_assistant = txt.trim().to_string();
                    }
                }
            }
            _ => {}
        }
    }

    let sid = ns_sid(&provider_sid);
    out.insert("provider".into(), Value::String(PROVIDER.to_string()));
    out.insert("source".into(), Value::String("Codex".to_string()));
    out.insert("brand".into(), Value::String("Codex".to_string()));
    out.insert("provider_sid".into(), Value::String(provider_sid.clone()));
    out.insert(
        "display_sid".into(),
        Value::String(short_sid(&provider_sid)),
    );
    out.insert("session_id".into(), Value::String(sid.clone()));
    out.insert(
        "transcript_path".into(),
        Value::String(path.to_string_lossy().to_string()),
    );
    if !cwd.is_empty() {
        out.insert("cwd".into(), Value::String(cwd.clone()));
        out.insert("name".into(), Value::String(name_from_cwd(&cwd)));
    }
    if !model.is_empty() {
        out.insert("model".into(), Value::String(model));
    }
    if !mode.is_empty() {
        out.insert("mode".into(), Value::String(mode));
    }
    if ctx_max.is_some() || ctx_used.is_some() {
        let mut ctx = Map::new();
        if let Some(u) = ctx_used {
            ctx.insert("used_tokens".into(), Value::from(u));
        }
        if let Some(m) = ctx_max {
            ctx.insert("max_tokens".into(), Value::from(m));
            if let Some(u) = ctx_used {
                if m > 0 {
                    ctx.insert(
                        "used_pct".into(),
                        Value::from(((u as f64) * 100.0 / (m as f64)).clamp(0.0, 100.0)),
                    );
                }
            }
        }
        out.insert("context".into(), Value::Object(ctx));
    }
    if !last_assistant.is_empty() {
        out.insert("last_assistant".into(), Value::String(last_assistant));
    }
    if let Some(v) = five_hour {
        out.insert("five_hour".into(), v);
    }
    if let Some(v) = seven_day {
        out.insert("seven_day".into(), v);
    }
    out.insert("captured_at".into(), Value::from(activity_at));
    out.insert("activity_at".into(), Value::from(activity_at));
    if running_at > 0 {
        out.insert("running".into(), Value::Bool(running));
        out.insert("running_at".into(), Value::from(running_at));
    }

    // Hook-provided run state, if trusted, wins over static JSONL metadata.
    if let Some(run) = sessions::load_run(&sid) {
        for (k, v) in run {
            if k == "running" || k == "running_at" || k == "activity_at" {
                out.insert(k, v);
            }
        }
    }
    Some(out)
}

pub fn list_instances() -> Vec<Map<String, Value>> {
    if !any_codex_process() {
        return Vec::new();
    }
    let parsed: Vec<_> = collect_jsonl(&sessions_root())
        .into_iter()
        .filter_map(|p| parse_jsonl_file(&p))
        .collect();
    let mut out = reduce_recent_by_project(parsed, util::now_secs());
    out.sort_by(|a, b| {
        let aa = a.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0);
        let ba = b.get("activity_at").and_then(|v| v.as_i64()).unwrap_or(0);
        ba.cmp(&aa).then_with(|| {
            a.get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("session_id").and_then(|v| v.as_str()).unwrap_or(""))
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_jsonl_context() {
        let dir = std::env::temp_dir().join(format!("code_mate_codex_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("rollout-2026-01-01T00-00-00-abc.jsonl");
        let data = [
            r#"{"type":"session_meta","payload":{"session_id":"a","cwd":"D:\\proj\\x"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_started","model_context_window":1000,"started_at":10}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-5","collaboration_mode":{"mode":"default","settings":{"reasoning_effort":"high"}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":800},"last_token_usage":{"total_tokens":250},"model_context_window":1000},"rate_limits":{"primary":{"used_percent":9.0,"window_minutes":300,"resets_at":1783020218},"secondary":{"used_percent":1.0,"window_minutes":10080,"resets_at":1783607018}}}}"#,
        ]
        .join("\n");
        fs::write(&p, data).unwrap();
        let parsed = parse_jsonl_file(&p).unwrap();
        assert_eq!(
            parsed.get("session_id").and_then(|v| v.as_str()),
            Some("codex:a")
        );
        assert_eq!(parsed.get("model").and_then(|v| v.as_str()), Some("gpt-5"));
        let ctx = parsed.get("context").unwrap();
        assert_eq!(ctx.get("used_tokens").and_then(|v| v.as_i64()), Some(250));
        assert_eq!(ctx.get("max_tokens").and_then(|v| v.as_i64()), Some(1000));
        assert_eq!(
            ctx.get("used_pct")
                .and_then(|v| v.as_f64())
                .unwrap()
                .round() as i64,
            25
        );
        assert_eq!(
            parsed
                .get("five_hour")
                .and_then(|v| v.get("used_pct"))
                .and_then(|v| v.as_f64()),
            Some(9.0)
        );
        assert_eq!(
            parsed
                .get("seven_day")
                .and_then(|v| v.get("resets_at"))
                .and_then(|v| v.as_i64()),
            Some(1783607018)
        );
        assert_eq!(parsed.get("running").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(parsed.get("running_at").and_then(|v| v.as_i64()), Some(10));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_task_complete_clears_running() {
        let dir = std::env::temp_dir().join(format!("code_mate_codex_done_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("rollout-2026-01-01T00-00-00-done.jsonl");
        let data = [
            r#"{"type":"session_meta","payload":{"session_id":"done","cwd":"D:\\proj\\x"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_started","started_at":20,"model_context_window":1000}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
        ]
        .join("\n");
        fs::write(&p, data).unwrap();
        let parsed = parse_jsonl_file(&p).unwrap();
        assert_eq!(parsed.get("running").and_then(|v| v.as_bool()), Some(false));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_latest_recent_session_per_project() {
        fn inst(sid: &str, cwd: &str, at: i64) -> Map<String, Value> {
            let mut m = Map::new();
            m.insert("session_id".into(), Value::String(sid.to_string()));
            m.insert("cwd".into(), Value::String(cwd.to_string()));
            m.insert("activity_at".into(), Value::from(at));
            m
        }
        let got = reduce_recent_by_project(
            vec![
                inst("codex:old", "D:\\proj\\alpha", 99_990),
                inst("codex:new", "D:\\proj\\alpha", 100_000),
                inst("codex:ancient", "D:\\proj\\beta", 1),
            ],
            100_000,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].get("session_id").and_then(|v| v.as_str()),
            Some("codex:new")
        );
    }
}
