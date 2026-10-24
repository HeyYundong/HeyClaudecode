//! 按显示宽度折行、截断、补齐。中文等宽字符按 2 列算。

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

pub fn str_width(s: &str) -> usize {
    s.width()
}

pub fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// 把 tab 换成空格，去掉其他控制字符
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\t' => out.push_str("    "),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// 截断到 max 列，超出时末尾加 …
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if w + cw > max - 1 {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// 从左边截断（保留结尾，适合路径）
pub fn truncate_left(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut rev = Vec::new();
    let mut w = 0;
    for c in s.chars().rev() {
        let cw = char_width(c);
        if w + cw > max - 1 {
            break;
        }
        rev.push(c);
        w += cw;
    }
    let mut out = String::from("…");
    out.extend(rev.into_iter().rev());
    out
}

/// 一组 span 截断到 max 列
pub fn truncate_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut left = max;
    for s in spans {
        let w = s.content.width();
        if w < left {
            left -= w;
            out.push(s);
        } else {
            out.push(Span::styled(truncate(&s.content, left), s.style));
            break;
        }
    }
    out
}

/// 在行尾补空格到 width 列，补的部分用 style
pub fn pad(line: &mut Line<'static>, width: usize, style: Style) {
    let w = spans_width(&line.spans);
    if w < width {
        line.spans.push(Span::styled(" ".repeat(width - w), style));
    }
}

/// 左边内容 + 右对齐内容，凑满 width 列；左边太长就截断
pub fn split_row(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let rw = spans_width(&right);
    let right = if rw + 1 > width { Vec::new() } else { right };
    let rw = spans_width(&right);
    let left_max = if rw > 0 { width.saturating_sub(rw + 1) } else { width };
    let mut spans = truncate_spans(left, left_max);
    let lw = spans_width(&spans);
    let gap = width.saturating_sub(lw + rw);
    if gap > 0 {
        spans.push(Span::raw(" ".repeat(gap)));
    }
    spans.extend(right);
    Line::from(spans)
}

struct Token {
    start: usize,
    end: usize,
    width: usize,
    space: bool,
}

/// 把带样式的文本折成多行。width 是含前缀的总宽度；
/// 第一行用 first 前缀，后面的行用 rest 前缀。文本里不应有换行符。
/// 英文按单词折，中文等宽字符逐字折，过长的单词硬切。
pub fn wrap(
    spans: Vec<Span<'static>>,
    width: usize,
    first: Vec<Span<'static>>,
    rest: Vec<Span<'static>>,
) -> Vec<Line<'static>> {
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| {
            let style = s.style;
            s.content
                .chars()
                .filter(|c| *c != '\n' && *c != '\r')
                .map(move |c| (if c == '\t' { ' ' } else { c }, style))
        })
        .collect();
    let avail_first = width.saturating_sub(spans_width(&first)).max(1);
    let avail_rest = width.saturating_sub(spans_width(&rest)).max(1);

    // 切成 token：单个空白、单个宽字符、或一串连续的窄字符
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].0;
        let cw = char_width(c);
        if c.is_whitespace() || cw >= 2 {
            tokens.push(Token { start: i, end: i + 1, width: cw, space: c.is_whitespace() });
            i += 1;
            continue;
        }
        let start = i;
        let mut w = 0;
        while i < chars.len() && !chars[i].0.is_whitespace() && char_width(chars[i].0) < 2 {
            w += char_width(chars[i].0);
            i += 1;
        }
        tokens.push(Token { start, end: i, width: w, space: false });
    }

    let avail = |row: usize| if row == 0 { avail_first } else { avail_rest };
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut cur = 0usize;
    for t in tokens {
        let row = rows.len() - 1;
        if t.space {
            // 续行开头的空格丢掉
            if cur == 0 && row > 0 {
                continue;
            }
            if cur + t.width <= avail(row) {
                rows[row].push(t.start);
                cur += t.width;
            } else {
                rows.push(Vec::new());
                cur = 0;
            }
            continue;
        }
        if cur + t.width <= avail(row) {
            rows[row].extend(t.start..t.end);
            cur += t.width;
            continue;
        }
        if cur > 0 && t.width <= avail(row + 1) {
            rows.push((t.start..t.end).collect());
            cur = t.width;
            continue;
        }
        // 过长的单词：逐字符硬切
        for (k, &(c, _)) in chars.iter().enumerate().take(t.end).skip(t.start) {
            let cw = char_width(c);
            let row = rows.len() - 1;
            if cur > 0 && cur + cw > avail(row) {
                rows.push(Vec::new());
                cur = 0;
            }
            rows.last_mut().unwrap().push(k);
            cur += cw;
        }
    }

    rows.into_iter()
        .enumerate()
        .map(|(n, idx)| {
            let mut spans: Vec<Span<'static>> = if n == 0 { first.clone() } else { rest.clone() };
            let mut buf = String::new();
            let mut style: Option<Style> = None;
            for k in idx {
                let (c, st) = chars[k];
                if style != Some(st) {
                    if let Some(s) = style
                        && !buf.is_empty()
                    {
                        spans.push(Span::styled(std::mem::take(&mut buf), s));
                    }
                    style = Some(st);
                }
                buf.push(c);
            }
            if let Some(s) = style
                && !buf.is_empty()
            {
                spans.push(Span::styled(buf, s));
            }
            Line::from(spans)
        })
        .collect()
}

/// 纯文本按 \n 分段再折行，每行都用同样的前缀
pub fn wrap_plain(text: &str, width: usize, style: Style, prefix: &str) -> Vec<Line<'static>> {
    text.split('\n')
        .flat_map(|l| {
            wrap(
                vec![Span::styled(l.to_string(), style)],
                width,
                vec![Span::raw(prefix.to_string())],
                vec![Span::raw(prefix.to_string())],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn wrap_words_and_cjk() {
        let l = wrap(vec![Span::raw("hello world foo")], 11, vec![], vec![]);
        assert_eq!(texts(&l), vec!["hello world", "foo"]);
        let l = wrap(vec![Span::raw("中文折行测试")], 7, vec![Span::raw("● ")], vec![Span::raw("  ")]);
        assert_eq!(texts(&l), vec!["● 中文", "  折行", "  测试"]);
        for line in &l {
            assert!(spans_width(&line.spans) <= 7);
        }
        let l = wrap(vec![Span::raw("abcdefghij")], 4, vec![], vec![]);
        assert_eq!(texts(&l), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn truncate_width() {
        assert_eq!(truncate("中文字符", 5), "中文…");
        assert_eq!(truncate_left("src/a/b.rs", 6), "…/b.rs");
    }
}
