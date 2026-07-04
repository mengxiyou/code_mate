#include "ui_cc_usage.h"
#include "ui_common.h"   // HUD 配色 + ui_build_logo(两屏共享)
#include "leds.h"        // 阶段8:LED 灯效由本布局选择(显示方式由布局定)
#include <Arduino.h>
#include <lvgl.h>
#include <stdio.h>
#include <string.h>

// ---- 控件指针 ----
static lv_obj_t *s_scr;
static lv_obj_t *s_logo, *s_logo_codex, *s_wordmark, *s_topr; // 顶栏:logo / 字标 / 右上容器
static lv_obj_t *s_dot, *s_lbl_session;             // 右上:每会话色点 + 会话标题
static lv_obj_t *s_bar5, *s_lbl5_pct, *s_lbl5_reset;
static lv_obj_t *s_bar7, *s_lbl7_pct, *s_lbl7_reset;
static lv_obj_t *s_arc, *s_lbl_arc_pct, *s_lbl_tokens;
static lv_obj_t *s_lbl_model, *s_lbl_mode;          // 左下:模型名 + effort 徽章
static lv_obj_t *s_sq_cont;                         // 右下:会话指示方块容器
static bool s_revealed = false;                     // 首帧 data 后置 true(只用于入场动画判定)

#define SQ_MAX 8
static lv_obj_t *s_squares[SQ_MAX];

// 每会话身份色调色板(主题色;PC 下发的 dot 值 % 个数 选色)
static const uint32_t SESS_PALETTE[] = {C_CORAL, C_CYAN, C_AMBER, C_CLAUDE, C_CODEX};
#define SESS_PALETTE_N 5
static uint32_t sess_color(uint16_t dot) { return SESS_PALETTE[dot % SESS_PALETTE_N]; }

static bool is_codex_payload(const UsagePayload &p) {
  return strstr(p.source, "Codex") || strstr(p.brand, "Codex");
}

static uint32_t theme_color(uint32_t v, uint32_t fallback) {
  return v ? v : fallback;
}

static uint32_t provider_color(const UsagePayload &p) {
  return p.theme.has ? theme_color(p.theme.primary, C_CLAUDE)
                     : (is_codex_payload(p) ? C_CODEX : C_CLAUDE);
}

static uint32_t meter_a_color(const UsagePayload &p) {
  return p.theme.has ? theme_color(p.theme.meter_a, C_CORAL) : C_CORAL;
}

static uint32_t meter_b_color(const UsagePayload &p) {
  return p.theme.has ? theme_color(p.theme.meter_b, C_CYAN) : C_CYAN;
}

static uint32_t meter_c_color(const UsagePayload &p) {
  return p.theme.has ? theme_color(p.theme.meter_c, C_AMBER)
                     : (is_codex_payload(p) ? C_CODEX : C_AMBER);
}

static uint32_t instance_color(const UsagePayload &p) {
  return is_codex_payload(p) ? provider_color(p) : sess_color(p.dot);
}

// ---- 倒计时 / 状态 ----
static UsagePayload s_pl;
static uint32_t s_pc_ts = 0;      // 数据帧里的电脑 unix 秒
static uint32_t s_rx_ms = 0;      // 收到该帧时的 millis()
static DeviceState s_state = ST_BOOT;

static uint32_t device_now() {    // 设备推算的当前 unix 秒
  if (s_pc_ts == 0) return 0;
  return s_pc_ts + (millis() - s_rx_ms) / 1000;
}

static int pct_i(float p) { return (int)(p + 0.5f); }

static void fmt_tokens_human(uint32_t v, char *buf, size_t n) {
  if (v >= 1000000)      snprintf(buf, n, "%.1fM", v / 1000000.0);
  else if (v >= 1000)    snprintf(buf, n, "%uK", (unsigned)(v / 1000));
  else                   snprintf(buf, n, "%u", (unsigned)v);
}

