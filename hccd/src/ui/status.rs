//! 顶栏和底部状态栏。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::text::split_row;
use super::theme::*;
use crate::app::App;

pub fn draw_top(f: &mut Frame, area: Rect, app: &App) {
    let conv = app.current_conv();
    let place = match conv {
        Some(c) if c.project.is_some() => home_short(&c.cwd),
        Some(_) => "对话 · 不关联项目".into(),
        None => app.expanded.and_then(|i| app.projects.get(i)).map(|p| home_short(&p.path)).unwrap_or_default(),
    };
    let left = vec![
        Span::styled(" ✻ HeyClaude", bold(accent())),
        Span::raw("   "),
        Span::styled(place, dim()),
    ];
    let model = conv.and_then(|c| c.model.as_deref()).map(pretty_model).unwrap_or_default();
    let right = vec![Span::styled(format!("{model} "), fg(FG))];
    let line = split_row(left, right, area.width as usize);
    f.render_widget(Paragraph::new(line).style(Style::new().bg(BAR_BG)), area);
}

pub fn draw_bottom(f: &mut Frame, area: Rect, app: &App) {
    let left = match &app.flash {
        Some(msg) => vec![Span::styled(format!(" {msg}"), fg(WARN))],
        None => vec![Span::styled(" Tab 切换焦点 · Ctrl+K 切换会话 · Ctrl+B 侧栏 · Ctrl+P 面板 · ? 帮助", dim())],
    };
    let mut right = Vec::new();
    if let Some(c) = app.current_conv() {
        let u = &c.usage;
        if let Some(win) = u.context_window.filter(|w| *w > 0) {
            let pct = (u.context_tokens as f64 / win as f64 * 100.0).round().min(100.0) as usize;
            let filled = (pct + 5) / 10;
            right.push(Span::styled("上下文 ", dim()));
            right.push(Span::styled(format!("{pct}% "), fg(FG)));
            right.push(Span::styled("▮".repeat(filled.min(10)), accent()));
            right.push(Span::styled("▯".repeat(10 - filled.min(10)), dim()));
            right.push(Span::raw("  "));
        }
        if u.context_tokens > 0 || u.context_window.is_some() {
            right.push(Span::styled(format!("{} tokens", fmt_tokens(u.context_tokens)), dim()));
        }
        if u.cost_usd > 0.0 {
            right.push(Span::styled(format!("  ${:.2}", u.cost_usd), dim()));
        }
        right.push(Span::raw(" "));
    }
    let line = split_row(left, right, area.width as usize);
    f.render_widget(Paragraph::new(line).style(Style::new().bg(BAR_BG)), area);
}
