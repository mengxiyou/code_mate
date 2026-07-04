#pragma once
// code_mate — 布局抽象(阶段7)。一套渲染模板 + 它消费的视图模型(payload schema)。
// 设备侧用「布局注册表」管理多块屏:main 按布局 id 查表派发数据/切屏,
// 彻底取代写死的两屏指针 + 字符串 if-else。布局与数据源无关——只认 payload schema。
#include <lvgl.h>
#include "app_types.h"

struct Layout {
  const char *id;                                    // 布局 id("dashboard"/"terminal")
  lv_obj_t   *scr;                                   // 该布局的屏对象(setup 时建,父=NULL)
  void (*build)(lv_obj_t *scr);                      // 在 scr 上搭控件
  void (*set_data)(const void *payload, bool init);  // 推一帧视图模型(各布局内部 cast 回各自 payload)
                                                     //   init=绑定新实例/首帧 → 0增长动画;否则 tween 刷新
  void (*set_state)(DeviceState st);                 // 连接态(STALE 变暗);可 NULL
  void (*on_enter)(void);                            // 切入此布局(reveal / 清屏 / 复位滚动);可 NULL
  void (*on_exit)(void);                             // 切出此布局;可 NULL
  void (*tick)(void);                                // ~1Hz(倒计时 / 维护);可 NULL
  void (*led)(DeviceState st, bool cc_running, uint8_t disk_lvl); // 板载 LED:显示由布局定;阶段12 加磁盘级别(仅 system 屏用),可 NULL
};
