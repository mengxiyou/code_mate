# CodeMate Firmware

Firmware for the **Waveshare ESP32-S3-LCD-1.47** (ESP32-S3R8, 16 MB flash / 8 MB PSRAM,
ST7789 172×320 IPS, native USB — no CH340/CP210x bridge).
Stack: PlatformIO + Arduino framework ([pioarduino](https://github.com/pioarduino/platform-espressif32)) +
TFT_eSPI + LVGL v9.5.

Board wiki: <https://www.waveshare.net/wiki/ESP32-S3-LCD-1.47>

## Pinout (verified on hardware)

| Signal | GPIO | Signal | GPIO |
|---|---|---|---|
| LCD MOSI | 45 | LCD SCLK | 40 |
| LCD CS | 42 | LCD DC | 41 |
| LCD RST | 39 | LCD BL (backlight) | 48 |
| RGB LED (WS2812 data) | 38 | BOOT button | 0 |
| TF/SD CMD | 15 | TF/SD SCK | 14 |
| TF/SD D0/D1/D2/D3 | 16/18/17/21 | *(TF slot unused)* | |

Hardware facts baked into the config (`platformio.ini` build flags are the source of
truth, mirrored in `include/board_config.h`):

- Color order is **BGR**; `-D USE_HSPI_PORT` is **required** — without it the screen
  stays black.
- Display rotation is `3` (landscape, USB connector on the left, text upright) —
  verified on hardware. If your unit shows everything upside-down, toggle
  `CM_LCD_ROTATION` between `3` and `1`.
- The onboard WS2812 LED's byte order is **RGB, not the usual GRB** (`NEO_RGB`).
  With GRB, blue turns purple and red/green swap.
- Backlight is active-high and driven manually (LEDC PWM fade-in after the first
  frame). `TFT_BACKLIGHT_ON` is deliberately **not** defined — letting TFT_eSPI raise
  the backlight during init causes a white flash at power-on.
- GPIO0 (BOOT) is a strapping pin — the firmware configures it as an input only
  after boot. Holding it through a reset enters the ROM download mode instead
  (that is how you flash the OTG build, see below).
- Cold-start quirk: after a long unpowered rest the panel may show a dim picture
  with horizontal bands for 1–2 s before it self-heals (ST7789 charge pump / VCOM
  settling). The PWM backlight fade masks it; harmless.

## Build environments

| env | USB stack | Purpose |
|---|---|---|
| `waveshare_s3_lcd_147_debug` *(default)* | `ARDUINO_USB_MODE=1` — hardware USB-Serial-JTAG CDC | Daily development: auto-reset flashing works, host connects with `DTR=false` |
| `waveshare_s3_lcd_147_release` | `ARDUINO_USB_MODE=0` — TinyUSB OTG, plus `-D CM_USB_DISK` | Production: adds the read-only USB-disk (MSC) self-installer and the NVS mode-select logic; uses `partitions_release.csv`; host connects with `DTR=true` |

The host application probes both automatically (`DTR=false`, then `DTR=true`), so one
host binary works with either firmware.

## Prerequisites

- **Windows: enable long paths first** (one-time, admin) — the prebuilt ESP32
  libraries contain paths longer than `MAX_PATH` and unpacking fails otherwise.
  See [`TOOLCHAIN.md`](TOOLCHAIN.md) for the command, pinned versions, offline
  restore and mirror tips.
- The toolchain installs **inside the project** (`.venv` + `firmware/.platformio`),
  leaving your global PlatformIO untouched:

  ```powershell
  python -m venv .venv
  .\.venv\Scripts\python -m pip install platformio==6.1.19
  ```

- The first `pio run` downloads the pioarduino platform (arduino-esp32 3.3.9 /
  ESP-IDF 5.5.4) and libraries — roughly 5.9 GB and ~6 minutes.

## Build / flash / monitor

```powershell
# From the repository root, using the project-local pio:
$env:PYTHONIOENCODING = "utf-8"                  # see "Encoding pitfalls" below
.\.venv\Scripts\pio run -d firmware              # build (debug env)
.\.venv\Scripts\pio run -d firmware -t upload    # build + flash (auto port/reset)
.\.venv\Scripts\pio device monitor -d firmware   # 115200 serial monitor
```

### Release build (OTG + USB-disk)

```powershell
.\.venv\Scripts\pio run -d firmware -e waveshare_s3_lcd_147_release             # app image
.\.venv\Scripts\pio run -d firmware -e waveshare_s3_lcd_147_release -t buildfs  # FAT image from firmware/data/
.\.venv\Scripts\pio run -d firmware -e waveshare_s3_lcd_147_release -t upload   # flash app
.\.venv\Scripts\pio run -d firmware -e waveshare_s3_lcd_147_release -t uploadfs # flash FAT image
```

Flashing a device that is **already running the OTG (release) firmware** needs manual
help, because esptool's auto-reset only works on the debug (USB-Serial-JTAG) stack:

1. **Exit the CodeMate host app first** — it scans all Espressif (VID 303A) serial
   ports and will grab the download port.
2. Enter download mode by hand: **hold BOOT, tap RST, release BOOT**.
3. Run the upload (pass `--upload-port COMx` if auto-detect picks a ghost port; for
   raw esptool use `--before no-reset` since the device is already in download mode).
4. After flashing, **press RST** to boot the new firmware (auto-reset after write
   also doesn't work under OTG).
5. When switching between debug and release firmware, replug the cable once —
   repeated OTG↔JTAG mode changes can leave the USB state dirty.

> Windows note: after replugging, phantom "ghost" 303A COM ports may linger in the
> port list. The host app finds the real one by handshake probing; when flashing,
> don't trust the port list blindly — pass the port explicitly if in doubt.

## USB-disk contents & partitions (release)

`firmware/data/` is the source of the device's read-only USB disk (it is
git-ignored; you populate it yourself). Put a freshly built host executable in it
before `buildfs`:

```powershell
cd pcrs; cargo build --release
Copy-Item pcrs\target\release\CodeMate.exe firmware\data\ -Force
```

`buildfs` wraps `firmware/data/` into a wear-leveled FAT image and `uploadfs` writes
it to the `storage` partition. In U-disk mode the device exposes that partition as a
read-only USB drive carrying the self-installing host app — plug-and-install with no
downloads.

`partitions_release.csv` (16 MB flash):

| Name | Type | Offset | Size |
|---|---|---|---|
| nvs | data/nvs | 0x9000 | 0x7000 |
| factory | app | 0x10000 | 3 MB |
| storage | data/fat | 0x310000 | 8 MB |

Implementation notes:

- The FAT image is wrapped in ESP-IDF's **wear-leveling layer** — MSC must read it
  through `wl_mount`/`wl_read` (sector size 4096); raw partition reads return
  scrambled data.
- **RST and replug are indistinguishable** on this board (both report `POWERON`
  with cleared RTC), so mode memory uses a one-shot **NVS flag set by the BOOT
  button**, not reset reasons: hold BOOT (~0.7 s) on any screen → mode-select menu →
  short press moves the selection, long press applies it and reboots
  (Normal = CDC protocol mode, U-Disk = MSC). The flag clears itself on the next
  boot, so a plain reset always returns to normal mode.
- The mode-select menu and MSC code exist only in the release env
  (`#ifdef CM_USB_DISK`).

## Encoding pitfalls (Windows)

- `partitions_release.csv` must contain **ASCII-only comments** — PlatformIO parses
  the CSV with the system codepage and non-ASCII bytes raise `UnicodeDecodeError`.
- Set `PYTHONIOENCODING=utf-8` (or `PYTHONUTF8=1`) before flashing — esptool's
  progress bar prints `░` characters that crash PlatformIO's output pipe on
  non-UTF-8 consoles, leaving a zombie process.

## Troubleshooting

| Symptom | Check first |
|---|---|
| Screen stays black | `-D USE_HSPI_PORT` present in `platformio.ini` (critical for this board); backlight `TFT_BL=48` |
| Port enumerates but no data | `ARDUINO_USB_CDC_ON_BOOT=1` and the intended `ARDUINO_USB_MODE`; try a proper **data** cable |
| Red/blue swapped | `TFT_RGB_ORDER` (this board is BGR) |
| LED colors wrong (blue looks purple) | WS2812 byte order must be `NEO_RGB`, not `NEO_GRB` |
| Image shifted / wrapped | `TFT_WIDTH=172` must stay 172 (auto column offset), not 240 |
| Text upside-down / connector on the wrong side | Toggle `CM_LCD_ROTATION` 3 ↔ 1 in `include/board_config.h` |
| Flash fails on release firmware | Manual BOOT+RST download mode; kill the host app; see the OTG section above |
| `UnicodeDecodeError` / hung flash | Encoding pitfalls above |
| Platform download fails | First build needs network for the pioarduino platform; see `TOOLCHAIN.md` for mirrors/offline restore |
