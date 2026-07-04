//! 把钩子注册到 Claude Code 的 settings.json(对应 pc/installer.py)。
//!
//! statusLine → `"<exe>" --statusline --code-mate-hook`(+ refreshInterval);
//! hooks.{UserPromptSubmit,Stop,SessionEnd} → `"<exe>" --activity --code-mate-hook`。
//! 合并写入(保留用户其它键 + 他人钩子)、幂等、改前自动备份。Rust exe 自带 `--statusline`/`--activity`
//! 子命令,无需独立脚本(故不分 frozen/dev,命令恒指向 current_exe)。
//! CLI:`code_mate --install` / `--status` / `--uninstall`。
use crate::{codex_source, util};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

// 显式哨兵:附在每条钩子命令尾,作为「是否本项目所装」的判据(与安装路径解耦)。
// 它是被 exe 忽略的多余参数(main 只认 --statusline/--activity)。
const HOOK_TAG: &str = "--code-mate-hook";
// 旧名(vibe_mate 时期)哨兵:仅用于迁移识别 —— 改名(vibe_mate→code_mate)后重装时,
// 把指向已不存在旧 exe 的残留条目一并清掉,否则旧钩子每次触发都报 "No such file or directory"。
const HOOK_TAG_LEGACY: &str = "--vibe-mate-hook";

// 活动/生命周期钩子:同一 --activity 命令,activity.py/hooks.rs 靠 hook_event_name 区分:
//   UserPromptSubmit/Stop = 工作状态 running;SessionEnd = 关窗即删会话文件。
const ACTIVITY_EVENTS: [&str; 3] = ["UserPromptSubmit", "Stop", "SessionEnd"];
const CODEX_EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "Stop"];

// statusLine 周期刷新(秒):空闲 CC 窗口也每 ~10s 跑一次钩子 → activity_at 持续新鲜,
// host 才能据「近期活跃」精确区分『开着』vs『已关闭』(见阶段7)。
const REFRESH_INTERVAL: i64 = 10;

fn settings_path() -> PathBuf {
    util::home().join(".claude").join("settings.json")
}

/// current_exe 的正斜杠绝对路径(Windows settings.json 一律用正斜杠)。
fn exe_posix() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.replace('\\', "/")))
        .unwrap_or_default()
}

fn hook_command(subcmd: &str) -> String {
    format!("\"{}\" --{} {}", exe_posix(), subcmd, HOOK_TAG)
}

fn statusline_command() -> String {
    hook_command("statusline")
}

fn activity_command() -> String {
    hook_command("activity")
}

fn codex_activity_command() -> String {
    format!("\"{}\" --codex-activity {}", exe_posix(), HOOK_TAG)
}

/// 命令是否本项目所装。优先认哨兵;兼容旧名(vibe_mate)与旧 Python 装(指向钩子脚本)以便迁移清理。
fn is_ours(command: &str) -> bool {
    command.contains(HOOK_TAG)
        || command.contains(HOOK_TAG_LEGACY)
        || command.contains("statusline_snapshot.py")
        || command.contains("activity.py")
}

fn cmd_of(group_hook: &Value) -> &str {
    group_hook
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or("")
}

/// 从某事件的 hook 组列表里移除本项目的条目(保留他人钩子);非数组原样返回。
fn strip_marker(groups: &Value) -> Value {
    let arr = match groups.as_array() {
        Some(a) => a,
        None => return groups.clone(),
    };
    let mut out: Vec<Value> = Vec::new();
    for g in arr {
        if let Some(hooks) = g.get("hooks").and_then(|h| h.as_array()) {
            let kept: Vec<Value> = hooks
                .iter()
                .filter(|h| !is_ours(cmd_of(h)))
                .cloned()
                .collect();
            if !kept.is_empty() {
                let mut g2 = g.clone();
                if let Some(o) = g2.as_object_mut() {
                    o.insert("hooks".into(), Value::Array(kept));
                }
                out.push(g2);
            }
            // 该组只有本项目钩子 → 整组丢弃
        } else {
            out.push(g.clone());
        }
    }
    Value::Array(out)
}

