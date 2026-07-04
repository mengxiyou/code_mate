# 固件工具链:版本、备份与还原

> **EN summary**: pinned toolchain versions (PlatformIO 6.1.19, pioarduino 55.03.39 =
> arduino-esp32 3.3.9 / ESP-IDF 5.5.4), the mandatory Windows long-path prerequisite,
> and optional local backup/restore tricks. Fresh clones just need `pio run`
> (downloads everything online); nothing here is required beyond the long-path step.

本工程的 PlatformIO 工具链**装在项目内**(隔离全局,不影响其他工程),可选地压缩备份一份,
**下次换机/重置可免下载还原**,或只增量更新。

## 已锁版本(2026-06 实测可编译)

| 组件 | 版本 | 说明 |
|---|---|---|
| PlatformIO Core | **6.1.19** | 由仓库根 `.venv` 提供(`pip install platformio==6.1.19`) |
| 平台 (pioarduino) | **55.03.39** | = arduino-esp32 **3.3.9** + ESP-IDF **5.5.4**(最接近文档 §6 的 3.3.10) |
| 工具链 | xtensa-esp-elf **14.2.0** (20260121) | GCC 交叉编译器 |
| 框架库 | framework-arduinoespressif32-libs **5.5.4** | 预编译 IDF 库(含各 SoC,体量大) |
| 显示库 | TFT_eSPI **2.5.43** | `lib_deps` 自动装 |

- 核心缓存目录:`firmware/.platformio`(由 `platformio.ini` 的 `core_dir = .platformio` 指定)。
- pio CLI venv:仓库根 `.venv`(27MB,可随时 `pip` 重建)。
- 首次全新安装实测:工具链 ~5.9GB,编译耗时约 6 分钟(`[SUCCESS] Took 355.77s`)。

## ⚠️ 前置(Windows 必读):开启长路径

ESP32 Arduino 预编译库里有 esp-matter/connectedhomeip 的超深路径(>260 字符),
**Windows MAX_PATH 未开启会解包失败**(无论装在项目内还是全局 `~/.platformio` 都会)。
**安装、还原备份前都必须先开启长路径**:

```powershell
# 管理员 PowerShell 执行一次(系统级,良性;之后开新进程或重启生效):
New-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' `
  -Name LongPathsEnabled -Value 1 -PropertyType DWord -Force
# 校验:
(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem').LongPathsEnabled  # 应为 1
```

## 备份内容

> 本节为**本地可选优化**:备份档不入库,克隆本仓库的用户没有它,直接 `pio run` 在线下载即可。

- 路径:`.toolchain-backup/platformio_pio6.1.19_plat55.03.39_arduino3.3.9.tar.zst`(已 gitignore,不入库)。
- 内容:整个 `firmware/.platformio`,**排除** `.cache/`(431MB 原始下载档,装好的 packages 已自足)。
- 格式:`tar + zstd`(bsdtar 自带,`--zstd`)。
- 大小:**1.58 GB**(从 ~5.4GB 压缩约 3.4×)。
- SHA256:`5F2D0E08866FD73CF5AF51580A40735F10267CC9C13BD42807F2B97942471FD7`
- 校验:`(Get-FileHash .toolchain-backup\platformio_*.tar.zst -Algorithm SHA256).Hash`

> 重新生成备份:
> ```powershell
> tar --zstd -cf .toolchain-backup\platformio_<指纹>.tar.zst -C firmware --exclude '.platformio/.cache' .platformio
> ```

## 还原(免下载,换机/重置后)

```powershell
# 0) 先开启长路径(见上),否则解包同样会因 MAX_PATH 失败
# 1) 还原工具链到 firmware/.platformio
tar --zstd -xf .toolchain-backup\platformio_*.tar.zst -C firmware
# 2) 重建 pio CLI(小,~30s)
python -m venv .venv
.\.venv\Scripts\python -m pip install platformio==6.1.19
# 3) 编译验证(应直接编过,无网络下载)
.\.venv\Scripts\pio run -d firmware
```

## 只更新部分(不全量重下)

```powershell
.\.venv\Scripts\pio pkg list   -d firmware   # 看已装平台/库版本
.\.venv\Scripts\pio pkg update -d firmware   # 增量更新已装包
.\.venv\Scripts\pio pkg outdated -d firmware # 看哪些有新版
```

> 升级平台:改 `platformio.ini` 里 pioarduino 的 release URL(版本号)后 `pio run` 即按需增量下载。

## 下载慢 / 总中断时(镜像与代理)

pioarduino 从 `github.com/pioarduino` 与 `github.com/espressif` 的 release 拉取。国内网络:

1. **首选**:若本机做过 `.toolchain-backup` 备份,直接还原(免下载)。
2. **代理**:`$env:HTTPS_PROXY = "http://<代理地址>:<端口>"` 后再 `pio run`。
3. **手动喂缓存**:从镜像(或另一台机)下好对应 release 资产,放进
   `firmware/.platformio/.cache/downloads/`,pio 会优先用缓存、跳过下载。
4. 断点续传:`pio run` 失败后重跑会续装未完成的包(已装的不重下)。
