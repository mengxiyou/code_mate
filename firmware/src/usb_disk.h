#pragma once
#include <stdint.h>   // uint8_t(mode_save 形参)
// 阶段10/11:双模式 USB。NVS 持久化模式(两套 env 共用)+ 只读 U 盘 MSC(仅 release)。
//
// 模式状态持久化在 NVS:长按 BOOT 弹模式菜单(ui_modesel),选「U-Disk」→ mode_save(1) + 重启;
// 开机读 NVS(mode_is_udisk,**不清零**)进入上次保存的模式,跨复位/拔插保持,
// 直到菜单切回 Normal(mode_save(0) + 重启)。
// ⚠️ debug 与 release 逻辑一致(存 NVS、重启、U盘屏、按键响应),唯一区别:只有 release 真起 USBMSC。
// U盘屏视觉由 ui_udisk 布局负责(debug/release 共用一份)。

// NVS 持久化模式(两套 env 共用):
bool mode_is_udisk();          // 读:保存的是 U-Disk 返回 true。**不清零**(持久)。
void mode_save(uint8_t mode);  // 写:0=Normal / 1=U-Disk;调用方随后按需 esp_restart。

// 真 MSC(仅 release):wl_mount + USBMSC 只读暴露 storage + USB.begin。不绘制、不碰 LED。
#ifdef CM_USB_DISK
void msc_usb_begin();
#endif
