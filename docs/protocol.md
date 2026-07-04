# Serial Protocol & Extension Guide

How the CodeMate host (PC) talks to the device, and where to hook in when adding
your own screens or data sources.

Source of truth: `pcrs/src/serial_link.rs`, `pcrs/src/cc_source.rs`,
`pcrs/src/sys_source.rs` on the host side; `firmware/src/protocol.cpp` on the device
side. This document describes the shipped behavior — when in doubt, read those files.

## Transport

- USB CDC serial, **115200 8N1**.
- **Newline-delimited JSON**: one complete JSON object per line.
- Every frame carries a `t` field with the frame type.
- Both ends **tolerate unknown frame types and unknown fields** — that is the
  forward-compatibility contract. Extensions should add fields/types, not change
  existing semantics.

## Handshake (device discovery)

The host enumerates serial ports (filtering on Espressif VID `303A`), opens each
candidate and probes it. No VID/PID lock-in — identification is by response:

```
PC  → {"t":"hello","v":1}
DEV → {"t":"id","name":"code_mate","fw":"1.0.0","screens":["dashboard","terminal","system"]}
```

Only a device answering with `name:"code_mate"` is locked in. The host opens with
`DTR=false` first (safe: doesn't reset the debug/HWCDC firmware), and if there is no
answer retries with `DTR=true` (required by the OTG/release firmware) — so one host
binary works with both firmware builds.

The host keeps the link alive with `{"t":"ping"}` roughly every 15 s (device answers
`{"t":"pong"}`) and reconnects automatically after unplug/sleep/timeouts.

## Screens

`screen` names a **layout** on the device:

| id | Layout | Shown when |
|---|---|---|
| `dashboard` | Claude Code / Codex usage gauges | a coding-agent session is selected |
| `terminal` | scrolling text of the assistant's replies | laptop lid closed |
| `system` | CPU / RAM / VRAM / disk / IP monitor | no session available, or selected via BOOT |
| `loading` | brand logo + waiting bars | before the handshake only (CONNECTING) |

The host switches layouts with a config frame:

```
PC → {"t":"cfg","screen":"terminal"}          // also: "dashboard" | "system" | "loading"
PC → {"t":"cfg","screen":"loading","msg":"…"} // optional text instead of waiting bars
```

The device plays a bottom-up slide transition on layout changes. A `cfg` frame is
always sent **before** the `data` frames that belong to the new screen.

## Data frames (PC → device)

Common envelope:

```json
{
  "t": "data",
  "screen": "dashboard",
  "ts": 1742650008,
  "fresh": true,
  "stale_sec": 8,
  "init": false,
  "payload": { ... }
}
```

- `ts` — the PC's current Unix seconds. The device computes
  `remaining = resets_at - ts` once and then counts down locally with `millis()`;
  the PC does **not** push every second.
- `fresh` / `stale_sec` — whether the underlying usage snapshot is recent. Drives
  the LIVE (cyan) / STALE (dim) indicator; the device also falls back to STALE on
  its own after ~30 s without data.
- `init` — `true` when a different session was just selected (or on the first
  frame): the device plays a grow-from-zero animation instead of tweening from the
  previous value.

Cadence: **push-on-change plus a ~3 s heartbeat** — state flips (e.g. the agent
started/stopped generating) are sent immediately (~0.4 s detection), and an
unchanged frame is repeated every ~3 s so the device stays LIVE and keeps its
countdowns anchored.

### `screen:"dashboard"` payload

| Field | Type | Meaning |
|---|---|---|
| `model` | string | Model display name (short `"Opus"` or long `"Claude Opus 4.8"`; device truncates) |
| `mode` | string, optional | Reasoning-effort level (`LOW`…`MAX`); colors the bottom-left badge |
| `context` | `{used_pct, used_tokens, max_tokens}` | Context-window gauge (right ring) |
| `five_hour` | `{used_pct, resets_at}` | 5-hour rate-limit window (top bar) |
| `seven_day` | `{used_pct, resets_at}` | Weekly rate-limit window (bottom bar) |
| `cc_running` | bool, optional | Agent currently generating → drives the LED breathing effect |
| `session` | string, optional | Session title, shown top-right |
| `dot` / `idx` / `cnt` | int | Session identity color hash + "session i of n" square indicators (bottom-right) |
| `provider` / `theme` | optional | Data-source name and per-provider accent colors (Claude / Codex) |
| `extra` | object, optional | Auxiliary totals (tokens / cost); omitted when unavailable |

Percentages may be fractional; the device rounds for display. **Every field may be
missing** — the device renders `--` instead of crashing. `resets_at` is Unix seconds.

### `screen:"system"` payload

| Field | Type | Meaning |
|---|---|---|
| `cpu` | `{used_pct}` | CPU ring |
| `cpu_sub` | string, optional | Ring subtitle: CPU temperature (if a sensor helper is running) or current GHz |
| `ram` | `{used_pct, used_mb, total_mb}` | RAM bar |
| `vram` | same, optional | Dedicated GPU memory bar; omitted → `--` |
| `disk` | int 0–255 | Disk-activity level, drives the HDD-style LED flicker |
| `host` | string | Computer name |
| `net` | string, optional | Public IP (falls back to LAN IP) |
| `dot` / `idx` / `cnt` | int | Same session indicators — the system screen is a permanent pseudo-session at the end of the cycle |

### `screen:"terminal"` payload (text frames)

```json
{"t":"data","screen":"terminal","ts":…,"payload":{
  "clear": true,
  "runs": [ {"s":"h","t":"Heading"}, {"s":"n","t":"Body text"} ]
}}
```

- `runs[].s` styles: `h` heading / `b` bold / `c` code / `u` list / `d` leading
  bullet dot / `n` normal — the device colors each style differently.
- `runs[].t` is UTF-8, at most **99 bytes** per run; at most **12 runs** per frame.
  Long content is split across frames; `clear:true` redraws from scratch.
- The device processes **one frame per poll loop**, so the host paces multi-frame
  bursts (the ESP32 USB CDC RX buffer drops bytes silently when full). Only BMP
  characters render (the CJK font has no emoji); the host sanitizes text before
  sending.

## Button events (device → PC)

```
DEV → {"t":"btn","action":"next"}
```

Sent on a short BOOT press. The host advances the session cycle
(`session 1 → … → session N → system → back`) and answers with fresh `cfg`/`data`
frames (`init:true`). A long press (~0.7 s) is handled on-device (mode-select menu
on release firmware) and does not reach the host.

## LEDs

LED behavior is part of the device layout, driven by connection state plus payload
fields (`cc_running`, `disk`): blue slow blink = connecting, blue solid = connected
and idle, random-color breathing = agent working, activity flicker = disk I/O on the
system screen, white sweep = screen transition.

## Extending

### Add a new screen

1. **Firmware**: create `ui_myscreen.{cpp,h}` implementing the `Layout` interface
   (see `firmware/src/ui_system.cpp` for a compact example — it derives from the
   dashboard). Register it in `main.cpp` next to the existing layouts and add its id
   to the `screens` list in `firmware/src/protocol.cpp`'s `id` frame. Parse your
   payload fields in `protocol.cpp`.
2. **Host**: build your `data` frames (follow `pcrs/src/sys_source.rs`), and switch
   to the screen with a `cfg` frame from the screen state machine in
   `pcrs/src/host.rs` (`apply_screen`).
3. Keep the tolerance rules: unknown fields must not break older firmware, and every
   field your layout reads needs a `--` fallback.

### Add a new data source

Model it on `pcrs/src/codex_source.rs` (the Codex CLI source) against the shapes in
`pcrs/src/datasource.rs`:

1. Discover live sessions (`list_instances()` equivalent) — return one entry per
   session with a stable `session_id`, a display `name`, and freshness timestamps.
2. Produce a dashboard payload per session (`dashboard_payload` shape: `model`,
   `context`, `five_hour`/`seven_day`, `cc_running`, …) and, if your provider has
   account-level windows, a shared payload merged across sessions.
3. Wire it into `datasource.rs` so the selector (`instance_select.rs`) sees your
   sessions in the unified cycle, and (optionally) give it an accent theme in
   `ui_theme.rs`.

No firmware change is needed for a new data source — the dashboard renders whatever
payload it receives.
