//! 实例选择器(对应 pc/instance_select.py)。决定「当前显示哪个实例」。
//! auto = 跟随最近活跃(BOOT 临时手动 5s);manual = 锁定 + 仅 BOOT 环绕切。「切换」= 重绑 pinned + 标记 init 闩锁,
//! init 只在帧真正发出(mark_sent)后才清。线程安全(Mutex:resolve/cycle/mark_sent 跨线程)。
use crate::{config, sessions, util};
use serde_json::{Map, Value};
use std::sync::Mutex;

/// 系统监控伪实例的 session_id:永久挂在统一会话列表末尾(也是无 CC 会话时的唯一项)。
/// 真 CC session_id 是 UUID,绝不与此保留串冲突。resolve 返回 None 即代表「显示 system」。
pub const SYSTEM_SID: &str = "__system__";

/// 统一会话上限(设备右下方块池 SQ_MAX=8 对齐)。
pub const MAX_SESSIONS: usize = 8;
const AUTO_MANUAL_HOLD_SEC: i64 = 5;

/// 统一会话 id 列表 = Agent 实例 + system 伪会话(末尾),上限 MAX_SESSIONS。
/// 输入通常按 activity_at 倒序,先截取最近候选以保证活跃会话进入设备上限;随后按 sid
/// 稳定排序,让 BOOT 环绕和右下方块序号不会随着 activity_at 抖动。system 永占最后一位。
pub fn unified_ids(agent_instances: &[Map<String, Value>]) -> Vec<String> {
    let mut ids: Vec<String> = agent_instances.iter().filter_map(sid_opt).collect();
    if config::system_screen() {
        ids.truncate(MAX_SESSIONS - 1);
        ids.sort();
        ids.push(SYSTEM_SID.to_string()); // system 永占最后一位(启用时;CC 截到 MAX-1)
    } else {
        ids.truncate(MAX_SESSIONS); // 系统屏关闭:统一列表只含 CC
        ids.sort();
    }
    ids
}

