//! 读 CC 转录助手回复 + markdown→带样式 runs + emoji 净化 + 设备行数估算 + 切帧(对应 pc/transcript.py)。
//! ⚠️ 本会话刚调好的边界全在这:行首圆点(有序/引用行豁免、去孤立圆点)、`" / "→"/"`、空行折叠、每帧剥首尾空白。
use regex_lite::Regex;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::LazyLock;

pub const TEXT_RUN_MAX: usize = 12; // 每帧最多 run 数
pub const TEXT_RUN_LEN: usize = 100; // 每 run 文本字节上限(UTF-8)
const TERM_CONTENT_W: i64 = 304; // 设备文本容器内容宽(320 - pad 8*2)

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+(.*)$").unwrap());
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)[-*+]\s+(.*)$").unwrap());
static ORDERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+[.)]\s+").unwrap());
static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*```").unwrap());
static INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\*\*.+?\*\*|`[^`]+`)").unwrap());
static SLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" +/ +").unwrap());
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// 一个带样式文本段:(style, text)。style ∈ h/b/c/u/d/n。
type Run = (char, String);

// ---------- 读转录:助手 text 块,按 uuid 增量 ----------
pub fn read_assistant_texts(
    path: &str,
    after_uuid: Option<&str>,
    limit: Option<usize>,
) -> (Vec<(String, String)>, Option<String>) {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = after_uuid.is_none();
    if let Ok(file) = std::fs::File::open(path) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let e: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let typ = e.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let is_claude = typ == "assistant";
            let is_codex = typ == "response_item"
                && e.get("payload")
                    .and_then(|p| p.get("type"))
                    .and_then(|v| v.as_str())
                    == Some("message")
                && e.get("payload")
                    .and_then(|p| p.get("role"))
                    .and_then(|v| v.as_str())
                    == Some("assistant");
            if !is_claude && !is_codex {
                continue;
            }
            let u = e
                .get("uuid")
                .or_else(|| e.get("payload").and_then(|p| p.get("id")))
                .or_else(|| e.get("timestamp"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !seen {
                if Some(u.as_str()) == after_uuid {
                    seen = true;
                }
                continue;
            }
            let content = if is_codex {
                e.get("payload").and_then(|p| p.get("content"))
            } else {
                e.get("message").and_then(|m| m.get("content"))
            };
            let txt: String = content
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|b| {
                            let ty = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if ty == "text" || ty == "output_text" {
                                b.get("text").and_then(|v| v.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<String>()
                })
                .or_else(|| content.and_then(|c| c.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            let txt = txt.trim().to_string();
            if !txt.is_empty() {
                out.push((u, txt));
            }
        }
    }
    if let Some(l) = limit {
        if out.len() > l {
            out = out.split_off(out.len() - l);
        }
    }
    let last = out
        .last()
        .map(|(u, _)| u.clone())
        .or_else(|| after_uuid.map(|s| s.to_string()));
    (out, last)
}

// ---------- markdown → runs ----------
/// 行内解析:**强调**→b,`代码`→c,其余→n(对齐 _inline_runs;跳过空段)。
fn inline_runs(s: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut last = 0;
    for m in INLINE.find_iter(s) {
        if m.start() > last {
            out.push(('n', s[last..m.start()].to_string()));
        }
        let part = m.as_str();
        if part.starts_with("**") && part.ends_with("**") && part.len() >= 4 {
            out.push(('b', part[2..part.len() - 2].to_string()));
        } else if part.starts_with('`') && part.ends_with('`') && part.len() >= 2 {
            out.push(('c', part[1..part.len() - 1].to_string()));
        } else {
            out.push(('n', part.to_string()));
        }
        last = m.end();
    }
    if last < s.len() {
        out.push(('n', s[last..].to_string()));
    }
    out
}

/// 连续空行压到至多 1 行(保留 1):逐行去行尾空白 + 3+ 换行 → 2。
pub fn collapse_blanks(text: &str) -> String {
    let joined: String = text
        .split('\n')
        .map(|ln| ln.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    BLANKS.replace_all(&joined, "\n\n").into_owned()
}

/// 整段 markdown → runs。每非空行行首加圆点(style 'd');有序/引用行豁免。
pub fn md_to_runs(text: &str) -> Vec<Run> {
    let text = collapse_blanks(text);
    let mut runs: Vec<Run> = Vec::new();
    let mut in_code = false;
    for raw in text.split('\n') {
        if FENCE.is_match(raw) {
            in_code = !in_code;
            continue;
        }
        if in_code {
            runs.push(('d', "• ".to_string()));
            runs.push(('c', format!("{}\n", raw)));
            continue;
        }
        if let Some(cap) = HEADING.captures(raw) {
            runs.push(('d', "• ".to_string()));
            runs.push(('h', format!("{}\n", cap.get(1).map_or("", |m| m.as_str()))));
            continue;
        }
        // 非代码行:去斜杠两侧空格,避免窄屏把 " / " 孤立成一行
        let line = SLASH.replace_all(raw, "/");
        let line: &str = line.as_ref();
        if ORDERED.is_match(line) || QUOTE.is_match(line) {
            // 本身有序号/引用标记 → 整行按正文,不加圆点
            runs.extend(inline_runs(line));
            runs.push(('n', "\n".to_string()));
            continue;
        }
        if let Some(cap) = BULLET.captures(line) {
            runs.push(('d', "• ".to_string())); // 圆点即列表标记
            runs.extend(inline_runs(cap.get(2).map_or("", |m| m.as_str())));
            runs.push(('n', "\n".to_string()));
            continue;
        }
        if line.trim().is_empty() {
            runs.push(('n', "\n".to_string())); // 空行不加圆点
            continue;
        }
        runs.push(('d', "• ".to_string()));
        runs.extend(inline_runs(line));
        runs.push(('n', "\n".to_string()));
    }
    runs
}

// ---------- 设备换行后行数估算 ----------
fn glyph_w(cp: u32) -> i64 {
    let full = (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp);
    if full {
        16
    } else {
        8
    }
}

fn device_lines(line: &str) -> i64 {
    let w: i64 = line.chars().map(|c| glyph_w(c as u32)).sum();
    ((w + TERM_CONTENT_W - 1) / TERM_CONTENT_W).max(1) // ceil(w/304),≥1
}

/// 从末尾取若干源行,使换行后的设备行数累计达 budget(让不同会话显示量一致)。
pub fn tail_by_device_lines(text: &str, budget: i64) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::new();
    let mut used = 0i64;
    for ln in lines.iter().rev() {
        out.push(ln);
        used += device_lines(ln);
        if used >= budget {
            break;
        }
    }
    out.reverse();
    out.join("\n")
}

// ---------- 字符净化:设备字体只含 BMP CJK,emoji/杂符替换或剔除 ----------
fn font_ok(cp: u32) -> bool {
    cp == 0x0A
        || (0x20..=0x7E).contains(&cp)
        || (0x2010..=0x2027).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0x4E00..=0x9FFF).contains(&cp)
        || (0xFF01..=0xFF5E).contains(&cp)
}

fn symbol_map(c: char) -> Option<&'static str> {
    Some(match c {
        '→' => "->",
        '⇒' => "=>",
        '←' => "<-",
        '↔' => "<->",
        '↑' => "^",
        '↓' => "v",
        '×' => "x",
        '÷' => "/",
        '≈' => "~=",
        '≤' => "<=",
        '≥' => ">=",
        '≠' => "!=",
        '±' => "+/-",
        '·' => "-",
        '─' => "-",
        '—' => "-",
        '│' => "|",
        '├' => "|",
        '└' => "|",
        '┌' => "|",
        '┐' => "|",
        '┘' => "|",
        '┬' => "+",
        '┴' => "+",
        '┼' => "+",
        '✅' => "[v]",
        '❌' => "[x]",
        '⚠' => "!",
        _ => return None,
    })
}

fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if font_ok(c as u32) {
            out.push(c);
        } else if let Some(r) = symbol_map(c) {
            out.push_str(r);
        }
        // 其余(emoji 等)直接剔除
    }
    out
}

