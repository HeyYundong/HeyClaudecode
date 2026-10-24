//! 应用状态。ui/ 只读这里的字段来画界面，按键和事件处理也在这里。

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::layout::{Position, Rect};

use crate::claude::process::ClaudeProcess;
use crate::model::{ConvId, Conversation, FileChange, Project, SessionInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Input,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTab {
    Changes,
    Todos,
}

/// 侧栏从上到下的可选条目
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarItem {
    NewChat,
    /// app.chats[i]：不关联项目的对话
    Chat(usize),
    /// app.projects[i]
    Project(usize),
    /// app.sessions[i]：当前展开项目的会话
    Session(usize),
}

#[derive(Debug, Clone, Default)]
pub struct InputState {
    pub text: String,
    /// 光标位置，按字符（char）计
    pub cursor: usize,
}

/// 鼠标能点的区域，ui 每次绘制时登记
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Sidebar,
    /// sidebar_items() 的下标
    SidebarItem(usize),
    Chat,
    Input,
    Panel,
    PanelTab(PanelTab),
    /// 权限确认框的第几个选项
    Permission(usize),
    /// 弹窗框内（点框外关闭弹窗）
    OverlayBox,
    /// switcher_matches() 的下标
    SwitcherItem(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchTarget {
    Chat(usize),
    Project(usize),
    Session { project: usize, session_id: String },
}

/// Ctrl+K 里的一条候选
#[derive(Debug, Clone)]
pub struct SwitchEntry {
    pub label: String,
    /// 分组标题：项目名，或者 "对话"
    pub group: String,
    pub status: crate::model::SessionStatus,
    pub updated: std::time::SystemTime,
    pub target: SwitchTarget,
}

#[derive(Debug, Clone)]
pub enum Overlay {
    /// Ctrl+K 快速切换：query 过滤 app.switcher_items()
    Switcher { query: String, selected: usize },
    /// 添加项目，输入目录路径
    AddProject { input: String },
    Help,
}

pub struct App {
    pub projects: Vec<Project>,
    /// 不关联项目的对话，新的在前
    pub chats: Vec<SessionInfo>,
    /// 当前展开的项目（app.projects 下标）
    pub expanded: Option<usize>,
    /// 展开项目的会话，新的在前
    pub sessions: Vec<SessionInfo>,
    /// 所有项目和对话的会话，供 Ctrl+K 搜索
    pub switch_index: Vec<SwitchEntry>,
    pub sidebar_selected: usize,
    pub focus: Focus,
    pub sidebar_visible: bool,
    pub panel_visible: bool,
    pub panel_tab: PanelTab,
    /// 当前对话所在目录的 git 变更
    pub changes: Vec<FileChange>,
    pub convs: HashMap<ConvId, Conversation>,
    pub procs: HashMap<ConvId, ClaudeProcess>,
    /// 中间区域显示的对话
    pub current: Option<ConvId>,
    pub input: InputState,
    /// 对话区离底部滚了多少行，0 表示跟随最新
    pub chat_scroll: u16,
    pub overlay: Option<Overlay>,
    /// 权限确认框里选中的选项 0..3
    pub permission_choice: usize,
    /// 状态栏左侧的临时提示（出错、已复制等）
    pub flash: Option<String>,
    /// 传给每个 claude 进程的额外参数（hccd 命令行里多余的参数）
    pub claude_args: Vec<String>,
    pub next_conv: ConvId,
    pub should_quit: bool,
    /// 上一帧登记的可点击区域，后登记的在上层
    pub hits: RefCell<Vec<(Rect, Hit)>>,
}

impl App {
    /// 侧栏条目：新对话、所有对话、所有项目，展开项目的会话紧跟在该项目后面
    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = vec![SidebarItem::NewChat];
        items.extend((0..self.chats.len()).map(SidebarItem::Chat));
        for i in 0..self.projects.len() {
            items.push(SidebarItem::Project(i));
            if self.expanded == Some(i) {
                items.extend((0..self.sessions.len()).map(SidebarItem::Session));
            }
        }
        items
    }

    pub fn current_conv(&self) -> Option<&Conversation> {
        self.current.and_then(|id| self.convs.get(&id))
    }

    pub fn add_hit(&self, area: Rect, hit: Hit) {
        if area.width > 0 && area.height > 0 {
            self.hits.borrow_mut().push((area, hit));
        }
    }

    /// 坐标处最上层的可点击区域
    pub fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.hits.borrow().iter().rev().find(|(r, _)| r.contains(Position { x, y })).map(|(_, h)| *h)
    }

    /// 按标题或分组做不区分大小写的子串匹配，最多 50 条。
    /// 同组的排在一起：组按其中最新一条的时间排，组内新的在前
    pub fn switcher_matches(&self, query: &str) -> Vec<&SwitchEntry> {
        let q = query.to_lowercase();
        let mut hits: Vec<&SwitchEntry> = self
            .switch_index
            .iter()
            .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q) || e.group.to_lowercase().contains(&q))
            .collect();
        let mut newest: HashMap<&str, std::time::SystemTime> = HashMap::new();
        for e in &hits {
            let t = newest.entry(e.group.as_str()).or_insert(e.updated);
            *t = (*t).max(e.updated);
        }
        hits.sort_by(|a, b| {
            newest[b.group.as_str()]
                .cmp(&newest[a.group.as_str()])
                .then_with(|| a.group.cmp(&b.group))
                .then_with(|| b.updated.cmp(&a.updated))
        });
        hits.truncate(50);
        hits
    }
}
