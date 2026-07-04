//! 用户设置 config.ini(对应 pc/config_store.py;阶段9 JSON→INI)。
//! 路径 ~/.claude/code_mate/config.ini。缺文件/坏文件 → 默认,不报错。
//! per-frame 高频读的项(fresh_sec/temp_unit/system_screen)镜像到进程级原子缓存,
//! load()/save() 时 publish;热路径读缓存访问器,不每帧碰磁盘(与 cc_source 的 PRUNE_LAST 同风格)。
use crate::util;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

fn config_path() -> PathBuf {
    util::home().join(".claude").join("code_mate").join("config.ini")
}

// 快照新鲜阈值(秒)合法区间:太小无意义、太大失去 STALE 意义。
const FRESH_MIN: i64 = 15;
const FRESH_MAX: i64 = 600;
const FRESH_DEFAULT: i64 = 60;

#[derive(Clone, Debug)]
pub struct Config {
    pub instance_mode: String, // "auto" | "manual"
    pub fresh_sec: i64,        // 快照新鲜阈值(秒),clamp [FRESH_MIN, FRESH_MAX]
    pub temp_unit: String,     // "C" | "F"
    pub system_screen: bool,   // PC 系统监控屏 是否启用
    pub lang: String,          // 配置界面语言 "zh" | "en"
}

impl Default for Config {
    fn default() -> Self {
        Config {
            instance_mode: "manual".to_string(),
            fresh_sec: FRESH_DEFAULT,
            temp_unit: "C".to_string(),
            system_screen: true,
            lang: "zh".to_string(),
        }
    }
}

// ---- 进程级缓存(热路径快读,避免每帧读文件)----
static G_FRESH_SEC: AtomicI64 = AtomicI64::new(FRESH_DEFAULT);
static G_TEMP_F: AtomicBool = AtomicBool::new(false);
static G_SYSTEM_SCREEN: AtomicBool = AtomicBool::new(true);

fn publish(cfg: &Config) {
    G_FRESH_SEC.store(cfg.fresh_sec, Ordering::Relaxed);
    G_TEMP_F.store(cfg.temp_unit.eq_ignore_ascii_case("F"), Ordering::Relaxed);
    G_SYSTEM_SCREEN.store(cfg.system_screen, Ordering::Relaxed);
}

/// 快照新鲜阈值(秒)。热路径读缓存。
pub fn fresh_sec() -> i64 {
    G_FRESH_SEC.load(Ordering::Relaxed)
}
/// CPU 温度是否用华氏(否则摄氏)。
pub fn temp_fahrenheit() -> bool {
    G_TEMP_F.load(Ordering::Relaxed)
}
/// PC 系统监控屏是否启用。
pub fn system_screen() -> bool {
    G_SYSTEM_SCREEN.load(Ordering::Relaxed)
}

fn clamp_fresh(n: i64) -> i64 {
    n.clamp(FRESH_MIN, FRESH_MAX)
}

fn parse_bool(s: &str, default: bool) -> bool {
    match s.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => true,
        "false" | "0" | "no" | "off" => false,
        _ => default,
    }
}

fn parse_ini(text: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    m
}

pub fn load() -> Config {
    let mut cfg = Config::default();
    if let Ok(text) = fs::read_to_string(config_path()) {
        let kv = parse_ini(&text);
        if let Some(im) = kv.get("instance_mode") {
            if im == "auto" || im == "manual" {
                cfg.instance_mode = im.clone();
            }
        }
        if let Some(n) = kv.get("fresh_sec").and_then(|s| s.parse::<i64>().ok()) {
            cfg.fresh_sec = clamp_fresh(n);
        }
        if let Some(tu) = kv.get("temp_unit") {
            let u = tu.to_uppercase();
            if u == "C" || u == "F" {
                cfg.temp_unit = u;
            }
        }
        if let Some(ss) = kv.get("system_screen") {
            cfg.system_screen = parse_bool(ss, true);
        }
        if let Some(lg) = kv.get("lang") {
            let l = lg.to_lowercase();
            if l == "zh" || l == "en" {
                cfg.lang = l;
            }
        }
    }
    publish(&cfg);
    cfg
}

pub fn save(cfg: &Config) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = format!(
        "[code_mate]\n\
         ; 活跃窗口切换:auto 跟随最近活跃 / manual 锁定 + 仅 BOOT 切\n\
         instance_mode = {}\n\
         ; 快照新鲜阈值(秒):captured_at 在此秒内 = 新鲜(LIVE),否则 STALE\n\
         fresh_sec = {}\n\
         ; CPU 温度单位:C 摄氏 / F 华氏\n\
         temp_unit = {}\n\
         ; PC 系统监控屏:true 启用 / false 关闭(无 CC 会话时改显等待屏,BOOT 环绕不切系统屏)\n\
         system_screen = {}\n\
         ; 配置界面语言:zh 中文 / en English\n\
         lang = {}\n",
        cfg.instance_mode, cfg.fresh_sec, cfg.temp_unit, cfg.system_screen, cfg.lang
    );
    // config 低频、非关键,坏了 load 会回退默认 → 直接写即可
    let _ = fs::write(&path, body);
    publish(cfg);
}

// ---- 单项设置(配置界面调:load → 改 → save;save 内部 publish 刷新缓存)----
pub fn set_fresh_sec(n: i64) {
    let mut cfg = load();
    cfg.fresh_sec = clamp_fresh(n);
    save(&cfg);
}
pub fn set_temp_unit(u: &str) {
    let u = u.to_uppercase();
    if u == "C" || u == "F" {
        let mut cfg = load();
        cfg.temp_unit = u;
        save(&cfg);
    }
}
pub fn set_system_screen(on: bool) {
    let mut cfg = load();
    cfg.system_screen = on;
    save(&cfg);
}
/// 配置界面语言(UI 专用,非热路径,直接读文件)。
pub fn lang() -> String {
    load().lang
}
pub fn set_lang(l: &str) {
    let l = l.to_lowercase();
    if l == "zh" || l == "en" {
        let mut cfg = load();
        cfg.lang = l;
        save(&cfg);
    }
}
