#pragma once
// code_mate — U盘(安装盘)屏。统一 Layout,debug/release 共用一份视觉:
//   - debug:纯 UI 预览(选「U-Disk」只切到此屏,**不真正起 U 盘**,方便 HWCDC 自动烧录迭代)。
//   - release(CM_USB_DISK):真 MSC 时的显示屏(背后跑 USBMSC 只读暴露 storage)。
// 此屏长按 BOOT 可弹模式菜单(见 ui_modesel);LED 品红区分色。
#include "layout.h"

Layout *ui_udisk_layout();
