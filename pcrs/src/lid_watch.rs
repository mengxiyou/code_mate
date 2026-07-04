//! 盒盖开/合监听 LidEventSource(对应 pc/lid_watch.py)。
//!
//! Windows:隐藏顶层窗口 + `RegisterPowerSettingNotification(GUID_LIDSWITCH_STATE_CHANGE)`,
//! 在独立线程跑消息循环;盒盖变化 → 向总线 emit LID_CLOSED / LID_OPENED。注册后会立刻回报一次
//! 当前状态(意外的好处)。⚠️ 仅「合盖不休眠」时本进程才存活、回调才有意义(休眠冻结整个进程)。
//! 非 Windows(Linux=logind / macOS=IOKit)留待实现 —— 优雅降级、不崩。
use crate::events::{EventBus, EventSource};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct LidEventSource {
    #[allow(dead_code)] // 非 Windows 上不用
    bus: Arc<EventBus>,
    tid: Arc<AtomicU32>, // 消息循环线程 id(0=未起;stop 用它 PostThreadMessage(WM_QUIT))
    handle: Option<JoinHandle<()>>,
}

impl LidEventSource {
    pub fn new(bus: Arc<EventBus>) -> Self {
        LidEventSource { bus, tid: Arc::new(AtomicU32::new(0)), handle: None }
    }
}

impl EventSource for LidEventSource {
    fn start(&mut self) {
        #[cfg(windows)]
        {
            // 全局 bus(供 C 回调 wndproc 读;单 Host 进程,set 一次即可)
            let _ = imp::BUS.set(self.bus.clone());
            let tid = self.tid.clone();
            self.handle = std::thread::Builder::new()
                .name("lid_watch".into())
                .spawn(move || imp::run(tid))
                .ok();
        }
    }

    fn stop(&mut self) {
        #[cfg(windows)]
        imp::post_quit(self.tid.load(std::sync::atomic::Ordering::SeqCst));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(windows)]
mod imp {
    use crate::events::{Event, EventBus, EventType};
    use std::ptr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, OnceLock};
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostThreadMessageW,
        RegisterClassW, TranslateMessage, MSG, WNDCLASSW,
    };

    const WM_POWERBROADCAST: u32 = 0x0218;
    const WM_QUIT: u32 = 0x0012;
    const PBT_POWERSETTINGCHANGE: u32 = 0x8013;
    const DEVICE_NOTIFY_WINDOW_HANDLE: u32 = 0;

    // GUID_LIDSWITCH_STATE_CHANGE {BA3E0F4D-B817-4094-A2D1-D56379E6A0F3};Data[0]:0=合 1=开
    const GUID_LID: GUID = GUID {
        data1: 0xBA3E_0F4D,
        data2: 0xB817,
        data3: 0x4094,
        data4: [0xA2, 0xD1, 0xD5, 0x63, 0x79, 0xE6, 0xA0, 0xF3],
    };

    pub(super) static BUS: OnceLock<Arc<EventBus>> = OnceLock::new();

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_POWERBROADCAST && wparam as u32 == PBT_POWERSETTINGCHANGE && lparam != 0 {
            let ps = &*(lparam as *const POWERBROADCAST_SETTING);
            if guid_eq(&ps.PowerSetting, &GUID_LID) {
                let closed = *ps.Data.as_ptr() == 0; // 0=合
                if let Some(bus) = BUS.get() {
                    let kind = if closed { EventType::LidClosed } else { EventType::LidOpened };
                    bus.publish(Event::new(kind, None));
                }
            }
            return 1;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn run(tid: Arc<AtomicU32>) {
        unsafe {
            tid.store(GetCurrentThreadId(), Ordering::SeqCst); // 先存 tid(stop 才能 PostThreadMessage)
            let hinst = GetModuleHandleW(ptr::null());
            let class_name = wide("code_mate_lidwatch");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinst;
            wc.lpszClassName = class_name.as_ptr();
            if RegisterClassW(&wc) == 0 {
                return;
            }
            let win_name = wide("code_lid");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                win_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                hinst,
                ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }
            // HWND 与 HANDLE 在 windows-sys 同为 *mut c_void,直接传
            RegisterPowerSettingNotification(hwnd, &GUID_LID, DEVICE_NOTIFY_WINDOW_HANDLE);
            let mut msg: MSG = std::mem::zeroed();
            loop {
                let r = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
                if r == 0 || r == -1 {
                    break; // WM_QUIT(0) 或错误(-1)
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    pub(super) fn post_quit(tid: u32) {
        if tid != 0 {
            unsafe {
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }
        }
    }
}
