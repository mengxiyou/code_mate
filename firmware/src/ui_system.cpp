#include "ui_system.h"
#include "ui_common.h"   // HUD 配色
#include "leds.h"        // 磁盘活动灯
#include <Arduino.h>
#include <lvgl.h>
#include <stdio.h>
#include <string.h>

// system 屏(阶段12):布局镜像 dashboard —— 左两条为冷色系辅助色、右 CPU 环固定跟随 System 主题色;
//   顶左 显示器图标 + SYSTEM、顶右 主机名。磁盘活动只驱动板载 LED(忙=暖琥珀闪 / 闲=柔和蓝)。

static lv_obj_t *s_scr;
static lv_obj_t *s_icon, *s_lbl_title, *s_lbl_host, *s_lbl_net;
static lv_obj_t *s_bar_ram, *s_lbl_ram_pct, *s_lbl_ram_sub;
static lv_obj_t *s_bar_vram, *s_lbl_vram_pct, *s_lbl_vram_sub;
static lv_obj_t *s_arc_cpu, *s_lbl_cpu_pct, *s_lbl_cpu_sub;
static bool s_revealed = false;

static UsagePayload s_pl;

// 会话指示(与 dashboard 同款):右上身份块 + 右下会话方块。system 也是统一会话之一。
static lv_obj_t *s_dot, *s_sq_cont;
#define SQ_MAX 8
static lv_obj_t *s_squares[SQ_MAX];

static int pct_i(float p) { return (int)(p + 0.5f); }
static int clamp01(int v) { return v < 0 ? 0 : (v > 100 ? 100 : v); }
static uint32_t theme_color(uint32_t v, uint32_t fallback) { return v ? v : fallback; }
static uint32_t sys_primary(const UsagePayload &p) { return p.theme.has ? theme_color(p.theme.primary, C_SYSTEM) : C_SYSTEM; }
static uint32_t sys_ram(const UsagePayload &p) { return p.theme.has ? theme_color(p.theme.meter_a, C_SYS_RAM) : C_SYS_RAM; }
static uint32_t sys_vram(const UsagePayload &p) { return p.theme.has ? theme_color(p.theme.meter_b, C_SYS_VRAM) : C_SYS_VRAM; }
static uint32_t sys_cpu(const UsagePayload &p) { return sys_primary(p); }

// ---- 小工具(与 ui_cc_usage 同款,各布局自持)----
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

// used_mb/total_mb → "12.3 / 23.6 GB"(GiB,与任务管理器一致)
static void fmt_gb(uint32_t used_mb, uint32_t total_mb, char *buf, size_t n) {
  snprintf(buf, n, "%.1f / %.1f GB", used_mb / 1024.0, total_mb / 1024.0);
}

// 把文本截断到 ≤max_w 像素(超出补 "...");右上主机名「至多到屏幕正中」、不挤占左侧 SYSTEM 字标。
static void fit_w(const char *src, char *out, size_t outn, int max_w) {
  lv_point_t sz;
  lv_text_get_size(&sz, src, &lv_font_montserrat_14, 0, 0, LV_COORD_MAX, LV_TEXT_FLAG_NONE);
  if (sz.x <= max_w) { snprintf(out, outn, "%s", src); return; }
  for (int n = (int)strlen(src); n > 0; n--) {
    char tmp[48];
    snprintf(tmp, sizeof tmp, "%.*s...", n, src);
    lv_text_get_size(&sz, tmp, &lv_font_montserrat_14, 0, 0, LV_COORD_MAX, LV_TEXT_FLAG_NONE);
    if (sz.x <= max_w) { snprintf(out, outn, "%s", tmp); return; }
  }
  snprintf(out, outn, "...");
}