fn event_has_marker(groups: &Value) -> bool {
    groups.as_array().is_some_and(|arr| {
        arr.iter().any(|g| {
            g.get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|hooks| hooks.iter().any(|h| is_ours(cmd_of(h))))
        })
    })
}

/// 保留他人钩子、移除本项目旧条目、追加当前 cmd(幂等)。
fn merge_event_hooks(groups: Option<&Value>, cmd: &str) -> Value {
    let mut arr = match groups.map(strip_marker) {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    };
    arr.push(json!({"hooks":[{"type":"command","command":cmd}]}));
    Value::Array(arr)
}

fn load_settings(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({}))
}

fn codex_activity_seen() -> bool {
    let rd = match fs::read_dir(crate::sessions::sessions_dir()) {
        Ok(rd) => rd,
        Err(_) => return false,
    };
    for e in rd.flatten() {
        let p = e.path();
        let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !fname.ends_with(".run.json") {
            continue;
        }
        let sid = fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| {
                v.get("session_id")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
            });
        if sid.as_deref().is_some_and(|s| s.starts_with("codex:")) {
            return true;
        }
    }
    false
}

fn atomic_write_json(path: &Path, obj: &Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(obj).unwrap_or_default(); // indent=2,对齐 Python
    let tmp = path.with_file_name(format!(".settings-{}.tmp", std::process::id()));
    let wrote = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(data.as_bytes())?;
        f.sync_all()
    });
    if wrote.is_ok() {
        let _ = fs::rename(&tmp, path);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

/// 结构化返回当前钩子安装状态(供配置界面用;不打印)。
pub fn hook_status() -> Value {
    let path = settings_path();
    let settings = load_settings(&path);
    let cmd = settings
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str());
    let hooks = settings.get("hooks");
    let activity_installed = ACTIVITY_EVENTS
        .iter()
        .all(|ev| event_has_marker(hooks.and_then(|h| h.get(ev)).unwrap_or(&Value::Null)));
    let claude = json!({
        "settings_path": path.to_string_lossy(),
        "settings_exists": path.exists(),
        "command": cmd,
        "installed": cmd.is_some_and(is_ours),
        "activity_installed": activity_installed,
        "expected": statusline_command(),
    });

    let cpath = codex_source::hooks_path();
    let csettings = load_settings(&cpath);
    let chooks = csettings.get("hooks");
    let codex_installed = CODEX_EVENTS
        .iter()
        .all(|ev| event_has_marker(chooks.and_then(|h| h.get(ev)).unwrap_or(&Value::Null)));
    let codex_trusted = codex_installed && codex_activity_seen();
    let codex = json!({
        "hooks_path": cpath.to_string_lossy(),
        "hooks_exists": cpath.exists(),
        "installed": codex_installed,
        "trusted": codex_trusted,
        "trust_required": codex_installed && !codex_trusted,
        "expected": codex_activity_command(),
        "note": if codex_installed && !codex_trusted { "Open Codex /hooks and trust the code_mate hook to enable live running state." } else { "" },
    });
    json!({
        "settings_path": path.to_string_lossy(),
        "settings_exists": path.exists(),
        "command": cmd,
        "installed": cmd.is_some_and(is_ours),
        "activity_installed": activity_installed,
        "expected": statusline_command(),
        "claude": claude,
        "codex": codex,
    })
}

