//! 滚动文件日志(对应 pc/host.py 的 RotatingFileHandler)。
//! → ~/.claude/code_mate/code_mate.log,512KB × 3 备份;本地时间戳(Windows GetLocalTime)。
//! 线程安全(全局 Mutex 串行化写),失败一律静默 —— 日志绝不能拖垮主流程。
use crate::util;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 512 * 1024;
const BACKUPS: u32 = 3;

fn log_path() -> PathBuf {
    util::home().join(".claude").join("code_mate").join("code_mate.log")
}

fn lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// 本地时间戳 "YYYY-MM-DD HH:MM:SS"(Windows GetLocalTime;其它平台退回 Unix 秒)。
#[cfg(windows)]
fn stamp() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

#[cfg(not(windows))]
fn stamp() -> String {
    util::now_secs().to_string()
}

/// 滚动:code_mate.log → .1 → .2 → .3(删最老)。仅在超阈值时调。
fn rotate(path: &PathBuf) {
    let oldest = path.with_extension(format!("log.{}", BACKUPS));
    let _ = fs::remove_file(&oldest);
    for i in (1..BACKUPS).rev() {
        let src = path.with_extension(format!("log.{}", i));
        let dst = path.with_extension(format!("log.{}", i + 1));
        let _ = fs::rename(&src, &dst);
    }
    let _ = fs::rename(path, path.with_extension("log.1"));
}

fn write(level: &str, msg: &str) {
    let _g = lock().lock();
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // 超阈值先滚动(对齐 RotatingFileHandler:写前判 size)
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() >= MAX_BYTES {
            rotate(&path);
        }
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {} {}", stamp(), level, msg);
    }
    // 开发期(有控制台)也打到 stderr,对齐 Python 的 StreamHandler
    eprintln!("[code_mate] {}", msg);
}

pub fn info(msg: impl AsRef<str>) {
    write("INFO", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    write("WARNING", msg.as_ref());
}

#[allow(dead_code)]
pub fn error(msg: impl AsRef<str>) {
    write("ERROR", msg.as_ref());
}
