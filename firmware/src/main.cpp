// =============================================================================
// code_mate — 阶段1:cc_usage 屏(LVGL v9.5)+ 串口协议 + 状态机
//   - 内置假 payload 离线即显示完整屏(WAIT/DEMO)
//   - 收到串口 data 帧 → LIVE 刷新;断流 > 30s → STALE
//   - 倒计时本地用 millis() 每秒递减
// 配置见 platformio.ini / include/lv_conf.h;协议见 CLAUDE.md §5
// =============================================================================
#include <Arduino.h>
#include <lvgl.h>
#include <TFT_eSPI.h>
#include "board_config.h"
#include "app_types.h"
#include "protocol.h"
#include "ui_cc_usage.h"
#include "ui_text_scroll.h"
#include "ui_loading.h"
#include "ui_system.h"   // 阶段12:系统监控屏(CPU/内存/显存)
#include "ui_modesel.h"   // 阶段11 #1:模式选择菜单(debug/release 共用,纯 UI)
#include "ui_udisk.h"     // 阶段11 #4:U盘屏(debug 预览 / release 真 MSC 显示)
#include "ui_common.h"
#include "layout.h"
#include "input.h"
#include "leds.h"
#include "usb_disk.h"   // 阶段10:双模式 USB(只读 U 盘 / MSC + NVS 切换;#ifdef CM_USB_DISK)
#include "driver/gpio.h"
#include <string.h>

// 背光极性(该板 active-high)。**不**用 TFT_eSPI 的 TFT_BACKLIGHT_ON(已从 build_flags 移除):
// 否则 TFT_eSPI init 会自行拉亮背光,而它的 DISPON 这时显示的是面板上电的白 GRAM → 开机白闪。
// 诊断(CM_BLANK_TEST 完全不启动显示)确认:不碰显示就不闪 → 白闪是 init 引起的、固件可修。
// 全程背光由本文件手控:关到画好首屏才开。
#define CM_BL_ON  HIGH
#define CM_BL_OFF LOW

// 静态初始化期(`__libc_init_array`,先于 initArduino/USB)就把背光拉到「关」,尽早压住。
struct BlOffEarly {
  BlOffEarly() {
    gpio_set_direction((gpio_num_t)TFT_BL, GPIO_MODE_OUTPUT);
    gpio_set_level((gpio_num_t)TFT_BL, 0);   // 0=LOW=关(active-high)
  }
};
static BlOffEarly s_bl_off_early;

// 阶段12 #4:冷启动背光「渐亮」——面板冷态上电时 charge pump/VCOM 未稳(发暗+横条,约 3~5s 自愈)。
// 改用 LEDC PWM(官方原生背光方式)在首帧渲染后从 0 二次 ease-in 渐亮到全亮:低亮度段盖住横条、
// 渐亮期间面板正好同步稳定。非阻塞(loop 里 bl_tick 推进);首帧前 BL 仍由 GPIO 拉低 → 不回退防白闪。
#define BL_FREQ      20000   // PWM 频率(Hz),避开可见/可闻闪烁
#define BL_RES_BITS  8       // 分辨率(占空 0~255)
#define BL_MAX_DUTY  255     // 全亮占空
#define BL_FADE_MS   2000    // 渐亮时长(可调);与开机 logo splash(SPLASH_MS)同步:2s 内由暗渐亮
#define BL_HOLD_MS   0       // 渐亮前额外全黑保持。实测加大无助于右上白块——那是面板/背光「上电点亮」的硬件瞬态(整屏闪、右上为热点),非时序,纯软件压不掉
static bool     s_bl_fade_active = false;
static uint32_t s_bl_t0          = 0;
static int      s_bl_last_duty   = -1;   // 去抖:仅占空变化才 ledcWrite

// 开始背光渐亮:把 BL 引脚接管到 LEDC(初值占空 0=灭,接管前 GPIO 本就低 → 无闪),记起点。
static void bl_begin_fade() {
  ledcAttach(TFT_BL, BL_FREQ, BL_RES_BITS);
  ledcWrite(TFT_BL, 0);
  s_bl_last_duty   = 0;
  s_bl_t0          = millis();
  s_bl_fade_active = true;
}

