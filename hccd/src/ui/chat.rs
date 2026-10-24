//! 中间对话区：标题行、消息、权限确认框、输入框。

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, BorderType, Borders, Paragraph};

use super::text::{self, pad, split_row, truncate, wrap, wrap_plain};
use super::theme::*;
use super::{input, markdown, tool};
use crate::app::App;
use crate::model::{Block, Conversation, PermissionRequest};

/// Bash 输出最多显示几行
const BASH_PREVIEW: usize = 5;
/// diff 最多显示几行
const DIFF_MAX: usize = 12;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    // 左右各留 2 列边距
    let pad_x = if area.width > 20 { 2 } else { 0 };
    let area = Rect { x: area.x + pad_x, width: area.width.saturating_sub(pad_x * 2), ..area };
    let w = area.width as usize;
    let conv = app.current_conv();

    let perm = conv.and_then(|c| c.permission.as_ref().map(|p| permission_lines(app, c, p, w.saturating_sub(4))));
    let perm_h = perm.as_ref().map_or(0, |l| l.len() as u16 + 2);
    let input_layout = input::layout(&app.input.text, app.input.cursor, w.saturating_sub(4));
    let input_h = input_layout.rows.len().clamp(1, 6) as u16 + 2;

    let [title, sep, body, perm_area, input_area, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(perm_h),
        Constraint::Length(input_h),
        Constraint::Length(1),
    ])
    .areas(area);

    match conv {
        Some(c) => {
            draw_title(f, title, c);
            f.render_widget(Paragraph::new(Line::styled("─".repeat(w), fg(BORDER))), sep);
            let lines = conv_lines(c, w);
            let h = body.height as usize;
            let max_scroll = lines.len().saturating_sub(h);
            let scroll = (app.chat_scroll as usize).min(max_scroll);
            let start = lines.len().saturating_sub(h + scroll);
            let visible: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
            f.render_widget(Paragraph::new(visible), body);
            if scroll > 0 && body.height > 0 {
                // 右上角提示离底部的距离
                let tag = format!(" ↓ {scroll} 行 ");
                let tw = text::str_width(&tag) as u16;
                if tw < body.width {
                    let r = Rect { x: body.x + body.width - tw, y: body.y + body.height - 1, width: tw, height: 1 };
                    f.render_widget(Paragraph::new(Line::styled(tag, Style::new().fg(BG).bg(DIM))), r);
                }
            }
        }
        None => {
            let full = Rect { height: title.height + sep.height + body.height, ..title };
            draw_empty(f, full);
        }
    }

    if let Some(lines) = perm {
        let block = WBlock::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(fg(WARN))
            .padding(ratatui::widgets::Padding::horizontal(1));
        f.render_widget(Paragraph::new(lines).block(block), perm_area);
    }

    input::draw(f, input_area, app, &input_layout);

    let (left, right) = match conv {
        Some(c) if c.permission.is_some() => (
            vec![Span::styled("↑↓ 选择 · Enter 确认 · 1–3 直接选 · Esc 拒绝", fg(WARN))],
            vec![Span::styled("其他会话不受影响，继续运行", dim())],
        ),
        Some(c) if c.project.is_none() => (
            vec![Span::styled("Enter 发送 · Alt+Enter 换行 · Esc 中断 · Ctrl+K 切换", dim())],
            vec![Span::styled("不关联项目 · 不显示变更面板", dim())],
        ),
        _ => (vec![Span::styled("Enter 发送 · Alt+Enter 换行 · Esc 中断 · Ctrl+K 切换", dim())], vec![]),
    };
    f.render_widget(Paragraph::new(split_row(left, right, w)), hint);
}

fn draw_title(f: &mut Frame, area: Rect, c: &Conversation) {
    let status = c.status();
    let (sym, color) = status_symbol(status);
    let turns = c.blocks.iter().filter(|b| matches!(b, Block::User(_))).count();
    let mut right = vec![Span::styled(format!("{sym} {}", status_label(status)), fg(color))];
    if let Some(t) = c.turn_started.filter(|_| c.busy) {
        right.push(Span::styled(format!(" · {}s", t.elapsed().as_secs()), dim()));
    }
    if turns > 0 {
        right.push(Span::styled(format!(" · {turns} 轮"), dim()));
    }
    if c.project.is_none() {
        right.push(Span::styled(format!(" · {}", home_short(&c.cwd)), dim()));
    }
    let left = vec![Span::styled(c.title.clone(), bold(fg(FG)))];
    f.render_widget(Paragraph::new(split_row(left, right, area.width as usize)), area);
}

