//! 覆盖层：Ctrl+K 快速切换、添加项目、帮助。

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, BorderType, Borders, Clear, Padding, Paragraph};

use super::text::{self, pad, split_row, str_width, truncate_left};
use super::theme::*;
use crate::app::{App, Overlay};

pub fn draw(f: &mut Frame, area: Rect, app: &App, ov: &Overlay) {
    shade(f, area);
    match ov {
        Overlay::Switcher { query, selected } => switcher(f, area, app, query, *selected),
        Overlay::AddProject { input } => add_project(f, area, input),
        Overlay::Help => help(f, area),
    }
}

/// 压暗背后的界面
fn shade(f: &mut Frame, area: Rect) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(SHADE_FG).set_bg(SHADE_BG);
            }
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

/// 画框并返回框内区域
fn frame(f: &mut Frame, r: Rect, title: Option<&str>) -> Rect {
    // 框左边如果压住了宽字符的后半，把这个宽字符换成空格，免得终端里错位
    if r.x > 0 {
        let buf = f.buffer_mut();
        for y in r.top()..r.bottom() {
            if let Some(cell) = buf.cell_mut((r.x - 1, y))
                && str_width(cell.symbol()) > 1
            {
                cell.set_symbol(" ");
            }
        }
    }
    f.render_widget(Clear, r);
    let mut block = WBlock::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(accent())
        .style(Style::new().bg(OVERLAY_BG).fg(FG))
        .padding(Padding::horizontal(1));
    if let Some(t) = title {
        block = block.title(Span::styled(format!(" {t} "), bold(accent())));
    }
    let inner = block.inner(r);
    f.render_widget(block, r);
    inner
}

/// 把 label 里和 query 匹配（不区分大小写）的部分高亮
fn highlight(label: &str, query: &str, base: Style) -> Vec<Span<'static>> {
    let lc: Vec<char> = label.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let q: Vec<char> = query.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let chars: Vec<char> = label.chars().collect();
    let hl = bold(accent());
    if q.is_empty() || q.len() > lc.len() {
        return vec![Span::styled(label.to_string(), base)];
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + q.len() <= lc.len() && lc[i..i + q.len()] == q[..] {
            if !buf.is_empty() {
                out.push(Span::styled(std::mem::take(&mut buf), base));
            }
            out.push(Span::styled(chars[i..i + q.len()].iter().collect::<String>(), hl));
            i += q.len();
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, base));
    }
    out
}

fn switcher(f: &mut Frame, area: Rect, app: &App, query: &str, selected: usize) {
    // 框宽：两侧边框 + 各 1 列内边距
    let box_w = 80.min(area.width.saturating_sub(4)).max(20).min(area.width);
    let w = box_w.saturating_sub(4) as usize;
    let matches = app.switcher_matches(query);
    let now = SystemTime::now();

    let head = split_row(
        vec![Span::styled("› ", bold(accent())), Span::styled(query.to_string(), fg(FG)), Span::styled("▌", accent())],
        vec![Span::styled(format!("{} / {}", matches.len(), app.switch_index.len()), dim())],
        w,
    );
    let mut fixed = vec![head, Line::styled("─".repeat(w), fg(BORDER))];

    // 列表：按顺序遇到分组变化就插标题
    let mut rows: Vec<(Line<'static>, bool)> = Vec::new();
    let mut last_group: Option<&str> = None;
    for (i, e) in matches.iter().enumerate() {
        if last_group != Some(e.group.as_str()) {
            if last_group.is_some() {
                rows.push((Line::default(), false));
            }
            rows.push((Line::from(highlight(&e.group, query, bold(dim()))), false));
            last_group = Some(&e.group);
        }
        let (sym, color) = status_symbol(e.status);
        let mut left = vec![Span::styled(sym, fg(color)), Span::raw(" ")];
        left.extend(highlight(&e.label.replace('\n', " "), query, fg(FG)));
        let right = vec![Span::styled(format!("{} · {}", status_label(e.status), rel_time(e.updated, now)), dim())];
        let mut line = split_row(left, right, w);
        let sel = i == selected;
        if sel {
            pad(&mut line, w, Style::new());
            for s in &mut line.spans {
                s.style = s.style.bg(SEL_BG);
            }
        }
        rows.push((line, sel));
    }
    if matches.is_empty() {
        rows.push((Line::styled("没有匹配的会话", dim()), false));
    }

    // 框高随内容，最多 24 行：输入行、分隔线、列表、空行、提示，加上下边框
    let want = (rows.len() + 6).clamp(10, 24) as u16;
    let r = centered(area, box_w, want.min(area.height.saturating_sub(2)).max(5));
    let inner = frame(f, r, None);
    if inner.height == 0 {
        return;
    }
    let list_h = (inner.height as usize).saturating_sub(fixed.len() + 2);
    let sel_row = rows.iter().position(|(_, s)| *s).unwrap_or(0);
    let offset = (sel_row + 1).saturating_sub(list_h).min(rows.len().saturating_sub(list_h));
    fixed.extend(rows.into_iter().skip(offset).take(list_h).map(|(l, _)| l));
    f.render_widget(Paragraph::new(fixed), inner);
    if inner.height as usize > 3 {
        let hint = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(Paragraph::new(Line::styled("↑↓ 选择 · Enter 打开 · Esc 关闭", dim())), hint);
    }
}

fn add_project(f: &mut Frame, area: Rect, input: &str) {
    let r = centered(area, 64.min(area.width.saturating_sub(4)).max(20), 7);
    let inner = frame(f, r, Some("添加项目"));
    let w = inner.width as usize;
    let shown = truncate_left(input, w.saturating_sub(3));
    let lines = vec![
        Line::styled("输入目录路径", dim()),
        Line::from(vec![Span::styled("› ", bold(accent())), Span::styled(shown, fg(FG)), Span::styled("▌", accent())]),
        Line::default(),
        Line::styled("Enter 添加 · Esc 取消", dim()),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

pub const HELP_KEYS: &[(&str, &str)] = &[
    ("Ctrl+N", "新对话"),
    ("Enter", "打开选中项 / 发送消息"),
    ("Alt+Enter", "输入框换行"),
    ("Tab", "切换焦点"),
    ("↑↓ / j k", "移动"),
    ("Ctrl+K", "快速切换会话"),
    ("Ctrl+B", "显示 / 隐藏侧栏"),
    ("Ctrl+P", "显示 / 隐藏面板"),
    ("Ctrl+T", "切换面板标签"),
    ("PgUp / PgDn", "滚动对话"),
    ("a", "添加项目"),
    ("d", "移除项目"),
    ("Esc", "中断当前轮 / 关闭弹窗"),
    ("Ctrl+C", "退出"),
];

fn help(f: &mut Frame, area: Rect) {
    let h = HELP_KEYS.len() as u16 + 4;
    let r = centered(area, 48.min(area.width), h);
    let inner = frame(f, r, Some("快捷键"));
    let key_w = HELP_KEYS.iter().map(|(k, _)| str_width(k)).max().unwrap_or(0);
    let mut lines: Vec<Line> = HELP_KEYS
        .iter()
        .map(|(k, d)| {
            let pad_n = key_w - text::str_width(k);
            Line::from(vec![
                Span::styled(format!("{k}{}", " ".repeat(pad_n)), accent()),
                Span::raw("   "),
                Span::styled(*d, fg(FG)),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::styled("Esc 或 ? 关闭", dim()));
    f.render_widget(Paragraph::new(lines), inner);
}