// 每拍推进渐亮(loop 调):HOLD 段维持灭,之后二次 ease-in 拉亮到全亮;占空变化才写,到顶即停。
static void bl_tick() {
  if (!s_bl_fade_active) return;
  uint32_t el = millis() - s_bl_t0;
  int duty;
  if (el < BL_HOLD_MS) {
    duty = 0;
  } else {
    uint32_t fe = el - BL_HOLD_MS;
    if (fe >= BL_FADE_MS) {
      duty = BL_MAX_DUTY;
    } else {
      float p = (float)fe / (float)BL_FADE_MS;     // 0..1
      duty = (int)(BL_MAX_DUTY * p * p + 0.5f);    // 二次 ease-in:前段暗、尾段亮
    }
  }
  if (duty != s_bl_last_duty) { ledcWrite(TFT_BL, duty); s_bl_last_duty = duty; }
  if (duty >= BL_MAX_DUTY) s_bl_fade_active = false;
}

static TFT_eSPI tft = TFT_eSPI();

static const uint16_t DISP_W = 320;
static const uint16_t DISP_H = 172;
// 局部绘制缓冲:1/4 屏高,内部 RAM(DMA 友好),~25KB
static uint16_t s_drawbuf[DISP_W * 40];

static DeviceState s_state = ST_BOOT;
static bool s_cc_running = false;       // CC 是否在工作(来自 data 帧 cc_running),驱动 LED
static uint8_t s_disk_lvl = 0;          // 阶段12:磁盘活动级别(来自 system 帧 disk),驱动 system 屏 LED
static bool g_udisk_mode = false;       // true=本次 boot 进了 U盘模式(release 起真 MSC;debug 仅 U盘屏)
static uint32_t s_last_data_ms = 0;
static uint32_t s_last_tick_ms = 0;
static const uint32_t STALE_MS = 30000;

// 开机 logo splash(阶段12 #5):开机后至少展示 SPLASH_MS 的纯 logo(与背光渐亮同窗口)——
// 期间**推迟 host 切屏、不出等待条**;满 2s 后:已连(splash 内收到 cfg/data)→ 切到目标屏(等待条不出现);
// 未连 → 揭出等待条。从 s_bl_t0(首帧渲染/渐亮起点)计时。设备开机即可握手连接(loop 早于 2s 就在跑)。
#define SPLASH_MS 2000
static bool    s_splash_done   = false;
static Layout *s_splash_target = nullptr;   // splash 内 host 想切到的真实屏(3s 到再切;null=未连→显等待条)

// ---- 布局注册表(阶段7):dashboard(仪表盘) + terminal(文本滚动)----
//   用 Layout vtable + 注册表取代写死的屏指针 + 字符串 if-else;main 按布局 id 查表派发。
static Layout *g_layouts[8];   // loading/dashboard/terminal/modesel/udisk;留余量
static int     g_layout_n = 0;
static Layout *g_cur      = nullptr;
static Layout *g_loading  = nullptr;   // 占位屏(开机默认;无会话/未连)
static Layout *g_dash     = nullptr;   // 已知布局指针(EV_DATA/EV_TEXT 路由用)
static Layout *g_term     = nullptr;
static Layout *g_system   = nullptr;   // 阶段12:系统监控屏(无 CC 会话默认 + BOOT 循环常驻项)
static Layout *g_modesel  = nullptr;   // 阶段11 #1:模式选择菜单(长按 BOOT 进;debug/release 共用)
static Layout *g_udisk    = nullptr;   // 阶段11 #4:U盘屏(debug 预览 / release 真 MSC 显示)
static Layout *g_app_layout = nullptr; // 最近的「app 屏」(loading/dash/term):菜单选 Normal 时回到它
static lv_display_t *g_disp = nullptr; // 显示句柄(setup 内 lv_refr_now 用)

static Layout *find_layout(const char *id) {
  for (int i = 0; i < g_layout_n; i++)
    if (!strcmp(g_layouts[i]->id, id)) return g_layouts[i];
  return nullptr;
}

