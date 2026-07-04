#include "ui_loading.h"
#include "ui_common.h"
#include "leds.h"        // 阶段8:loading 自己的 LED 灯效
#include "protocol.h"   // protocol_cfg_msg():cfg 可携带的消息
#include <lvgl.h>
#include <string.h>

// 第一排是品牌字标 "CODE MATE"(全大写),做成「呼应仪表盘」的视觉:
//   · "O" 用一个真·环形弧(lv_arc,缺口在下,与 dashboard 上下文环 bg_angles(135,45) 同款)
//     替代普通字母,即「把 O 下面去掉」模拟环形仪表盘 —— 这是唯一的品牌标记
//     (同款琥珀环也用作 PC 端托盘 / exe / 配置窗图标,见 pcrs);
//   · "C" 与 "DE MATE" 全部用主文字色(C_TEXT)、同字体,让琥珀环 O 成为唯一视觉焦点。
//   · 阶段12:原 "C" 也用琥珀,现改为与 "DE MATE" 同色,只保留 O 的设计。
// 第二排默认是「等待动画」:三条白色矩形条,高亮在三条间轮流跳动制造「连接中」的等待感
//   (取代旧的 "Connecting..." 文字)。host 若经 cfg.msg 下发文案(如占位提示)则改显文字、停动画。
//   现行 host 连上即切到 system/dashboard/terminal、不再发 loading 文案 → 实际恒为等待条。
static lv_obj_t *s_lbl_msg;                 // 文字行(默认隐藏;仅 cfg.msg 非空时显示)
static char s_msg[64] = "";                 // host 经 cfg.msg 改写;空 = 走等待条动画

// ---- 等待条:三条矩形,高亮(白)在三条间轮流,其余灰 ----
#define BAR_COUNT     3
#define BAR_W         28          // 条宽(长方形:宽 > 高)
#define BAR_H         7           // 条高
#define BAR_GAP       8           // 条间距
#define BAR_PERIOD_MS 1000        // 高亮每隔多久跳到下一条(越大越慢)
#define BAR_HI        C_TEXT      // 高亮色(近白)
#define BAR_LO        0x39414E    // 灰(暗淡,= logo 水印色)
static lv_obj_t  *s_bars_row;               // 三条的容器(text 模式时隐藏)
static lv_obj_t  *s_bars[BAR_COUNT];
static lv_timer_t *s_bar_timer;
static int         s_bar_phase;             // 当前高亮的条序号
static lv_obj_t  *s_scr;                     // 本布局屏对象(timer 判活用)

// 仅当 loading 是当前屏且等待条可见时,推进高亮(其余情况免做无谓重绘)
static void bar_tick(lv_timer_t *) {
  if (!s_bars_row || lv_screen_active() != s_scr) return;
  if (lv_obj_has_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN)) return;   // text 模式 → 不动
  s_bar_phase = (s_bar_phase + 1) % BAR_COUNT;
  for (int i = 0; i < BAR_COUNT; i++)
    lv_obj_set_style_bg_color(s_bars[i], lv_color_hex(i == s_bar_phase ? BAR_HI : BAR_LO), 0);
}

// "O" 用一个缺口朝下的环形弧替代普通字母,呼应仪表盘的上下文环(同 bg_angles)。
static lv_obj_t *make_ring_o(lv_obj_t *par) {
  lv_obj_t *o = lv_arc_create(par);
  lv_obj_set_size(o, 22, 22);                 // ≈ Montserrat 28 的大写字高;偏大/小改这里
  lv_obj_remove_flag(o, LV_OBJ_FLAG_CLICKABLE);
  lv_arc_set_bg_angles(o, 135, 45);           // 270°,缺口朝下(与 dashboard 上下文环一致)
  lv_arc_set_value(o, 0);                      // 不要指示段,只保留纯环(背景弧)
  lv_obj_set_style_arc_color(o, lv_color_hex(C_AMBER), LV_PART_MAIN);
  lv_obj_set_style_arc_width(o, 3, LV_PART_MAIN);
  lv_obj_set_style_arc_rounded(o, true, LV_PART_MAIN);
  lv_obj_set_style_arc_opa(o, LV_OPA_TRANSP, LV_PART_INDICATOR);   // 隐藏指示弧
  lv_obj_set_style_bg_opa(o, LV_OPA_TRANSP, LV_PART_KNOB);         // 隐藏旋钮圆点
  lv_obj_set_style_pad_all(o, 0, LV_PART_KNOB);
  // 若环相对字母偏高/偏低,加 lv_obj_set_style_translate_y(o, ±n, 0) 微调。
  return o;
}

