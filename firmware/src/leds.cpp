#include "leds.h"
#include <Arduino.h>
#include <math.h>
#include <Adafruit_NeoPixel.h>
#include "board_config.h"

// ⚠️ 该板载 WS2812 字节序为 RGB(非常见的 GRB)——实测:按 GRB 驱动会把「绿」错送到红通道,
//    导致蓝色掺红发紫、工作色红绿对调。用 NEO_RGB 后各颜色才正确。
static Adafruit_NeoPixel s_px(CM_LED_COUNT, CM_LED_PIN, NEO_RGB + NEO_KHZ800);

#define C_BLUE  0x0050C0u                 // 深蓝(零红;通道顺序修正后才不偏紫):连接中 / 已连未工作
static const float TWO_PI_F = 6.2831853f;

static inline uint8_t cR(uint32_t c) { return (c >> 16) & 0xFF; }
static inline uint8_t cG(uint32_t c) { return (c >> 8) & 0xFF; }
static inline uint8_t cB(uint32_t c) { return c & 0xFF; }

void leds_begin() {
  s_px.begin();
  s_px.setBrightness(180);   // 总亮度上限(单颗 WS2812 满亮过刺眼);各态再用自己的 k 调
  s_px.clear();
  s_px.show();
  randomSeed(micros());      // 工作灯效三通道随机周期用
}

static uint32_t s_flash_until = 0;
void leds_flash() { s_flash_until = millis() + 250; }

// 低层:~50Hz 节流 + 白扫光覆盖(闪光期间强制白色,压过布局选的任何效果)+ 按 k 铺色 show()。
void leds_fill(uint32_t c, float k) {
  static uint32_t last = 0;
  uint32_t now = millis();
  if (now - last < 20) return;   // ~50Hz 足够顺滑,省 show() 开销(单颗约 30µs)
  last = now;

  if ((int32_t)(s_flash_until - now) > 0) { c = 0xFFFFFFu; k = 1.0f; }  // 切屏白扫光覆盖
  if (k < 0) k = 0;
  if (k > 1) k = 1;
  uint32_t col = s_px.Color((uint8_t)(cR(c) * k), (uint8_t)(cG(c) * k), (uint8_t)(cB(c) * k));
  for (int i = 0; i < CM_LED_COUNT; i++) s_px.setPixelColor(i, col);
  s_px.show();
}

// ---- 现成效果 ----
void leds_fx_connecting() {            // 深蓝慢闪(2 秒 1 次短脉冲,灭时留底光)
  bool on = (millis() % 2000) < 350;
  leds_fill(C_BLUE, on ? 1.0f : 0.06f);
}

void leds_fx_idle() {                  // 深蓝常亮(柔和)
  leds_fill(C_BLUE, 0.55f);
}

void leds_fx_working() {               // 固定周期呼吸 + 每次呼吸随机一个颜色
  // 回到最初的固定周期(每个 PERIOD 一次呼吸);颜色不再固定 6 色轮转,而是每次呼吸(谷底)随机一个
  // R/G/B,但三分量之和 ≤ 512(避免发白)→ 呼吸频率固定、颜色随机。
  const uint32_t PERIOD = 1200;
  uint32_t now = millis();
  uint32_t phase = now % PERIOD;
  uint32_t cycle = now / PERIOD;
  static uint32_t last_cycle = 0xFFFFFFFFu;
  static uint8_t cr = 255, cg = 80, cb = 0;        // 当前这次呼吸的颜色
  if (cycle != last_cycle) {                        // 进入新呼吸 → 重掷颜色(谷底处换,不刺眼)
    last_cycle = cycle;
    int rr = random(0, 256), gg = random(0, 256), bb = random(0, 256);
    int sum = rr + gg + bb;
    if (sum > 512) { rr = rr * 512 / sum; gg = gg * 512 / sum; bb = bb * 512 / sum; }  // 封顶总分量 ≤512
    cr = (uint8_t)rr; cg = (uint8_t)gg; cb = (uint8_t)bb;
  }
  float s = (1.0f - cosf(TWO_PI_F * (float)phase / (float)PERIOD)) * 0.5f;  // 0→1→0
  float bright = 0.12f + 0.88f * s;
  leds_fill(((uint32_t)cr << 16) | ((uint32_t)cg << 8) | cb, bright);
}

void leds_fx_disk(uint8_t level) {     // 磁盘活动灯(阶段12,system 屏):忙=暖琥珀平滑呼吸,闲=柔和蓝
  // 迟滞:level≥ON 进入活动并记时;活动中 level≤OFF 且保持期已过才退出 ——
  //   避免磁盘活动量在帧间抖动(忽高忽低)造成 LED 乱闪。
  static bool active = false;
  static uint32_t last_busy_ms = 0;
  const uint8_t DISK_ON = 24, DISK_OFF = 8;
  const uint32_t HOLD_MS = 1600;        // 活动后至少保持呼吸这么久,跨过突发写之间的空隙
  uint32_t now = millis();
  if (level >= DISK_ON) { active = true; last_busy_ms = now; }
  else if (active && level <= DISK_OFF && (now - last_busy_ms) > HOLD_MS) { active = false; }

  if (!active) { leds_fx_idle(); return; }   // 空闲:柔和蓝
  // 活动:固定周期平滑呼吸(琥珀)。亮度由 millis 余弦驱动 → 与帧率/活动量抖动无关,不乱闪。
  const uint32_t PERIOD = 1100;
  float s = (1.0f - cosf(TWO_PI_F * (float)(now % PERIOD) / (float)PERIOD)) * 0.5f;  // 0→1→0
  leds_fill(0xFFC061u, 0.16f + 0.84f * s);
}
