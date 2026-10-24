//! 右侧面板：变更 / 待办。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, Borders, Paragraph};

use super::text::{self, split_row, truncate_left, wrap};
use super::theme::*;
use crate::app::{App, Focus, PanelTab};
use crate::model::{Conversation, TodoStatus};

pub fn draw(f: &mut Frame, area: Rect, app: &App, conv: &Conversation) {
    let focused = app.focus == Focus::Panel && app.overlay.is_none();
    let block = WBlock::new().borders(Borders::LEFT).border_style(fg(if focused { ACCENT } else { BORDER }));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 6 {
        return;
    }
    let w = inner.width as usize;
    let done = conv.todos.iter().filter(|t| t.status == TodoStatus::Completed).count();

    let tab = |label: String, active: bool| {
        if active {
            let st = bold(fg(if focused { ACCENT } else { FG })).add_modifier(Modifier::UNDERLINED);
            Span::styled(label, st)
        } else {
            Span::styled(label, dim())
        }
    };
    let mut lines = vec![
        Line::from(vec![
            Span::raw(" "),
            tab(format!("变更 {}", app.changes.len()), app.panel_tab == PanelTab::Changes),
            Span::raw("   "),
            tab(format!("待办 {done}/{}", conv.todos.len()), app.panel_tab == PanelTab::Todos),
        ]),
        Line::styled("─".repeat(w), fg(BORDER)),
    ];

    match app.panel_tab {
        PanelTab::Changes => {
            if app.changes.is_empty() {
                lines.push(Line::styled(" 没有未提交的变更", dim()));
            }
            for c in &app.changes {
                let color = match c.status {
                    'M' => WARN,
                    'A' => OK,
                    'D' => ERR,
                    'R' => BLUE,
                    _ => DIM,
                };
                let mut right = Vec::new();
                if c.added > 0 {
                    right.push(Span::styled(format!("+{}", c.added), fg(OK)));
                }
                if c.removed > 0 {
                    if !right.is_empty() {
                        right.push(Span::raw(" "));
                    }
                    right.push(Span::styled(format!("−{}", c.removed), fg(ERR)));
                }
                right.push(Span::raw(" "));
                let rw = text::spans_width(&right);
                let path = truncate_left(&c.path, w.saturating_sub(rw + 4));
                let left = vec![Span::raw(" "), Span::styled(c.status.to_string(), bold(fg(color))), Span::raw(" "), Span::styled(path, fg(FG))];
                lines.push(split_row(left, right, w));
            }
        }
        PanelTab::Todos => {
            if conv.todos.is_empty() {
                lines.push(Line::styled(" 暂无待办", dim()));
            }
            for t in &conv.todos {
                let (mark, mstyle, tstyle) = match t.status {
                    TodoStatus::Completed => ("✔ ", fg(OK), dim()),
                    TodoStatus::InProgress => ("▸ ", accent(), bold(fg(FG))),
                    TodoStatus::Pending => ("☐ ", dim(), fg(FG)),
                };
                lines.extend(wrap(
                    vec![Span::styled(t.content.replace('\n', " "), tstyle)],
                    w,
                    vec![Span::raw(" "), Span::styled(mark, mstyle)],
                    vec![Span::raw("   ")],
                ));
            }
        }
    }

    let h = inner.height as usize;
    let footer = h > 4;
    lines.truncate(if footer { h - 1 } else { h });
    f.render_widget(Paragraph::new(lines), inner);
    if footer {
        let r = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(Paragraph::new(Line::styled(" Ctrl+T 切换标签 · Ctrl+P 隐藏", dim())), r);
    }
}
