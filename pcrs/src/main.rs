//! code_mate PC 控制端入口(Rust 重写;阶段9)。
//! 子命令(对应 Python pc/main.py):
//!   --statusline / --activity  → CC 钩子(读 stdin、原子写快照)
//!   --install / --uninstall    → 部署/卸载钩子 + 自启(阶段5)
//!   --dump-frame               → 调试:打印一帧 data(对拍 Python cc_source)
//!   无参数                      → headless host:发现/握手/下发(阶段4)
//! 完整版(--features ui)再起 Tauri 配置窗 + 托盘(阶段6)。
//
// GUI 子系统:不分配控制台 → 双击/自启/自安装时不再弹黑窗(issue「弹出命令行」)。
// CC 调钩子(--statusline/--activity)走重定向管道,与子系统无关、stdin/stdout 照常工作;
// 终端里手动跑 CLI 子命令(--install/--status/调试)则在 main 开头 AttachConsole 回连父终端补回输出。
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(dead_code)] // 重写期:许多函数写在用到之前(逐阶段接通)

mod autostart;
mod cc_registry;
mod cc_source;
mod codex_source;
mod config;
mod datasource;
mod events;
mod hooks;
mod host;
mod installer;
mod instance_select;
mod lid_watch;
mod log;
mod netinfo;
#[cfg(all(windows, feature = "ui"))]
mod self_install;
mod serial_link;
mod sessions;
#[cfg(windows)]
mod sys_source;
#[cfg(windows)]
mod sysmon;
mod transcript;
#[cfg(feature = "ui")]
mod ui;
mod ui_theme;
mod util;

