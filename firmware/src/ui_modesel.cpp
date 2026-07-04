// 模式选择界面(阶段11 #1;#4 起正常/U盘两态共用):统一框架下的又一个 Layout。**debug/release 共用**。
// 英文菜单 + 内置 Montserrat 字体;无标题/无提示,只两行模式;选中=亮白+大、未选中=暗白+小(纯亮度+字号区分)。
// 收尾:① 弹出时优先高亮「当前模式」(由 main 在弹菜单前 modesel_set_current 设);
//       ② 切换时字号带「变大/变小」动画(transform_scale,绕中心)。
#include "ui_modesel.h"
#include "leds.h"        // 菜单态 LED
#include <lvgl.h>

static const char *MODE_NAMES[2] = { "Normal Mode", "U-Disk Mode" };
static lv_obj_t   *s_rows[2];
static int         s_sel = 1;       // 当前高亮项(0=Normal / 1=U-Disk)
static int         s_current = 1;   // 设备「当前所处模式」:弹菜单前由 main 设,on_enter 默认高亮它

#define MODE_SEL_COLOR    0xFFFFFF   // 选中:亮白
#define MODE_UNSEL_COLOR  0x707888   // 未选中:暗白(灰)
#define SCALE_SEL    256             // 选中:100%(Montserrat 20 原大小,LV_SCALE_NONE,清晰)
#define SCALE_UNSEL  210             // 未选中:~82%(字号小一点,但不过小)

// ---- 字号缩放(绕中心):切换时带变大/变小动画,进入时瞬时套用 ----
static void scale_cb(void *o, int32_t v) {
  lv_obj_set_style_transform_scale_x((lv_obj_t *)o, v, 0);
  lv_obj_set_style_transform_scale_y((lv_obj_t *)o, v, 0);
}
static void scale_to(lv_obj_t *o, int32_t to, bool animate) {
  lv_anim_delete(o, scale_cb);                 // 停掉在跑的缩放动画
  if (!animate) { scale_cb(o, to); return; }   // 进入/重画:瞬时
  lv_anim_t a; lv_anim_init(&a);
  lv_anim_set_var(&a, o);
  lv_anim_set_values(&a, lv_obj_get_style_transform_scale_x(o, LV_PART_MAIN), to);
  lv_anim_set_duration(&a, 160);
  lv_anim_set_path_cb(&a, lv_anim_path_ease_out);
  lv_anim_set_exec_cb(&a, scale_cb);
  lv_anim_start(&a);
}

// 应用选中/未选中的「色 + 字号」;animate=true 时字号带变大变小动画(切换用)
static void apply_styles(bool animate) {
  for (int i = 0; i < 2; i++) {
    bool sel = (i == s_sel);
    lv_obj_set_style_text_color(s_rows[i], lv_color_hex(sel ? MODE_SEL_COLOR : MODE_UNSEL_COLOR), 0);
    scale_to(s_rows[i], sel ? SCALE_SEL : SCALE_UNSEL, animate);
  }
}

static void modesel_build(lv_obj_t *scr) {
  lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(scr, lv_color_black(), 0);
  lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(scr, 0, 0);

  for (int i = 0; i < 2; i++) {
    s_rows[i] = lv_label_create(scr);
    lv_obj_set_style_text_font(s_rows[i], &lv_font_montserrat_20, 0);
    lv_label_set_text(s_rows[i], MODE_NAMES[i]);
    lv_obj_align(s_rows[i], LV_ALIGN_CENTER, 0, i == 0 ? -24 : 24);
    // 缩放绕自身中心(否则缩放会朝右下偏移、行会跑位)
    lv_obj_set_style_transform_pivot_x(s_rows[i], LV_PCT(50), 0);
    lv_obj_set_style_transform_pivot_y(s_rows[i], LV_PCT(50), 0);
  }
  apply_styles(false);
}

// 每次切入菜单:高亮「当前模式」s_current(不再强制 U盘),瞬时套用(无动画)
static void modesel_on_enter() { s_sel = s_current; apply_styles(false); }

// 菜单态 LED:稳定琥珀(区分正常的蓝/青/工作色与 U盘的品红,提示「在选模式」)
static void modesel_led(DeviceState, bool, uint8_t) { leds_fill(0xFFB44E, 0.4f); }

void modesel_set_current(int cur) { s_current = (cur == 1) ? 1 : 0; }
void modesel_next() { s_sel = (s_sel + 1) % 2; apply_styles(true); }   // 切换:字号带变大变小动画
int  modesel_selected() { return s_sel; }

Layout *ui_modesel_layout() {
  static Layout L = {
    "modesel",        // id
    nullptr,          // scr(main 创建后回填)
    modesel_build,    // build
    nullptr,          // set_data(无数据)
    nullptr,          // set_state
    modesel_on_enter, // on_enter
    nullptr,          // on_exit
    nullptr,          // tick
    modesel_led,      // led
  };
  return &L;
}
