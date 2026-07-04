#pragma once
// code_mate — 模式选择界面(阶段11 #1)。长按 BOOT 任意态(含 U盘模式,#4)弹出:黑底两行英文模式、
// 选中=亮白/未选中=暗白;该屏短按 BOOT 切换选中、长按 BOOT 生效。
// **纯 UI,debug/release 共用**(实际「进 MSC」只在 release 真做,见 main.cpp 的 #ifdef CM_USB_DISK)。
// 用统一框架(Layout 注册表)扩展——只是又一个布局。
#include "layout.h"

Layout *ui_modesel_layout();
void modesel_set_current(int cur);  // 弹菜单前设「当前模式」(0=Normal/1=U-Disk):on_enter 默认高亮它
void modesel_next();       // 短按:切换选中项(环绕)+ 带变大变小动画
int  modesel_selected();   // 当前选中:0=正常模式(CDC) / 1=U盘安装模式(MSC/预览)
