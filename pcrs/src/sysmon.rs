//! 阶段12 #2 — 系统监控数据源(spike 阶段:验证采集 + 体积,暂不接 host)。
//! 全用 windows-sys 裸 API(无 COM、无新 crate):
//!   CPU%   = GetSystemTimes 两次差分((kernelΔ+userΔ-idleΔ)/(kernelΔ+userΔ);kernel 含 idle)
//!   RAM    = GlobalMemoryStatusEx(used=Total-Avail)
//!   磁盘    = PDH \PhysicalDisk(_Total)\Disk Bytes/sec(英文计数器,避中文 locale 坑)
//!   显存    = PDH \GPU Adapter Memory(*)\Dedicated Usage 求和(用量)
//!            + 注册表 {4d36e968…}\NNNN\HardwareInformation.qwMemorySize 取最大(容量)
//!   ⚠️ windows-sys 0.59 **不含 DXGI/COM**(Graphics 下无 Dxgi 模块),故显存改走 PDH+注册表
//!      —— 仍全程 windows-sys 裸 API,与 D3「不引 crate、守体积」一致。
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_FMT_LARGE,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;
// 阶段12:CPU 温度 = 蹭已加载的 WinRing0(体感助手等监控工具的驱动)经 SMN/PCI 读 AMD Tctl;
//   CPU 频率 = PDH。CreateFileW/DeviceIoControl/CloseHandle 全裸 FFI。
use std::ffi::c_void;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
use windows_sys::Win32::System::IO::DeviceIoControl;

#[derive(Default, Debug, Clone)]
pub struct SystemSnapshot {
    pub cpu_pct: f64,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub ram_pct: f64,
    pub vram_used_mb: u64,
    pub vram_total_mb: u64,
    pub vram_pct: f64,
    pub vram_ok: bool,
    pub disk_bps: f64,         // 字节/秒(原始;归一到 LED 级别在 sys_source 做)
    pub cpu_temp: Option<f64>, // CPU 温度 °C(WinRing0/体感助手在跑才有;否则 None)
    pub cpu_ghz: f64,          // 当前 CPU 频率 GHz(无温度时显示它)
}

fn ft_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 持续采样器:CPU/磁盘是速率量,需保留上次状态做差分;显存容量读一次缓存。
pub struct SysMonitor {
    prev_idle: u64,
    prev_kernel: u64,
    prev_user: u64,
    have_prev: bool,
    pdh_query: isize, // PDH_HQUERY(windows-sys 用 isize)
    pdh_disk: isize,  // PDH_HCOUNTER:磁盘字节/秒
    pdh_gpu: isize,   // PDH_HCOUNTER:GPU 显存用量(通配多实例)
    pdh_ok: bool,     // query 已开
    disk_ok: bool,
    gpu_ok: bool,
    vram_cap: u64,       // 显存容量字节(注册表,缓存)
    vram_cap_tried: bool,
    wr0: isize,      // WinRing0 设备句柄(存 isize 保持 Send;0=未开 / 体感助手没跑)
    pdh_perf: isize, // PDH:% Processor Performance(算当前频率)
    perf_ok: bool,
    base_mhz: u32,   // CPU 基准频率 MHz(注册表 ~MHz,缓存)
}

impl SysMonitor {
    pub fn new() -> Self {
        let mut m = SysMonitor {
            prev_idle: 0,
            prev_kernel: 0,
            prev_user: 0,
            have_prev: false,
            pdh_query: 0,
            pdh_disk: 0,
            pdh_gpu: 0,
            pdh_ok: false,
            disk_ok: false,
            gpu_ok: false,
            vram_cap: 0,
            vram_cap_tried: false,
            wr0: 0,
            pdh_perf: 0,
            perf_ok: false,
            base_mhz: read_base_mhz(),
        };
        m.pdh_init();
        m.sample_cpu(); // 建 CPU 基线(首帧返回 0)
        m
    }

