//! 画界面。只读 App，不改状态。

mod chat;
mod input;
mod markdown;
mod overlay;
mod panel;
mod sidebar;
mod status;
mod text;
mod theme;
mod tool;

#[cfg(test)]
mod tests;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block as WBlock;

use crate::app::App;
use theme::{BG, FG};

/// 侧栏宽度
const SIDEBAR_W: u16 = 32;
/// 右侧面板宽度
const PANEL_W: u16 = 40;
/// 终端宽度低于这个值就不画面板 / 侧栏
const PANEL_MIN_TERM_W: u16 = 140;
const SIDEBAR_MIN_TERM_W: u16 = 90;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(WBlock::new().style(Style::new().bg(BG).fg(FG)), area);
    if area.width == 0 || area.height == 0 {
        return;
    }

    let [top, mid, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    status::draw_top(f, top, app);
    status::draw_bottom(f, bottom, app);

    let conv = app.current_conv();
    let show_sidebar = app.sidebar_visible && area.width >= SIDEBAR_MIN_TERM_W;
    let panel_conv = conv.filter(|c| app.panel_visible && area.width >= PANEL_MIN_TERM_W && c.project.is_some());

    let mut x = mid.x;
    let mut w = mid.width;
    if show_sidebar {
        sidebar::draw(f, Rect { width: SIDEBAR_W, ..mid }, app);
        x += SIDEBAR_W;
        w -= SIDEBAR_W;
    }
    if let Some(c) = panel_conv {
        w -= PANEL_W;
        panel::draw(f, Rect { x: x + w, width: PANEL_W, ..mid }, app, c);
    }
    chat::draw(f, Rect { x, width: w, ..mid }, app);

    if let Some(ov) = &app.overlay {
        overlay::draw(f, area, app, ov);
    }
}
