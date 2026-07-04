#include "ui_common.h"

// 像素风 Claude logo:由字符画解码成 18x5 位图,x 向 3 倍、y 向 5 倍放大(54x25)
#define LOGO_W 18
#define LOGO_H 5
#define LOGO_SX 3
#define LOGO_SY 5
static const uint8_t s_logo_bits[LOGO_H][LOGO_W] = {
  {0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0},
  {0,0,0,1,1,0,1,1,1,1,1,1,0,1,1,0,0,0},
  {0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0},
  {0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0},
  {0,0,0,0,1,0,1,0,0,0,0,1,0,1,0,0,0,0},
};
// 每个 logo 实例自带一块 LVGL 堆缓冲(永久 UI 元素,不释放):两屏颜色可不同
// (仪表盘=品牌橙 C_CLAUDE;文本屏=暗色水印),不再共享缓冲。
lv_obj_t *ui_build_logo(lv_obj_t *parent, int x, int y, uint32_t fg_hex) {
  const int W = LOGO_W * LOGO_SX, H = LOGO_H * LOGO_SY;
  uint8_t *buf = (uint8_t *)lv_malloc(W * H * 2 + 8);
  lv_obj_t *cv = lv_canvas_create(parent);
  lv_obj_remove_style_all(cv);
  lv_canvas_set_buffer(cv, buf, W, H, LV_COLOR_FORMAT_RGB565);
  lv_canvas_fill_bg(cv, lv_color_hex(C_BG), LV_OPA_COVER);
  lv_color_t fg = lv_color_hex(fg_hex);
  for (int r = 0; r < LOGO_H; r++)
    for (int c = 0; c < LOGO_W; c++)
      if (s_logo_bits[r][c])
        for (int yy = 0; yy < LOGO_SY; yy++)
          for (int xx = 0; xx < LOGO_SX; xx++)
            lv_canvas_set_px(cv, c * LOGO_SX + xx, r * LOGO_SY + yy, fg, LV_OPA_COVER);
  lv_obj_align(cv, LV_ALIGN_TOP_LEFT, x, y);
  return cv;
}

// Codex mark: compact 5x7 pixel "CODEX" wordmark, scaled to match the
// blocky terminal banner used by the Codex CLI wrapper.
#define CODEX_W 29
#define CODEX_H 7
#define CODEX_SX 3
#define CODEX_SY 3
static const uint8_t s_codex_bits[CODEX_H][CODEX_W] = {
  {0,1,1,1,0,0,0,1,1,1,0,0,1,1,1,1,0,0,1,1,1,1,1,0,1,0,0,0,1},
  {1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,0,0,1,0,0,0,1},
  {1,0,0,0,0,0,1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,0,0,0,1,0,1,0},
  {1,0,0,0,0,0,1,0,0,0,1,0,1,0,0,0,1,0,1,1,1,1,0,0,0,0,1,0,0},
  {1,0,0,0,0,0,1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,0,0,0,1,0,1,0},
  {1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,1,0,1,0,0,0,0,0,1,0,0,0,1},
  {0,1,1,1,0,0,0,1,1,1,0,0,1,1,1,1,0,0,1,1,1,1,1,0,1,0,0,0,1},
};

lv_obj_t *ui_build_codex_logo(lv_obj_t *parent, int x, int y, uint32_t fg_hex) {
  const int W = CODEX_W * CODEX_SX, H = CODEX_H * CODEX_SY;
  uint8_t *buf = (uint8_t *)lv_malloc(W * H * 2 + 8);
  lv_obj_t *cv = lv_canvas_create(parent);
  lv_obj_remove_style_all(cv);
  lv_canvas_set_buffer(cv, buf, W, H, LV_COLOR_FORMAT_RGB565);
  lv_canvas_fill_bg(cv, lv_color_hex(C_BG), LV_OPA_COVER);
  lv_color_t fg = lv_color_hex(fg_hex);
  for (int r = 0; r < CODEX_H; r++)
    for (int c = 0; c < CODEX_W; c++)
      if (s_codex_bits[r][c])
        for (int yy = 0; yy < CODEX_SY; yy++)
          for (int xx = 0; xx < CODEX_SX; xx++)
            lv_canvas_set_px(cv, c * CODEX_SX + xx, r * CODEX_SY + yy, fg, LV_OPA_COVER);
  lv_obj_align(cv, LV_ALIGN_TOP_LEFT, x, y);
  return cv;
}
