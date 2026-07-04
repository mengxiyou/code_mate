//! 完整版 GUI(feature=ui;对应 pc/tray.py + ui_bridge.py + main.py run_app)。
//!
//! wry(WebView)+ tao(窗口/事件循环)+ tray-icon(托盘),复用 ui/ 网页前端。
//! - 单实例(Windows 命名 Mutex)→ 已在运行则弹框退出。
//! - 起 Host(后台线程发现/握手/下发);隐藏配置窗(点托盘才显示)。
//! - IPC:前端 `window.ipc.postMessage({id,method,args})` → 本模块 dispatch → `evaluate_script(__vmResolve)`。
//! - 托盘:图标随连接态变色(青/灰),菜单 打开配置 / 重新连接 / 退出;关窗 = 回托盘(不退)。
use crate::host::{Host, HostShared};
use crate::{autostart, config, installer, log};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon as TaoIcon, WindowBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use wry::WebViewBuilder;

/// 跨线程送进事件循环的用户事件:连接态(serial 线程)+ IPC 结果(主线程 ipc handler)。
enum UserEvent {
    Conn(bool, Option<String>),
    Resolve(u64, String),
}

/// 把 ui/ 三件套内联成一个自包含 HTML(占位处替换;免自定义协议)。
fn page() -> String {
    const HTML: &str = include_str!("../ui/index.html");
    const STYLE: &str = include_str!("../ui/style.css");
    const SCRIPT: &str = include_str!("../ui/app.js");
    const ALPINE: &str = include_str!("../ui/alpine.min.js"); // 内联 vendored Alpine(离线,无构建步骤)
    HTML.replacen("<!--STYLE-->", STYLE, 1)
        .replacen("<!--ALPINE-->", ALPINE, 1)
        .replacen("<!--SCRIPT-->", SCRIPT, 1)
}

/// 品牌环 "O" 的 RGBA(直 alpha):270° 弧、缺口朝下、圆头、stroke≈16%。
/// 与固件 loading 屏的 "O"、build.rs 生成的 exe .ico、配置窗左上 SVG 同一几何。
fn ring_rgba(s: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut px = vec![0u8; (s * s * 4) as usize];
    let cf = s as f32 / 2.0;
    let r_out = cf * 0.90; // 外半径留 ~10% 边距防裁切
    let stroke = s as f32 * 0.16;
    let r_mid = r_out - stroke / 2.0; // 环中线半径
    let half = stroke / 2.0;
    let cap = |deg: f32| {
        let a = deg.to_radians();
        (cf + r_mid * a.cos(), cf + r_mid * a.sin())
    };
    let (p1x, p1y) = cap(45.0); // 缺口右沿圆头
    let (p2x, p2y) = cap(135.0); // 缺口左沿圆头
    const SS: u32 = 4; // 4×4 超采样抗锯齿
    for y in 0..s {
        for x in 0..s {
            let mut cov = 0.0f32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    let (dx, dy) = (fx - cf, fy - cf);
                    let d = (dx * dx + dy * dy).sqrt();
                    let mut inside = false;
                    if (d - r_mid).abs() <= half {
                        let ang = dy.atan2(dx).to_degrees(); // 下=+90°
                        if !(ang > 45.0 && ang < 135.0) {
                            inside = true; // 环身(排除底部缺口)
                        }
                    }
                    if !inside {
                        let d1 = ((fx - p1x).powi(2) + (fy - p1y).powi(2)).sqrt();
                        let d2 = ((fx - p2x).powi(2) + (fy - p2y).powi(2)).sqrt();
                        inside = d1 <= half || d2 <= half; // 两端圆头
                    }
                    if inside {
                        cov += 1.0;
                    }
                }
            }
            cov /= (SS * SS) as f32;
            let i = ((y * s + x) * 4) as usize;
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = (cov * 255.0).round() as u8;
        }
    }
    px
}

/// 托盘图标 = 品牌环 "O":已连=琥珀(品牌色 #FFB44E)/ 未连=灰(暗淡)。
/// 形状即品牌,颜色携带连接态(原实心圆青/灰 → 阶段12 改环形)。
fn make_icon(connected: bool) -> Option<Icon> {
    const S: u32 = 32;
    let (r, g, b) = if connected { (0xFF, 0xB4, 0x4E) } else { (0x5C, 0x66, 0x75) };
    Icon::from_rgba(ring_rgba(S, r, g, b), S, S).ok()
}

/// 运行时窗口图标(标题栏 / 任务栏 / Alt-Tab)= 琥珀品牌环。
fn make_window_icon() -> Option<TaoIcon> {
    const S: u32 = 64;
    TaoIcon::from_rgba(ring_rgba(S, 0xFF, 0xB4, 0x4E), S, S).ok()
}