    fn pdh_init(&mut self) {
        unsafe {
            let mut q: isize = 0;
            if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                return;
            }
            let dp = wide("\\PhysicalDisk(_Total)\\Disk Bytes/sec");
            let mut dc: isize = 0;
            self.disk_ok = PdhAddEnglishCounterW(q, dp.as_ptr(), 0, &mut dc) == 0;
            let gp = wide("\\GPU Adapter Memory(*)\\Dedicated Usage");
            let mut gc: isize = 0;
            self.gpu_ok = PdhAddEnglishCounterW(q, gp.as_ptr(), 0, &mut gc) == 0;
            let pp = wide("\\Processor Information(_Total)\\% Processor Performance");
            let mut pc: isize = 0;
            self.perf_ok = PdhAddEnglishCounterW(q, pp.as_ptr(), 0, &mut pc) == 0;
            if !self.disk_ok && !self.gpu_ok && !self.perf_ok {
                PdhCloseQuery(q);
                return;
            }
            PdhCollectQueryData(q); // 速率计数器首次 collect 作基线
            self.pdh_query = q;
            self.pdh_disk = dc;
            self.pdh_gpu = gc;
            self.pdh_perf = pc;
            self.pdh_ok = true;
        }
    }

    fn sample_cpu(&mut self) -> f64 {
        unsafe {
            let mut idle: FILETIME = zeroed();
            let mut kernel: FILETIME = zeroed();
            let mut user: FILETIME = zeroed();
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
                return 0.0;
            }
            let (i, k, u) = (ft_u64(&idle), ft_u64(&kernel), ft_u64(&user));
            if !self.have_prev {
                self.prev_idle = i;
                self.prev_kernel = k;
                self.prev_user = u;
                self.have_prev = true;
                return 0.0;
            }
            let di = i.saturating_sub(self.prev_idle);
            let dk = k.saturating_sub(self.prev_kernel);
            let du = u.saturating_sub(self.prev_user);
            self.prev_idle = i;
            self.prev_kernel = k;
            self.prev_user = u;
            let total = dk + du; // kernel 已含 idle
            if total == 0 {
                return 0.0;
            }
            let busy = total.saturating_sub(di);
            (busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
        }
    }

    /// 读磁盘字节/秒(format only;collect 由 sample 统一做)。
    fn read_disk(&self) -> f64 {
        unsafe {
            let mut val: PDH_FMT_COUNTERVALUE = zeroed();
            if PdhGetFormattedCounterValue(self.pdh_disk, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut val)
                != 0
            {
                return 0.0;
            }
            val.Anonymous.doubleValue.max(0.0)
        }
    }

    /// 读 GPU 显存用量:通配实例数组求和(各物理 GPU 的 Dedicated Usage)。
    fn read_gpu_usage(&self) -> Option<u64> {
        unsafe {
            let mut size: u32 = 0;
            let mut count: u32 = 0;
            // 首调:size=0/buf=null → 回 PDH_MORE_DATA 并填 size
            PdhGetFormattedCounterArrayW(
                self.pdh_gpu,
                PDH_FMT_LARGE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            );
            if size == 0 {
                return None;
            }
            let item = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let n = (size as usize).div_ceil(item).max(1);
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = Vec::with_capacity(n);
            if PdhGetFormattedCounterArrayW(
                self.pdh_gpu,
                PDH_FMT_LARGE,
                &mut size,
                &mut count,
                buf.as_mut_ptr(),
            ) != 0
            {
                return None;
            }
            let items = std::slice::from_raw_parts(buf.as_ptr(), count as usize);
            let mut total: u64 = 0;
            for it in items {
                let v = it.FmtValue.Anonymous.largeValue;
                if v > 0 {
                    total += v as u64;
                }
            }
            Some(total)
        }
    }

    /// 显存容量(注册表,读一次缓存)。
    fn vram_capacity(&mut self) -> Option<u64> {
        if !self.vram_cap_tried {
            self.vram_cap_tried = true;
            self.vram_cap = read_gpu_capacity_registry().unwrap_or(0);
        }
        (self.vram_cap > 0).then_some(self.vram_cap)
    }

    /// 当前 CPU 频率 GHz = 基准 × %Processor Performance/100(turbo 会 >100%)。
    fn read_cpu_speed(&self) -> f64 {
        if !self.perf_ok || self.base_mhz == 0 {
            return 0.0;
        }
        unsafe {
            let mut val: PDH_FMT_COUNTERVALUE = zeroed();
            if PdhGetFormattedCounterValue(self.pdh_perf, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut val) != 0 {
                return 0.0;
            }
            (self.base_mhz as f64 * val.Anonymous.doubleValue / 100.0 / 1000.0).max(0.0)
        }
    }

    /// CPU 温度 °C:蹭已加载的 WinRing0(体感助手在跑)经 SMN(PCI 0:0:0 的 0x60/0x64)读 AMD Tctl。
    /// 体感助手没跑 → WinRing0 设备不存在 → 返回 None(上层回退显示频率)。
    fn read_cpu_temp(&mut self) -> Option<f64> {
        if self.wr0 == 0 {
            self.wr0 = open_winring0(); // 0 = 没有
            if self.wr0 == 0 {
                return None;
            }
        }
        let h = self.wr0 as *mut c_void;
        // SMN:写温度寄存器地址 0x00059800 到 index(0x60),读 data(0x64)
        if !wr0_write_pci(h, 0, 0x60, 0x0005_9800) {
            self.close_wr0(); // 句柄失效(体感助手关了)→ 关 + 下次重开
            return None;
        }
        let v = match wr0_read_pci(h, 0, 0x64) {
            Some(v) => v,
            None => {
                self.close_wr0();
                return None;
            }
        };
        let mut t = (v >> 21) as f64 * 0.125;
        if v & 0x8_0000 != 0 {
            t -= 49.0; // bit19 CurTmpRangeSel(扩展量程)
        }
        (5.0..=115.0).contains(&t).then_some(t) // 合理性过滤:防与体感助手竞争 SMN 读到的垃圾值
    }

    fn close_wr0(&mut self) {
        if self.wr0 != 0 {
            unsafe { CloseHandle(self.wr0 as *mut c_void) };
            self.wr0 = 0;
        }
    }

    pub fn sample(&mut self) -> SystemSnapshot {
        let mut s = SystemSnapshot { cpu_pct: self.sample_cpu(), ..Default::default() };
        unsafe {
            let mut ms: MEMORYSTATUSEX = zeroed();
            ms.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut ms) != 0 {
                s.ram_total_mb = ms.ullTotalPhys / (1024 * 1024);
                let used = ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys);
                s.ram_used_mb = used / (1024 * 1024);
                if ms.ullTotalPhys > 0 {
                    s.ram_pct = used as f64 / ms.ullTotalPhys as f64 * 100.0;
                }
            }
        }
        // PDH:单次 collect 同时喂磁盘 + 显存
        if self.pdh_ok {
            unsafe {
                PdhCollectQueryData(self.pdh_query);
            }
            if self.disk_ok {
                s.disk_bps = self.read_disk();
            }
            if self.gpu_ok {
                if let Some(used) = self.read_gpu_usage() {
                    s.vram_used_mb = used / (1024 * 1024);
                    if let Some(cap) = self.vram_capacity() {
                        s.vram_total_mb = cap / (1024 * 1024);
                        s.vram_pct = used as f64 / cap as f64 * 100.0;
                        s.vram_ok = true;
                    }
                }
            }
        }
        // CPU 温度(WinRing0 在 → Some)+ 频率(总是算,作无温度时回退)
        s.cpu_temp = self.read_cpu_temp();
        s.cpu_ghz = self.read_cpu_speed();
        s
    }
}

