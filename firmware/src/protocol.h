#pragma once
// code_mate — 串口协议(CLAUDE.md §5):按行读 JSON,握手/心跳/数据帧分发
#include "app_types.h"

enum ProtoEvent { EV_NONE, EV_HELLO, EV_DATA, EV_PING, EV_CFG, EV_TEXT };

void protocol_begin();

// 在 loop 里轮询:读串口、按行解析。返回本次最值得关注的事件。
// EV_DATA 时把解析结果写入 out;id/pong 应答在内部直接发出。容忍坏行/未知帧。
ProtoEvent protocol_poll(UsagePayload &out);

// 阶段6:EV_CFG / EV_TEXT 的附带数据(下次 poll 前有效)
const char *protocol_cfg_screen();           // EV_CFG:目标布局 id("loading"/"dashboard"/"terminal")
const char *protocol_cfg_msg();               // EV_CFG:可选消息(loading 布局用,如 "No Session Available")
const TextPayload &protocol_text();           // EV_TEXT:本帧追加的带样式文本段
const char *protocol_data_screen();           // 阶段12:EV_DATA 本帧目标布局(dashboard / system)

// 阶段7:DEV→PC 上报 BOOT 按键(action="next" 切下一个实例)
void protocol_send_button(const char *action);