/// 执行安装(改前备份、幂等覆盖),返回结果 Value;**不打印**(供 GUI 调)。
pub fn do_install() -> Value {
    let cmd = statusline_command();
    let acmd = activity_command();
    let path = settings_path();
    let mut settings = load_settings(&path);

    let old_cmd_external = settings
        .get("statusLine")
        .map(|o| o.is_object() && !is_ours(cmd_of(o)))
        .unwrap_or(false);

    let mut backup: Option<String> = None;
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            let bak = path.with_file_name(format!("settings.json.bak.{}", util::now_secs()));
            if fs::write(&bak, content).is_ok() {
                backup = bak
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());
            }
        }
    }

    {
        let obj = settings.as_object_mut().expect("load_settings 保证 object");
        obj.insert(
            "statusLine".into(),
            json!({"type":"command","command":cmd,"refreshInterval":REFRESH_INTERVAL}),
        );
        // 活动钩子:合并注册 UserPromptSubmit + Stop + SessionEnd(保留他人钩子、幂等)
        let mut hooks: Map<String, Value> = obj
            .get("hooks")
            .and_then(|h| h.as_object())
            .cloned()
            .unwrap_or_default();
        for ev in ACTIVITY_EVENTS {
            let merged = merge_event_hooks(hooks.get(ev), &acmd);
            hooks.insert(ev.to_string(), merged);
        }
        obj.insert("hooks".into(), Value::Object(hooks));
    }
    atomic_write_json(&path, &settings);

    let codex = do_install_codex();

    json!({
        "ok": true,
        "command": cmd,
        "activity_command": acmd,
        "codex_activity_command": codex_activity_command(),
        "codex": codex,
        "backup": backup,
        "overwrote_external": old_cmd_external,
    })
}

fn do_install_codex() -> Value {
    let cmd = codex_activity_command();
    let path = codex_source::hooks_path();
    let mut settings = load_settings(&path);
    let mut backup: Option<String> = None;
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            let bak = path.with_file_name(format!("hooks.json.bak.{}", util::now_secs()));
            if fs::write(&bak, content).is_ok() {
                backup = bak
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());
            }
        }
    }
    {
        let obj = settings.as_object_mut().expect("load_settings 保证 object");
        let mut hooks: Map<String, Value> = obj
            .get("hooks")
            .and_then(|h| h.as_object())
            .cloned()
            .unwrap_or_default();
        for ev in CODEX_EVENTS {
            hooks.insert(ev.to_string(), merge_event_hooks(hooks.get(ev), &cmd));
        }
        obj.insert("hooks".into(), Value::Object(hooks));
    }
    atomic_write_json(&path, &settings);
    json!({
        "ok": true,
        "hooks_path": path.to_string_lossy(),
        "command": cmd,
        "backup": backup,
        "trust_required": true,
    })
}