/// GUI 子系统下,从父终端运行的 CLI 子命令需 AttachConsole(ATTACH_PARENT_PROCESS) 才有 stdout。
/// 仅给「会 println 的 CLI/调试子命令」用;**不给** 钩子(--statusline/--activity:走 CC 管道)、
/// --installed-launch(GUI)、无参 GUI —— 免得动到它们既有的(管道/无)标准句柄。
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    } // 无父控制台(双击/GUI)→ 返回 0,无副作用
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    // CLI/调试子命令(非钩子、非 GUI)→ 回连父终端补回控制台输出。
    #[cfg(windows)]
    if !args.is_empty()
        && !has("--statusline")
        && !has("--activity")
        && !has("--codex-activity")
        && !has("--installed-launch")
    {
        attach_parent_console();
    }

    if has("--statusline") {
        hooks::run_statusline();
    } else if has("--activity") {
        hooks::run_activity();
    } else if has("--codex-activity") {
        hooks::run_codex_activity();
    } else if has("--dump-frame") {
        // 调试:resolve + 分发,打一帧 data(选中 CC → dashboard;无 CC / 选中 system → system 帧)
        let cfg = config::load();
        let sel = instance_select::InstanceSelector::new(&cfg.instance_mode);
        let now = util::now_secs();
        let instances = datasource::list_instances();
        let (sess, init) = sel.resolve(&instances);
        let frame = match sess {
            Some(cc) => cc_source::build_dashboard_frame(&cc, &instances, init, now),
            #[cfg(windows)]
            None => sys_source::build_system_frame(
                &mut sysmon::SysMonitor::new(),
                &instances,
                init,
                now,
            ),
            #[cfg(not(windows))]
            None => serde_json::Map::new(),
        };
        println!("{}", serde_json::to_string(&frame).unwrap_or_default());
    } else if has("--dump-sys") {
        // 调试:直接组一帧 system data(绕过 resolve);先采基线 + 等 1s 再采 → CPU/磁盘有真实读数
        #[cfg(windows)]
        {
            let mut mon = sysmon::SysMonitor::new();
            std::thread::sleep(std::time::Duration::from_secs(1));
            let inst = datasource::list_instances();
            let frame = sys_source::build_system_frame(&mut mon, &inst, true, util::now_secs());
            println!("{}", serde_json::to_string(&frame).unwrap_or_default());
        }
        #[cfg(not(windows))]
        println!("--dump-sys 仅 Windows 可用");
    } else if has("--dump-ip") {
        // 调试:起 netinfo 后台抓取,等几秒打印公网/本地 IP(排查 system 屏 IP 显示)
        netinfo::start();
        std::thread::sleep(std::time::Duration::from_secs(5));
        println!("public_ip = {:?}", netinfo::public_ip());
        println!("local_ip  = {:?}", netinfo::local_ip());
    } else if has("--frames") {
        // 阶段3 调试:stdin 文本 → text_to_frames(clear=true) → 打 JSON(对拍 Python)
        use std::io::Read;
        let mut text = String::new();
        let _ = std::io::stdin().read_to_string(&mut text);
        let payloads: Vec<serde_json::Value> = transcript::text_to_frames(&text, true)
            .iter()
            .map(|f| f.to_payload())
            .collect();
        println!("{}", serde_json::to_string(&payloads).unwrap_or_default());
    } else if has("--install") {
        installer::install();
    } else if has("--uninstall") {
        installer::uninstall();
    } else if has("--status") {
        installer::status();
    } else if has("--autostart-enable") {
        // 阶段5 调试:自启 .lnk 由阶段6 配置界面驱动;这里供 CLI 单测
        println!("autostart enable -> {}", autostart::enable());
    } else if has("--autostart-disable") {
        println!("autostart disable -> {}", autostart::disable());
    } else if has("--autostart-status") {
        println!(
            "supported={} enabled={} lnk={}",
            autostart::is_supported(),
            autostart::is_enabled(),
            autostart::lnk_path().to_string_lossy()
        );
    } else if has("--sysmon") {
        // 阶段12 spike:采本机 CPU/RAM/VRAM/磁盘,每秒打印一行,对拍任务管理器。
        #[cfg(windows)]
        {
            let mut mon = sysmon::SysMonitor::new();
            println!("sysmon spike: 每秒一行(首行 CPU/磁盘为基线,可能偏低)");
            for n in 0..8 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let s = mon.sample();
                println!(
                    "[{n}] cpu={:5.1}%  temp={}  ghz={:.2}  ram={:4.1}%  vram={:4.1}%  disk={:6.1}MB/s",
                    s.cpu_pct,
                    s.cpu_temp.map(|t| format!("{:.1}C", t)).unwrap_or_else(|| "--".into()),
                    s.cpu_ghz, s.ram_pct, s.vram_pct,
                    s.disk_bps / 1024.0 / 1024.0
                );
            }
        }
        #[cfg(not(windows))]
        println!("--sysmon 仅 Windows 可用");
    } else if has("--selftest") {
        // 阶段4 自测:起 host 跑 ~12s,打印状态(无设备时应一直 connected=false)
        let mut host = host::Host::new();
        host.add_listener(Box::new(|c, p| {
            eprintln!("  [listener] connected={} port={:?}", c, p);
        }));
        host.start();
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_secs(3));
            eprintln!("status: {}", host.get_status());
        }
        host.stop();
    } else if has("--installed-launch") {
        // 阶段10 自安装副本首次启动:注册钩子 + 自启 + 提示,然后进 GUI(完整版)。
        #[cfg(all(windows, feature = "ui"))]
        {
            self_install::finalize();
            ui::run_app();
        }
    } else {
        // 默认:完整版(--features ui)起配置窗 + 托盘 + host;精简版(headless)只起 host。
        #[cfg(feature = "ui")]
        {
            // 从 U 盘运行 → 提示自安装(复制到 LOCALAPPDATA + 钩子 + 自启);已复制则退出本实例。
            #[cfg(windows)]
            if self_install::maybe_offer() {
                std::process::exit(0);
            }
            ui::run_app();
        }
        #[cfg(not(feature = "ui"))]
        {
            // headless host(发现/握手/下发)。Ctrl+C / kill 退出(信号优雅退出留待后续)。
            let mut host = host::Host::new();
            host.start();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
    std::process::exit(0);
}