// 切到某布局 + LED 白扫光;on_exit/on_enter 各布局自理。
// 统一过场约定(阶段11):**只有仪表盘↔终端有方向滑屏**,其余一律「从黑直接出现」(无滑屏)。
//   仪表盘→终端 = 下→上(OVER_TOP:新屏自底部向上,LVGL 命名反直觉);
//   终端→仪表盘 = 上→下(OVER_BOTTOM:新屏自顶部向下);
//   其它(loading / 模式菜单 / U盘屏 进出)= NONE 瞬切——配合长按时的黑遮罩,呈「从黑中出现」。
// on_enter 总会跑(即便已在该屏)——保留原「cfg terminal 必清屏 / cfg dashboard 必揭开」语义。
static void switch_to_layout(Layout *target) {
  if (!target) return;
  if (target != g_cur) {
    if (g_cur && g_cur->on_exit) g_cur->on_exit();
    lv_screen_load_anim_t anim = LV_SCREEN_LOAD_ANIM_NONE;            // 默认:从黑直接出现
    if (g_cur == g_dash && target == g_term)      anim = LV_SCREEN_LOAD_ANIM_OVER_TOP;     // 仪表盘→终端:下→上
    else if (g_cur == g_term && target == g_dash) anim = LV_SCREEN_LOAD_ANIM_OVER_BOTTOM;  // 终端→仪表盘:上→下
    lv_screen_load_anim(target->scr, anim, anim == LV_SCREEN_LOAD_ANIM_NONE ? 0 : 300, 0, false);
    g_cur = target;
    leds_flash();   // 板载 LED 白扫光(物理切换提示)
  }
  if (target->on_enter) target->on_enter();
  // 记住最近的「app 屏」(非菜单/U盘):菜单选 Normal 退回它
  if (target == g_loading || target == g_dash || target == g_term || target == g_system) g_app_layout = target;
}

// ---- 实例切换黑遮罩(阶段7)----
// 置于 LVGL top layer,盖住任何布局:BOOT 按下立刻黑屏(本地反馈)、收到新实例数据后揭开。
// 与 dashboard 内的 CONNECTING 覆盖层是**两个不同层**,互不干扰。
static lv_obj_t      *s_black = nullptr;
static uint32_t       s_black_ms = 0;            // 显示时刻(失败保护:超时自动揭开,杜绝卡黑屏)
static const uint32_t BLACK_TIMEOUT_MS = 2000;

static void black_show() { if (s_black) { lv_obj_remove_flag(s_black, LV_OBJ_FLAG_HIDDEN); s_black_ms = millis(); } }
static void black_hide() { if (s_black) lv_obj_add_flag(s_black, LV_OBJ_FLAG_HIDDEN); }
static bool black_shown() { return s_black && !lv_obj_has_flag(s_black, LV_OBJ_FLAG_HIDDEN); }

static uint32_t tick_cb() { return millis(); }

static void flush_cb(lv_display_t *disp, const lv_area_t *area, uint8_t *px_map) {
  uint32_t w = area->x2 - area->x1 + 1;
  uint32_t h = area->y2 - area->y1 + 1;
  tft.startWrite();
  tft.setAddrWindow(area->x1, area->y1, w, h);
  tft.pushPixels((uint16_t *)px_map, w * h);  // 字节序由 setSwapBytes(true) 处理
  tft.endWrite();
  lv_display_flush_ready(disp);
}

// ---- LVGL 初始化 + 布局登记小工具(setup 的 normal / MSC 两路共用)----
static void lvgl_init() {
  lv_init();
  lv_tick_set_cb(tick_cb);
  g_disp = lv_display_create(DISP_W, DISP_H);
  lv_display_set_flush_cb(g_disp, flush_cb);
  lv_display_set_buffers(g_disp, s_drawbuf, NULL, sizeof(s_drawbuf),
                         LV_DISPLAY_RENDER_MODE_PARTIAL);
}
static void register_layout(Layout *L) {
  L->scr = lv_obj_create(NULL);   // v9:父=NULL 即建一个屏
  L->build(L->scr);
  g_layouts[g_layout_n++] = L;
}

