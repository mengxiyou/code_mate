#include "ui_text_scroll.h"
#include "ui_common.h"
#include "leds.h"        // 阶段8:LED 灯效由本布局选择(跟随所选会话工作态)
#include <lvgl.h>

LV_FONT_DECLARE(font_cjk16);   // 思源黑体 16px(src/fonts/font_cjk16.c;自带正确度量,不再 hack line_height)

static lv_obj_t *s_cont;       // 可滚文本容器(spangroup 块逐个追加)

#define MAX_BLOCKS 24          // 封顶块数:超出删最老。每块=1 spangroup(≤12 span,各含文本+样式堆),
                               // 与两块屏控件共用 LVGL 堆(LV_MEM_SIZE);设小留足余量,防长合盖累积 OOM(OOM=assert 硬死机)

// markdown run 样式 → 颜色(复用 HUD 配色)
static uint32_t color_for(char s) {
  switch (s) {
    case 'h': return C_CORAL;   // 标题
    case 'b': return C_AMBER;   // 强调(高亮黄橙,醒目)
    case 'c': return C_CYAN;    // 代码
    case 'u': return C_AMBER;   // 列表
    case 'd': return C_TEXT2;   // 行首小圆点:与正文同色
    default:  return C_TEXT2;   // 正文
  }
}

// ---- 逐行顿挫滚屏:每隔一拍把内容快速上滚一行(单行快、行间停顿),且每拍只滚一行(不随内容量加速)----
#define LINE_H           22        // 一行高(= font_cjk16 line_height)
#define STEP_INTERVAL_MS 340       // 每行节拍(快速上滚 + 停顿);拉大=行间更慢、顿挫更明显
#define STEP_SNAP_MS     90        // 单行快速上滚的时长(顿挫的"进",越小越脆)
static lv_timer_t *s_scroll_timer;
static int32_t s_step_acc;         // 本次单行 snap 已滚量(增量驱动 scroll_by)

static void step_exec(void *var, int32_t v) {
  int32_t d = v - s_step_acc;
  s_step_acc = v;
  if (d) lv_obj_scroll_by((lv_obj_t *)var, 0, -d, LV_ANIM_OFF);  // 负 = 上滚露出底部最新
}

static void scroll_tick(lv_timer_t *t) {
  (void)t;
  if (!s_cont) return;
  int32_t sb = lv_obj_get_scroll_bottom(s_cont);
  if (sb <= 0) return;                              // 已到底 → 本拍停顿,不滚
  int32_t step = sb < LINE_H ? sb : LINE_H;         // 这一拍只快速上滚「一行」
  s_step_acc = 0;
  lv_anim_t a;
  lv_anim_init(&a);
  lv_anim_set_var(&a, s_cont);
  lv_anim_set_exec_cb(&a, step_exec);
  lv_anim_set_values(&a, 0, step);
  lv_anim_set_duration(&a, STEP_SNAP_MS);           // 单行快速上滚(快)
  lv_anim_set_path_cb(&a, lv_anim_path_ease_out);
  lv_anim_start(&a);
}

void ui_text_build(lv_obj_t *scr) {
  lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(scr, lv_color_hex(C_BG), 0);
  lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(scr, 0, 0);

  // 文本容器**恰好 7 行高(154=7×22)**,定位在 y=9 → 屏上下各 9px 黑边(留白在容器**外**)。
  // 关键:盒子边界 = 行边界,LVGL 裁剪到盒子时正好卡在整行上 → 留白区不会露出半行(第 8 行)。
  s_cont = lv_obj_create(scr);
  lv_obj_remove_style_all(s_cont);
  lv_obj_set_size(s_cont, 320, 154);
  lv_obj_align(s_cont, LV_ALIGN_TOP_LEFT, 0, 9);
  lv_obj_set_style_pad_left(s_cont, 8, 0);
  lv_obj_set_style_pad_right(s_cont, 8, 0);
  lv_obj_set_style_pad_top(s_cont, 0, 0);      // 容器内 0 留白(留白在容器外的屏黑边),裁剪卡行边界
  lv_obj_set_style_pad_bottom(s_cont, 0, 0);
  lv_obj_set_style_pad_row(s_cont, 0, 0);      // 块间无间隙,保持行网格
  lv_obj_set_flex_flow(s_cont, LV_FLEX_FLOW_COLUMN);
  lv_obj_set_scroll_dir(s_cont, LV_DIR_VER);
  lv_obj_set_scrollbar_mode(s_cont, LV_SCROLLBAR_MODE_OFF);

  if (!s_scroll_timer) s_scroll_timer = lv_timer_create(scroll_tick, STEP_INTERVAL_MS, NULL);
}

