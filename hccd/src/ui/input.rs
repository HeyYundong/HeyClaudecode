//! 输入框：多行、按字符折行、光标定位。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, BorderType, Borders, Paragraph};

use super::text::char_width;
use super::theme::*;
use crate::app::{App, Focus};

/// 前缀 "› " 的宽度，续行同样缩进
const PREFIX: usize = 2;

pub struct InputLayout {
    /// 每个显示行的文字
    pub rows: Vec<String>,
    /// 光标所在的 (行, 列)，列不含前缀
    pub cursor: (usize, usize),
}

/// 按字符折行。width 是含前缀的可用宽度
pub fn layout(text: &str, cursor: usize, width: usize) -> InputLayout {
    let avail = width.saturating_sub(PREFIX).max(1);
    let mut rows = vec![String::new()];
    let mut col = 0;
    let mut pos = None;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            if i == cursor {
                pos = Some((rows.len() - 1, col));
            }
            rows.push(String::new());
            col = 0;
            continue;
        }
        let c = if c == '\t' { ' ' } else { c };
        let cw = char_width(c);
        if col + cw > avail && col > 0 {
            rows.push(String::new());
            col = 0;
        }
        if i == cursor {
            pos = Some((rows.len() - 1, col));
        }
        rows.last_mut().unwrap().push(c);
        col += cw;
    }
    let cursor = pos.unwrap_or_else(|| {
        // 光标在末尾：如果最后一行刚好写满，放到下一行开头
        if col >= avail {
            rows.push(String::new());
            (rows.len() - 1, 0)
        } else {
            (rows.len() - 1, col)
        }
    });
    InputLayout { rows, cursor }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App, lay: &InputLayout) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let focused = app.focus == Focus::Input && app.overlay.is_none();
    let block = WBlock::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(if focused { ACCENT } else { BORDER }));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
    let h = inner.height as usize;
    if h == 0 {
        return;
    }

    if app.input.text.is_empty() {
        let line = Line::from(vec![Span::styled("› ", accent()), Span::styled("回复 Claude…", dim())]);
        f.render_widget(Paragraph::new(line), inner);
        if focused {
            f.set_cursor_position((inner.x + PREFIX as u16, inner.y));
        }
        return;
    }

    // 行数超过框高时滚到光标所在行
    let (crow, ccol) = lay.cursor;
    let start = (crow + 1).saturating_sub(h);
    let lines: Vec<Line> = lay
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(h)
        .map(|(i, r)| {
            let pfx = if i == 0 { Span::styled("› ", accent()) } else { Span::raw("  ") };
            Line::from(vec![pfx, Span::styled(r.clone(), fg(FG))])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    if focused {
        let x = (inner.x as usize + PREFIX + ccol).min((inner.x + inner.width) as usize) as u16;
        let y = inner.y + (crow - start) as u16;
        f.set_cursor_position((x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_positions() {
        let l = layout("ab\n中文", 4, 10);
        assert_eq!(l.rows, vec!["ab", "中文"]);
        assert_eq!(l.cursor, (1, 2));
        let l = layout("abcdef", 6, 5);
        assert_eq!(l.rows, vec!["abc", "def", ""]);
        assert_eq!(l.cursor, (2, 0));
        let l = layout("abcdef", 3, 5);
        assert_eq!(l.cursor, (1, 0));
        let l = layout("", 0, 5);
        assert_eq!(l.cursor, (0, 0));
    }
}
