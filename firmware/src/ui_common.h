#pragma once
#include <lvgl.h>

// ---- HUD 配色(CLAUDE.md §6;两屏共享)----
#define C_BG       0x000000   // 纯黑背景(整个 UI)
#define C_CORAL    0xFF7A4D   // 珊瑚橙
#define C_CYAN     0x4DECEF   // 青
#define C_AMBER    0xFFB44E   // 琥珀
#define C_TRACK    0x1B2330   // 轨道/胶囊底
#define C_TEXT     0xF4F7FB   // 主文字
#define C_TEXT2    0xC4CCD8   // 次文字/标签
#define C_DIM      0x8E99A8   // 暗淡文字
#define C_DIVIDER  0x2A323D   // 分隔线
#define C_CLAUDE   0xF08A5E   // Claude 橙(字标 + logo)
#define C_CODEX    0x19C37D   // Codex 绿(字标 + logo)
#define C_SYSTEM   0x6DCBFF   // System 浅蓝(字标 + 身份色)
#define C_SYS_RAM  0x2DE2E6   // System RAM 青蓝
#define C_SYS_VRAM 0x8EA7FF   // System VRAM 蓝紫
#define C_SYS_CPU  C_SYSTEM   // System CPU 环始终使用主题色

#define C_LOGO_DIM 0x39414E   // 文本屏用的暗色 logo 水印(文字盖其上)

// 像素风 Claude logo:在 parent 上画 canvas 对齐到 (x,y),fg_hex 指定前景色(橙/暗)
lv_obj_t *ui_build_logo(lv_obj_t *parent, int x, int y, uint32_t fg_hex);
// 像素风 Codex logo:独立于 Claude logo 的 C/terminal mark。
lv_obj_t *ui_build_codex_logo(lv_obj_t *parent, int x, int y, uint32_t fg_hex);
