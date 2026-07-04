#include "input.h"
#include "board_config.h"
#include <Arduino.h>

// 去抖:电平连续稳定 DEBOUNCE_MS 才认。GPIO0 上拉:HIGH=未按,LOW=按下。
#define DEBOUNCE_MS 25
#define LONG_MS     700   // 按住 ≥ 此时长 = 长按(阶段11 #1:进/应用模式选择菜单)

static int      s_stable = HIGH;     // 去抖后的稳定电平
static int      s_last = HIGH;       // 上一次原始读数
static uint32_t s_change_ms = 0;     // 原始电平最近一次变动时刻
static uint32_t s_press_ms = 0;      // 本次按下的时刻(算长按用)
static bool     s_long_fired = false; // 本次按住已触发过 LONG(避免重复 + 弹起不再算短按)

void input_begin() {
  pinMode(CM_BTN_PIN, INPUT_PULLUP);   // boot 之后才配置(strapping 脚,见 board_config.h)
  s_stable = digitalRead(CM_BTN_PIN);
  s_last = s_stable;
  s_change_ms = millis();
  s_long_fired = false;
}

BtnEvent input_poll() {
  int raw = digitalRead(CM_BTN_PIN);
  uint32_t now = millis();
  if (raw != s_last) {                 // 原始电平变动 → 重置去抖计时
    s_last = raw;
    s_change_ms = now;
  }
  // 去抖后稳定电平变化 → 按下 / 弹起
  if (raw != s_stable && (now - s_change_ms) >= DEBOUNCE_MS) {
    s_stable = raw;
    if (s_stable == LOW) {             // 按下
      s_press_ms = now;
      s_long_fired = false;
      return BTN_DOWN;
    }
    // 弹起:长按已触发过 → 吞掉(别再当短按);否则算短按
    return s_long_fired ? BTN_NONE : BTN_SHORT;
  }
  // 持续按住:跨过长按阈值 → 触发一次 LONG(松手前生效)
  if (s_stable == LOW && !s_long_fired && (now - s_press_ms) >= LONG_MS) {
    s_long_fired = true;
    return BTN_LONG;
  }
  return BTN_NONE;
}