static void set_reset_5h(lv_obj_t *lbl, const RateWindow &w) {
  if (!w.has || w.resets_at == 0 || s_pc_ts == 0) { lv_label_set_text(lbl, "RESETS --"); return; }
  long rem = (long)w.resets_at - (long)device_now();
  if (rem <= 0) { lv_label_set_text(lbl, "RESETS NOW"); return; }
  int h = rem / 3600, m = (rem % 3600) / 60;
  if (h > 0) lv_label_set_text_fmt(lbl, "RESETS IN %dH %02dM", h, m);
  else       lv_label_set_text_fmt(lbl, "RESETS IN %dM", m);
}

static void set_reset_7d(lv_obj_t *lbl, const RateWindow &w) {
  if (!w.has || w.resets_at == 0 || s_pc_ts == 0) { lv_label_set_text(lbl, "RESETS --"); return; }
  long rem = (long)w.resets_at - (long)device_now();
  if (rem <= 0) { lv_label_set_text(lbl, "RESETS NOW"); return; }
  int d = rem / 86400, h = (rem % 86400) / 3600;
  lv_label_set_text_fmt(lbl, "RESETS IN %dD %dH", d, h);
}

// 工作模式徽章 → 语义色。取 effort.level(思考强度),颜色随强度递增。
static uint32_t mode_color(const char *m) {
  if (!m) return 0x9AA3B0;
  if (strstr(m, "MAX"))    return 0xFF5C5C; // 红:最高
  if (strstr(m, "XHIGH"))  return 0xFF6B3D; // 珊瑚:极高(必须先于 HIGH 判断)
  if (strstr(m, "HIGH"))   return 0xF2A53C; // 琥珀:高
  if (strstr(m, "MEDIUM")) return 0x2DE2E6; // 青:中
  if (strstr(m, "LOW"))    return 0x9AA3B0; // 灰:低
  return 0x9AA3B0;                          // 未知/默认:灰
}

// ---- 小工具 ----
static lv_obj_t *mk_label(lv_obj_t *par, const lv_font_t *font, uint32_t color,
                          lv_align_t al, int x, int y, const char *txt) {
  lv_obj_t *l = lv_label_create(par);
  lv_obj_set_style_text_font(l, font, 0);
  lv_obj_set_style_text_color(l, lv_color_hex(color), 0);
  lv_label_set_text(l, txt);
  lv_obj_align(l, al, x, y);
  return l;
}

