//! CC 自带的活会话注册表(对应 pc/cc_registry.py)。
//! 读 ~/.claude/sessions/<pid>.json(CC 内部文件:pid/sessionId/name/cwd);活性 = 文件存在 + pid 存活。
//! 目录缺失/空/格式不认识 → None(上层回退到基于 activity_at 的近期活跃过滤,兼容旧版 CC)。
use crate::util;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct LiveSession {
    pub session_id: String,
    pub title: Option<String>, // CC 会话标题(name;未生成时 None)
    pub cwd: Option<String>,
}

fn registry_dir() -> PathBuf {
    util::home().join(".claude").join("sessions")
}

#[cfg(windows)]
fn pid_alive(pid: i64) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid <= 0 {
        return true; // 拿不到 pid → 保守判活,交给 CC 自清
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if !h.is_null() {
            CloseHandle(h);
            return true;
        }
        // 打不开:仅「参数无效(pid 不存在)」才判死;权限等其它错误保守判活
        GetLastError() != ERROR_INVALID_PARAMETER
    }
}

#[cfg(not(windows))]
fn pid_alive(_pid: i64) -> bool {
    true
}

pub fn live_sessions() -> Option<Vec<LiveSession>> {
    let dir = registry_dir();
    if !dir.is_dir() {
        return None;
    }
    let files: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    if files.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    let mut parsed_any = false; // 至少认出一条(含 sessionId)→ 注册表可用;否则视为不认识 → 回退
    for p in files {
        let d: Value = match fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let sid = match d.get("sessionId").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        parsed_any = true;
        let pid = d.get("pid").and_then(|v| v.as_i64()).unwrap_or(0);
        if !pid_alive(pid) {
            continue; // 强杀残留:进程已死
        }
        let title = d
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let cwd = d.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());
        out.push(LiveSession { session_id: sid, title, cwd });
    }
    if parsed_any {
        Some(out)
    } else {
        None
    }
}
