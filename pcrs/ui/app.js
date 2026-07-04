"use strict";
// 配置界面前端:Alpine.data('app') + wry IPC + 中英双语 i18n。
// 传输层:前端 window.ipc.postMessage(JSON) → Rust dispatch → evaluate_script(window.__vmResolve(id, 结果))。
// Alpine 内联在 body 末尾、在本脚本之后执行:本脚本先注册 alpine:init 监听,Alpine 再 start → 次序正确。
// 语言:t(key) 读 this.lang(Alpine 响应式追踪)→ 切换语言时所有 t() 绑定即时重渲染,无需重载。
//       技术符号/单位(CPU/%/GB/GHz/CTX/5H/型号名/IP)保持通用不译。

let __vmSeq = 0;
const __vmPending = {};
window.__vmResolve = (id, result) => {
  const cb = __vmPending[id];
  if (cb) { delete __vmPending[id]; cb(result); }
};
function call(method, args) {
  return new Promise((resolve) => {
    const id = ++__vmSeq;
    __vmPending[id] = resolve;
    window.ipc.postMessage(JSON.stringify({ id, method, args: args || [] }));
  });
}
const api = {
  get_status: () => call("get_status"),
  get_config: () => call("get_config"),
  get_hook_status: () => call("get_hook_status"),
  get_instances: () => call("get_instances"),
  get_sysmon: () => call("get_sysmon"),
  set_instance_mode: (m) => call("set_instance_mode", [m]),
  set_autostart: (e) => call("set_autostart", [e]),
  set_fresh_sec: (n) => call("set_fresh_sec", [n]),
  set_temp_unit: (u) => call("set_temp_unit", [u]),
  set_system_screen: (b) => call("set_system_screen", [b]),
  set_lang: (l) => call("set_lang", [l]),
  install_hook: () => call("install_hook"),
  reconnect: () => call("reconnect"),
};