static lv_obj_t *mk_box(lv_obj_t *par, int w, int h, uint32_t color, lv_opa_t opa) {
  lv_obj_t *o = lv_obj_create(par);
  lv_obj_remove_style_all(o);
  lv_obj_remove_flag(o, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(o, w, h);
  lv_obj_set_style_bg_color(o, lv_color_hex(color), 0);
  lv_obj_set_style_bg_opa(o, opa, 0);
  return o;
}

static lv_obj_t *mk_bar(lv_obj_t *par, uint32_t fill, int x, int y, int w, int h) {
  lv_obj_t *b = lv_bar_create(par);
  lv_obj_set_size(b, w, h);
  lv_obj_align(b, LV_ALIGN_TOP_LEFT, x, y);
  lv_obj_set_style_bg_color(b, lv_color_hex(C_TRACK), LV_PART_MAIN);
  lv_obj_set_style_bg_opa(b, LV_OPA_COVER, LV_PART_MAIN);
  lv_obj_set_style_radius(b, h / 2, LV_PART_MAIN);
  lv_obj_set_style_bg_color(b, lv_color_hex(fill), LV_PART_INDICATOR);
  lv_obj_set_style_radius(b, h / 2, LV_PART_INDICATOR);
  lv_obj_set_style_anim_duration(b, 500, LV_PART_INDICATOR);
  lv_bar_set_range(b, 0, 100);
  lv_bar_set_value(b, 0, LV_ANIM_OFF);
  return b;
}

// 把文本截断到不超过 max_w 像素(超出补 "..."),用于不让它越过相邻元素
static void fit_model_text(const char *src, char *out, size_t outn, int max_w) {
  lv_point_t sz;
  lv_text_get_size(&sz, src, &lv_font_montserrat_14, 0, 0, LV_COORD_MAX, LV_TEXT_FLAG_NONE);
  if (sz.x <= max_w) { snprintf(out, outn, "%s", src); return; }
  for (int n = (int)strlen(src); n > 0; n--) {
    char tmp[64];
    snprintf(tmp, sizeof tmp, "%.*s...", n, src);
    lv_text_get_size(&sz, tmp, &lv_font_montserrat_14, 0, 0, LV_COORD_MAX, LV_TEXT_FLAG_NONE);
    if (sz.x <= max_w) { snprintf(out, outn, "%s", tmp); return; }
  }
  snprintf(out, outn, "...");
}

// 去掉模型名的 " (...)" / " [...]" 后缀(如 "Opus 4.8 (1M context)" → "Opus 4.8")
static void strip_model_suffix(const char *src, char *out, size_t n) {
  size_t i = 0;
  while (src[i] && i + 1 < n) {
    if (src[i] == ' ' && (src[i + 1] == '(' || src[i + 1] == '[')) break;
    out[i] = src[i];
    i++;
  }
  out[i] = 0;
}

// ---- 入场动画:进度条/环从 0 带回弹(overshoot)增长到目标,同步刷新百分比文字 ----
static int clamp01(int v) { return v < 0 ? 0 : (v > 100 ? 100 : v); }
static void anim_bar5_cb(void *, int32_t v) { int c = clamp01(v); lv_bar_set_value(s_bar5, c, LV_ANIM_OFF); lv_label_set_text_fmt(s_lbl5_pct, "%d%%", c); }
static void anim_bar7_cb(void *, int32_t v) { int c = clamp01(v); lv_bar_set_value(s_bar7, c, LV_ANIM_OFF); lv_label_set_text_fmt(s_lbl7_pct, "%d%%", c); }
static void anim_arc_cb(void *, int32_t v)  { int c = clamp01(v); lv_arc_set_value(s_arc, c);              lv_label_set_text_fmt(s_lbl_arc_pct, "%d%%", c); }
static void intro_anim_to(lv_obj_t *obj, int32_t to, lv_anim_exec_xcb_t cb) {
  lv_anim_t a;
  lv_anim_init(&a);
  lv_anim_set_var(&a, obj);
  lv_anim_set_values(&a, 0, to);
  lv_anim_set_duration(&a, 850);
  lv_anim_set_path_cb(&a, lv_anim_path_overshoot);  // 回弹:略冲过目标再落回
  lv_anim_set_exec_cb(&a, cb);
  lv_anim_start(&a);
}

// 从 0 重播三个仪表(环 + 两条)的增长动画到「最近一帧」的目标值;无数据的项保持 0(动画即空)。
// 供 on_enter 用:每次「切入仪表盘」统一走它(不分来源:终端/菜单/loading),先置 0 再增长。
static void cc_intro_gauges() {
  if (s_pl.ctx_has) {
    lv_arc_set_value(s_arc, 0); lv_label_set_text(s_lbl_arc_pct, "0%");
    intro_anim_to(s_arc, pct_i(s_pl.ctx_pct), anim_arc_cb);
  }
  if (s_pl.five_hour.has) {
    lv_bar_set_value(s_bar5, 0, LV_ANIM_OFF); lv_label_set_text(s_lbl5_pct, "0%");
    intro_anim_to(s_bar5, pct_i(s_pl.five_hour.used_pct), anim_bar5_cb);
  }
  if (s_pl.seven_day.has) {
    lv_bar_set_value(s_bar7, 0, LV_ANIM_OFF); lv_label_set_text(s_lbl7_pct, "0%");
    intro_anim_to(s_bar7, pct_i(s_pl.seven_day.used_pct), anim_bar7_cb);
  }
}

void ui_build(lv_obj_t *scr) {
  s_scr = scr;
  lv_obj_remove_flag(s_scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(s_scr, lv_color_hex(C_BG), 0);
  lv_obj_set_style_bg_opa(s_scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(s_scr, 0, 0);

  // 顶部状态栏:像素 logo + 橙色字标
  s_logo = ui_build_logo(s_scr, 8, 6, C_CLAUDE);
  s_logo_codex = ui_build_codex_logo(s_scr, 8, 6, C_CODEX);
  lv_obj_add_flag(s_logo_codex, LV_OBJ_FLAG_HIDDEN);
  s_wordmark = mk_label(s_scr, &lv_font_montserrat_14, C_CLAUDE, LV_ALIGN_TOP_LEFT, 66, 10, "Claude Code");

  // 右上:每会话色点 + 会话标题(flex 行容器,标题变长自动把点推开)
  lv_obj_t *topr = lv_obj_create(s_scr);
  lv_obj_remove_style_all(topr);
  lv_obj_remove_flag(topr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(topr, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(topr, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(topr, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(topr, 6, 0);
  lv_obj_align(topr, LV_ALIGN_TOP_RIGHT, -10, 7);
  s_topr = topr;

  s_dot = mk_box(topr, 7, 7, C_DIM, LV_OPA_COVER);   // 每会话身份色方块(与右下指示方块同款)
  lv_obj_set_style_radius(s_dot, 2, 0);

  s_lbl_session = lv_label_create(topr);
  lv_obj_set_style_text_font(s_lbl_session, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_session, lv_color_hex(C_TEXT2), 0);   // 与 5-HOUR/WEEKLY 标签同色(非主题色)
  lv_label_set_text(s_lbl_session, "--");

  // 顶部分隔线
  lv_obj_t *div_h = mk_box(s_scr, 304, 1, C_DIVIDER, LV_OPA_COVER);
  lv_obj_align(div_h, LV_ALIGN_TOP_MID, 0, 30);
  // 左右半分隔线
  lv_obj_t *div_v = mk_box(s_scr, 1, 126, C_DIVIDER, LV_OPA_COVER);
  lv_obj_align(div_v, LV_ALIGN_TOP_LEFT, 184, 36);

  // 左半:5 小时
  mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 38, "5-HOUR");
  s_lbl5_pct = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl5_pct, &lv_font_montserrat_20, 0);
  lv_obj_set_style_text_color(s_lbl5_pct, lv_color_hex(C_CORAL), 0);
  lv_obj_set_width(s_lbl5_pct, 56);
  lv_obj_set_style_text_align(s_lbl5_pct, LV_TEXT_ALIGN_RIGHT, 0);
  lv_label_set_text(s_lbl5_pct, "--%");
  lv_obj_align(s_lbl5_pct, LV_ALIGN_TOP_LEFT, 110, 35);
  s_bar5 = mk_bar(s_scr, C_CORAL, 8, 57, 158, 10);
  s_lbl5_reset = mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 71, "RESETS --");

  // 左半:每周
  mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 96, "WEEKLY");
  s_lbl7_pct = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl7_pct, &lv_font_montserrat_20, 0);
  lv_obj_set_style_text_color(s_lbl7_pct, lv_color_hex(C_CYAN), 0);
  lv_obj_set_width(s_lbl7_pct, 56);
  lv_obj_set_style_text_align(s_lbl7_pct, LV_TEXT_ALIGN_RIGHT, 0);
  lv_label_set_text(s_lbl7_pct, "--%");
  lv_obj_align(s_lbl7_pct, LV_ALIGN_TOP_LEFT, 110, 93);
  s_bar7 = mk_bar(s_scr, C_CYAN, 8, 115, 158, 10);
  s_lbl7_reset = mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 129, "RESETS --");

  // 右半:上下文环
  s_arc = lv_arc_create(s_scr);
  lv_obj_set_size(s_arc, 96, 96);
  lv_obj_align(s_arc, LV_ALIGN_TOP_RIGHT, -19, 38);   // 左移一点(原 -14;-24 偏多,回一半)
  lv_obj_remove_flag(s_arc, LV_OBJ_FLAG_CLICKABLE);
  lv_arc_set_bg_angles(s_arc, 135, 45);   // 270° 仪表,缺口在下
  lv_arc_set_range(s_arc, 0, 100);
  lv_arc_set_value(s_arc, 0);
  lv_obj_set_style_arc_color(s_arc, lv_color_hex(C_TRACK), LV_PART_MAIN);
  lv_obj_set_style_arc_width(s_arc, 10, LV_PART_MAIN);
  lv_obj_set_style_arc_rounded(s_arc, true, LV_PART_MAIN);
  lv_obj_set_style_arc_color(s_arc, lv_color_hex(C_AMBER), LV_PART_INDICATOR);
  lv_obj_set_style_arc_width(s_arc, 10, LV_PART_INDICATOR);
  lv_obj_set_style_arc_rounded(s_arc, true, LV_PART_INDICATOR);
  lv_obj_set_style_anim_duration(s_arc, 500, LV_PART_INDICATOR);
  lv_obj_set_style_bg_opa(s_arc, LV_OPA_TRANSP, LV_PART_KNOB);
  lv_obj_set_style_pad_all(s_arc, 0, LV_PART_KNOB);

  s_lbl_arc_pct = lv_label_create(s_arc);
  lv_obj_set_style_text_font(s_lbl_arc_pct, &lv_font_montserrat_28, 0);
  lv_obj_set_style_text_color(s_lbl_arc_pct, lv_color_hex(C_AMBER), 0);   // 环中间数字:黄色(琥珀)
  lv_label_set_text(s_lbl_arc_pct, "--");
  lv_obj_align(s_lbl_arc_pct, LV_ALIGN_CENTER, 0, -7);

  lv_obj_t *lbl_ctx = lv_label_create(s_arc);
  lv_obj_set_style_text_font(lbl_ctx, &lv_font_montserrat_12, 0);
  lv_obj_set_style_text_color(lbl_ctx, lv_color_hex(C_TEXT2), 0);
  lv_label_set_text(lbl_ctx, "CONTEXT");
  lv_obj_align(lbl_ctx, LV_ALIGN_CENTER, 0, 16);

  s_lbl_tokens = mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_RIGHT, -14, 134, "-- / --");
  lv_obj_set_width(s_lbl_tokens, 120);
  lv_obj_set_style_text_align(s_lbl_tokens, LV_TEXT_ALIGN_CENTER, 0);
  lv_obj_align_to(s_lbl_tokens, s_arc, LV_ALIGN_OUT_BOTTOM_MID, 0, -6);

  // 左下:模型名 + effort 徽章(flex 行)
  lv_obj_t *botl = lv_obj_create(s_scr);
  lv_obj_remove_style_all(botl);
  lv_obj_remove_flag(botl, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(botl, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(botl, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(botl, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(botl, 6, 0);
  lv_obj_align(botl, LV_ALIGN_BOTTOM_LEFT, 8, -5);

  s_lbl_model = lv_label_create(botl);
  lv_obj_set_style_text_font(s_lbl_model, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_model, lv_color_hex(C_CLAUDE), 0);   // 与 logo 同色
  lv_label_set_text(s_lbl_model, "--");

  s_lbl_mode = lv_label_create(botl);
  lv_obj_set_style_text_font(s_lbl_mode, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_mode, lv_color_hex(C_CORAL), 0);
  lv_obj_set_style_bg_color(s_lbl_mode, lv_color_hex(C_TRACK), 0);
  lv_obj_set_style_bg_opa(s_lbl_mode, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(s_lbl_mode, 3, 0);
  lv_obj_set_style_radius(s_lbl_mode, 5, 0);
  lv_label_set_text(s_lbl_mode, "--");

  // 右下:会话指示方块(N 块,当前会话亮其余暗;预建固定池)
  s_sq_cont = lv_obj_create(s_scr);
  lv_obj_remove_style_all(s_sq_cont);
  lv_obj_remove_flag(s_sq_cont, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(s_sq_cont, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(s_sq_cont, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(s_sq_cont, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(s_sq_cont, 4, 0);
  lv_obj_align(s_sq_cont, LV_ALIGN_BOTTOM_RIGHT, -17, -8);   // 跟随环左移(原 -12;回一半)
  for (int i = 0; i < SQ_MAX; i++) {
    s_squares[i] = mk_box(s_sq_cont, 7, 7, C_DIM, LV_OPA_COVER);
    lv_obj_set_style_radius(s_squares[i], 2, 0);
    lv_obj_add_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
  }
}

void ui_set_payload(const UsagePayload &p, bool init) {
  s_pl = p;
  s_pc_ts = p.pc_ts;
  s_rx_ms = millis();

  // 右上:会话标题截断到「至多到屏幕正中」≤140px(不挤占左侧字标);越界隐藏字标作兜底
  char st[72];
  fit_model_text(p.has_session ? p.session : "--", st, sizeof st, 140);
  lv_label_set_text(s_lbl_session, st);
  // 右上:每会话身份色点
  bool codex = is_codex_payload(p);
  lv_label_set_text(s_wordmark, p.brand[0] ? p.brand : (p.source[0] ? p.source : "Claude Code"));
  uint32_t pc = provider_color(p);
  uint32_t ca = meter_a_color(p);
  uint32_t cb = meter_b_color(p);
  uint32_t cc = meter_c_color(p);
  lv_obj_set_style_bg_color(s_dot, lv_color_hex(instance_color(p)), 0);
  lv_obj_set_style_text_color(s_wordmark, lv_color_hex(pc), 0);
  lv_obj_set_style_text_color(s_lbl_model, lv_color_hex(pc), 0);
  lv_obj_set_style_bg_color(s_bar5, lv_color_hex(ca), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl5_pct, lv_color_hex(ca), 0);
  lv_obj_set_style_bg_color(s_bar7, lv_color_hex(cb), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl7_pct, lv_color_hex(cb), 0);
  lv_obj_set_style_arc_color(s_arc, lv_color_hex(cc), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl_arc_pct, lv_color_hex(cc), 0);
  if (codex) {
    lv_obj_add_flag(s_logo, LV_OBJ_FLAG_HIDDEN);
    lv_obj_remove_flag(s_logo_codex, LV_OBJ_FLAG_HIDDEN);
    lv_obj_add_flag(s_wordmark, LV_OBJ_FLAG_HIDDEN);
  } else {
    lv_obj_remove_flag(s_logo, LV_OBJ_FLAG_HIDDEN);
    lv_obj_add_flag(s_logo_codex, LV_OBJ_FLAG_HIDDEN);
    lv_obj_remove_flag(s_wordmark, LV_OBJ_FLAG_HIDDEN);
  }

  lv_obj_update_layout(s_scr);
  lv_coord_t wm_right = lv_obj_get_x(s_wordmark) + lv_obj_get_width(s_wordmark);
  if (!codex) {
    if (lv_obj_get_x(s_topr) < wm_right + 8) lv_obj_add_flag(s_wordmark, LV_OBJ_FLAG_HIDDEN);
    else                                     lv_obj_remove_flag(s_wordmark, LV_OBJ_FLAG_HIDDEN);
  }

  // 左下:模型名(去括号后缀)+ effort 徽章
  char mtmp[48], mdl[48];
  strip_model_suffix(p.has_model ? p.model : "--", mtmp, sizeof mtmp);
  fit_model_text(mtmp, mdl, sizeof mdl, 120);
  lv_label_set_text(s_lbl_model, mdl);
  lv_label_set_text(s_lbl_mode, p.has_mode ? p.mode : "--");
  lv_obj_set_style_text_color(s_lbl_mode, lv_color_hex(p.has_mode ? mode_color(p.mode) : C_DIM), 0);

  // 右下:会话指示方块(显前 cnt 个;当前 idx 那块用身份色亮,其余暗)
  int cnt = p.inst_cnt; if (cnt > SQ_MAX) cnt = SQ_MAX;
  uint32_t hi = instance_color(p);
  for (int i = 0; i < SQ_MAX; i++) {
    if (i < cnt) {
      lv_obj_remove_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
      lv_obj_set_style_bg_color(s_squares[i], lv_color_hex(i == (int)p.inst_idx - 1 ? hi : C_DIM), 0);
    } else {
      lv_obj_add_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
    }
  }

  // init(PC 显式:首帧/换实例)或首次收到 data → 进度条/环「0→目标」回弹增长;否则 tween 刷新
  bool intro = init || !s_revealed;
  if (!s_revealed) s_revealed = true;

  // 上下文环
  if (p.ctx_has) {
    int tc = pct_i(p.ctx_pct);
    if (intro) intro_anim_to(s_arc, tc, anim_arc_cb);
    else { lv_arc_set_value(s_arc, tc); lv_label_set_text_fmt(s_lbl_arc_pct, "%d%%", tc); }
    char a[16], b[16];
    fmt_tokens_human(p.ctx_used, a, sizeof a);
    fmt_tokens_human(p.ctx_max, b, sizeof b);
    lv_label_set_text_fmt(s_lbl_tokens, "%s / %s", a, b);
  } else {
    lv_arc_set_value(s_arc, 0);
    lv_label_set_text(s_lbl_arc_pct, "--");
    lv_label_set_text(s_lbl_tokens, "-- / --");
  }

  // 5 小时
  if (p.five_hour.has) {
    int t5 = pct_i(p.five_hour.used_pct);
    if (intro) intro_anim_to(s_bar5, t5, anim_bar5_cb);
    else { lv_bar_set_value(s_bar5, t5, LV_ANIM_ON); lv_label_set_text_fmt(s_lbl5_pct, "%d%%", t5); }
  } else {
    lv_bar_set_value(s_bar5, 0, LV_ANIM_OFF);
    lv_label_set_text(s_lbl5_pct, "--%");
  }
  set_reset_5h(s_lbl5_reset, p.five_hour);

  // 每周
  if (p.seven_day.has) {
    int t7 = pct_i(p.seven_day.used_pct);
    if (intro) intro_anim_to(s_bar7, t7, anim_bar7_cb);
    else { lv_bar_set_value(s_bar7, t7, LV_ANIM_ON); lv_label_set_text_fmt(s_lbl7_pct, "%d%%", t7); }
  } else {
    lv_bar_set_value(s_bar7, 0, LV_ANIM_OFF);
    lv_label_set_text(s_lbl7_pct, "--%");
  }
  set_reset_7d(s_lbl7_reset, p.seven_day);
}

void ui_set_state(DeviceState st) {
  s_state = st;
  // 仅在真正断流(STALE,30s 无帧)时把主信息变暗;连着(含 CC 空闲)始终全亮。
  // 状态点不再表连接态(改为每会话身份色);连接「未连/无会话」由 loading 布局体现。
  lv_opa_t opa = (st == ST_STALE) ? LV_OPA_60 : LV_OPA_COVER;
  lv_obj_t *dim[] = {s_arc, s_bar5, s_bar7, s_lbl_arc_pct, s_lbl5_pct, s_lbl7_pct,
                     s_lbl_tokens, s_dot, s_lbl_session, s_lbl_model, s_sq_cont};
  for (lv_obj_t *o : dim) lv_obj_set_style_opa(o, opa, 0);
}

void ui_tick() {
  // 本地倒计时(右上标题在 ui_set_payload 里随帧更新,这里不动)
  set_reset_5h(s_lbl5_reset, s_pl.five_hour);
  set_reset_7d(s_lbl7_reset, s_pl.seven_day);
}

// ---- 布局注册项(阶段7):把上面的函数封进统一 vtable ----
static void cc_set_data(const void *p, bool init) { ui_set_payload(*(const UsagePayload *)p, init); }

// on_enter(阶段11):每次「切入仪表盘」都从 0 重播增长动画 —— 统一机制,不分来源(终端/菜单/loading)。
// 同屏的实例切换仍由 init 帧触发同一套 intro(见 ui_set_payload),无需在此处理。
static void cc_on_enter() { cc_intro_gauges(); }

// 阶段8:仪表盘的 LED 灯效 —— 按所选会话工作态(未连/断流→连接中蓝闪;空闲→蓝常亮;工作→6 色呼吸)
static void cc_led(DeviceState st, bool cc_running, uint8_t) {
  if (st != ST_LIVE)   leds_fx_connecting();
  else if (!cc_running) leds_fx_idle();
  else                  leds_fx_working();
}

Layout *ui_cc_usage_layout() {
  static Layout L = {
    "dashboard",
    nullptr,         // scr:由 main 创建后回填
    ui_build,
    cc_set_data,
    ui_set_state,
    cc_on_enter,     // on_enter(阶段11):切入仪表盘即从 0 重播增长动画
    nullptr,         // on_exit
    ui_tick,
    cc_led,          // led(阶段8):工作态灯效
  };
  return &L;
}
