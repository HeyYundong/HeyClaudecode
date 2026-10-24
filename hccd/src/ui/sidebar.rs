//! 左侧栏：新对话、不关联项目的对话、项目和展开项目的会话。

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, Borders, Paragraph};

use super::text::{pad, split_row};
use super::theme::*;
use crate::app::{App, Focus};
use crate::model::{SessionInfo, SessionStatus};

struct Row {
    line: Line<'static>,
    /// 对应 app.sidebar_items() 的下标；标题、分隔线为 None
    item: Option<usize>,
}

/// 会话的实际状态：hccd 正托管时以 Conversation 为准
pub fn effective_status(app: &App, s: &SessionInfo) -> SessionStatus {
    s.conv.and_then(|id| app.convs.get(&id)).map_or(s.status, |c| c.status())
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Sidebar && app.overlay.is_none();
    let block = WBlock::new().borders(Borders::RIGHT).border_style(fg(if focused { ACCENT } else { BORDER }));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 4 {
        return;
    }
    let w = inner.width as usize;
    let rows = build_rows(app, w);

    // 底部留一行提示
    let footer_h = if inner.height > 4 { 1 } else { 0 };
    let h = (inner.height - footer_h) as usize;
    let sel_row = rows.iter().position(|r| r.item == Some(app.sidebar_selected)).unwrap_or(0);
    // 让选中行可见，并尽量在它下面多露一行
    let max_off = rows.len().saturating_sub(h);
    let offset = (sel_row + 2).saturating_sub(h).min(max_off);

    let lines: Vec<Line> = rows
        .into_iter()
        .skip(offset)
        .take(h)
        .map(|r| {
            if r.item == Some(app.sidebar_selected) {
                highlight(r.line, w, focused)
            } else {
                r.line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: h as u16, ..inner });
    if footer_h > 0 {
        let r = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(Paragraph::new(Line::styled(" n 新建  a 添加项目  / 搜索", dim())), r);
    }
}

/// 选中行：背景色 + 左侧强调色竖条
fn highlight(mut line: Line<'static>, w: usize, focused: bool) -> Line<'static> {
    // 第一列是留白，换成竖条
    if let Some(first) = line.spans.first_mut() {
        let rest: String = first.content.chars().skip(1).collect();
        let bar = Span::styled("▎", Style::new().fg(if focused { ACCENT } else { DIM }));
        let st = first.style;
        line.spans[0] = Span::styled(rest, st);
        line.spans.insert(0, bar);
    }
    pad(&mut line, w, Style::new());
    line.style = Style::new().bg(SEL_BG);
    for s in &mut line.spans {
        s.style = s.style.bg(SEL_BG);
    }
    line
}

fn build_rows(app: &App, w: usize) -> Vec<Row> {
    let now = SystemTime::now();
    let mut rows = Vec::new();
    let mut k = 0usize;
    let mut push = |rows: &mut Vec<Row>, line: Line<'static>, selectable: bool| {
        let item = selectable.then(|| {
            k += 1;
            k - 1
        });
        rows.push(Row { line, item });
    };
    let sep = || Line::styled("─".repeat(w), fg(BORDER));

    // 新对话
    push(
        &mut rows,
        split_row(vec![Span::styled(" + 新对话", accent())], vec![Span::styled("Ctrl+N ", dim())], w),
        true,
    );
    push(&mut rows, Line::default(), false);
    push(&mut rows, Line::styled(" 对话 · 不关联项目", bold(dim())), false);
    if app.chats.is_empty() {
        push(&mut rows, Line::styled("   暂无对话", dim()), false);
    }
    for s in &app.chats {
        push(&mut rows, session_row(app, s, "  ", w, now), true);
    }

    push(&mut rows, sep(), false);
    push(
        &mut rows,
        Line::from(vec![Span::styled(" 项目 ", bold(dim())), Span::styled(app.projects.len().to_string(), dim())]),
        false,
    );
    if app.projects.is_empty() {
        push(&mut rows, Line::styled("   按 a 添加项目", dim()), false);
    }
    for (i, p) in app.projects.iter().enumerate() {
        let expanded = app.expanded == Some(i);
        let left = if expanded {
            vec![Span::styled(format!(" ▾ {}", p.name), bold(accent()))]
        } else {
            vec![Span::styled(format!("   {}", p.name), fg(FG))]
        };
        push(&mut rows, split_row(left, project_counts(app, i), w), true);
        if expanded {
            if app.sessions.is_empty() {
                push(&mut rows, Line::styled("     暂无会话", dim()), false);
            }
            let statuses: Vec<SessionStatus> = app.sessions.iter().map(|s| effective_status(app, s)).collect();
            for (j, s) in app.sessions.iter().enumerate() {
                let st = statuses[j];
                if j == 0 || statuses[j - 1] != st {
                    let n = statuses[j..].iter().take_while(|x| **x == st).count();
                    if j > 0 {
                        push(&mut rows, Line::default(), false);
                    }
                    push(&mut rows, Line::styled(format!("   {} {n}", group_title(st)), fg(status_color(st))), false);
                }
                push(&mut rows, session_row(app, s, "    ", w, now), true);
            }
        }
    }
    debug_assert_eq!(k, app.sidebar_items().len());
    rows
}

fn group_title(s: SessionStatus) -> &'static str {
    status_label(s)
}

fn session_row(app: &App, s: &SessionInfo, indent: &str, w: usize, now: SystemTime) -> Line<'static> {
    let (sym, color) = status_symbol(effective_status(app, s));
    let current = s.conv.is_some() && s.conv == app.current;
    let title_style = if current { bold(fg(FG)) } else { fg(FG) };
    split_row(
        vec![
            Span::raw(format!(" {indent}")),
            Span::styled(sym, fg(color)),
            Span::raw(" "),
            Span::styled(s.title.replace('\n', " "), title_style),
        ],
        vec![Span::styled(format!("{} ", rel_time(s.updated, now)), dim())],
        w,
    )
}

/// 项目右侧的状态计数：●2 ◐1，没有活动就显示 ✓N 或 ○
fn project_counts(app: &App, i: usize) -> Vec<Span<'static>> {
    let mut counts = [0usize; 4];
    let idx = |s: SessionStatus| match s {
        SessionStatus::Working => 0,
        SessionStatus::Blocked => 1,
        SessionStatus::Idle => 2,
        SessionStatus::Failed => 3,
    };
    if app.expanded == Some(i) {
        for s in &app.sessions {
            counts[idx(effective_status(app, s))] += 1;
        }
    } else {
        let name = &app.projects[i].name;
        for e in app.switch_index.iter().filter(|e| &e.group == name) {
            counts[idx(e.status)] += 1;
        }
    }
    let mut out = Vec::new();
    for (n, st) in [(counts[0], SessionStatus::Working), (counts[1], SessionStatus::Blocked), (counts[3], SessionStatus::Failed)] {
        if n > 0 {
            let (sym, c) = status_symbol(st);
            if !out.is_empty() {
                out.push(Span::raw(" "));
            }
            out.push(Span::styled(format!("{sym}{n}"), fg(c)));
        }
    }
    if out.is_empty() {
        out.push(Span::styled(if counts[2] > 0 { format!("✓{}", counts[2]) } else { "○".into() }, dim()));
    }
    out.push(Span::raw(" "));
    out
}