// ---- 词典(zh / en);缺失键回退中文再回退键名 ----
const I18N = {
  zh: {
    tab_status: "状态", tab_setting: "设置", tab_about: "关于",
    host: "主机", st_live: "在线", st_wait: "等待", st_offline: "离线",
    ov: "概览", device: "设备", port: "端口", fw: "固件", sync: "同步", reg: "注册",
    connected: "已连接", disconnected: "未连接 · 搜索中",
    reconnect: "重新连接", install_hook: "安装 / 修复钩子",
    no_snapshot: "尚无快照 · 在 Claude Code 或 Codex 里发条消息即可生成", fresh: "新鲜", stale: "陈旧",
    hook_installed: "已安装", hook_foreign: "已设置 · 非本项目", hook_none: "未安装 · 点下方安装 / 修复",
    codex_trust: "已安装 · 需在 Codex /hooks 信任", codex_ready: "可用", codex_none: "未安装",
    account: "账号", sessions: "Agent 会话", no_sessions: "尚无打开的 Agent 会话。",
    basic: "基本", sys: "系统", codex: "Codex",
    autostart: "开机自启", active_win: "活跃窗口",
    mode_manual: "手动 · BOOT 键切", mode_auto: "自动 · 跟随最近活跃",
    fresh_thresh: "快照新鲜阈值", unit_sec: "秒", snap_path: "快照路径",
    pc_screen: "PC 系统屏", temp_unit: "CPU 温度单位", unit_c: "°C 摄氏", unit_f: "°F 华氏",
    sys_hint: "关闭 PC 系统屏后,无 Claude Code 会话时设备显示等待屏,BOOT 环绕也不再切到系统屏。",
    codex_hint: "Codex hook 需在 Codex /hooks 中信任后,才会提供实时运行态；未信任时仍会读取本地 JSONL 会话。",
    lang: "语言",
    ram: "内存", vram: "显存", disk: "磁盘", temp: "温度", freq: "频率", net: "网络",
    about_desc: "Claude Code 用量监控 · USB 桌面小屏", version: "版本",
    unnamed: "(未命名)", idle: "空闲",
    f_reconnect: "已请求重连", f_installed: "已安装 · 重启 Claude Code / Codex 后生效", f_install_fail: "安装失败 · ",
    f_mode_auto: "活跃窗口 · 自动跟随", f_mode_manual: "活跃窗口 · 手动 BOOT 切",
    f_autostart_unsupported: "当前系统不支持开机自启", f_autostart_on: "已设开机自启", f_autostart_off: "已取消开机自启",
    f_fresh: "快照新鲜阈值 · ", f_temp: "CPU 温度单位 · ",
    f_screen_on: "PC 系统屏 · 已启用", f_screen_off: "PC 系统屏 · 已关闭",
    f_lang: "语言 · ", f_unknown: "未知错误",
  },
  en: {
    tab_status: "STATUS", tab_setting: "SETTING", tab_about: "ABOUT",
    host: "HOST", st_live: "LIVE", st_wait: "WAIT", st_offline: "OFFLINE",
    ov: "OVERVIEW", device: "Device", port: "Port", fw: "Firmware", sync: "Sync", reg: "Hook",
    connected: "Connected", disconnected: "Disconnected · scanning",
    reconnect: "Reconnect", install_hook: "Install / repair hook",
    no_snapshot: "No snapshot yet · send a message in Claude Code or Codex", fresh: "Fresh", stale: "Stale",
    hook_installed: "Installed", hook_foreign: "Set · not this app", hook_none: "Not installed · install below",
    codex_trust: "Installed · trust in Codex /hooks", codex_ready: "Ready", codex_none: "Not installed",
    account: "ACCOUNT", sessions: "AGENT SESSIONS", no_sessions: "No agent sessions open.",
    basic: "BASIC", sys: "SYSTEM", codex: "CODEX",
    autostart: "Auto-start", active_win: "Active window",
    mode_manual: "Manual · BOOT key", mode_auto: "Auto · follow latest",
    fresh_thresh: "Snapshot freshness", unit_sec: "s", snap_path: "Snapshot path",
    pc_screen: "PC system screen", temp_unit: "CPU temp unit", unit_c: "°C Celsius", unit_f: "°F Fahrenheit",
    sys_hint: "When off, the device shows a waiting screen while no Claude Code session is open, and BOOT cycling skips the system screen.",
    codex_hint: "Trust the Codex hook in /hooks to enable live running state; local JSONL sessions still appear before trust.",
    lang: "Language",
    ram: "RAM", vram: "VRAM", disk: "Disk", temp: "Temp", freq: "Freq", net: "Net",
    about_desc: "Claude Code usage monitor · USB desk display", version: "Version",
    unnamed: "(unnamed)", idle: "idle",
    f_reconnect: "Reconnect requested", f_installed: "Installed · restart Claude Code / Codex to apply", f_install_fail: "Install failed · ",
    f_mode_auto: "Active window · auto-follow", f_mode_manual: "Active window · manual BOOT",
    f_autostart_unsupported: "Auto-start not supported on this system", f_autostart_on: "Auto-start enabled", f_autostart_off: "Auto-start disabled",
    f_fresh: "Snapshot freshness · ", f_temp: "CPU temp unit · ",
    f_screen_on: "PC system screen · enabled", f_screen_off: "PC system screen · disabled",
    f_lang: "Language · ", f_unknown: "Unknown error",
  },
};

function fmtAge(s) {
  if (s == null) return "—";
  if (s < 60) return s + "s";
  const m = Math.floor(s / 60);
  return m + "m" + (s % 60) + "s";
}
function kfmt(n) {
  if (n == null) return "—";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return Math.round(n / 1e3) + "K";
  return "" + n;
}