// 显示器图标(像素风,与 Claude logo 同款 canvas 放大块):13×9 位图 ×3 → 39×27,system 蓝。
//   实心蓝屏 + 黑色内嵌矩形框(只边框、内部仍蓝)+ 加宽 + 缩小立柱/底座,块状像素与 logo 风格一致。
#define MON_W 13
#define MON_H 9
#define MON_S 3
static const uint8_t s_mon_bits[MON_H][MON_W] = {
  {1,1,1,1,1,1,1,1,1,1,1,1,1},   // 屏外框(蓝)
  {1,0,0,0,0,0,0,0,0,0,0,0,1},   // 黑色内嵌框:上边
  {1,0,1,1,1,1,1,1,1,1,1,0,1},   // 黑框左右、内部仍蓝
  {1,0,1,1,1,1,1,1,1,1,1,0,1},
  {1,0,1,1,1,1,1,1,1,1,1,0,1},
  {1,0,0,0,0,0,0,0,0,0,0,0,1},   // 黑色内嵌框:下边
  {1,1,1,1,1,1,1,1,1,1,1,1,1},   // 屏外框(蓝)
  {0,0,0,0,0,1,1,1,0,0,0,0,0},   // 立柱(缩小)
  {0,0,0,0,1,1,1,1,1,0,0,0,0},   // 底座(缩小)
};
static void paint_monitor_icon(lv_obj_t *cv, uint32_t color) {
  lv_canvas_fill_bg(cv, lv_color_hex(C_BG), LV_OPA_COVER);
  lv_color_t fg = lv_color_hex(color);
  for (int r = 0; r < MON_H; r++)
    for (int c = 0; c < MON_W; c++)
      if (s_mon_bits[r][c])
        for (int yy = 0; yy < MON_S; yy++)
          for (int xx = 0; xx < MON_S; xx++)
            lv_canvas_set_px(cv, c * MON_S + xx, r * MON_S + yy, fg, LV_OPA_COVER);
}

static lv_obj_t *build_monitor_icon(lv_obj_t *par, int x, int y) {
  const int W = MON_W * MON_S, H = MON_H * MON_S;
  uint8_t *buf = (uint8_t *)lv_malloc(W * H * 2 + 8);
  lv_obj_t *cv = lv_canvas_create(par);
  lv_obj_remove_style_all(cv);
  lv_canvas_set_buffer(cv, buf, W, H, LV_COLOR_FORMAT_RGB565);
  paint_monitor_icon(cv, C_SYSTEM);
  lv_obj_align(cv, LV_ALIGN_TOP_LEFT, x, y);
  return cv;
}

// ---- 入场动画(环/条从 0 回弹增长)----
static void anim_ram_cb(void *, int32_t v)  { int c = clamp01(v); lv_bar_set_value(s_bar_ram, c, LV_ANIM_OFF);  lv_label_set_text_fmt(s_lbl_ram_pct, "%d%%", c); }
static void anim_vram_cb(void *, int32_t v) { int c = clamp01(v); lv_bar_set_value(s_bar_vram, c, LV_ANIM_OFF); lv_label_set_text_fmt(s_lbl_vram_pct, "%d%%", c); }
static void anim_cpu_cb(void *, int32_t v)  { int c = clamp01(v); lv_arc_set_value(s_arc_cpu, c);               lv_label_set_text_fmt(s_lbl_cpu_pct, "%d%%", c); }
static void intro_anim_to(lv_obj_t *obj, int32_t to, lv_anim_exec_xcb_t cb) {
  lv_anim_t a;
  lv_anim_init(&a);
  lv_anim_set_var(&a, obj);
  lv_anim_set_values(&a, 0, to);
  lv_anim_set_duration(&a, 850);
  lv_anim_set_path_cb(&a, lv_anim_path_overshoot);
  lv_anim_set_exec_cb(&a, cb);
  lv_anim_start(&a);
}

// 切入 system 即从 0 重播三仪表增长(无数据的项保持 0)。
static void sys_intro_gauges() {
  if (s_pl.cpu_has)  { lv_arc_set_value(s_arc_cpu, 0);  lv_label_set_text(s_lbl_cpu_pct, "0%");  intro_anim_to(s_arc_cpu, pct_i(s_pl.cpu_pct), anim_cpu_cb); }
  if (s_pl.ram_has)  { lv_bar_set_value(s_bar_ram, 0, LV_ANIM_OFF);  lv_label_set_text(s_lbl_ram_pct, "0%");  intro_anim_to(s_bar_ram, pct_i(s_pl.ram_pct), anim_ram_cb); }
  if (s_pl.vram_has) { lv_bar_set_value(s_bar_vram, 0, LV_ANIM_OFF); lv_label_set_text(s_lbl_vram_pct, "0%"); intro_anim_to(s_bar_vram, pct_i(s_pl.vram_pct), anim_vram_cb); }
}

