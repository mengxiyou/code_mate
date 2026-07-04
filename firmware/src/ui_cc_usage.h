#pragma once
// code_mate — cc_usage 屏(LVGL v9.5)。布局/配色见 CLAUDE.md §6。
#include <lvgl.h>
#include "app_types.h"
#include "layout.h"

void ui_build(lv_obj_t *scr);                          // 在指定屏上搭好所有控件(初始全 "--")
void ui_set_payload(const UsagePayload &p, bool init); // 把一帧数据推进控件 + 设置倒计时基准;init=首帧/换实例→0增长动画
void ui_set_state(DeviceState st);        // 切状态(STALE 变暗;连接态由 loading 布局体现)
void ui_tick();                           // 约 1Hz:本地倒计时

Layout *ui_cc_usage_layout();             // 阶段7:返回该布局的注册项(供 main 注册表)
