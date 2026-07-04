// U盘屏布局(阶段11 #4 起 debug/release 共用):深紫底 + 标题 + 两行说明,长按 BOOT 弹模式菜单。
#include "ui_udisk.h"
#include "leds.h"
#include <lvgl.h>

#define UD_BG     0x240033   // 深紫底(区分正常的深色 HUD)
#define UD_TITLE  0xFFFFFF   // 标题:白
#define UD_SUB    0xC89AFF   // 说明:浅紫

static void udisk_build(lv_obj_t *scr) {
  lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_style_bg_color(scr, lv_color_hex(UD_BG), 0);
  lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);
  lv_obj_set_style_pad_all(scr, 0, 0);

  lv_obj_t *title = lv_label_create(scr);
  lv_obj_set_style_text_font(title, &lv_font_montserrat_20, 0);
  lv_obj_set_style_text_color(title, lv_color_hex(UD_TITLE), 0);
  lv_label_set_text(title, "U-DISK MODE");
  lv_obj_align(title, LV_ALIGN_CENTER, 0, -28);

  lv_obj_t *s1 = lv_label_create(scr);
  lv_obj_set_style_text_font(s1, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s1, lv_color_hex(UD_SUB), 0);
  lv_label_set_text(s1, "Installer drive mounted");
  lv_obj_align(s1, LV_ALIGN_CENTER, 0, 6);

  lv_obj_t *s2 = lv_label_create(scr);
  lv_obj_set_style_text_font(s2, &lv_font_montserrat_14, 0);
  lv_obj_set_style_text_color(s2, lv_color_hex(UD_SUB), 0);
  lv_label_set_text(s2, "Hold BOOT to switch mode");
  lv_obj_align(s2, LV_ALIGN_CENTER, 0, 30);
}

static void udisk_led(DeviceState, bool, uint8_t) { leds_fill(0xFF00FF, 0.28f); }   // 品红常亮:区分色

Layout *ui_udisk_layout() {
  static Layout L = {
    "udisk",       // id
    nullptr,       // scr(main 创建后回填)
    udisk_build,   // build
    nullptr,       // set_data
    nullptr,       // set_state
    nullptr,       // on_enter
    nullptr,       // on_exit
    nullptr,       // tick
    udisk_led,     // led
  };
  return &L;
}
