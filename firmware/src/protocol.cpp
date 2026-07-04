#include "protocol.h"
#include <Arduino.h>
#include <ArduinoJson.h>
#include <string.h>

static String s_line;
static char s_cfg_screen[24] = {0};   // EV_CFG 目标布局 id
static char s_cfg_msg[64] = {0};       // EV_CFG 可选消息(loading 布局用,如 "No Session Available")
static TextPayload s_text;             // EV_TEXT 本帧文本段
static char s_data_screen[24] = "dashboard";  // 阶段12:EV_DATA 本帧目标布局(dashboard / system)

void protocol_begin() {
  s_line.reserve(1600);   // 文本帧较长(多 run)
}

const char *protocol_cfg_screen() { return s_cfg_screen; }
const char *protocol_cfg_msg() { return s_cfg_msg; }
const TextPayload &protocol_text() { return s_text; }
const char *protocol_data_screen() { return s_data_screen; }

static void send_line(const char *s) {
  Serial.print(s);
  Serial.print('\n');
}

void protocol_send_button(const char *action) {
  char buf[48];
  snprintf(buf, sizeof buf, "{\"t\":\"btn\",\"action\":\"%s\"}", action ? action : "next");
  send_line(buf);
}

static ProtoEvent handle_line(const String &line, UsagePayload &out) {
  JsonDocument doc;
  if (deserializeJson(doc, line)) return EV_NONE;  // 坏 JSON:跳过

  const char *t = doc["t"] | "";

  if (!strcmp(t, "hello")) {
    send_line("{\"t\":\"id\",\"name\":\"code_mate\",\"fw\":\"1.0.0\",\"screens\":[\"dashboard\",\"terminal\",\"system\"]}");
    return EV_HELLO;
  }
  if (!strcmp(t, "ping")) {
    send_line("{\"t\":\"pong\"}");
    return EV_PING;
  }
  if (!strcmp(t, "cfg")) {
    const char *scr = doc["screen"] | "";
    strncpy(s_cfg_screen, scr, sizeof(s_cfg_screen) - 1);
    s_cfg_screen[sizeof(s_cfg_screen) - 1] = 0;
    const char *msg = doc["msg"] | "";   // 可选(loading 布局消息)
    strncpy(s_cfg_msg, msg, sizeof(s_cfg_msg) - 1);
    s_cfg_msg[sizeof(s_cfg_msg) - 1] = 0;
    return EV_CFG;   // 切屏
  }
  if (!strcmp(t, "data")) {
    // 文本滚动屏:screen=="terminal" → 解析带样式 runs(EV_TEXT);否则 EV_DATA(dashboard)
    const char *scr = doc["screen"] | "dashboard";
    if (!strcmp(scr, "terminal")) {
      JsonObjectConst pl = doc["payload"];
      s_text.clear = pl["clear"] | false;
      int i = 0;
      for (JsonObjectConst r : pl["runs"].as<JsonArrayConst>()) {
        if (i >= TEXT_RUN_MAX) break;
        const char *st = r["s"] | "n";
        s_text.runs[i].style = (st && st[0]) ? st[0] : 'n';
        const char *txt = r["t"] | "";
        strncpy(s_text.runs[i].text, txt, TEXT_RUN_LEN - 1);
        s_text.runs[i].text[TEXT_RUN_LEN - 1] = 0;
        i++;
      }
      s_text.n = i;
      return EV_TEXT;
    }
    UsagePayload p;  // 全新,带兜底默认值
    p.pc_ts     = doc["ts"]        | 0u;
    p.fresh     = doc["fresh"]     | true;
    p.stale_sec = doc["stale_sec"] | 0u;
    p.init      = doc["init"]      | false;   // 阶段7:绑定新实例/首帧 → 0增长动画;缺省=刷新

    JsonObjectConst pl = doc["payload"];
    if (!pl.isNull()) {
      const char *m = pl["model"] | "";
      if (m && *m) {
        strncpy(p.model, m, sizeof(p.model) - 1);
        p.has_model = true;
      }
      const char *src = pl["source"] | "";
      if (src && *src) strncpy(p.source, src, sizeof(p.source) - 1);
      const char *br = pl["brand"] | "";
      if (br && *br) strncpy(p.brand, br, sizeof(p.brand) - 1);
      JsonObjectConst th = pl["theme"];
      if (!th.isNull()) {
        p.theme.has     = true;
        p.theme.primary = th["primary"] | 0u;
        p.theme.meter_a = th["meter_a"] | 0u;
        p.theme.meter_b = th["meter_b"] | 0u;
        p.theme.meter_c = th["meter_c"] | 0u;
      }
      const char *md = pl["mode"] | "";
      if (md && *md) {
        strncpy(p.mode, md, sizeof(p.mode) - 1);
        p.has_mode = true;
      }
      JsonObjectConst ctx = pl["context"];
      if (!ctx.isNull()) {
        p.ctx_has  = true;
        p.ctx_pct  = ctx["used_pct"]    | 0.0f;
        p.ctx_used = ctx["used_tokens"] | 0u;
        p.ctx_max  = ctx["max_tokens"]  | 0u;
      }
      JsonObjectConst fh = pl["five_hour"];
      if (!fh.isNull()) {
        p.five_hour.has       = true;
        p.five_hour.used_pct  = fh["used_pct"]  | 0.0f;
        p.five_hour.resets_at = fh["resets_at"] | 0u;
      }
      JsonObjectConst sd = pl["seven_day"];
      if (!sd.isNull()) {
        p.seven_day.has       = true;
        p.seven_day.used_pct  = sd["used_pct"]  | 0.0f;
        p.seven_day.resets_at = sd["resets_at"] | 0u;
      }
      // 工作状态 + 会话名(阶段5)
      p.cc_running = pl["cc_running"] | false;
      const char *ss = pl["session"] | "";
      if (ss && *ss) {
        strncpy(p.session, ss, sizeof(p.session) - 1);
        p.has_session = true;
      }
      // 会话指示(阶段7):身份色 + 序号/总数
      p.dot      = pl["dot"] | 0u;
      p.inst_idx = pl["idx"] | 0u;
      p.inst_cnt = pl["cnt"] | 0u;
      JsonObjectConst ex = pl["extra"];
      if (!ex.isNull()) {
        p.extra_has     = true;
        p.total_tokens  = ex["total_tokens"] | 0u;
        p.api_cost_usd  = ex["api_cost_usd"] | 0.0f;
        p.burn_tpm      = ex["burn_tpm"]     | 0u;
      }
      // 系统监控(阶段12;screen=system 帧):CPU / 内存 / 显存 / 磁盘活动 / 主机名
      JsonObjectConst cpu = pl["cpu"];
      if (!cpu.isNull()) { p.cpu_has = true; p.cpu_pct = cpu["used_pct"] | 0.0f; }
      JsonObjectConst ram = pl["ram"];
      if (!ram.isNull()) {
        p.ram_has = true;
        p.ram_pct = ram["used_pct"] | 0.0f;
        p.ram_used_mb = ram["used_mb"] | 0u;
        p.ram_total_mb = ram["total_mb"] | 0u;
      }
      JsonObjectConst vram = pl["vram"];
      if (!vram.isNull()) {
        p.vram_has = true;
        p.vram_pct = vram["used_pct"] | 0.0f;
        p.vram_used_mb = vram["used_mb"] | 0u;
        p.vram_total_mb = vram["total_mb"] | 0u;
      }
      p.disk_lvl = pl["disk"] | 0u;
      const char *hn = pl["host"] | "";
      if (hn && *hn) strncpy(p.host, hn, sizeof(p.host) - 1);
      const char *nt = pl["net"] | "";
      if (nt && *nt) strncpy(p.net, nt, sizeof(p.net) - 1);
      const char *cs = pl["cpu_sub"] | "";
      if (cs && *cs) strncpy(p.cpu_sub, cs, sizeof(p.cpu_sub) - 1);
    }
    strncpy(s_data_screen, scr, sizeof(s_data_screen) - 1);
    s_data_screen[sizeof(s_data_screen) - 1] = 0;
    out = p;
    return EV_DATA;
  }
  return EV_NONE;  // 未知类型:容忍
}

ProtoEvent protocol_poll(UsagePayload &out) {
  // ⚠️ 逐行交付:解析到第一个有效事件就立即 return,余下字节留在 RX、下个 loop 再读。
  // 不能在一次调用里 drain 多行只回最后一个事件——cfg/text/data 的附带数据都存在单槽静态
  // 缓冲(s_cfg_screen/s_text/out),后一行会覆盖前一行;合盖时 host 突发下发 cfg+多条 text,
  // 折叠成一个事件会导致「切屏被吞 / 多帧只剩末帧 / clear 丢失」。main loop 用 while 取空逐个处理。
  while (Serial.available() > 0) {
    char c = (char)Serial.read();
    if (c == '\n') {
      if (s_line.length() > 0) {
        ProtoEvent e = handle_line(s_line, out);
        s_line = "";
        if (e != EV_NONE) return e;
      }
    } else if (c != '\r') {
      if (s_line.length() < 2400) {   // 文本帧多 run 较长,放宽上限
        s_line += c;
      } else {
        s_line = "";  // 超长保护:丢弃异常行
      }
    }
  }
  return EV_NONE;
}
