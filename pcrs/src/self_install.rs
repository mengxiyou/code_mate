//! 自安装(阶段10):从 U 盘(可移动盘)运行 → 提示把 exe 复制到 LOCALAPPDATA + 注册钩子 + 自启。
//!
//! 设备「只读 U 盘」里放的就是本 exe;用户在 U 盘里双击它 → 本模块把它复制到固定目录再从那运行
//! (直接从 U 盘跑的话,设备一退出 MSC 那个盘符就没了,钩子/自启会指向失效路径)。
//! 仅可移动盘触发,免扰开发期/普通位置运行。
use crate::{autostart, installer};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONINFORMATION, MB_ICONQUESTION, MB_YESNO,
};

const DRIVE_REMOVABLE: u32 = 2;
const IDYES: i32 = 6;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 安装目标:%LOCALAPPDATA%\code_mate\CodeMate.exe(目录仍 code_mate;exe 文件名用户可见,用 CodeMate.exe)。
fn install_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::util::home().join("AppData").join("Local"));
    base.join("code_mate").join("CodeMate.exe")
}

fn paths_eq(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// path 所在盘是否可移动(U 盘)。
fn is_removable(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    if b.len() < 2 || b[1] != b':' {
        return false;
    }
    let root = wide(&format!("{}:\\", &s[..1]));
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOVABLE }
}

fn ask_yes(text: &str) -> bool {
    let t = wide(text);
    let title = wide("code_mate 安装");
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

fn info(text: &str) {
    let t = wide(text);
    let title = wide("code_mate");
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), title.as_ptr(), MB_ICONINFORMATION);
    }
}

/// 从可移动盘运行 → 提示安装。返回 true = 已复制 + 启动安装副本(调用方应退出本实例)。
pub fn maybe_offer() -> bool {
    let cur = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let target = install_path();
    if paths_eq(&cur, &target) {
        return false; // 已在安装位置
    }
    if !is_removable(&cur) {
        return false; // 仅 U 盘自安装;开发/普通位置直接运行
    }
    let ok = ask_yes(
        "检测到从 U 盘运行 code_mate 控制端。\n\n\
         安装到本机?\n\
         • 复制到 LOCALAPPDATA\\code_mate\n\
         • 注册 Claude Code 钩子 + 开机自启\n\n\
         (选「否」= 直接从 U 盘运行、不安装)",
    );
    if !ok {
        return false;
    }
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::copy(&cur, &target).is_err() {
        info("复制失败(目标被占用?)。改为从当前位置运行。");
        return false;
    }
    // 启动安装副本(--installed-launch:注册钩子 + 自启 + 进 GUI)
    let _ = std::process::Command::new(&target).arg("--installed-launch").spawn();
    true
}

/// 安装副本首次启动(--installed-launch):注册钩子 + 自启 + 提示;之后调用方进 GUI。
pub fn finalize() {
    installer::do_install();
    let _ = autostart::enable();
    info(
        "code_mate 安装完成 ✓\n\n\
         • 已复制到 LOCALAPPDATA\\code_mate\n\
         • 已注册 Claude Code 钩子(重启 CC 生效)\n\
         • 已设开机自启\n\n\
         请复位设备(或拔插)让它退出 U 盘模式 → 回 CDC,即会连上显示用量。",
    );
}
