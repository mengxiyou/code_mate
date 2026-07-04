// 阶段10/11:双模式 USB。NVS 持久化模式(两套 env 共用)+ 只读 U 盘 MSC(仅 release)。
//   模式状态存 NVS,开机读取进入上次保存的模式(mode_is_udisk 不清零、持久)。
//   ⚠️ debug 与 release 逻辑一致(存 NVS、重启、U盘屏、按键响应),唯一区别:只有 release 真起 USBMSC。
#include "usb_disk.h"
#include <Arduino.h>
#include <Preferences.h>

static const char *NVS_NS  = "code_mate";
static const char *NVS_KEY = "boot_msc";

// 读 NVS 持久化模式:==1(U-Disk)返回 true,否则 false(Normal/CDC)。**不清零**(持久)。两套 env 共用。
bool mode_is_udisk() {
  Preferences p;
  if (!p.begin(NVS_NS, false)) return false;
  uint8_t v = p.getUChar(NVS_KEY, 0);
  p.end();
  return v == 1;
}

// 持久化保存模式(菜单长按生效时调:0=Normal，1=U-Disk),随后由调用方按需 esp_restart。两套 env 共用。
void mode_save(uint8_t mode) {
  Preferences p;
  if (p.begin(NVS_NS, false)) {
    p.putUChar(NVS_KEY, mode ? 1 : 0);
    p.end();
  }
}

// ===== 以下仅 release 编译:真 USBMSC(ARDUINO_USB_MODE=0 / TinyUSB-OTG)=====
#ifdef CM_USB_DISK
#include "USB.h"
#include "USBMSC.h"
#include "esp_partition.h"
#include "wear_levelling.h"   // storage 是 ESP-IDF WL-FAT(buildfs 产物含磨损均衡层),须经 wl_read 读

static USBMSC s_msc;
static const esp_partition_t *s_part = nullptr;
static wl_handle_t s_wl = WL_INVALID_HANDLE;
static uint32_t s_sec = 4096;           // WL 扇区大小 = MSC 块大小(FAT 内部 4096B/扇区)

// ---- MSC 读/写回调:经 WL 层读 storage(磨损均衡映射)→ 主机看到干净的 FAT;只读拒绝写 ----
static int32_t on_read(uint32_t lba, uint32_t offset, void *buffer, uint32_t bufsize) {
  if (s_wl == WL_INVALID_HANDLE) return -1;
  size_t addr = (size_t)lba * s_sec + offset;
  if (wl_read(s_wl, addr, buffer, bufsize) != ESP_OK) return -1;
  return (int32_t)bufsize;
}

static int32_t on_write(uint32_t /*lba*/, uint32_t /*offset*/, uint8_t * /*buffer*/, uint32_t /*bufsize*/) {
  return -1;   // 只读:拒绝写(报写保护)
}

// ---- 进真 MSC(只读 U 盘)----  U盘屏视觉由 ui_udisk 布局负责;这里只起 USB。
void msc_usb_begin() {
  // storage 分区(partitions_release.csv:data/fat)→ wl_mount 取磨损均衡句柄,MSC 经 wl_read 只读暴露
  s_part = esp_partition_find_first(
      ESP_PARTITION_TYPE_DATA, (esp_partition_subtype_t)ESP_PARTITION_SUBTYPE_DATA_FAT, "storage");
  uint32_t blocks = 0;
  if (s_part && wl_mount(s_part, &s_wl) == ESP_OK) {
    s_sec  = (uint32_t)wl_sector_size(s_wl);   // 通常 4096(= FAT 内部扇区)
    blocks = (uint32_t)(wl_size(s_wl) / s_sec);
  }

  s_msc.vendorID("codemate");
  s_msc.productID("Installer");
  s_msc.productRevision("1.0");
  s_msc.onRead(on_read);
  s_msc.onWrite(on_write);   // 拒绝 → 只读盘
  s_msc.mediaPresent(blocks > 0);
  s_msc.begin(blocks, (uint16_t)s_sec);   // 块大小 = WL 扇区(4096),与 FAT 扇区一致
  USB.begin();
}
#endif