void setup() {
#ifdef CM_BLANK_TEST
  while (true) { delay(1000); }   // 诊断:完全不启动显示/背光,观察上电纯硬件态是否白闪
#endif
#ifdef CM_BLACK_LIT_TEST
  // 诊断:init + 整屏填黑 + 背光全亮,但**完全不跑 LVGL/不画任何 UI**,冻在这里。
  // 若这块「本该全黑」的屏仍现右上白块 → 面板/ST7789 init 层面(fillScreen 没盖住 / 局部反相),与 UI 无关;
  // 若干净全黑 → 白块来自 LVGL 渲染。
  pinMode(TFT_BL, OUTPUT); digitalWrite(TFT_BL, CM_BL_OFF);
  tft.init(); tft.setRotation(CM_LCD_ROTATION); tft.setSwapBytes(true);
  tft.fillScreen(TFT_BLACK);
  digitalWrite(TFT_BL, CM_BL_ON);
  while (true) { delay(1000); }
#endif
  // 背光先关:消除开机闪屏 —— ST7789 上电 framebuffer 是花屏,TFT_eSPI 在清屏前就拉亮背光会闪一下。
  // 先关背光、画好首屏(splash / U盘屏)再开,第一眼就是正确内容。
  pinMode(TFT_BL, OUTPUT);
  digitalWrite(TFT_BL, CM_BL_OFF);
  tft.init();                    // TFT_eSPI init 末尾会拉高 BL → 下一句立刻再关
  digitalWrite(TFT_BL, CM_BL_OFF);
  tft.setRotation(CM_LCD_ROTATION);
  tft.setSwapBytes(true);
  // 背光仍关时,先用 TFT 直接把**整块面板**填黑——覆盖 LVGL 够不着的边角(172 宽屏列/行偏移 +
  // 圆角,LVGL 只画 320×172、面板边缘那一小条它盖不到 → 否则露出上电白 GRAM,表现为右下三角白闪)。
  tft.fillScreen(TFT_BLACK);

  // 最早读 NVS 持久化模式:上次保存为「U-Disk」→ 进 U盘模式,不做正常 CDC 初始化。
  // mode_is_udisk() **不清零**(持久):U-Disk 跨复位/拔插保持,直到菜单切回 Normal(mode_save(0)+重启)。
  // debug/release 走同一段逻辑;唯一区别:只有 release 真起 USBMSC(下面 #ifdef)。
  if (mode_is_udisk()) {
    g_udisk_mode = true;
    // U盘模式也跑 LVGL:U盘屏(ui_udisk)+ 长按弹模式菜单。先画好 U盘屏再开背光、(release)再起真 USB。
    lvgl_init();
    g_udisk   = ui_udisk_layout();   register_layout(g_udisk);
    g_modesel = ui_modesel_layout(); register_layout(g_modesel);
    lv_screen_load(g_udisk->scr);
    g_cur = g_udisk;
    g_app_layout = g_udisk;
    lv_refr_now(g_disp);             // 立即画好 U盘屏(背光仍关)
    bl_begin_fade();                 // 背光渐亮(冷启动遮横条):第一眼 U盘屏,无白闪
#ifdef CM_USB_DISK
    msc_usb_begin();                 // 仅 release:真 USBMSC 只读暴露 storage(视觉交给 ui_udisk 布局)
#endif
    input_begin();                   // BOOT 按钮(U盘模式也要响应长按)
    leds_begin();                    // 区分色 LED(由 udisk/modesel 布局 led 钩子驱动)
    return;
  }

  // LVGL + 布局注册。开机首屏直接渲染 loading(与运行时同一个 code_mate)——**不再有单独的 splash**,
  // 消除「暗底 splash(font4) → 黑底 loading(montserrat_28)、字号/底色不一」的开机不统一。
  lvgl_init();

  // 注册布局:loading / dashboard / terminal + modesel(模式菜单) + udisk(U盘屏)。
  // modesel/udisk **debug/release 都注册**(纯 UI);debug 选「U-Disk」只切到 udisk 预览,不真起 MSC。
  g_loading = ui_loading_layout();
  g_dash    = ui_cc_usage_layout();
  g_term    = ui_text_scroll_layout();
  g_system  = ui_system_layout();
  g_modesel = ui_modesel_layout();
  g_udisk   = ui_udisk_layout();
  Layout *reg[] = { g_loading, g_dash, g_term, g_system, g_modesel, g_udisk };
  for (Layout *L : reg) register_layout(L);
  lv_screen_load(g_loading->scr);   // 开机默认 loading(CONNECTING);host 按「有无会话/盒盖」发 cfg 切走
  g_cur = g_loading;
  g_app_layout = g_loading;

  // 实例切换黑遮罩:top layer 全屏黑,默认隐藏(BOOT 按下显示、新数据揭开)
  s_black = lv_obj_create(lv_layer_top());
  lv_obj_remove_style_all(s_black);
  lv_obj_remove_flag(s_black, LV_OBJ_FLAG_SCROLLABLE);
  lv_obj_set_size(s_black, DISP_W, DISP_H);
  lv_obj_set_style_bg_color(s_black, lv_color_black(), 0);
  lv_obj_set_style_bg_opa(s_black, LV_OPA_COVER, 0);
  lv_obj_add_flag(s_black, LV_OBJ_FLAG_HIDDEN);

  // **强制立即渲染** loading 首屏(不等刷新 timer 到点)→ 再开背光:第一眼就是 loading 的 code_mate,
  // 无 splash、无花屏。
  lv_refr_now(g_disp);
  bl_begin_fade();   // 背光渐亮(冷启动遮发暗/横条);非阻塞,loop 里 bl_tick 推进

  input_begin();   // BOOT 按钮(GPIO0;boot 之后才配置)

  // Serial / 协议 / LED 放显示之后:开机视觉优先;CDC 晚几十 ms 起、host 重试无碍。
  Serial.setRxBufferSize(4096);  // host 帧间限速逐步喂;留 2-3 帧抖动余量兜底
  Serial.begin(115200);
  Serial.setTxTimeoutMs(0);      // ⚠️ USB CDC 无人读时写会阻塞、冻住主循环 → 设 0 非阻塞
  delay(300);
  protocol_begin();
  leds_begin();

  // 开机停在 loading(CONNECTING);host 连上后按「有无活会话 + 盒盖」发 cfg 切到 dashboard/terminal
  s_state = ST_WAIT;
  s_last_tick_ms = millis();
  Serial.println("[code_mate] up on loading screen. host drives cfg(loading/dashboard/terminal).");
}