static void sys_build(lv_obj_t *scr) {
  s_scr = scr;
  lv_obj_remove_flag(s_scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(s_scr, lv_color_hex(C_BG), 0);
  lv_obj_set_style_bg_opa(s_scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(s_scr, 0, 0);

  // 顶栏:显示器图标 + SYSTEM / 右上 主机名(灰)
  s_icon = build_monitor_icon(s_scr, 8, 2);
  s_lbl_title = mk_label(s_scr, &lv_font_montserrat_14, C_SYSTEM, LV_ALIGN_TOP_LEFT, 53, 6, "SYSTEM");

  // 右上:身份块 + 主机名(flex 行,与 dashboard 右上同款)
  lv_obj_t *topr = lv_obj_create(s_scr);
  lv_obj_remove_style_all(topr);
  lv_obj_remove_flag(topr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(topr, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(topr, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(topr, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(topr, 6, 0);
  lv_obj_align(topr, LV_ALIGN_TOP_RIGHT, -10, 7);
  s_dot = mk_box(topr, 7, 7, C_DIM, LV_OPA_COVER);
  lv_obj_set_style_radius(s_dot, 2, 0);
  s_lbl_host = lv_label_create(topr);
  lv_obj_set_style_text_font(s_lbl_host, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_host, lv_color_hex(C_TEXT2), 0);
  lv_label_set_text(s_lbl_host, "--");

  // 分隔线(同 dashboard)
  lv_obj_t *div_h = mk_box(s_scr, 304, 1, C_DIVIDER, LV_OPA_COVER);
  lv_obj_align(div_h, LV_ALIGN_TOP_MID, 0, 30);
  lv_obj_t *div_v = mk_box(s_scr, 1, 126, C_DIVIDER, LV_OPA_COVER);
  lv_obj_align(div_v, LV_ALIGN_TOP_LEFT, 184, 36);

  // 左半:内存
  mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 38, "MEMORY");
  s_lbl_ram_pct = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl_ram_pct, &lv_font_montserrat_20, 0);
  lv_obj_set_style_text_color(s_lbl_ram_pct, lv_color_hex(C_SYS_RAM), 0);
  lv_obj_set_width(s_lbl_ram_pct, 56);
  lv_obj_set_style_text_align(s_lbl_ram_pct, LV_TEXT_ALIGN_RIGHT, 0);
  lv_label_set_text(s_lbl_ram_pct, "--%");
  lv_obj_align(s_lbl_ram_pct, LV_ALIGN_TOP_LEFT, 110, 35);
  s_bar_ram = mk_bar(s_scr, C_SYS_RAM, 8, 57, 158, 10);
  s_lbl_ram_sub = mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 71, "-- / -- GB");

  // 左半:显存
  mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 96, "VRAM");
  s_lbl_vram_pct = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl_vram_pct, &lv_font_montserrat_20, 0);
  lv_obj_set_style_text_color(s_lbl_vram_pct, lv_color_hex(C_SYS_VRAM), 0);
  lv_obj_set_width(s_lbl_vram_pct, 56);
  lv_obj_set_style_text_align(s_lbl_vram_pct, LV_TEXT_ALIGN_RIGHT, 0);
  lv_label_set_text(s_lbl_vram_pct, "--%");
  lv_obj_align(s_lbl_vram_pct, LV_ALIGN_TOP_LEFT, 110, 93);
  s_bar_vram = mk_bar(s_scr, C_SYS_VRAM, 8, 115, 158, 10);
  s_lbl_vram_sub = mk_label(s_scr, &lv_font_montserrat_14, C_TEXT2, LV_ALIGN_TOP_LEFT, 8, 129, "-- / -- GB");

  // 右半:CPU 环
  s_arc_cpu = lv_arc_create(s_scr);
  lv_obj_set_size(s_arc_cpu, 96, 96);
  lv_obj_align(s_arc_cpu, LV_ALIGN_TOP_RIGHT, -19, 38);
  lv_obj_remove_flag(s_arc_cpu, LV_OBJ_FLAG_CLICKABLE);
  lv_arc_set_bg_angles(s_arc_cpu, 135, 45);   // 270° 缺口在下
  lv_arc_set_range(s_arc_cpu, 0, 100);
  lv_arc_set_value(s_arc_cpu, 0);
  lv_obj_set_style_arc_color(s_arc_cpu, lv_color_hex(C_TRACK), LV_PART_MAIN);
  lv_obj_set_style_arc_width(s_arc_cpu, 10, LV_PART_MAIN);
  lv_obj_set_style_arc_rounded(s_arc_cpu, true, LV_PART_MAIN);
  lv_obj_set_style_arc_color(s_arc_cpu, lv_color_hex(C_SYS_CPU), LV_PART_INDICATOR);
  lv_obj_set_style_arc_width(s_arc_cpu, 10, LV_PART_INDICATOR);
  lv_obj_set_style_arc_rounded(s_arc_cpu, true, LV_PART_INDICATOR);
  lv_obj_set_style_anim_duration(s_arc_cpu, 500, LV_PART_INDICATOR);
  lv_obj_set_style_bg_opa(s_arc_cpu, LV_OPA_TRANSP, LV_PART_KNOB);
  lv_obj_set_style_pad_all(s_arc_cpu, 0, LV_PART_KNOB);

  s_lbl_cpu_pct = lv_label_create(s_arc_cpu);
  lv_obj_set_style_text_font(s_lbl_cpu_pct, &lv_font_montserrat_28, 0);
  lv_obj_set_style_text_color(s_lbl_cpu_pct, lv_color_hex(C_SYS_CPU), 0);
  lv_label_set_text(s_lbl_cpu_pct, "--");
  lv_obj_align(s_lbl_cpu_pct, LV_ALIGN_CENTER, 0, -7);

  lv_obj_t *lbl_cpu = lv_label_create(s_arc_cpu);
  lv_obj_set_style_text_font(lbl_cpu, &lv_font_montserrat_12, 0);
  lv_obj_set_style_text_color(lbl_cpu, lv_color_hex(C_TEXT2), 0);
  lv_label_set_text(lbl_cpu, "CPU");
  lv_obj_align(lbl_cpu, LV_ALIGN_CENTER, 0, 16);

  // CPU 副读数(温度 / 频率)在环正下方居中
  s_lbl_cpu_sub = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl_cpu_sub, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_cpu_sub, lv_color_hex(C_SYS_CPU), 0);
  lv_obj_set_width(s_lbl_cpu_sub, 110);
  lv_obj_set_style_text_align(s_lbl_cpu_sub, LV_TEXT_ALIGN_CENTER, 0);
  lv_label_set_text(s_lbl_cpu_sub, "--");
  lv_obj_align_to(s_lbl_cpu_sub, s_arc_cpu, LV_ALIGN_OUT_BOTTOM_MID, 0, -4);

  // 右下:统一会话指示方块(N=总会话数含 system;当前亮、其余暗)
  s_sq_cont = lv_obj_create(s_scr);
  lv_obj_remove_style_all(s_sq_cont);
  lv_obj_remove_flag(s_sq_cont, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(s_sq_cont, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
  lv_obj_set_flex_flow(s_sq_cont, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(s_sq_cont, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_column(s_sq_cont, 4, 0);
  lv_obj_align(s_sq_cont, LV_ALIGN_BOTTOM_RIGHT, -17, -8);
  for (int i = 0; i < SQ_MAX; i++) {
    s_squares[i] = mk_box(s_sq_cont, 7, 7, C_DIM, LV_OPA_COVER);
    lv_obj_set_style_radius(s_squares[i], 2, 0);
    lv_obj_add_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
  }

  // 左下:公网 IP(纯文字、无背景;与右上主机名同字号 14)
  s_lbl_net = lv_label_create(s_scr);
  lv_obj_set_style_text_font(s_lbl_net, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s_lbl_net, lv_color_hex(C_TEXT2), 0);
  lv_label_set_text(s_lbl_net, "--");
  lv_obj_align(s_lbl_net, LV_ALIGN_BOTTOM_LEFT, 8, -6);
}

static void sys_set_payload(const UsagePayload &p, bool init) {
  s_pl = p;
  uint32_t primary = sys_primary(p);
  uint32_t ram_c = sys_ram(p);
  uint32_t vram_c = sys_vram(p);
  uint32_t cpu_c = sys_cpu(p);
  paint_monitor_icon(s_icon, primary);
  lv_obj_set_style_text_color(s_lbl_title, lv_color_hex(primary), 0);
  lv_obj_set_style_bg_color(s_bar_ram, lv_color_hex(ram_c), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl_ram_pct, lv_color_hex(ram_c), 0);
  lv_obj_set_style_bg_color(s_bar_vram, lv_color_hex(vram_c), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl_vram_pct, lv_color_hex(vram_c), 0);
  lv_obj_set_style_arc_color(s_arc_cpu, lv_color_hex(cpu_c), LV_PART_INDICATOR);
  lv_obj_set_style_text_color(s_lbl_cpu_pct, lv_color_hex(cpu_c), 0);
  lv_obj_set_style_text_color(s_lbl_cpu_sub, lv_color_hex(cpu_c), 0);

  char hst[40];
  fit_w(p.host[0] ? p.host : "--", hst, sizeof hst, 140);   // 至多到屏幕正中
  lv_label_set_text(s_lbl_host, hst);
  lv_label_set_text(s_lbl_net, p.net[0] ? p.net : "--");   // 左下:公网 IP
  lv_label_set_text(s_lbl_cpu_sub, p.cpu_sub[0] ? p.cpu_sub : "--");   // 环下:温度/频率

  // 右上身份块 + 右下会话方块(统一会话指示;system 即列表里的一个 session)
  lv_obj_set_style_bg_color(s_dot, lv_color_hex(primary), 0);
  int cnt = p.inst_cnt; if (cnt > SQ_MAX) cnt = SQ_MAX;
  uint32_t hi = primary;
  for (int i = 0; i < SQ_MAX; i++) {
    if (i < cnt) {
      lv_obj_remove_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
      lv_obj_set_style_bg_color(s_squares[i], lv_color_hex(i == (int)p.inst_idx - 1 ? hi : C_DIM), 0);
    } else {
      lv_obj_add_flag(s_squares[i], LV_OBJ_FLAG_HIDDEN);
    }
  }

  bool intro = init || !s_revealed;
  if (!s_revealed) s_revealed = true;

  // CPU 环
  if (p.cpu_has) {
    int c = pct_i(p.cpu_pct);
    if (intro) intro_anim_to(s_arc_cpu, c, anim_cpu_cb);
    else { lv_arc_set_value(s_arc_cpu, c); lv_label_set_text_fmt(s_lbl_cpu_pct, "%d%%", c); }
  } else { lv_arc_set_value(s_arc_cpu, 0); lv_label_set_text(s_lbl_cpu_pct, "--"); }

  // 内存
  if (p.ram_has) {
    int c = pct_i(p.ram_pct);
    if (intro) intro_anim_to(s_bar_ram, c, anim_ram_cb);
    else { lv_bar_set_value(s_bar_ram, c, LV_ANIM_ON); lv_label_set_text_fmt(s_lbl_ram_pct, "%d%%", c); }
    char gb[24]; fmt_gb(p.ram_used_mb, p.ram_total_mb, gb, sizeof gb); lv_label_set_text(s_lbl_ram_sub, gb);
  } else { lv_bar_set_value(s_bar_ram, 0, LV_ANIM_OFF); lv_label_set_text(s_lbl_ram_pct, "--%"); lv_label_set_text(s_lbl_ram_sub, "-- / -- GB"); }

  // 显存
  if (p.vram_has) {
    int c = pct_i(p.vram_pct);
    if (intro) intro_anim_to(s_bar_vram, c, anim_vram_cb);
    else { lv_bar_set_value(s_bar_vram, c, LV_ANIM_ON); lv_label_set_text_fmt(s_lbl_vram_pct, "%d%%", c); }
    char gb[24]; fmt_gb(p.vram_used_mb, p.vram_total_mb, gb, sizeof gb); lv_label_set_text(s_lbl_vram_sub, gb);
  } else { lv_bar_set_value(s_bar_vram, 0, LV_ANIM_OFF); lv_label_set_text(s_lbl_vram_pct, "--%"); lv_label_set_text(s_lbl_vram_sub, "-- / -- GB"); }
}

static void sys_set_state(DeviceState st) {
  lv_opa_t opa = (st == ST_STALE) ? LV_OPA_60 : LV_OPA_COVER;
  lv_obj_t *dim[] = {s_arc_cpu,      s_bar_ram,     s_bar_vram,     s_lbl_cpu_pct, s_lbl_ram_pct,
                     s_lbl_vram_pct, s_lbl_ram_sub, s_lbl_vram_sub, s_lbl_host,    s_dot,
                     s_sq_cont,      s_lbl_net,     s_lbl_cpu_sub};
  for (lv_obj_t *o : dim) lv_obj_set_style_opa(o, opa, 0);
}

// ---- vtable ----
static void sys_set_data_cb(const void *p, bool init) { sys_set_payload(*(const UsagePayload *)p, init); }
static void sys_on_enter() { sys_intro_gauges(); }   // 切入 system 即从 0 重播增长
// LED:未连/断流→蓝闪;已连→磁盘活动灯(忙=暖琥珀闪 / 闲=柔和蓝)
static void sys_led(DeviceState st, bool, uint8_t disk_lvl) {
  if (st != ST_LIVE) leds_fx_connecting();
  else               leds_fx_disk(disk_lvl);
}

Layout *ui_system_layout() {
  static Layout L = {
    "system",
    nullptr,         // scr:由 main 创建后回填
    sys_build,
    sys_set_data_cb,
    sys_set_state,
    sys_on_enter,
    nullptr,         // on_exit
    nullptr,         // tick(系统数据每帧刷新,无需本地倒计时)
    sys_led,
  };
  return &L;
}
