//! Claude Code 钩子(对应 Python pc/hook/statusline_snapshot.py + activity.py)。
//! 读 stdin JSON、按 CC schema 提取、原子写 session 快照。永不 panic、永不污染 stdout。
use crate::sessions;
use serde_json::{Map, Value};
use std::io::Read;

fn read_stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 嵌套取值 data[path[0]][path[1]]...
fn g<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    Some(cur)
}

/// 项目文件夹名(cwd 的 basename),空则 "?"(对齐 pc/sessions.name_from_cwd)
fn name_from_cwd(cwd: &str) -> String {
    let n = std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if n.is_empty() {
        "?".to_string()
    } else {
        n.to_string()
    }
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
            if let Some(s) = cur.as_str().filter(|s| !s.trim().is_empty()) {
                return Some(s);
            }
        }
    }
    None
}

/// statusline 钩子:抽 model/effort→mode/context/rate_limits → 写 sessions/<id>.json。
/// SHOW_STATUSLINE=false → 不写 stdout(空状态栏)。
pub fn run_statusline() {
    let raw = read_stdin();
    let data: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut usage = Map::new();

    if let Some(m) = g(&data, &["model", "display_name"]) {
        usage.insert("model".into(), m.clone());
    }
    // 工作模式徽章:effort.level 大写(statusline 无权限模式,故用思考强度)
    if let Some(e) = g(&data, &["effort", "level"]).and_then(|v| v.as_str()) {
        usage.insert("mode".into(), Value::String(e.to_uppercase()));
    }
    if let Some(cw) = data.get("context_window") {
        let mut ctx = Map::new();
        if let Some(p) = cw.get("used_percentage") {
            if p.is_number() {
                ctx.insert("used_pct".into(), p.clone());
            }
        }
        if let Some(sz) = cw.get("context_window_size") {
            if !sz.is_null() {
                ctx.insert("max_tokens".into(), sz.clone());
            }
        }
        // 已用 token:优先 total_input_tokens(已含 cache),否则 current_usage 输入侧合计
        let used: Option<i64> = cw
            .get("total_input_tokens")
            .and_then(|v| v.as_i64())
            .or_else(|| {
                cw.get("current_usage").and_then(|cu| {
                    let keys = [
                        "input_tokens",
                        "cache_creation_input_tokens",
                        "cache_read_input_tokens",
                    ];
                    let any = keys
                        .iter()
                        .any(|k| cu.get(*k).is_some_and(|v| v.is_number()));
                    if any {
                        Some(
                            keys.iter()
                                .filter_map(|k| cu.get(*k))
                                .filter_map(|v| v.as_i64())
                                .sum(),
                        )
                    } else {
                        None
                    }
                })
            });
        if let Some(u) = used {
            ctx.insert("used_tokens".into(), Value::from(u));
        }
        if !ctx.is_empty() {
            usage.insert("context".into(), Value::Object(ctx));
        }
    }
    if let Some(rl) = data.get("rate_limits") {
        for key in ["five_hour", "seven_day"] {
            if let Some(win) = rl.get(key) {
                let mut out = Map::new();
                if let Some(p) = win.get("used_percentage") {
                    if p.is_number() {
                        out.insert("used_pct".into(), p.clone());
                    }
                }
                if let Some(r) = win.get("resets_at") {
                    out.insert("resets_at".into(), r.clone());
                }
                if !out.is_empty() {
                    usage.insert(key.into(), Value::Object(out));
                }
            }
        }
    }

    let now = now_secs();
    usage.insert("captured_at".into(), Value::from(now));
    if let Some(tp) = data.get("transcript_path").and_then(|v| v.as_str()) {
        usage.insert("transcript_path".into(), Value::String(tp.to_string()));
    }

    let sid = match data.get("session_id").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s,
        _ => return,
    };
    // 名字优先项目根目录名(workspace.project_dir),否则 current_dir / cwd
    let proj = g(&data, &["workspace", "project_dir"])
        .and_then(|v| v.as_str())
        .or_else(|| g(&data, &["workspace", "current_dir"]).and_then(|v| v.as_str()))
        .or_else(|| data.get("cwd").and_then(|v| v.as_str()));
    let name = proj.map(name_from_cwd);
    sessions::update(
        &sessions::session_path(sid),
        sid,
        usage,
        name.as_deref(),
        now,
    );
}

/// 活动钩子:UserPromptSubmit→running=true / Stop→running=false / SessionEnd→删会话。
pub fn run_activity() {
    let raw = read_stdin();
    let data: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    let event = data
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sid = match data.get("session_id").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s,
        _ => return,
    };
    if event == "SessionEnd" {
        // clear/resume 会话仍在继续,别删(否则徒增闪烁)
        let reason = data.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        if reason != "clear" && reason != "resume" {
            sessions::remove(sid);
        }
        return;
    }
    let running = match event {
        "UserPromptSubmit" => Some(true),
        "Stop" => Some(false),
        _ => None,
    };
    if let Some(r) = running {
        let now = now_secs();
        let mut patch = Map::new();
        patch.insert("running".into(), Value::Bool(r));
        patch.insert("running_at".into(), Value::from(now));
        if let Some(tp) = data.get("transcript_path").and_then(|v| v.as_str()) {
            patch.insert("transcript_path".into(), Value::String(tp.to_string()));
        }
        sessions::update(&sessions::run_path(sid), sid, patch, None, now);
    }
}

/// Codex lifecycle hook: tolerate several stdin schemas and write only code_mate
/// run-state snapshots keyed by `codex:<session_id>`.
pub fn run_codex_activity() {
    let raw = read_stdin();
    let data: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    let event = first_str(
        &data,
        &[
            &["hook_event_name"],
            &["event"],
            &["event_name"],
            &["hook", "event"],
            &["payload", "hook_event_name"],
            &["payload", "event"],
        ],
    )
    .unwrap_or("");
    let sid_raw = match first_str(
        &data,
        &[
            &["session_id"],
            &["id"],
            &["conversation_id"],
            &["thread_id"],
            &["payload", "session_id"],
            &["payload", "id"],
            &["session", "id"],
        ],
    ) {
        Some(s) => s,
        None => return,
    };
    let sid = if sid_raw.starts_with("codex:") {
        sid_raw.to_string()
    } else {
        format!("codex:{}", sid_raw)
    };
    let now = now_secs();
    let mut patch = Map::new();
    let running = match event {
        "UserPromptSubmit" => Some(true),
        "Stop" => Some(false),
        "SessionStart" => Some(false),
        _ => None,
    };
    if let Some(r) = running {
        patch.insert("running".into(), Value::Bool(r));
        patch.insert("running_at".into(), Value::from(now));
    }
    if let Some(cwd) = first_str(
        &data,
        &[&["cwd"], &["payload", "cwd"], &["workspace", "cwd"]],
    ) {
        patch.insert("cwd".into(), Value::String(cwd.to_string()));
    }
    if let Some(tp) = first_str(
        &data,
        &[&["transcript_path"], &["payload", "transcript_path"]],
    ) {
        patch.insert("transcript_path".into(), Value::String(tp.to_string()));
    }
    if patch.is_empty() {
        return;
    }
    sessions::update(&sessions::run_path(&sid), &sid, patch, None, now);
}