/// 处理一条 IPC 请求 → (请求 id, 结果 JSON 字符串)。配置界面命令集(状态/配置读写 + 只读查询)。
fn dispatch_ipc(shared: &Arc<HostShared>, body: &str) -> Option<(u64, String)> {
    let req: Value = serde_json::from_str(body).ok()?;
    let id = req.get("id").and_then(|v| v.as_u64())?;
    let method = req.get("method").and_then(|v| v.as_str())?;
    let args = req.get("args").and_then(|v| v.as_array());
    let arg0_str = args.and_then(|a| a.first()).and_then(|v| v.as_str()).unwrap_or("");
    let arg0_bool = args.and_then(|a| a.first()).and_then(|v| v.as_bool()).unwrap_or(false);
    let arg0_i64 = args
        .and_then(|a| a.first())
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0);

    let result: Value = match method {
        "get_status" => {
            let mut s = shared.status_json();
            if let Some(o) = s.as_object_mut() {
                o.insert("ok".into(), Value::Bool(true));
            }
            s
        }
        "get_config" => json!({
            "instance_mode": shared.instance_mode(),
            "autostart": autostart::is_enabled(),
            "fresh_sec": config::fresh_sec(),
            "temp_unit": if config::temp_fahrenheit() { "F" } else { "C" },
            "system_screen": config::system_screen(),
            "lang": config::lang(),
            "version": env!("CARGO_PKG_VERSION"),
            "ok": true,
        }),
        "set_instance_mode" => {
            shared.set_instance_mode_persist(arg0_str);
            json!({"instance_mode": shared.instance_mode(), "ok": true})
        }
        "set_autostart" => {
            let ok = autostart::sync(arg0_bool);
            json!({
                "autostart": autostart::is_enabled(),
                "ok": ok,
                "supported": autostart::is_supported(),
            })
        }
        "get_hook_status" => {
            let mut s = installer::hook_status();
            if let Some(o) = s.as_object_mut() {
                o.insert("ok".into(), Value::Bool(true));
            }
            s
        }
        "install_hook" => json!({
            "ok": true,
            "result": installer::do_install(),
            "status": installer::hook_status(),
        }),
        "reconnect" => {
            shared.request_reconnect();
            json!({"ok": true})
        }
        "set_fresh_sec" => {
            config::set_fresh_sec(arg0_i64);
            json!({"fresh_sec": config::fresh_sec(), "ok": true})
        }
        "set_temp_unit" => {
            config::set_temp_unit(arg0_str);
            json!({"temp_unit": if config::temp_fahrenheit() { "F" } else { "C" }, "ok": true})
        }
        "set_system_screen" => {
            config::set_system_screen(arg0_bool);
            json!({"system_screen": config::system_screen(), "ok": true})
        }
        "set_lang" => {
            config::set_lang(arg0_str);
            json!({"lang": config::lang(), "ok": true})
        }
        "get_instances" => shared.instances_json(),
        "get_sysmon" => shared.sysmon_json(),
        _ => json!({"ok": false, "error": "unknown method"}),
    };
    Some((id, result.to_string()))
}

fn msg_box(text: &str) {
    #[cfg(windows)]
    single_instance::message_box(text);
    #[cfg(not(windows))]
    crate::log::warn(text);
}

/// 打开配置窗(webview 在 → 显示;不在 → 提示降级)。menu「打开配置」与左键单击共用。
fn open_config(window: &tao::window::Window, has_webview: bool) {
    if has_webview {
        window.set_visible(true);
        window.set_focus();
        // 不强制还原尺寸:默认小窗由启动时 SW_HIDE 消费「首个 show 覆盖」保证;
        // 用户手动最大化后关窗回托盘、重开时沿用最大化态(maximizable=true)。
    } else {
        msg_box("配置界面不可用(缺 WebView2 运行时)。host 仍在后台运行。");
    }
}

/// 直发 Win32 ShowWindow(经 tao 暴露的原生 HWND),绕过 tao set_visible 的标志位 + 线程执行器时序
/// (其 MAXIMIZED 标志要等 WM_SIZE 泵到才同步,同步调 set_maximized 会与之竞态)。
#[cfg(windows)]
fn win_show(window: &tao::window::Window, cmd: i32) {
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow;
    unsafe {
        ShowWindow(window.hwnd() as *mut core::ffi::c_void, cmd);
    }
}

