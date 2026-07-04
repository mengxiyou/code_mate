//! 组装设备下发的 data 帧(对应 pc/cc_source.py)。
//! datasource + 选择器 → dashboard 视图模型 + 账号级公共 5h/周 + dot/idx/cnt + fresh/stale/init。
use crate::instance_select::InstanceSelector;
use crate::{config, datasource, sessions, util};
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicI64, Ordering};

// 快照新鲜阈值改为可配置(config::fresh_sec(),默认 60):captured_at 在此秒内视为新鲜(fresh=true)。
const PRUNE_AGE_SEC: i64 = 24 * 3600; // 超此未活动的 session 文件惰性删除
const PRUNE_INTERVAL: i64 = 60; // build 高频调用,裁剪最多每 60s 跑一次

static PRUNE_LAST: AtomicI64 = AtomicI64::new(0);

/// build 高频调用 → 裁剪最多每 PRUNE_INTERVAL 跑一次(对齐 pc/cc_source._maybe_prune)。
pub(crate) fn maybe_prune(now: i64) {
    let last = PRUNE_LAST.load(Ordering::Relaxed);
    if now - last > PRUNE_INTERVAL {
        PRUNE_LAST.store(now, Ordering::Relaxed);
        sessions::prune(PRUNE_AGE_SEC, now);
    }
}

/// 枚举实例 → 选择器定实例 → 组 dashboard data 帧;选中 CC 实例才有,选中 system / 无 CC 返回 None。
/// (仅供 status_json / 调试用;主下发路径 host::provide_frame 直接 resolve + 分发 dashboard/system。)
pub fn build_data_frame(selector: &InstanceSelector, now: i64) -> Option<Map<String, Value>> {
    maybe_prune(now);
    let instances = datasource::list_instances();
    let (sess, init) = selector.resolve(&instances);
    let sess = sess?; // 选中 system 或无 CC → None
    Some(build_dashboard_frame(&sess, &instances, init, now))
}

/// 由已选定的 CC 会话 + 实例列表组 dashboard data 帧(resolve 由调用方先做)。
pub fn build_dashboard_frame(
    sess: &Map<String, Value>,
    instances: &[Map<String, Value>],
    init: bool,
    now: i64,
) -> Map<String, Value> {
    let cap = sess
        .get("captured_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(now);
    let stale = (now - cap).max(0);

    let mut payload = datasource::dashboard_payload(sess, now);
    let provider = sess
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("ClaudeCode");
    match provider {
        "ClaudeCode" => {
            for (k, v) in datasource::shared_payload(instances) {
                payload.insert(k, v); // Claude 账号级公共 5h/周
            }
        }
        "Codex" => {
            for (k, v) in datasource::provider_shared_payload(instances, "Codex") {
                payload.insert(k, v); // Codex JSONL token_count.rate_limits
            }
        }
        _ => {}
    }

    // 会话指示(统一会话列表 = CC + system):身份色 + 当前序号 / 总数(含 system)
    let sid = sess
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let unified = crate::instance_select::unified_ids(instances);
    let dot = (util::crc32(sid.as_bytes()) & 0xFFFF) as i64;
    let idx = unified
        .iter()
        .position(|x| x == sid)
        .map(|p| p + 1)
        .unwrap_or(1) as i64;
    payload.insert("dot".to_string(), Value::from(dot));
    payload.insert("idx".to_string(), Value::from(idx));
    payload.insert("cnt".to_string(), Value::from(unified.len() as i64));

    let mut frame = Map::new();
    frame.insert("t".to_string(), Value::String("data".to_string()));
    frame.insert("screen".to_string(), Value::String("dashboard".to_string()));
    frame.insert("ts".to_string(), Value::from(now));
    frame.insert(
        "fresh".to_string(),
        Value::Bool(stale < config::fresh_sec()),
    );
    frame.insert("stale_sec".to_string(), Value::from(stale));
    frame.insert("init".to_string(), Value::Bool(init));
    frame.insert("payload".to_string(), Value::Object(payload));
    frame
}
