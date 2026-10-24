//! 简单的 markdown 渲染：标题、列表、引用、代码块、**粗体**、`行内代码`。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::text::{self, wrap};
use super::theme::*;

/// 渲染成折好行的 Line。first 是整段第一行的前缀，rest 是其余行的前缀（两者宽度应相同）
pub fn render(md: &str, width: usize, base: Style, first: Vec<Span<'static>>, rest: Vec<Span<'static>>) -> Vec<Line<'static>> {
    let md = text::clean(md);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;
    let mut prev_blank = false;
    let code_style = Style::new().fg(FG).bg(BLOCK_BG);

    for raw in md.trim_matches('\n').split('\n') {
        let pfx = if out.is_empty() { first.clone() } else { rest.clone() };
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            let mut lines = wrap(vec![Span::styled(format!(" {raw}"), code_style)], width, pfx, rest.clone());
            for l in &mut lines {
                text::pad(l, width, code_style);
            }
            out.extend(lines);
            prev_blank = false;
            continue;
        }
        if trimmed.is_empty() {
            if !prev_blank && !out.is_empty() {
                out.push(Line::from(pfx));
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;
        let indent = raw.len() - trimmed.len();

        // 标题
        if let Some(h) = heading(trimmed) {
            let st = bold(base).fg(FG);
            out.extend(wrap(inline(h, st), width, pfx, rest.clone()));
            continue;
        }
        // 分隔线
        if trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') && trimmed.len() >= 3 {
            let w = width.saturating_sub(text::spans_width(&pfx));
            let mut spans = pfx;
            spans.push(Span::styled("─".repeat(w.min(40)), fg(BORDER)));
            out.push(Line::from(spans));
            continue;
        }
        // 引用
        if let Some(q) = trimmed.strip_prefix('>') {
            let mut f = pfx;
            f.push(Span::styled("│ ", dim()));
            let mut r = rest.clone();
            r.push(Span::styled("│ ", dim()));
            out.extend(wrap(inline(q.trim_start(), dim().add_modifier(Modifier::ITALIC)), width, f, r));
            continue;
        }
        // 列表
        if let Some((marker, body)) = list_item(trimmed) {
            let pad = " ".repeat(indent.min(8));
            let mut f = pfx;
            f.push(Span::raw(pad.clone()));
            f.push(Span::styled(marker.clone(), dim()));
            let mut r = rest.clone();
            r.push(Span::raw(format!("{pad}{}", " ".repeat(text::str_width(&marker)))));
            out.extend(wrap(inline(body, base), width, f, r));
            continue;
        }
        out.extend(wrap(inline(trimmed, base), width, pfx, rest.clone()));
    }
    if out.is_empty() {
        out.push(Line::from(first));
    }
    out
}

fn heading(s: &str) -> Option<&str> {
    let n = s.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) {
        s[n..].strip_prefix(' ').map(str::trim)
    } else {
        None
    }
}

/// 列表项：返回显示用的标记（"• " 或 "1. "）和正文
fn list_item(s: &str) -> Option<(String, &str)> {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(m) {
            // 任务列表
            if let Some(r) = rest.strip_prefix("[ ] ") {
                return Some(("☐ ".into(), r));
            }
            if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
                return Some(("✔ ".into(), r));
            }
            return Some(("• ".into(), rest));
        }
    }
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 4 {
        let rest = &s[digits..];
        if let Some(body) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((format!("{}. ", &s[..digits]), body));
        }
    }
    None
}

/// 行内样式：**粗体**、`代码`、*斜体*。不配对的标记原样保留
pub fn inline(s: &str, base: Style) -> Vec<Span<'static>> {
    let code = Style::new().fg(BLUE);
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut rest = s;
    let flush = |buf: &mut String, out: &mut Vec<Span<'static>>| {
        if !buf.is_empty() {
            out.push(Span::styled(std::mem::take(buf), base));
        }
    };
    while let Some(c) = rest.chars().next() {
        if c == '`'
            && let Some(end) = rest[1..].find('`')
            && end > 0
        {
            flush(&mut buf, &mut out);
            out.push(Span::styled(rest[1..1 + end].to_string(), code));
            rest = &rest[end + 2..];
            continue;
        }
        if let Some(r) = rest.strip_prefix("**")
            && let Some(end) = r.find("**")
            && end > 0
        {
            flush(&mut buf, &mut out);
            for sp in inline(&r[..end], base) {
                let st = sp.style.add_modifier(Modifier::BOLD);
                out.push(Span::styled(sp.content.into_owned(), st));
            }
            rest = &r[end + 2..];
            continue;
        }
        if c == '*'
            && let Some(r) = rest.strip_prefix('*')
            && !r.starts_with([' ', '*'])
            && let Some(end) = r.find('*')
            && end > 0
        {
            flush(&mut buf, &mut out);
            out.push(Span::styled(r[..end].to_string(), base.add_modifier(Modifier::ITALIC)));
            rest = &r[end + 1..];
            continue;
        }
        buf.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut buf, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_styles() {
        let spans = inline("用 **ratatui** 和 `Layout` 做", Style::new());
        let texts: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, vec!["用 ", "ratatui", " 和 ", "Layout", " 做"]);
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        // 不配对的标记保留
        let spans = inline("a ** b `c", Style::new());
        assert_eq!(spans.iter().map(|s| s.content.as_ref()).collect::<String>(), "a ** b `c");
    }

    #[test]
    fn blocks() {
        let md = "# 标题\n\n- 一\n- 二\n\n```rust\nfn main() {}\n```";
        let lines = render(md, 30, Style::new(), vec![Span::raw("● ")], vec![Span::raw("  ")]);
        let texts: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect();
        assert_eq!(texts[0], "● 标题");
        assert!(texts.iter().any(|t| t.starts_with("  • 一")));
        assert!(texts.iter().any(|t| t.contains("fn main() {}")));
        assert!(!texts.iter().any(|t| t.contains("```")));
    }
}
