//! 配色（取自设计稿）和一些共用的小格式化函数。

use std::path::Path;
use std::time::SystemTime;

use ratatui::style::{Color, Modifier, Style};

use crate::model::SessionStatus;

pub const BG: Color = Color::Rgb(0x1B, 0x1A, 0x17);
pub const BAR_BG: Color = Color::Rgb(0x24, 0x22, 0x1E);
pub const FG: Color = Color::Rgb(0xE6, 0xE3, 0xD8);
pub const DIM: Color = Color::Rgb(0x9C, 0x99, 0x8E);
pub const BORDER: Color = Color::Rgb(0x3A, 0x38, 0x32);
pub const ACCENT: Color = Color::Rgb(0xE8, 0x94, 0x6F);
pub const OK: Color = Color::Rgb(0x8F, 0xC7, 0x9A);
pub const WARN: Color = Color::Rgb(0xE5, 0xC0, 0x7B);
pub const ERR: Color = Color::Rgb(0xE8, 0x87, 0x87);
pub const BLUE: Color = Color::Rgb(0x7F, 0xB2, 0xE5);
/// 选中行背景
pub const SEL_BG: Color = Color::Rgb(0x33, 0x30, 0x2A);
/// 用户消息、代码块背景
pub const BLOCK_BG: Color = Color::Rgb(0x26, 0x24, 0x1F);
pub const DEL_BG: Color = Color::Rgb(0x3A, 0x24, 0x23);
pub const DEL_FG: Color = Color::Rgb(0xF0, 0xB4, 0xB0);
pub const ADD_BG: Color = Color::Rgb(0x21, 0x32, 0x24);
pub const ADD_FG: Color = Color::Rgb(0xB5, 0xDE, 0xBC);
/// 覆盖层框体背景
pub const OVERLAY_BG: Color = Color::Rgb(0x1F, 0x1D, 0x1A);
/// 覆盖层打开时，压暗背后的界面
pub const SHADE_BG: Color = Color::Rgb(0x12, 0x11, 0x10);
pub const SHADE_FG: Color = Color::Rgb(0x6E, 0x6B, 0x63);

pub fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

pub fn dim() -> Style {
    fg(DIM)
}

pub fn accent() -> Style {
    fg(ACCENT)
}

pub fn bold(s: Style) -> Style {
    s.add_modifier(Modifier::BOLD)
}

pub fn status_symbol(s: SessionStatus) -> (&'static str, Color) {
    match s {
        SessionStatus::Working => ("●", OK),
        SessionStatus::Blocked => ("◐", WARN),
        SessionStatus::Idle => ("✓", DIM),
        SessionStatus::Failed => ("✗", ERR),
    }
}

pub fn status_color(s: SessionStatus) -> Color {
    status_symbol(s).1
}

pub fn status_label(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Working => "运行中",
        SessionStatus::Blocked => "等待确认",
        SessionStatus::Idle => "已完成",
        SessionStatus::Failed => "失败",
    }
}

/// 相对时间：刚刚 / 3m / 2h / 昨天 / 3d
pub fn rel_time(t: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => "刚刚".into(),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        86400..172800 => "昨天".into(),
        _ => format!("{}d", secs / 86400),
    }
}

/// 家目录换成 ~
pub fn home_short(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty())
        && let Ok(rest) = p.strip_prefix(Path::new(&home))
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

/// 路径相对 cwd 显示，不在 cwd 下就用 ~ 缩写
pub fn rel_path(p: &str, cwd: &Path) -> String {
    let path = Path::new(p);
    match path.strip_prefix(cwd) {
        Ok(r) if !r.as_os_str().is_empty() => r.display().to_string(),
        _ => home_short(path),
    }
}

/// claude-opus-4-5-20251101 → Opus 4.5；认不出就原样返回
pub fn pretty_model(m: &str) -> String {
    let base = m.strip_prefix("claude-").unwrap_or(m);
    if base.contains(['[', ' ', '/', '.']) {
        return m.to_string();
    }
    let mut names = Vec::new();
    let mut nums = Vec::new();
    for part in base.split('-') {
        if part.is_empty() {
            continue;
        }
        if part.chars().all(|c| c.is_ascii_digit()) {
            if part.len() < 8 {
                nums.push(part);
            }
        } else if part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut cs = part.chars();
            let first = cs.next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
            names.push(format!("{first}{}", cs.as_str()));
        } else {
            return m.to_string();
        }
    }
    if names.is_empty() {
        return m.to_string();
    }
    let mut out = names.join(" ");
    if !nums.is_empty() {
        out.push(' ');
        out.push_str(&nums.join("."));
    }
    out
}

/// 38200 → 38.2k
pub fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    }
}