pub fn install() {
    let r = do_install();
    if let Some(b) = r.get("backup").and_then(|v| v.as_str()) {
        println!("已备份原 settings.json → {}", b);
    }
    if r.get("overwrote_external")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        println!("⚠️ 已覆盖原有的(非本项目)statusLine");
    }
    println!(
        "✅ statusLine → {}",
        r.get("command").and_then(|v| v.as_str()).unwrap_or("")
    );
    println!(
        "✅ 活动/生命周期钩子(UserPromptSubmit/Stop/SessionEnd)→ {}",
        r.get("activity_command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );
    println!(
        "✅ Codex hooks(SessionStart/UserPromptSubmit/Stop)→ {}",
        r.get("codex_activity_command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );
    println!("   settings:{}", settings_path().to_string_lossy());
    println!(
        "   codex hooks:{}",
        codex_source::hooks_path().to_string_lossy()
    );
    println!(
        "   在 Claude Code 里发一条消息即可触发;按 session 落盘到 ~/.claude/code_mate/sessions/"
    );
    println!("   Codex 需要在 /hooks 中信任 code_mate hook 后,才能获得实时运行态。");
}

pub fn status() {
    let path = settings_path();
    let settings = load_settings(&path);
    let sl = settings.get("statusLine");
    println!(
        "settings.json: {} ({})",
        path.to_string_lossy(),
        if path.exists() { "存在" } else { "不存在" }
    );
    match sl {
        Some(v) => println!(
            "当前 statusLine: {}",
            serde_json::to_string(v).unwrap_or_default()
        ),
        None => println!("当前 statusLine: (未设置)"),
    }
    println!("本项目应为:     {}", statusline_command());
    let hs = hook_status();
    println!(
        "Codex hooks: {}",
        serde_json::to_string(hs.get("codex").unwrap_or(&Value::Null)).unwrap_or_default()
    );
}

pub fn uninstall() {
    let path = settings_path();
    let mut settings = load_settings(&path);
    let mut changed = false;
    {
        let obj = settings.as_object_mut().expect("object");
        // statusLine(仅当是本项目所装)
        let sl_ours = obj
            .get("statusLine")
            .map(|s| s.is_object() && is_ours(cmd_of(s)))
            .unwrap_or(false);
        if sl_ours {
            obj.shift_remove("statusLine"); // shift 而非 swap:保留其余键序(对齐 Python dict.pop)
            changed = true;
        }
        // 活动钩子
        if let Some(mut hooks) = obj.get("hooks").and_then(|h| h.as_object()).cloned() {
            for ev in ACTIVITY_EVENTS {
                if let Some(cur) = hooks.get(ev).cloned() {
                    let stripped = strip_marker(&cur);
                    if stripped != cur {
                        changed = true;
                        if stripped.as_array().is_some_and(|a| !a.is_empty()) {
                            hooks.insert(ev.to_string(), stripped);
                        } else {
                            hooks.shift_remove(ev);
                        }
                    }
                }
            }
            if hooks.is_empty() {
                obj.shift_remove("hooks");
            } else {
                obj.insert("hooks".into(), Value::Object(hooks));
            }
        }
    }
    if changed {
        atomic_write_json(&path, &settings);
        println!("✅ 已移除本项目的 statusLine + 活动钩子(其它键保留)");
    } else {
        println!("当前未安装本项目钩子,未改动");
    }
    uninstall_codex();
}

fn uninstall_codex() {
    let path = codex_source::hooks_path();
    let mut settings = load_settings(&path);
    let mut changed = false;
    {
        let obj = settings.as_object_mut().expect("object");
        if let Some(mut hooks) = obj.get("hooks").and_then(|h| h.as_object()).cloned() {
            for ev in CODEX_EVENTS {
                if let Some(cur) = hooks.get(ev).cloned() {
                    let stripped = strip_marker(&cur);
                    if stripped != cur {
                        changed = true;
                        if stripped.as_array().is_some_and(|a| !a.is_empty()) {
                            hooks.insert(ev.to_string(), stripped);
                        } else {
                            hooks.shift_remove(ev);
                        }
                    }
                }
            }
            if hooks.is_empty() {
                obj.shift_remove("hooks");
            } else {
                obj.insert("hooks".into(), Value::Object(hooks));
            }
        }
    }
    if changed {
        atomic_write_json(&path, &settings);
        println!("✅ 已移除本项目的 Codex hooks(其它 hooks 保留)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_hook_merge_preserves_foreign_and_is_idempotent() {
        let foreign = json!([{"hooks":[{"type":"command","command":"echo foreign"}]}]);
        let cmd = "\"CodeMate.exe\" --codex-activity --code-mate-hook";
        let once = merge_event_hooks(Some(&foreign), cmd);
        let twice = merge_event_hooks(Some(&once), cmd);
        let arr = twice.as_array().unwrap();
        let commands: Vec<&str> = arr
            .iter()
            .flat_map(|g| {
                g.get("hooks")
                    .and_then(|h| h.as_array())
                    .into_iter()
                    .flatten()
            })
            .filter_map(cmd_of_opt)
            .collect();
        assert_eq!(commands.iter().filter(|c| **c == "echo foreign").count(), 1);
        assert_eq!(commands.iter().filter(|c| **c == cmd).count(), 1);
    }

    fn cmd_of_opt(v: &Value) -> Option<&str> {
        v.get("command").and_then(|c| c.as_str())
    }
}
