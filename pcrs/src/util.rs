//! 小工具:Unix 秒、CRC-32、家目录。
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// 标准 zlib/IEEE CRC-32(会话身份色 dot 用;对齐 Python zlib.crc32)。
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// 项目文件夹名(cwd 的 basename),空则 "?"(对齐 pc/sessions.name_from_cwd)。
pub fn name_from_cwd(cwd: &str) -> String {
    let n = std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if n.is_empty() { "?".to_string() } else { n.to_string() }
}
