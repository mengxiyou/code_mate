#pragma once
// code_mate — 设备侧数据模型与状态(协议见 CLAUDE.md §5,显示见 §6)
#include <stdint.h>

// 设备状态机(CLAUDE.md §6)
enum DeviceState { ST_BOOT, ST_WAIT, ST_LIVE, ST_STALE };

// 限流窗口(five_hour / seven_day)
struct RateWindow {
  bool     has = false;
  float    used_pct = 0.0f;
  uint32_t resets_at = 0;   // Unix 秒(来自电脑时钟)
};

// UI 主题色(24-bit RGB):primary=品牌/身份,meter_a/b/c=左上/左下/右侧仪表。
struct UiTheme {
  bool     has = false;
  uint32_t primary = 0;
  uint32_t meter_a = 0;
  uint32_t meter_b = 0;
  uint32_t meter_c = 0;
};

// 一帧用量数据(对应协议 data.payload + 顶层 ts/fresh/stale_sec)
struct UsagePayload {
  // 模型
  bool  has_model = false;
  char  model[40] = {0};
  char  source[24] = {0};   // Claude Code / Codex / System
  char  brand[24] = {0};    // dashboard top-left title override
  UiTheme theme;

  // Claude Code 工作模式(电脑端从 permission_mode 映射:NORMAL/PLAN/BYPASS/ACCEPT)
  bool  has_mode = false;
  char  mode[16] = {0};

  // 上下文窗口
  bool     ctx_has = false;
  float    ctx_pct = 0.0f;
  uint32_t ctx_used = 0;
  uint32_t ctx_max = 0;

  // 限流窗口
  RateWindow five_hour;
  RateWindow seven_day;

  // 工作状态(阶段5):CC 是否正在生成(由活动钩子 running 经电脑端 cc_running 下发)
  bool  cc_running = false;

  // 会话标识(阶段5/7):会话标题(CC 注册表 name;空则项目文件夹名),右上角显示
  bool  has_session = false;
  char  session[64] = {0};   // 阶段8:加长(须 > PC SESSION_MAX_BYTES,避免设备端按字节切坏 UTF-8)

  // 会话指示(阶段7):身份色 + 序号/总数(右上色点 + 右下方块)
  uint16_t dot = 0;        // 会话身份色(PC 按 session_id 稳定哈希;设备 % 调色板数)
  uint8_t  inst_idx = 0;   // 当前会话序号(1-based;0=未知)
  uint8_t  inst_cnt = 0;   // 活会话总数

  // 补充信息(可选,ccusage)
  bool     extra_has = false;
  uint32_t total_tokens = 0;
  float    api_cost_usd = 0.0f;
  uint32_t burn_tpm = 0;

  // 系统监控(阶段12;screen=system 帧):CPU/内存/显存 + 磁盘活动级别 + 主机名
  bool     cpu_has = false;
  float    cpu_pct = 0.0f;
  bool     ram_has = false;
  float    ram_pct = 0.0f;
  uint32_t ram_used_mb = 0, ram_total_mb = 0;
  bool     vram_has = false;
  float    vram_pct = 0.0f;
  uint32_t vram_used_mb = 0, vram_total_mb = 0;
  uint8_t  disk_lvl = 0;       // 0..255 磁盘活动(驱动 system 屏板载 LED)
  char     host[40] = {0};     // 主机名(右上角显示)
  char     net[64] = {0};      // 局域网 IP + 公网归属(左下显示;离线 IP + 在线归属)
  char     cpu_sub[16] = {0};  // CPU 副读数(环下):温度(体感助手在跑)或当前频率 GHz

  // 新鲜度 / 时钟
  bool     fresh = true;
  uint32_t stale_sec = 0;
  uint32_t pc_ts = 0;       // 顶层 ts(Unix 秒),用于本地倒计时基准

  // 阶段7:绑定新实例/首帧 → 设备播「0→目标」增长动画(由 PC 的 data.init 显式驱动);缺省=刷新 tween
  bool     init = false;
};

// 文本滚动屏(阶段6):一帧追加的带样式文本段
#define TEXT_RUN_MAX 12     // 每帧最多 run 数(电脑端按此分块)
#define TEXT_RUN_LEN 100    // 每个 run 文本字节上限(UTF-8;电脑端按字符边界截)
struct TextRun {
  char style = 'n';                 // h 标题 / b 强调 / c 代码 / u 列表 / n 正文
  char text[TEXT_RUN_LEN] = {0};
};
struct TextPayload {
  bool clear = false;               // true=先清屏再追加
  int  n = 0;                       // 本帧 run 数
  TextRun runs[TEXT_RUN_MAX];
};
