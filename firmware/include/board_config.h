#pragma once
// =============================================================================
// code_mate — Waveshare ESP32-S3-LCD-1.47 板级定义 (board_config.h)
// -----------------------------------------------------------------------------
// 来源:CLAUDE.md §2「已核实引脚表」(Waveshare 官方 wiki + TFT_eSPI 讨论 #3527,
//       多源一致)。⚠️ 阶段0 仍须上板复核;旋转值必须实测(见文末)。
//
// 注意:TFT_eSPI 实际用的像素引脚由 platformio.ini 的 build_flags 注入
//       (TFT_MOSI/SCLK/CS/DC/RST/BL 等)——那里是唯一事实来源。本文件镜像
//       同一组值供业务代码引用、记录复核结论与旋转结果,改动两处需保持一致。
// =============================================================================

// ---- LCD (ST7789, 172x320, SPI, 颜色序 BGR) ----
#define CM_LCD_MOSI   45
#define CM_LCD_SCLK   40
#define CM_LCD_CS     42
#define CM_LCD_DC     41
#define CM_LCD_RST    39
#define CM_LCD_BL     48     // 背光,高电平点亮(TFT_eSPI 经 TFT_BL/TFT_BACKLIGHT_ON 自动驱动)
#define CM_LCD_W      172    // 物理宽(竖向);横向使用时逻辑为 320x172
#define CM_LCD_H      320

// ---- 板载外设 ----
#define CM_RGB_LED    38     // 板载 RGB LED 数据脚(WS2812 兼容)

// ---- BOOT 按钮(阶段7:多窗口切换)----
// GPIO0 = 板载 BOOT 键(strapping 脚)。⚠️ 仅在 setup()(boot 之后)配置为输入,
// 不影响进入下载模式 / 正常 boot;按下=接地(LOW),用内部上拉。上板复核。
#define CM_BTN_PIN    0

// ---- LED 灯效(阶段5)----
// 来源:Waveshare 官方 wiki(https://www.waveshare.net/wiki/ESP32-S3-LCD-1.47):
//   板载**仅 1 颗** WS2812 兼容 RGB,数据脚 GPIO38;wiki 无外接灯带说明。
// 外接 WS2812 灯环:把 CM_LED_COUNT 改成 N(灯环 DIN 串在 GPIO38 链路后,板载为像素 0)。
#define CM_LED_PIN    CM_RGB_LED
#define CM_LED_COUNT  1

// ---- TF / microSD (SDMMC) —— 阶段0 不使用,先记录 ----
#define CM_SD_CMD     15
#define CM_SD_CLK     14
#define CM_SD_D0      16
#define CM_SD_D1      18
#define CM_SD_D2      17
#define CM_SD_D3      21

// ---- 屏幕旋转 (TFT_eSPI setRotation) ----
// 需求(CLAUDE.md §2):接头在左、文字正立、仪表盘在右(右侧插入的横向镜像)。
// 候选:setRotation(1) 与 (3) 是两个 180° 相反的横向。
// 阶段0 任务5:两个都烧一遍,选「文字正立、接头在左」的那个,把结论填到这里。
#define CM_LCD_ROTATION 3   // ✅ 2026-06 上板实测确认:文字正立、接头在左(候选 1 为上下颠倒)