fn draw_empty(f: &mut Frame, area: Rect) {
    let keys: [(&str, &str); 4] = [
        ("Ctrl+N", "新对话"),
        ("Enter", "打开选中的会话"),
        ("Ctrl+K", "搜索、切换会话"),
        ("?", "查看全部快捷键"),
    ];
    let mut lines = vec![
        Line::styled("✻ HeyClaude", bold(accent())).centered(),
        Line::default(),
        Line::styled("在终端里和 Claude Code 对话", dim()).centered(),
        Line::default(),
    ];
    // 键右对齐、说明左对齐，整体居中
    let dw = keys.iter().map(|(_, d)| text::str_width(d)).max().unwrap_or(0);
    for (k, d) in keys {
        let d = format!("{d}{}", " ".repeat(dw - text::str_width(d)));
        lines.push(Line::from(vec![Span::styled(format!("{k:>8}"), accent()), Span::raw("  "), Span::styled(d, fg(FG))]).centered());
    }
    let h = lines.len() as u16;
    let top = area.height.saturating_sub(h) / 2;
    let r = Rect { y: area.y + top, height: area.height.saturating_sub(top), ..area };
    f.render_widget(Paragraph::new(lines), r);
}

/// 对话的全部行（已折好），用于滚动
pub fn conv_lines(c: &Conversation, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut prev_tool = false;
    for b in &c.blocks {
        let is_tool = matches!(b, Block::Tool { .. });
        // 块之间空一行，连续的工具调用挨着
        if !out.is_empty() && !(is_tool && prev_tool) {
            out.push(Line::default());
        }
        prev_tool = is_tool;
        match b {
            Block::User(t) => out.extend(user_lines(t, width)),
            Block::Text(t) => out.extend(text_lines(t, width)),
            Block::Tool { name, input, result, .. } => out.extend(tool_lines(c, name, input, result.as_ref(), width)),
            Block::Notice(t) => {
                let st = dim().add_modifier(Modifier::ITALIC);
                out.extend(wrap_plain(&text::clean(t), width, st, "  "));
            }
        }
    }
    if !c.streaming.is_empty() {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.extend(text_lines(&format!("{}▌", c.streaming), width));
    }
    if c.busy {
        if !out.is_empty() {
            out.push(Line::default());
        }
        let secs = c.turn_started.map(|t| Instant::now().saturating_duration_since(t).as_secs()).unwrap_or(0);
        out.push(Line::from(vec![
            Span::styled("✻ ", accent()),
            Span::styled(format!("思考中… {secs}s · Esc 中断"), dim()),
        ]));
    }
    if let Some(e) = &c.error {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.extend(wrap_plain(&format!("✗ {}", text::clean(e)), width, fg(ERR), ""));
    }
    out
}

fn user_lines(t: &str, width: usize) -> Vec<Line<'static>> {
    let bg = Style::new().bg(BLOCK_BG);
    let st = bg.fg(FG);
    let mut out = Vec::new();
    for (i, para) in text::clean(t).trim_end().split('\n').enumerate() {
        let first = if i == 0 { vec![Span::styled(" › ", bg.fg(DIM))] } else { vec![Span::styled("   ", bg)] };
        out.extend(wrap(vec![Span::styled(para.to_string(), st)], width.saturating_sub(1), first, vec![Span::styled("   ", bg)]));
    }
    for l in &mut out {
        pad(l, width, bg);
    }
    out
}

fn text_lines(t: &str, width: usize) -> Vec<Line<'static>> {
    markdown::render(t, width, fg(FG), vec![Span::styled("● ", accent())], vec![Span::raw("  ")])
}