impl Drop for SysMonitor {
    fn drop(&mut self) {
        self.close_wr0();
        if self.pdh_ok {
            unsafe {
                PdhCloseQuery(self.pdh_query);
            }
        }
    }
}

// ---------- WinRing0(蹭已加载的内核驱动)+ 注册表基准频率 ----------

const IOCTL_OLS_READ_PCI_CONFIG: u32 = 0x9C40_6144; // CTL_CODE(40000,0x851,BUFFERED,READ)
const IOCTL_OLS_WRITE_PCI_CONFIG: u32 = 0x9C40_A148; // CTL_CODE(40000,0x852,BUFFERED,WRITE)

/// 打开已加载的 WinRing0 设备(多个版本名挨个试);0 = 没有(体感助手/监控工具没在跑)。
fn open_winring0() -> isize {
    for name in ["\\\\.\\WinRing0_1_2_0", "\\\\.\\WinRing0_1_3_0", "\\\\.\\WinRing0"] {
        let w = wide(name);
        let h = unsafe {
            CreateFileW(w.as_ptr(), 0xC000_0000, 0, std::ptr::null(), 3, 0, std::ptr::null_mut())
        };
        if h as isize != -1 {
            return h as isize;
        }
    }
    0
}

fn wr0_write_pci(h: *mut c_void, pci: u32, off: u32, val: u32) -> bool {
    let mut inb = [0u8; 12];
    inb[0..4].copy_from_slice(&pci.to_le_bytes());
    inb[4..8].copy_from_slice(&off.to_le_bytes());
    inb[8..12].copy_from_slice(&val.to_le_bytes());
    let mut ret = 0u32;
    unsafe {
        DeviceIoControl(h, IOCTL_OLS_WRITE_PCI_CONFIG, inb.as_ptr() as *const c_void, 12, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut()) != 0
    }
}