document.addEventListener("alpine:init", () => {
  Alpine.data("app", () => ({
    tab: "status",
    lang: "zh",
    status: {},
    config: { instance_mode: "manual", autostart: false, fresh_sec: 60, temp_unit: "C", system_screen: true, lang: "zh" },
    hook: {},
    instances: [],
    account: {},
    sysmon: { ok: false },
    version: "",
    github: "https://github.com/mengxiyou/code_mate",
    msg: "",
    _msgT: null,

    init() {
      this.loadConfig();
      this.loadHook();
      this.loadInstances();
      this.refresh();
      setInterval(() => this.refresh(), 2000);
      this.$watch("tab", (t) => {
        if (t === "status") { this.pullSysmon(); this.loadInstances(); }
      });
    },

    // ---- i18n ----
    t(key) {
      const d = I18N[this.lang] || I18N.zh;
      return d[key] != null ? d[key] : (I18N.zh[key] != null ? I18N.zh[key] : key);
    },

    // ---- 派生(链路条 / 文案)----
    get linkState() {
      if (!this.status.connected) return "offline";
      return this.status.snapshot_present && this.status.snapshot_fresh ? "live" : "wait";
    },
    get linkLabel() { return this.t("st_" + this.linkState); },
    get snapText() {
      const s = this.status;
      if (!s.snapshot_present) return this.t("no_snapshot");
      const head = s.snapshot_fresh ? this.t("fresh") : this.t("stale");
      const ago = this.lang === "en" ? " ago" : "前";
      return head + " · Agent " + fmtAge(s.snapshot_age) + ago;
    },
    get hookText() {
      const h = this.hook.claude || this.hook;
      if (h.installed) return this.t("hook_installed");
      if (h.command) return this.t("hook_foreign");
      return this.t("hook_none");
    },
    get hookClass() { const h = this.hook.claude || this.hook; return h.installed ? "good" : "warn"; },
    get codexHookText() {
      const c = this.hook.codex || {};
      if (!c.installed) return this.t("codex_none");
      return c.trusted ? this.t("codex_ready") : this.t("codex_trust");
    },
    get codexHookClass() {
      const c = this.hook.codex || {};
      return c.installed && c.trusted ? "good" : "warn";
    },
    get cpuSub() {
      const t = this.sysmon.cpu_temp;
      if (t != null) return this.config.temp_unit === "F" ? Math.round(t * 9 / 5 + 32) + "°F" : Math.round(t) + "°C";
      return this.sysmon.cpu_ghz ? this.sysmon.cpu_ghz.toFixed(1) + " GHz" : "—";
    },

    // ---- 拉取 ----
    async refresh() {
      try {
        const s = await api.get_status();
        if (s && s.ok) this.status = s;          // 链路条在所有页签都需要
        if (this.tab === "status") { this.pullSysmon(); this.loadInstances(); }
      } catch (e) { /* GUI 偶发竞态,忽略 */ }
    },
    async pullSysmon() {
      const m = await api.get_sysmon();
      if (m) this.sysmon = m;
    },
    async loadConfig() {
      const c = await api.get_config();
      if (c && c.ok) { this.config = { ...this.config, ...c }; if (c.version) this.version = c.version; if (c.lang) this.lang = c.lang; }
    },
    async loadHook() {
      const h = await api.get_hook_status();
      if (h && h.ok) this.hook = h;
    },
    async loadInstances() {
      const r = await api.get_instances();
      if (r && r.ok) { this.instances = r.instances || []; this.account = r.shared || {}; }
    },

    // ---- 反馈 ----
    flash(t) {
      this.msg = t;
      clearTimeout(this._msgT);
      this._msgT = setTimeout(() => { this.msg = ""; }, 4000);
    },

    // ---- 动作 ----
    async reconnect() { await api.reconnect(); this.flash(this.t("f_reconnect")); },
    async install() {
      const r = await api.install_hook();
      if (r && r.ok) { this.flash(this.t("f_installed")); this.loadHook(); }
      else { this.flash(this.t("f_install_fail") + ((r && r.error) || this.t("f_unknown"))); }
    },
    async setMode() {
      await api.set_instance_mode(this.config.instance_mode);
      this.flash(this.config.instance_mode === "auto" ? this.t("f_mode_auto") : this.t("f_mode_manual"));
    },
    async setAutostart() {
      const r = await api.set_autostart(this.config.autostart);
      if (r && r.supported === false) { this.config.autostart = false; this.flash(this.t("f_autostart_unsupported")); return; }
      if (r) this.config.autostart = !!r.autostart;
      this.flash(this.config.autostart ? this.t("f_autostart_on") : this.t("f_autostart_off"));
    },
    async setFreshSec() {
      const r = await api.set_fresh_sec(this.config.fresh_sec);
      if (r && r.ok) { this.config.fresh_sec = r.fresh_sec; this.flash(this.t("f_fresh") + r.fresh_sec + "s"); }
    },
    async setTempUnit() {
      await api.set_temp_unit(this.config.temp_unit);
      this.flash(this.t("f_temp") + (this.config.temp_unit === "F" ? "°F" : "°C"));
    },
    async setSystemScreen() {
      await api.set_system_screen(this.config.system_screen);
      this.flash(this.config.system_screen ? this.t("f_screen_on") : this.t("f_screen_off"));
    },
    async setLang() {
      await api.set_lang(this.config.lang);
      this.lang = this.config.lang;
      this.flash(this.t("f_lang") + (this.lang === "en" ? "English" : "中文"));
    },

    // ---- 数据源主题 ----
    providerKey(i) {
      return ((i && (i.provider || i.source || i.brand)) || "").toString();
    },
    themeFallback(i, key) {
      const p = this.providerKey(i);
      if (p.includes("Codex")) {
        return key === "meter_a" ? "#e7c66a" : (key === "meter_b" ? "#62d6ff" : "#19c37d");
      }
      if (p.includes("System")) {
        if (key === "meter_a") return "#2de2e6";
        if (key === "meter_b") return "#8ea7ff";
        if (key === "meter_c") return "#6dcbff";
        return "#6dcbff";
      }
      if (key === "meter_a") return "#ff7a4d";
      if (key === "meter_b") return "#4decef";
      if (key === "meter_c") return "#ffb44e";
      return "#f08a5e";
    },
    themeColor(i, key, fallback) {
      const fb = fallback || this.themeFallback(i, key);
      const v = i && i.theme && i.theme[key];
      if (typeof v === "number" && Number.isFinite(v)) {
        const n = Math.max(0, Math.min(0xFFFFFF, Math.round(v)));
        return "#" + n.toString(16).padStart(6, "0");
      }
      if (typeof v === "string") {
        const s = v.trim();
        if (/^#[0-9a-f]{6}$/i.test(s)) return s;
        if (/^[0-9a-f]{6}$/i.test(s)) return "#" + s;
      }
      return fb;
    },
    colorAlpha(hex, alpha) {
      const s = (hex || "").replace("#", "");
      if (!/^[0-9a-f]{6}$/i.test(s)) return "rgba(142,153,168," + alpha + ")";
      const n = parseInt(s, 16);
      return "rgba(" + ((n >> 16) & 255) + "," + ((n >> 8) & 255) + "," + (n & 255) + "," + alpha + ")";
    },
    cardStyle(i) {
      const c = this.themeColor(i, "primary");
      return "border-color:" + this.colorAlpha(c, 0.38);
    },
    lampStyle(i) {
      const c = this.themeColor(i, "primary");
      const active = !!(i && (i.cc_running || i.ok || this.providerKey(i).includes("System")));
      return active
        ? "background:" + c + ";box-shadow:0 0 7px " + c
        : "background:" + this.colorAlpha(c, 0.45) + ";box-shadow:none";
    },
    tagStyle(i) {
      const c = this.themeColor(i, "primary");
      return "color:" + c + ";border-color:" + this.colorAlpha(c, 0.55);
    },
    valueStyle(i, key) {
      return "color:" + this.themeColor(i, key);
    },
    barFillStyle(i, pct, key) {
      const p = Math.max(0, Math.min(100, Math.round(pct || 0)));
      return "width:" + p + "%;background:" + this.themeColor(i, key);
    },

    // ---- 会话卡片 / 账号用量 ----
    ctxPct(i) {
      const c = i.context;
      return c && c.used_pct != null ? Math.round(c.used_pct) : 0;
    },
    tok(i) {
      const c = i.context;
      if (!c || c.used_tokens == null || c.max_tokens == null) return "";
      return kfmt(c.used_tokens) + " / " + kfmt(c.max_tokens);
    },
    providerLabel(i) {
      return i.provider === "ClaudeCode" ? "Claude Code" : (i.provider || i.source || "Agent");
    },
    acc(key) {
      const a = this.account[key];
      return a && a.used_pct != null ? Math.round(a.used_pct) : 0;
    },
    accTxt(key) {
      const a = this.account[key];
      return a && a.used_pct != null ? Math.round(a.used_pct) + "%" : "--";
    },

    // ---- 格式化(系统监控)----
    pct(x) { return x == null ? "—" : (Math.round(x * 10) / 10) + "%"; },
    mem(u, t, p) {
      if (u == null || t == null) return "—";
      return (u / 1024).toFixed(1) + " / " + (t / 1024).toFixed(1) + " GB · " + (Math.round(p * 10) / 10) + "%";
    },
    disk(bps) {
      if (bps == null) return "—";
      const mb = bps / 1048576;
      if (mb >= 1) return mb.toFixed(1) + " MB/s";
      const kb = bps / 1024;
      return kb >= 1 ? kb.toFixed(0) + " KB/s" : this.t("idle");
    },
  }));
});