// ---------- runs → 设备帧(切块 + UTF-8 截断)----------
fn split_bytes(style: char, t: &str, max_bytes: usize) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let bytes = t.as_bytes();
    let mut start = 0;
    while bytes.len() - start > max_bytes {
        let mut end = start + max_bytes;
        while end > start && !t.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = (start + max_bytes).min(bytes.len()); // 退化保护(不该发生)
        }
        out.push((style, t[start..end].to_string()));
        start = end;
    }
    out.push((style, t[start..].to_string()));
    out
}

#[derive(Clone)]
pub struct Frame {
    pub clear: bool,
    pub runs: Vec<Run>,
}

impl Frame {
    /// 序列化成设备 text payload:{"clear":bool,"runs":[{"s","t"}]}。
    pub fn to_payload(&self) -> Value {
        let runs: Vec<Value> = self
            .runs
            .iter()
            .map(|(s, t)| json!({"s": s.to_string(), "t": t}))
            .collect();
        json!({"clear": self.clear, "runs": runs})
    }
}

/// 一条助手回复(或 tail)→ 若干设备帧。首帧可带 clear。
pub fn text_to_frames(text: &str, clear: bool) -> Vec<Frame> {
    let mut split: Vec<Run> = Vec::new();
    for (style, seg) in md_to_runs(text) {
        let seg = sanitize(&seg); // 剔 emoji / 箭头转 ASCII
        if seg.is_empty() {
            continue;
        }
        split.extend(split_bytes(style, &seg, TEXT_RUN_LEN - 1));
    }
    // 去孤立圆点:'d' 后紧跟空白/换行(本行内容被清空)或在末尾 → 丢
    let split: Vec<Run> = split
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            !(r.0 == 'd' && (i + 1 >= split.len() || split[i + 1].1.trim().is_empty()))
        })
        .map(|(_, r)| r.clone())
        .collect();
    let mut split = split;
    while split.last().is_some_and(|r| r.1.trim().is_empty()) {
        split.pop();
    }

    let mut frames: Vec<Frame> = Vec::new();
    let n = split.len();
    let mut i = 0;
    while i < n {
        let mut end = (i + TEXT_RUN_MAX).min(n);
        if end < n && split[end - 1].0 == 'd' {
            end -= 1; // 帧末尾不留圆点(否则圆点与正文被切两帧 = 孤立圆点)
        }
        let mut chunk: Vec<Run> = split[i..end].to_vec();
        while chunk.first().is_some_and(|r| r.1.trim().is_empty()) {
            chunk.remove(0);
        }
        while chunk.last().is_some_and(|r| r.1.trim().is_empty()) {
            chunk.pop();
        }
        if !chunk.is_empty() {
            frames.push(Frame {
                clear: false,
                runs: chunk,
            });
        }
        i = end;
    }
    if frames.is_empty() {
        frames.push(Frame {
            clear: false,
            runs: vec![('n', "\n".to_string())],
        });
    }
    if clear {
        if let Some(f) = frames.first_mut() {
            f.clear = true;
        }
    }
    frames
}