fn wr0_read_pci(h: *mut c_void, pci: u32, off: u32) -> Option<u32> {
    let mut inb = [0u8; 8];
    inb[0..4].copy_from_slice(&pci.to_le_bytes());
    inb[4..8].copy_from_slice(&off.to_le_bytes());
    let mut outb = [0u8; 4];
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(h, IOCTL_OLS_READ_PCI_CONFIG, inb.as_ptr() as *const c_void, 8, outb.as_mut_ptr() as *mut c_void, 4, &mut ret, std::ptr::null_mut()) != 0
    };
    ok.then(|| u32::from_le_bytes(outb))
}

/// CPU 基准频率 MHz(注册表 ~MHz,REG_DWORD)。
fn read_base_mhz() -> u32 {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    unsafe {
        let path = wide("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0");
        let mut k: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut k) != 0 {
            return 0;
        }
        let val = wide("~MHz");
        let mut data: u32 = 0;
        let mut sz: u32 = 4;
        let r = RegQueryValueExW(
            k,
            val.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut u8,
            &mut sz,
        );
        RegCloseKey(k);
        if r == 0 {
            data
        } else {
            0
        }
    }
}

/// 枚举显卡类 {4d36e968-e325-11ce-bfc1-08002be10318} 各子键的 qwMemorySize,取最大(=主独显容量)。
fn read_gpu_capacity_registry() -> Option<u64> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };
    unsafe {
        let path = wide("SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}");
        let mut root: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut root) != 0 {
            return None;
        }
        let mut best: u64 = 0;
        let mut idx: u32 = 0;
        loop {
            let mut name = [0u16; 256];
            let mut nlen = name.len() as u32;
            let r = RegEnumKeyExW(
                root,
                idx,
                name.as_mut_ptr(),
                &mut nlen,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if r != 0 {
                break; // ERROR_NO_MORE_ITEMS
            }
            idx += 1;
            let mut sub: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(root, name.as_ptr(), 0, KEY_READ, &mut sub) != 0 {
                continue;
            }
            let val = wide("HardwareInformation.qwMemorySize");
            let mut data: u64 = 0;
            let mut dsize: u32 = 8;
            let q = RegQueryValueExW(
                sub,
                val.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut data as *mut u64 as *mut u8,
                &mut dsize,
            );
            RegCloseKey(sub);
            if q == 0 && data > best {
                best = data;
            }
        }
        RegCloseKey(root);
        (best > 0).then_some(best)
    }
}