void ui_text_clear() {
  if (s_cont) lv_obj_clean(s_cont);
}

void ui_text_add_block(const TextPayload &p) {
  if (!s_cont) return;
  if (p.clear) {
    ui_text_clear();
    lv_anim_delete(s_cont, step_exec);            // 停掉可能在途的滚动动画,免污染新内容起始位置
    s_step_acc = 0;
    lv_obj_scroll_to_y(s_cont, 0, LV_ANIM_OFF);   // 回到顶部:先填满首屏 7 行,再逐行滚出后面几行
  }
  if (p.n <= 0) return;

  lv_obj_t *sg = lv_spangroup_create(s_cont);
  lv_obj_set_width(sg, lv_pct(100));
  lv_spangroup_set_mode(sg, LV_SPAN_MODE_BREAK);          // 满宽自动换行
  lv_obj_set_style_text_font(sg, &font_cjk16, 0);
  for (int i = 0; i < p.n && i < TEXT_RUN_MAX; i++) {
    lv_span_t *sp = lv_spangroup_add_span(sg);
    lv_span_set_text(sp, p.runs[i].text);
    lv_style_t *st = lv_span_get_style(sp);
    lv_style_set_text_color(st, lv_color_hex(color_for(p.runs[i].style)));
    lv_style_set_text_font(st, &font_cjk16);
  }
  lv_spangroup_refresh(sg);

  // 封顶:超出删最老块(logo 不在此容器内,不受影响)
  while (lv_obj_get_child_count(s_cont) > MAX_BLOCKS) {
    lv_obj_delete(lv_obj_get_child(s_cont, 0));
  }
  // 滚动交给常驻定速 timer(scroll_tick):始终以恒定速度滚向底部,不随内容多少变快
  lv_obj_update_layout(s_cont);
}

// ---- 布局注册项(阶段7):把上面的函数封进统一 vtable ----
// init 参数忽略:文本屏的「init(清屏+回填)vs refresh(追加)」由 payload 的 clear 字段表达。
static void term_set_data(const void *p, bool init) { (void)init; ui_text_add_block(*(const TextPayload *)p); }

// 阶段8:文本屏 LED 与仪表盘一致 —— 跟随所选会话工作态(未连/断流→蓝闪;空闲→蓝常亮;工作→6 色呼吸)
static void term_led(DeviceState st, bool cc_running, uint8_t) {
  if (st != ST_LIVE)    leds_fx_connecting();
  else if (!cc_running) leds_fx_idle();
  else                  leds_fx_working();
}

Layout *ui_text_scroll_layout() {
  static Layout L = {
    "terminal",      // 布局 id(阶段7 步骤2:与协议线名一起从 "text" 改为 "terminal")
    nullptr,         // scr:由 main 创建后回填
    ui_text_build,
    term_set_data,
    nullptr,         // set_state:文本屏不显示连接态
    nullptr,         // on_enter:不在此清屏。清屏由 host 的 clear 帧驱动(进/切终端时 host 必发 clear);
                     //   若此处清屏,「终端→菜单→选 Normal 返回」会清成黑屏(本地返回无 host 重发文字)。
    nullptr,         // on_exit
    nullptr,         // tick:滚动由内部常驻 lv_timer 驱动,无需 1Hz tick
    term_led,        // led(阶段8):跟随会话工作态
  };
  return &L;
}
