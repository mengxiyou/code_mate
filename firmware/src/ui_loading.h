#pragma once
// code_mate — loading 布局(阶段7):全屏「CODE MATE 字标 + 消息」,通用占位屏。
//   字标里只有 "O" 是缺口朝下的琥珀环形(品牌标记,兼作 PC 端托盘/exe/配置窗图标);
//   其余字母("C" + "DE MATE")用主文字色、同字体(阶段12)。细节见 ui_loading.cpp。
// 未连 host(开机/重连)时显示「等待条」动画(三条矩形,高亮轮流跳动);
// host 若经 cfg.msg 下发文案则改显文字(现行 host 连上即切走、不再发 loading 文案)。
#include <lvgl.h>
#include "layout.h"

Layout *ui_loading_layout();   // 返回该布局注册项(供 main 注册表)
void ui_loading_show_bars(bool show);   // 揭出/隐藏开机等待条(splash 满 3s 后由 main 调)