fn sid_opt(m: &Map<String, Value>) -> Option<String> {
    m.get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn str_field<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    m.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn i64_field(m: &Map<String, Value>, key: &str) -> Option<i64> {
    m.get(key).and_then(|v| v.as_i64())
}

fn bool_field(m: &Map<String, Value>, key: &str) -> Option<bool> {
    m.get(key).and_then(|v| v.as_bool())
}

fn load_run_state(inst: &Map<String, Value>) -> Option<Map<String, Value>> {
    let sid = str_field(inst, "session_id");
    let raw = str_field(inst, "provider_sid");
    sid.and_then(sessions::load_run)
        .or_else(|| raw.and_then(sessions::load_run))
}

fn running_key(inst: &Map<String, Value>, now: i64) -> Option<i64> {
    let mut run = None;
    let running = if let Some(v) = bool_field(inst, "running") {
        v
    } else {
        run = load_run_state(inst);
        run.as_ref()
            .and_then(|r| bool_field(r, "running"))
            .unwrap_or(false)
    };
    let running_at = if let Some(v) = i64_field(inst, "running_at") {
        v
    } else {
        if run.is_none() {
            run = load_run_state(inst);
        }
        run.as_ref()
            .and_then(|r| i64_field(r, "running_at"))
            .unwrap_or(0)
    };
    if !running || running_at <= 0 || now - running_at >= crate::datasource::RUNNING_STUCK_SEC {
        return None;
    }
    let activity_at = if let Some(v) = i64_field(inst, "activity_at") {
        v
    } else {
        if run.is_none() {
            run = load_run_state(inst);
        }
        run.as_ref()
            .and_then(|r| i64_field(r, "activity_at"))
            .unwrap_or(0)
    };
    Some(running_at.max(activity_at))
}

/// 挑正在运行且最近更新的实例 sid。空闲 session 不参与 auto 跟随。
fn pick_active(instances: &[Map<String, Value>], now: i64) -> Option<String> {
    let mut best: Option<&Map<String, Value>> = None;
    let mut best_at = i64::MIN;
    for d in instances {
        let Some(a) = running_key(d, now) else {
            continue;
        };
        if a > best_at {
            best_at = a;
            best = Some(d);
        }
    }
    best.and_then(sid_opt)
}

struct Inner {
    mode: String, // "auto" | "manual"
    pinned: Option<String>,
    pending_init: bool,
    auto_manual_until: i64,
}

impl Inner {
    /// 绑定到 sid;变化则置 init 闩锁。
    fn pin(&mut self, sid: Option<String>) {
        if sid != self.pinned {
            self.pinned = sid;
            self.pending_init = true;
        }
    }
}

pub struct InstanceSelector {
    inner: Mutex<Inner>,
}

fn norm_mode(m: &str) -> String {
    if m == "auto" || m == "manual" {
        m.to_string()
    } else {
        "auto".to_string()
    }
}

impl InstanceSelector {
    pub fn new(mode: &str) -> Self {
        InstanceSelector {
            inner: Mutex::new(Inner {
                mode: norm_mode(mode),
                pinned: None,
                pending_init: false,
                auto_manual_until: 0,
            }),
        }
    }

    pub fn pinned(&self) -> Option<String> {
        self.inner.lock().unwrap().pinned.clone()
    }

    pub fn set_mode(&self, mode: &str) {
        if mode == "auto" || mode == "manual" {
            let mut g = self.inner.lock().unwrap();
            g.mode = mode.to_string();
            g.auto_manual_until = 0;
        }
    }

    pub fn mark_sent(&self) {
        self.inner.lock().unwrap().pending_init = false;
    }

    /// BOOT 弹起:同步切到下一个实例(环绕),立即推进 pinned。
    /// system 作常驻伪实例挂在末尾 → cycle = [cc1, cc2, …, system];无 CC 时即 [system]。
    pub fn cycle(&self, instances: &[Map<String, Value>]) {
        let mut g = self.inner.lock().unwrap();
        let ids = unified_ids(instances); // [cc…(, system)],上限 8;系统屏关+无 CC 时可能为空
        if ids.is_empty() {
            return; // 无可切目标(系统屏关闭且无 CC 会话)
        }
        let idx = match &g.pinned {
            Some(p) if ids.iter().any(|x| x == p) => {
                (ids.iter().position(|x| x == p).unwrap() + 1) % ids.len()
            }
            _ => 0,
        };
        let next = ids[idx].clone();
        // 显式 BOOT 切换:**总是**置 init —— 即便只 1 实例 / 环绕切回自己,也发一帧 init,
        // 设备据此立即揭黑遮罩 + 重播「0→目标」增长(否则切到自己时 pin 不变、无 init,
        // 设备黑遮罩要等 2s 失败保护超时才揭 → 单 session 切换慢)。
        g.pinned = Some(next);
        g.pending_init = true;
        if g.mode == "auto" {
            g.auto_manual_until = util::now_secs() + AUTO_MANUAL_HOLD_SEC;
        }
    }

    /// 选出当前应显示的实例 + 是否 init 帧。**不**清 init(清在 mark_sent)。
    pub fn resolve(&self, instances: &[Map<String, Value>]) -> (Option<Map<String, Value>>, bool) {
        let mut g = self.inner.lock().unwrap();
        let now = util::now_secs();
        let sys_on = config::system_screen();
        let ids = unified_ids(instances); // [cc…(, system)];system 是否在列取决于 sys_on

        // 无活跃 Agent 时的回退:系统屏启用 → system 伪实例;关闭 → None(= 等待屏)
        let fallback = |insts: &[Map<String, Value>]| -> Option<String> {
            pick_active(insts, now).or_else(|| {
                if sys_on {
                    Some(SYSTEM_SID.to_string())
                } else {
                    None
                }
            })
        };
        if g.mode == "auto" {
            // auto:只跟随真实运行中的 Agent;idle statusline / JSONL mtime 不参与。
            if now >= g.auto_manual_until {
                g.auto_manual_until = 0;
                g.pin(fallback(instances));
            }
        }
        // pinned 失效(窗口关 / prune / 系统屏刚关掉)→ 同样回退
        let valid = g
            .pinned
            .as_deref()
            .is_some_and(|p| ids.iter().any(|x| x == p));
        if !valid {
            g.auto_manual_until = 0;
            g.pin(fallback(instances));
        }
        // chosen:SYSTEM → None(= 显示 system 屏);否则查 Agent 实例 Map
        let chosen = match g.pinned.as_deref() {
            Some(p) if p != SYSTEM_SID => instances
                .iter()
                .find(|i| sid_opt(i).as_deref() == Some(p))
                .cloned(),
            _ => None,
        };
        (chosen, g.pending_init)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    fn inst(sid: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("session_id".into(), Value::String(sid.to_string()));
        m
    }

    fn inst_at(sid: &str, at: i64) -> Map<String, Value> {
        let mut m = inst(sid);
        m.insert("activity_at".into(), Value::from(at));
        m
    }

    fn running_inst(sid: &str, running: bool) -> Map<String, Value> {
        let mut m = inst_at(sid, crate::util::now_secs());
        m.insert("running".into(), Value::Bool(running));
        m.insert("running_at".into(), Value::from(crate::util::now_secs()));
        m
    }

    #[test]
    fn namespaced_provider_ids_do_not_collide() {
        let ids = unified_ids(&[inst("claude:a"), inst("codex:a")]);
        assert!(ids.contains(&"claude:a".to_string()));
        assert!(ids.contains(&"codex:a".to_string()));
        assert_eq!(ids.iter().filter(|id| id.ends_with(":a")).count(), 2);
    }

    #[test]
    fn keeps_system_slot_when_truncating() {
        let instances: Vec<_> = (0..10).map(|i| inst(&format!("codex:{i}"))).collect();
        let ids = unified_ids(&instances);
        assert!(ids.len() <= MAX_SESSIONS);
        if crate::config::system_screen() {
            assert_eq!(ids.last().map(|s| s.as_str()), Some(SYSTEM_SID));
        }
    }

    #[test]
    fn unified_ids_are_stable_when_activity_order_changes() {
        let newer_codex = vec![inst_at("codex:b", 20), inst_at("claude:a", 10)];
        let newer_claude = vec![inst_at("claude:a", 30), inst_at("codex:b", 20)];

        assert_eq!(unified_ids(&newer_codex), unified_ids(&newer_claude));
    }

    #[test]
    fn auto_ignores_idle_agents() {
        assert_eq!(
            pick_active(
                &[inst_at("codex:a", crate::util::now_secs())],
                crate::util::now_secs()
            ),
            None
        );
    }

    #[test]
    fn auto_uses_system_when_all_agents_are_idle() {
        let selector = InstanceSelector::new("auto");
        let (chosen, _) = selector.resolve(&[inst_at("codex:a", crate::util::now_secs())]);

        if crate::config::system_screen() {
            assert!(chosen.is_none());
            assert_eq!(selector.pinned().as_deref(), Some(SYSTEM_SID));
        }
    }

    #[test]
    fn auto_follows_running_agent() {
        let instances = vec![
            inst_at("claude:idle", crate::util::now_secs()),
            running_inst("codex:busy", true),
        ];
        let selector = InstanceSelector::new("auto");
        let (chosen, _) = selector.resolve(&instances);

        assert_eq!(
            chosen
                .as_ref()
                .and_then(|m| m.get("session_id"))
                .and_then(|v| v.as_str()),
            Some("codex:busy")
        );
    }

    #[test]
    fn auto_cycle_temporarily_overrides_active_agent() {
        let instances = vec![
            running_inst("claude:busy", true),
            inst_at("codex:idle", crate::util::now_secs()),
        ];
        let selector = InstanceSelector::new("auto");
        let (chosen, _) = selector.resolve(&instances);
        assert_eq!(
            chosen
                .as_ref()
                .and_then(|m| m.get("session_id"))
                .and_then(|v| v.as_str()),
            Some("claude:busy")
        );

        selector.cycle(&instances);
        let (chosen, _) = selector.resolve(&instances);

        assert_eq!(
            chosen
                .as_ref()
                .and_then(|m| m.get("session_id"))
                .and_then(|v| v.as_str()),
            Some("codex:idle")
        );
    }

    #[test]
    fn auto_returns_to_active_agent_after_manual_hold_expires() {
        let instances = vec![
            running_inst("claude:busy", true),
            inst_at("codex:idle", crate::util::now_secs()),
        ];
        let selector = InstanceSelector::new("auto");
        selector.resolve(&instances);
        selector.cycle(&instances);
        {
            let mut g = selector.inner.lock().unwrap();
            g.auto_manual_until = crate::util::now_secs() - 1;
        }

        let (chosen, _) = selector.resolve(&instances);

        assert_eq!(
            chosen
                .as_ref()
                .and_then(|m| m.get("session_id"))
                .and_then(|v| v.as_str()),
            Some("claude:busy")
        );
    }
}