pub fn run_app() {
    #[cfg(windows)]
    if !single_instance::acquire() {
        single_instance::message_box("code_mate 已在运行(见任务栏托盘图标)。");
        return;
    }

    let mut host = Host::new();
    host.start();
    let shared = host.shared();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // 连接态 → 托盘变色(serial 线程经 proxy 投递 UserEvent::Conn)
    {
        let p = proxy.clone();
        host.add_listener(Box::new(move |c, port| {
            let _ = p.send_event(UserEvent::Conn(c, port));
        }));
    }

    let window = WindowBuilder::new()
        .with_title("code_mate 配置")
        .with_inner_size(LogicalSize::new(560.0, 640.0))
        .with_min_inner_size(LogicalSize::new(480.0, 560.0))
        .with_maximizable(true) // 允许最大化(默认小窗 560×640,用户可点最大化 / 标题栏双击)
        .with_window_icon(make_window_icon()) // 标题栏/任务栏/Alt-Tab = 琥珀品牌环
        .with_visible(false) // 隐藏启动,点托盘再 show
        .build(&event_loop)
        .expect("create window");

    // ⚠️ Windows:进程的**首个 ShowWindow 会忽略 nCmdShow、改用启动器 STARTUPINFO.wShowWindow**。
    // 某些双击/快捷方式启动会传 SW_SHOWMAXIMIZED → 首次点托盘 show 时把 560×640 顶成最大化。
    // 趁窗口仍隐藏,先空发一次 ShowWindow 把这个「首个 show 覆盖」消耗掉,之后 set_visible 才按本值显示(无闪)。
    #[cfg(windows)]
    win_show(&window, windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE);

    // WebView2 运行时缺失 → build 失败:降级为仅 host+托盘(不 panic;对齐 Python run_app)。
    let webview: Option<wry::WebView> = {
        let shared = shared.clone();
        let p = proxy.clone();
        let built = WebViewBuilder::new()
            .with_html(page())
            .with_background_color((11, 14, 20, 255))
            .with_ipc_handler(move |req| {
                if let Some((id, jsons)) = dispatch_ipc(&shared, req.body()) {
                    let _ = p.send_event(UserEvent::Resolve(id, jsons));
                }
            })
            .build(&window);
        match built {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn(format!("webview 创建失败(缺 WebView2 运行时?),降级为仅 host+托盘: {}", e));
                msg_box(
                    "配置界面不可用:缺 Microsoft Edge WebView2 运行时。\n\
                     host 仍在后台驱动设备。装好 WebView2 Runtime 后重启本程序即可。",
                );
                None
            }
        }
    };

    let menu_rx = MenuEvent::receiver();
    let tray_rx = TrayIconEvent::receiver();
    // ⚠️ 托盘必须在事件循环线程**运行起来后**(Init)建,否则 Windows 上图标不显示
    // (pystray 是另起线程跑自己的消息循环,故没这问题)。先占位 None,Init 时填。
    let mut tray: Option<TrayIcon> = None;

    // 闭包按值持有 host/window/webview/tray/shared(进程退出时 ControlFlow::Exit 直接结束)
    event_loop.run(move |event, _, control_flow| {
        // WaitUntil 周期轮询托盘事件(免去 set_event_handler 的 Send+Sync 之累);UserEvent 仍即时唤醒
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(150));

        if let Event::NewEvents(StartCause::Init) = event {
            let menu = Menu::new();
            let _ = menu.append_items(&[
                &MenuItem::with_id("open", "打开配置", true, None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("reconnect", "重新连接", true, None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("quit", "退出", true, None),
            ]);
            tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("code_mate(未连接)")
                .with_icon(make_icon(false).expect("tray icon"))
                .build()
                .ok();
        }

        while let Ok(ev) = menu_rx.try_recv() {
            match ev.id.0.as_str() {
                "open" => open_config(&window, webview.is_some()),
                "reconnect" => shared.request_reconnect(),
                "quit" => {
                    host.stop();
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            }
        }
        while let Ok(ev) = tray_rx.try_recv() {
            // 左键单击图标 → 打开配置(对齐 pystray default=True)
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                open_config(&window, webview.is_some());
            }
        }

        match event {
            Event::UserEvent(UserEvent::Conn(c, port)) => {
                if let Some(t) = &tray {
                    if let Some(icon) = make_icon(c) {
                        let _ = t.set_icon(Some(icon));
                    }
                    let tip = match (c, port) {
                        (true, Some(p)) => format!("code_mate(已连接 {})", p),
                        _ => "code_mate(未连接)".to_string(),
                    };
                    let _ = t.set_tooltip(Some(tip));
                }
            }
            Event::UserEvent(UserEvent::Resolve(id, jsons)) => {
                if let Some(w) = &webview {
                    let _ = w.evaluate_script(&format!("window.__vmResolve({},{});", id, jsons));
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                window.set_visible(false); // 关窗 = 回托盘,不退出
            }
            _ => {}
        }
    });
}

// ---------- 单实例 + 提示框(Windows)----------
#[cfg(windows)]
mod single_instance {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION};

    const ERROR_ALREADY_EXISTS: u32 = 183;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// true = 本进程是唯一实例(已持有 mutex);false = 已有实例在跑。
    pub fn acquire() -> bool {
        let name = wide("code_mate_singleton_mutex");
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if h.is_null() {
            return true; // 创建失败 → 放行(对齐 Python None)
        }
        // CreateMutexW 后紧跟 GetLastError(无中间 FFI 调用)→ 可靠;不 CloseHandle,持有至退出
        let err = unsafe { GetLastError() };
        err != ERROR_ALREADY_EXISTS
    }

    pub fn message_box(text: &str) {
        let t = wide(text);
        let title = wide("code_mate");
        unsafe {
            MessageBoxW(std::ptr::null_mut(), t.as_ptr(), title.as_ptr(), MB_ICONINFORMATION);
        }
    }
}
