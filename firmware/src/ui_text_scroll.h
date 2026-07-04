#pragma once
// code_mate — 文本滚动屏(阶段6):logo 右上 + 满屏可滚文本,按 markdown run 分色。
#include <lvgl.h>
#include "app_types.h"
#include "layout.h"

void ui_text_build(lv_obj_t *scr);           // 在指定屏上搭文本滚动屏
void ui_text_clear();                         // 清空所有文本块
void ui_text_add_block(const TextPayload &p); // 追加一段带样式文本(p.clear 则先清),自动滚到底

Layout *ui_text_scroll_layout();              // 阶段7:返回该布局的注册项(供 main 注册表)