static void loading_build(lv_obj_t *scr) {
  lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(scr, lv_color_hex(C_BG), 0);
  lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(scr, 0, 0);

  // 第一排:字标 "C" + 环(O) + "DE MATE",用 flex 行横排成一个词、整体居中
  lv_obj_t *row = lv_obj_create(scr);
  lv_obj_remove_style_all(row);
  lv_obj_remove_flag(row, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(row, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(row, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(row, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(row, 2, 0);     // 字间距:让 C/环/DE 贴成一个词
  lv_obj_align(row, LV_ALIGN_CENTER, 0, -16);

  // "C"(主文字色 + 同字体,与 "DE MATE" 一致;阶段12 起不再用琥珀)
  lv_obj_t *c = lv_label_create(row);
  lv_obj_set_style_text_font(c, &lv_font_montserrat_28, 0);
  lv_obj_set_style_text_color(c, lv_color_hex(C_TEXT), 0);
  lv_label_set_text(c, "C");

  // "O" = 缺口朝下的环形仪表
  make_ring_o(row);

  // "DE MATE"(主文字色;前导空格给出 CODE 与 MATE 的词间距)
  lv_obj_t *rest = lv_label_create(row);
  lv_obj_set_style_text_font(rest, &lv_font_montserrat_28, 0);
  lv_obj_set_style_text_color(rest, lv_color_hex(C_TEXT), 0);
  lv_label_set_text(rest, "DE MATE");

  // 第二排(默认):三条等待矩形(高亮轮流跳动);与文字行同位、二选一显示
  s_scr = scr;
  s_bars_row = lv_obj_create(scr);
  lv_obj_remove_style_all(s_bars_row);                 // 去主题 → 容器透明、无边框/内边距
  lv_obj_remove_flag(s_bars_row, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(s_bars_row, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(s_bars_row, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(s_bars_row, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(s_bars_row, BAR_GAP, 0);
  lv_obj_align(s_bars_row, LV_ALIGN_CENTER, 0, 16);
  for (int i = 0; i < BAR_COUNT; i++) {
    lv_obj_t *b = lv_obj_create(s_bars_row);
    lv_obj_remove_style_all(b);
    lv_obj_set_size(b, BAR_W, BAR_H);
    lv_obj_set_style_radius(b, 2, 0);
    lv_obj_set_style_bg_opa(b, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(b, lv_color_hex(i == 0 ? BAR_HI : BAR_LO), 0);   // 初帧:第 0 条亮
    s_bars[i] = b;
  }
  s_bar_phase = 0;
  s_bar_timer = lv_timer_create(bar_tick, BAR_PERIOD_MS, nullptr);
  lv_obj_add_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN);   // 开机先只显 logo;满 3s splash 后由 main 揭出(ui_loading_show_bars)

  // 文字行:默认隐藏;仅当 host 经 cfg.msg 下发文案时显示(此时隐藏等待条)
  s_lbl_msg = lv_label_create(scr);
  lv_obj_set_style_text_font(s_lbl_msg, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_msg, lv_color_hex(C_AMBER), 0);
  lv_obj_set_width(s_lbl_msg, 300);
  lv_label_set_long_mode(s_lbl_msg, LV_LABEL_LONG_WRAP);
  lv_obj_set_style_text_align(s_lbl_msg, LV_TEXT_ALIGN_CENTER, 0);
  lv_label_set_text(s_lbl_msg, s_msg);
  lv_obj_align(s_lbl_msg, LV_ALIGN_CENTER, 0, 16);
  lv_obj_add_flag(s_lbl_msg, LV_OBJ_FLAG_HIDDEN);
}

// 切入 loading:cfg 带消息 → 显文字、停等待条;否则(默认/连接中)→ 显等待条
static void loading_on_enter() {
  const char *m = protocol_cfg_msg();
  bool has_msg = m && *m;
  if (has_msg) {
    strncpy(s_msg, m, sizeof(s_msg) - 1);
    s_msg[sizeof(s_msg) - 1] = 0;
    if (s_lbl_msg) lv_label_set_text(s_lbl_msg, s_msg);
  }
  if (s_lbl_msg)  has_msg ? lv_obj_remove_flag(s_lbl_msg, LV_OBJ_FLAG_HIDDEN)
                          : lv_obj_add_flag(s_lbl_msg, LV_OBJ_FLAG_HIDDEN);
  if (s_bars_row) has_msg ? lv_obj_add_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN)
                          : lv_obj_remove_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN);
}

// 阶段8:loading 的 LED —— 恒为「连接中」深蓝慢闪(无会话/未连;忽略 cc_running)
static void loading_led(DeviceState, bool, uint8_t) { leds_fx_connecting(); }

// 揭出/隐藏等待条(main 在开机 splash 满 3s 且仍在 loading 时调 true;显条时藏文字)。
void ui_loading_show_bars(bool show) {
  if (!s_bars_row) return;
  if (show) {
    if (s_lbl_msg) lv_obj_add_flag(s_lbl_msg, LV_OBJ_FLAG_HIDDEN);
    lv_obj_remove_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN);
  } else {
    lv_obj_add_flag(s_bars_row, LV_OBJ_FLAG_HIDDEN);
  }
}

Layout *ui_loading_layout() {
  static Layout L = {
    "loading",
    nullptr,         // scr:由 main 创建后回填
    loading_build,
    nullptr,         // set_data(无数据)
    nullptr,         // set_state
    loading_on_enter,
    nullptr,         // on_exit
    nullptr,         // tick(信息固定,无需周期刷新/点动画)
    loading_led,     // led(阶段8):连接中蓝闪
  };
  return &L;
}
