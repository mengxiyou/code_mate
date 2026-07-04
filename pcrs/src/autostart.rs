//! 开机自启(Windows 启动文件夹快捷方式)。对应 pc/autostart.py。
//!
//! 优先「启动文件夹放 .lnk」(用户在 设置→应用→启动 里可见可控)。Rust:.lnk 指向 current_exe
//! 无参(= 起 headless host / 阶段6 起托盘)。用 PowerShell `WScript.Shell` 建 .lnk(免装 COM 绑定)。
//! 非 Windows:全部 no-op(`is_supported()` 为 false)。
use crate::util;
use std::path::PathBuf;

pub const APP_NAME: &str = "code_mate";

pub fn is_supported() -> bool {
    cfg!(windows)
}

fn startup_dir() -> PathBuf {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::home().join("AppData").join("Roaming"));
    appdata.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Startup")
}

pub fn lnk_path() -> PathBuf {
    startup_dir().join(format!("{}.lnk", APP_NAME))
}

/// (目标程序, 参数串, 工作目录)。Rust:current_exe,无参,workdir=exe 父目录。
fn target() -> (String, String, String) {
    let exe = std::env::current_exe().unwrap_or_default();
    let workdir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    (exe.to_string_lossy().into_owned(), String::new(), workdir)
}

pub fn is_enabled() -> bool {
    is_supported() && lnk_path().exists()
}

#[cfg(windows)]
pub fn enable() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 调 powershell 时不闪控制台窗

    let (tgt, args, workdir) = target();
    let lnk = lnk_path();
    if let Some(parent) = lnk.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 单引号 PS 字符串:'' 转义内部单引号(路径含单引号时也稳)
    let esc = |s: &str| s.replace('\'', "''");
    let ps = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.Arguments = '{}'; \
         $s.WorkingDirectory = '{}'; \
         $s.WindowStyle = 7; \
         $s.Description = 'code_mate Claude Code 用量监控'; \
         $s.Save()",
        esc(&lnk.to_string_lossy()),
        esc(&tgt),
        esc(&args),
        esc(&workdir)
    );
    let ok = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok && lnk.exists()
}

#[cfg(not(windows))]
pub fn enable() -> bool {
    false
}

/// 移除启动项快捷方式。不存在也算成功。
pub fn disable() -> bool {
    if !is_supported() {
        return false;
    }
    let _ = std::fs::remove_file(lnk_path()); // 不存在 → Err,忽略(对齐 unlink missing_ok）
    true
}

/// 按目标状态启用/停用。
pub fn sync(enabled: bool) -> bool {
    if enabled {
        enable()
    } else {
        disable()
    }
}