void loop() {
  bl_tick();   // 背光渐亮推进(udisk / 正常两路都经此)
  if (g_udisk_mode) {
    // U盘模式(debug/release 共用):LVGL 跑 U盘屏 + 长按 BOOT 弹模式菜单。无 host / 无实例切换。
    // release 背后另跑真 USBMSC;debug 仅界面与按键(逻辑完全一致)。
    BtnEvent be = input_poll();
    if (g_cur == g_modesel) {                              // 在菜单
      if (be == BTN_LONG) {                                // 长按 = 生效:先持久化所选模式
        int sel = modesel_selected();
        mode_save(sel);                                    // 保存模式(0=Normal / 1=U-Disk)
        if (sel == 0) ESP.restart();                       // 「Normal」→ 重启回 CDC(下次开机读 NVS=0)
        else switch_to_layout(g_udisk);                    // 「U-Disk」→ 已在此模式,退菜单回 U盘屏
      } else if (be == BTN_SHORT) {
        modesel_next();                                    // 短按 = 切换选中项
      }
    } else {                                               // 在 U盘屏
      if (be == BTN_LONG) {                                // 长按 → 进菜单(当前=U盘,默认高亮 U-Disk)
        modesel_set_current(1);
        switch_to_layout(g_modesel);
      }
      // 短按 / 按下:U盘屏无操作
    }
    lv_timer_handler();
    if (g_cur && g_cur->led) g_cur->led(s_state, false, 0);   // udisk=品红 / modesel=琥珀
    delay(5);
    return;
  }
  lv_timer_handler();

  // 开机 logo splash 满 3s:已连→切到目标屏(等待条不出现);未连且仍在 loading→揭出等待条。
  if (!s_splash_done && millis() - s_bl_t0 >= SPLASH_MS) {
    s_splash_done = true;
    if (s_splash_target)         switch_to_layout(s_splash_target);
    else if (g_cur == g_loading) ui_loading_show_bars(true);
  }

  // 逐个取空本轮所有事件:protocol_poll 每次只回一个事件,这里 while 直到 EV_NONE,
  // 各自即时处理(附带数据在下一次 poll 覆盖前就被消费),不会多帧折叠丢失。
  UsagePayload in;
  ProtoEvent ev;
  while ((ev = protocol_poll(in)) != EV_NONE) {
    if (ev == EV_DATA) {            // EV_DATA:按帧 screen 路由到对应布局(dashboard / system)
      Layout *L = find_layout(protocol_data_screen());
      if (!L) L = g_dash;           // 兜底:未知 screen 当 dashboard
      if (!s_splash_done && !s_splash_target && L != g_loading) s_splash_target = L;  // splash 内已连 → 记下目标
      L->set_data(&in, in.init);
      if (in.init && g_cur == L) black_hide();   // 当前就在该布局时,被自己的 init 帧揭黑遮罩
      if (L->set_state) L->set_state(ST_LIVE);
      s_cc_running = in.cc_running; // CC 工作态 → dashboard/terminal LED(呼吸/常亮)
      s_disk_lvl   = in.disk_lvl;   // 磁盘活动 → system 屏 LED
      s_state = ST_LIVE;            // 收到帧 = 已连接 → 保持点亮
      s_last_data_ms = millis();
    } else if (ev == EV_CFG) {      // 切屏(合盖→terminal / 开盖→dashboard):按布局 id 查表
      // 在模式菜单 / U盘预览里:忽略 host 切屏,别把用户从菜单里拽走
      if (g_cur != g_modesel && g_cur != g_udisk) {
        Layout *target = find_layout(protocol_cfg_screen());
        if (!s_splash_done) {                                // splash(开机 3s)内:记下目标但先不切,让 logo 走完
          if (target && target != g_loading) s_splash_target = target;
        } else {
          switch_to_layout(target);
        }
      }
    } else if (ev == EV_TEXT) {     // EV_TEXT = terminal 视图模型:追加一段带样式文本
      const TextPayload &tp = protocol_text();
      g_term->set_data(&tp, false);
      if (tp.clear) black_hide();   // 切实例后的回填(clear)→ 揭开黑遮罩
      s_last_data_ms = millis();    // 文本帧也算"有数据"
    }
  }

  // BOOT 按钮(阶段7 + 阶段11):会话屏短按=实例切换;任意态长按=模式菜单;菜单内短按=切选项、长按=生效。
  BtnEvent be = input_poll();
  if (be == BTN_DOWN) {
    if (g_cur == g_dash || g_cur == g_term || g_cur == g_system)   // 实例/系统屏切换瞬时反馈黑遮罩
      black_show();                           // 若变长按,在 LONG 处隐藏
  } else if (be == BTN_LONG) {
    if (g_cur == g_modesel) {                 // 菜单里长按 = 生效选中模式
      if (modesel_selected() == 1) {          // U-Disk:持久化 + 重启,开机进 U盘模式(release 另起真 MSC)
        mode_save(1); ESP.restart();
      } else {                                // Normal:持久化 + 退回最近 app 屏(已在 CDC,无需重启)
        mode_save(0);
        switch_to_layout(g_app_layout ? g_app_layout : g_loading);
      }
    } else {                                  // 任意态长按 → 进模式选择菜单
      black_hide();
      modesel_set_current(g_cur == g_udisk ? 1 : 0);   // 默认高亮「当前模式」(U盘预览→U-Disk,其余→Normal)
      switch_to_layout(g_modesel);
    }
  } else if (be == BTN_SHORT) {
    if (g_cur == g_modesel)    modesel_next();              // 菜单里短按 = 切换选中项
    else if (g_cur == g_udisk) { /* U盘预览:短按无操作 */ }
    else                       protocol_send_button("next"); // 会话屏短按 = 实例切换(上报 host)
  }
  if (black_shown() && (millis() - s_black_ms) > BLACK_TIMEOUT_MS) black_hide();  // 失败保护:超时揭开

  // 断流判定:只有真正 30s 收不到任何帧(PC 停了/拔了)才变暗(只仪表盘有状态显示)
  if (s_state == ST_LIVE && s_last_data_ms != 0 &&
      (millis() - s_last_data_ms) > STALE_MS) {
    s_state = ST_STALE;
    if (g_cur && g_cur->set_state) g_cur->set_state(ST_STALE);   // 当前布局变暗(dashboard / system)
  }

  // 1Hz 本地维护:各布局 tick(仪表盘倒计时;文本屏滚动由内部 timer 驱动,tick=NULL)
  uint32_t now = millis();
  if (now - s_last_tick_ms >= 1000) {
    s_last_tick_ms += 1000;
    for (int i = 0; i < g_layout_n; i++)
      if (g_layouts[i]->tick) g_layouts[i]->tick();
  }

  // LED 灯效:阶段8 起由**当前布局**决定显示方式(主 loop 不含策略);状态由数据源传入。
  if (g_cur && g_cur->led) g_cur->led(s_state, s_cc_running, s_disk_lvl);
  delay(5);
}
