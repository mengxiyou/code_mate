#pragma once
#include "app_types.h"

// LED 效果库(阶段5 引入,阶段8 重构为「布局拥有」)。GPIO38 上的 WS2812(板载 1 颗;
// 外接灯环改 board_config.h 的 CM_LED_COUNT)。⚠️ 字节序 RGB(非 GRB),见 leds.cpp。
//
// 阶段8:LED 是 UI 的一部分——**显示方式由布局决定**(各布局的 led(st,cc) 钩子里选效果),
// **控制(连接态 st + 工作态 cc_running)由数据源经主循环传入**;主 loop 不再含灯效策略。
void leds_begin();

// 切屏时来一记白色扫光(~250ms)。渲染覆盖在 leds_fill 内完成(闪光期间任何效果都被压成白)。
void leds_flash();

// ---- 低层:把颜色 c 按比例 k(0..1)铺满所有像素;内部 ~50Hz 节流 + 白扫光覆盖 ----
void leds_fill(uint32_t c, float k);

// ---- 现成效果(布局直接调用其一;封装颜色 + 动画)----
void leds_fx_connecting();   // 深蓝慢闪(2s 一次短脉冲):连接中 / 无会话(loading)
void leds_fx_idle();         // 深蓝常亮(柔和):已连未工作
void leds_fx_working();      // 6 色逐次呼吸(红/黄/绿/青/蓝/品红;每色一次、谷底切换):CC 工作中
void leds_fx_disk(uint8_t level);  // 阶段12:磁盘活动灯(system 屏)。级别 0..255 → 暖琥珀亮度+抖动;0=回柔和蓝