fn tool_lines(
    c: &Conversation,
    name: &str,
    input: &serde_json::Value,
    result: Option<&crate::model::ToolResult>,
    width: usize,
) -> Vec<Line<'static>> {
    let summary = tool::summary(name, input, &c.cwd);
    let mut head = vec![Span::styled("  ⎿ ", dim()), Span::styled(name.to_string(), bold(fg(FG)))];
    let mut extra: Vec<Span<'static>> = Vec::new();
    let mut body: Vec<Line<'static>> = Vec::new();
    let indent = "      ";

    match result {
        Some(r) if r.is_error => {
            let lines: Vec<&str> = r.text.trim_end().lines().collect();
            for l in lines.iter().take(4) {
                body.extend(wrap_plain(&text::clean(l), width, fg(ERR), indent));
            }
            if lines.len() > 4 {
                body.push(Line::styled(format!("{indent}… 另外 {} 行", lines.len() - 4), dim()));
            }
        }
        _ if tool::is_edit(name) => {
            let (add, del) = tool::diff_stats(input);
            extra.push(Span::styled(format!("+{add}"), fg(OK)));
            extra.push(Span::raw(" "));
            extra.push(Span::styled(format!("−{del}"), fg(ERR)));
            body = tool::diff_view(input, indent.len(), width, DIFF_MAX);
        }
        Some(r) if name == "Bash" => {
            let out = r.text.trim_end();
            if out.is_empty() {
                extra.push(Span::styled("(无输出)", dim()));
            } else {
                let lines: Vec<&str> = out.lines().collect();
                for l in lines.iter().take(BASH_PREVIEW) {
                    let s = truncate(&text::clean(l), width.saturating_sub(indent.len()));
                    body.push(Line::styled(format!("{indent}{s}"), dim()));
                }
                if lines.len() > BASH_PREVIEW {
                    body.push(Line::styled(format!("{indent}… 另外 {} 行", lines.len() - BASH_PREVIEW), dim()));
                }
            }
        }
        Some(_) if name == "Write" => {
            let n = input.get("content").and_then(|v| v.as_str()).map_or(0, tool::line_count);
            extra.push(Span::styled(format!("({n} 行)"), dim()));
        }
        Some(r) => {
            let n = tool::line_count(&r.text);
            if n > 0 {
                extra.push(Span::styled(format!("({n} 行)"), dim()));
            }
        }
        None => {}
    }

    // 头一行：名字 + 摘要 + 额外信息，摘要过长截断
    let extra_w = text::spans_width(&extra);
    let head_w = text::spans_width(&head);
    let room = width.saturating_sub(head_w + 2 + if extra_w > 0 { extra_w + 2 } else { 0 });
    if !summary.is_empty() {
        head.push(Span::raw("  "));
        head.push(Span::styled(truncate(&text::clean(&summary), room), fg(FG)));
    }
    if extra_w > 0 {
        head.push(Span::raw("  "));
        head.extend(extra);
    }
    let mut out = vec![Line::from(text::truncate_spans(head, width))];
    out.extend(body);
    out
}

/// 权限确认框里的内容（不含边框）。width 是框内宽度
fn permission_lines(app: &App, c: &Conversation, p: &PermissionRequest, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::styled(tool::permission_title(&p.tool_name), bold(fg(WARN)))];
    let bg = Style::new().bg(BLOCK_BG);
    // 命令全文或文件路径
    let detail = match p.tool_name.as_str() {
        "Bash" => p.input.get("command").and_then(|v| v.as_str()).map(str::to_string),
        _ => None,
    }
    .unwrap_or_else(|| tool::summary(&p.tool_name, &p.input, &c.cwd));
    let mut detail_lines: Vec<Line<'static>> = Vec::new();
    for l in text::clean(&detail).split('\n') {
        detail_lines.extend(wrap(vec![Span::styled(l.to_string(), bg.fg(FG))], width, vec![Span::styled(" ", bg)], vec![Span::styled(" ", bg)]));
    }
    if detail_lines.len() > 6 {
        let more = detail_lines.len() - 5;
        detail_lines.truncate(5);
        detail_lines.push(Line::styled(format!(" … 另外 {more} 行"), bg.fg(DIM)));
    }
    for mut l in detail_lines {
        pad(&mut l, width, bg);
        out.push(l);
    }
    if tool::is_edit(&p.tool_name) {
        out.extend(tool::diff_view(&p.input, 0, width + 1, 6));
    }
    let mut info = format!("工作目录 {}", home_short(&c.cwd));
    if let Some(d) = &p.description {
        info.push_str(" · ");
        info.push_str(d);
    }
    out.extend(wrap_plain(&text::clean(&info), width, dim(), ""));
    out.push(Line::default());
    let options = ["1. 允许一次", "2. 允许，本会话不再询问", "3. 拒绝"];
    for (i, o) in options.iter().enumerate() {
        let mut l = if i == app.permission_choice {
            Line::from(vec![Span::styled(" › ", Style::new().fg(ACCENT).bg(SEL_BG)), Span::styled(*o, Style::new().fg(FG).bg(SEL_BG))])
        } else {
            Line::from(vec![Span::raw("   "), Span::styled(*o, fg(FG))])
        };
        if i == app.permission_choice {
            pad(&mut l, width, Style::new().bg(SEL_BG));
        }
        out.push(l);
    }
    out
}
