//! 按键和后台事件的处理：改 App 状态，必要时启动 claude 进程。

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::{App, Focus, Hit, InputState, Overlay, PanelTab, SidebarItem, SwitchEntry, SwitchTarget};
use crate::claude::process::{ClaudeProcess, SpawnOptions};
use crate::claude::protocol;
use crate::data::{git, sessions, store};
use crate::model::{AppEvent, Block, ConvId, Conversation, PermissionDecision, SessionInfo, SessionStatus};

type Tx = UnboundedSender<AppEvent>;

const CONFIRM_REMOVE: &str = "再按一次 d 从列表移除这个项目（不会删除目录）";
const TITLE_CHARS: usize = 40;

impl App {
    pub fn new(claude_args: Vec<String>) -> Self {
        let (projects, flash) = match store::load_projects() {
            Ok(p) => (p, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let mut app = App {
            projects,
            chats: Vec::new(),
            expanded: None,
            sessions: Vec::new(),
            switch_index: Vec::new(),
            sidebar_selected: 0,
            focus: Focus::Sidebar,
            sidebar_visible: true,
            panel_visible: true,
            panel_tab: PanelTab::Changes,
            changes: Vec::new(),
            convs: Default::default(),
            procs: Default::default(),
            current: None,
            input: InputState::default(),
            chat_scroll: 0,
            overlay: None,
            permission_choice: 0,
            flash,
            claude_args,
            next_conv: 1,
            should_quit: false,
            hits: Default::default(),
        };
        app.refresh();
        app
    }

    // ───────────── 列表刷新 ─────────────

    /// 重新读对话列表、展开项目的会话和当前目录的 git 变更，尽量保持侧栏选中同一条
    pub fn refresh(&mut self) {
        let keep = self.selected_key();

        self.chats = sessions::list_chats();
        self.merge_live(None);
        if let Some(i) = self.expanded {
            match self.projects.get(i).map(|p| p.path.clone()) {
                Some(path) => {
                    self.sessions = sessions::list_sessions(&path, true);
                    self.merge_live(Some(&path));
                }
                None => {
                    self.expanded = None;
                    self.sessions.clear();
                }
            }
        }
        self.refresh_changes();
        // 侧栏里未展开项目的状态计数也来自这个索引
        self.build_switch_index();

        if let Some(key) = keep
            && let Some(pos) = self.sidebar_items().iter().position(|it| self.item_key(*it).as_deref() == Some(key.as_str()))
        {
            self.sidebar_selected = pos;
        }
        self.clamp_selection();
    }

    fn refresh_changes(&mut self) {
        self.changes = match self.current_conv() {
            Some(c) if c.project.is_some() => git::changed_files(&c.cwd),
            _ => Vec::new(),
        };
    }

    /// 把 hccd 正在托管的对话合并进列表：已落盘的标上 conv 和实时状态，还没落盘的补在最前面。
    /// project 为 None 时合并到 chats，否则合并到 sessions
    fn merge_live(&mut self, project: Option<&Path>) {
        let list = if project.is_some() { &mut self.sessions } else { &mut self.chats };
        for conv in self.convs.values() {
            if conv.project.as_deref() != project {
                continue;
            }
            let found = conv.session_id.as_ref().and_then(|sid| list.iter_mut().find(|s| &s.session_id == sid));
            match found {
                Some(info) => {
                    info.conv = Some(conv.id);
                    info.status = conv.status();
                }
                // 只列出发过消息的新对话
                None if !conv.blocks.is_empty() => list.push(SessionInfo {
                    session_id: conv.session_id.clone().unwrap_or_default(),
                    title: conv.title.clone(),
                    cwd: conv.cwd.clone(),
                    transcript: None,
                    updated: SystemTime::now(),
                    status: conv.status(),
                    conv: Some(conv.id),
                }),
                None => {}
            }
        }
        if project.is_some() {
            // 按状态分组，组内新的在前
            list.sort_by(|a, b| status_rank(a.status).cmp(&status_rank(b.status)).then(b.updated.cmp(&a.updated)));
        } else {
            list.sort_by_key(|s| std::cmp::Reverse(s.updated));
        }
    }

    /// 侧栏条目的稳定标识，刷新后据此找回选中行
    fn item_key(&self, item: SidebarItem) -> Option<String> {
        match item {
            SidebarItem::NewChat => Some("new".into()),
            SidebarItem::Chat(i) => self.chats.get(i).map(session_key),
            SidebarItem::Project(i) => self.projects.get(i).map(|p| format!("p:{}", p.path.display())),
            SidebarItem::Session(i) => self.sessions.get(i).map(session_key),
        }
    }

    fn selected_key(&self) -> Option<String> {
        self.sidebar_items().get(self.sidebar_selected).and_then(|it| self.item_key(*it))
    }

    fn clamp_selection(&mut self) {
        let n = self.sidebar_items().len();
        if self.sidebar_selected >= n {
            self.sidebar_selected = n.saturating_sub(1);
        }
    }

    // ───────────── 后台事件 ─────────────

    pub fn on_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Claude { conv, msg } => {
                let Some(c) = self.convs.get_mut(&conv) else { return };
                let had_session = c.session_id.is_some();
                let had_permission = c.permission.is_some();
                let was_busy = c.busy;
                protocol::apply(c, &msg);
                if !had_permission && c.permission.is_some() && self.current == Some(conv) {
                    self.permission_choice = 0;
                }
                // 拿到 session_id 或一轮结束时，列表里的状态和变更都可能变了
                if (!had_session && c.session_id.is_some()) || (was_busy && !c.busy) || had_permission != c.permission.is_some() {
                    self.refresh();
                }
            }
            AppEvent::ClaudeExit { conv, code, stderr } => {
                self.procs.remove(&conv);
                if let Some(c) = self.convs.get_mut(&conv) {
                    let was_busy = c.busy;
                    c.busy = false;
                    c.turn_started = None;
                    c.permission = None;
                    if code != Some(0) || was_busy {
                        let tail = stderr.trim();
                        let reason = if tail.is_empty() { String::new() } else { format!("：{}", last_line(tail)) };
                        let msg = format!("claude 进程退出（{}）{}", code.map_or("被终止".into(), |c| format!("代码 {c}")), reason);
                        c.error = Some(msg.clone());
                        c.blocks.push(Block::Notice(msg));
                    }
                }
                self.refresh();
            }
        }
    }

    // ───────────── 终端事件 ─────────────

    pub fn on_term(&mut self, ev: Event, tx: &Tx) {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key, tx),
            Event::Paste(text) => self.on_paste(&text),
            Event::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    // ───────────── 鼠标 ─────────────

    fn on_mouse(&mut self, m: MouseEvent) {
        let hit = self.hit_at(m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollUp => self.on_wheel(hit, true),
            MouseEventKind::ScrollDown => self.on_wheel(hit, false),
            MouseEventKind::Down(MouseButton::Left) => self.on_click(hit),
            _ => {}
        }
    }

    /// 滚轮：弹窗里移动选中项，侧栏上移动选中行，其他地方滚动对话
    fn on_wheel(&mut self, hit: Option<Hit>, up: bool) {
        match &mut self.overlay {
            Some(Overlay::Switcher { query, selected }) => {
                let n = self.switch_index.len();
                *selected = if up { selected.saturating_sub(1) } else { (*selected + 1).min(n) };
                let (q, sel) = (query.clone(), *selected);
                let max = self.switcher_matches(&q).len().saturating_sub(1);
                if let Some(Overlay::Switcher { selected, .. }) = self.overlay.as_mut() {
                    *selected = sel.min(max);
                }
            }
            Some(_) => {}
            None => match hit {
                Some(Hit::Sidebar | Hit::SidebarItem(_)) => {
                    let last = self.sidebar_items().len().saturating_sub(1);
                    self.sidebar_selected =
                        if up { self.sidebar_selected.saturating_sub(1) } else { (self.sidebar_selected + 1).min(last) };
                }
                _ => self.scroll(if up { 3 } else { -3 }),
            },
        }
    }

    fn on_click(&mut self, hit: Option<Hit>) {
        if self.overlay.is_some() {
            match hit {
                Some(Hit::SwitcherItem(i)) => self.open_switcher_match(i),
                Some(Hit::OverlayBox) => {}
                // 点弹窗外面关闭
                _ => self.overlay = None,
            }
            return;
        }
        match hit {
            Some(Hit::SidebarItem(i)) => {
                self.sidebar_selected = i;
                self.focus = Focus::Sidebar;
                if let Some(item) = self.sidebar_items().get(i).copied() {
                    self.activate(item);
                }
            }
            Some(Hit::Sidebar) => self.focus = Focus::Sidebar,
            Some(Hit::Permission(i)) => {
                self.permission_choice = i;
                let decision = match i {
                    0 => PermissionDecision::AllowOnce,
                    1 => PermissionDecision::AllowAlways,
                    _ => PermissionDecision::Deny,
                };
                self.answer_permission(decision);
            }
            Some(Hit::Chat | Hit::Input) => self.focus = Focus::Input,
            Some(Hit::PanelTab(tab)) => {
                self.panel_tab = tab;
                self.focus = Focus::Panel;
            }
            Some(Hit::Panel) => self.focus = Focus::Panel,
            Some(Hit::OverlayBox | Hit::SwitcherItem(_)) | None => {}
        }
    }

    fn on_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        match &mut self.overlay {
            Some(Overlay::Switcher { query, selected }) => {
                query.push_str(text.lines().next().unwrap_or(""));
                *selected = 0;
            }
            Some(Overlay::AddProject { input }) => input.push_str(text.trim()),
            Some(Overlay::Help) => {}
            None => {
                self.focus = Focus::Input;
                for ch in text.chars() {
                    self.input_insert(ch);
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent, tx: &Tx) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // 任何按键都清掉临时提示；移除项目的确认提示要留给紧接着的第二次 d
        let confirming = self.flash.as_deref() == Some(CONFIRM_REMOVE) && key.code == KeyCode::Char('d') && !ctrl;
        if !confirming {
            self.flash = None;
        }

        // 全局快捷键
        if ctrl {
            match key.code {
                KeyCode::Char('c') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('n') => {
                    self.overlay = None;
                    self.new_conversation(None);
                    return;
                }
                KeyCode::Char('k') => {
                    self.open_switcher();
                    return;
                }
                KeyCode::Char('b') => {
                    self.sidebar_visible = !self.sidebar_visible;
                    if !self.sidebar_visible && self.focus == Focus::Sidebar {
                        self.focus = Focus::Input;
                    }
                    return;
                }
                KeyCode::Char('p') => {
                    self.panel_visible = !self.panel_visible;
                    if !self.panel_visible && self.focus == Focus::Panel {
                        self.focus = Focus::Input;
                    }
                    return;
                }
                KeyCode::Char('t') => {
                    self.panel_tab = match self.panel_tab {
                        PanelTab::Changes => PanelTab::Todos,
                        PanelTab::Todos => PanelTab::Changes,
                    };
                    return;
                }
                _ => {}
            }
        }

        if self.overlay.is_some() {
            self.on_overlay_key(key, tx);
            return;
        }

        match key.code {
            KeyCode::PageUp => return self.scroll(10),
            KeyCode::PageDown => return self.scroll(-10),
            _ => {}
        }

        // 当前对话在等权限确认：先处理确认框
        if self.current_conv().is_some_and(|c| c.permission.is_some())
            && self.focus != Focus::Sidebar
            && self.on_permission_key(key)
        {
            return;
        }

        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            self.cycle_focus(key.code == KeyCode::BackTab);
            return;
        }

        match self.focus {
            Focus::Sidebar => self.on_sidebar_key(key),
            Focus::Panel => self.on_panel_key(key),
            Focus::Input => self.on_input_key(key, ctrl, alt, tx),
        }
    }

    fn cycle_focus(&mut self, back: bool) {
        let mut order = Vec::new();
        if self.sidebar_visible {
            order.push(Focus::Sidebar);
        }
        order.push(Focus::Input);
        if self.panel_visible && self.current_conv().is_some_and(|c| c.project.is_some()) {
            order.push(Focus::Panel);
        }
        let pos = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = if back { (pos + order.len() - 1) % order.len() } else { (pos + 1) % order.len() };
        self.focus = order[next];
    }

    fn scroll(&mut self, delta: i32) {
        self.chat_scroll = (self.chat_scroll as i32 + delta).max(0) as u16;
    }

    /// 返回 true 表示按键被确认框用掉了
    fn on_permission_key(&mut self, key: KeyEvent) -> bool {
        let decision = match key.code {
            KeyCode::Up | KeyCode::Char('k') if self.focus != Focus::Input || self.input.text.is_empty() => {
                self.permission_choice = self.permission_choice.saturating_sub(1);
                return true;
            }
            KeyCode::Down | KeyCode::Char('j') if self.focus != Focus::Input || self.input.text.is_empty() => {
                self.permission_choice = (self.permission_choice + 1).min(2);
                return true;
            }
            KeyCode::Char('1') if self.input.text.is_empty() => PermissionDecision::AllowOnce,
            KeyCode::Char('2') if self.input.text.is_empty() => PermissionDecision::AllowAlways,
            KeyCode::Char('3') if self.input.text.is_empty() => PermissionDecision::Deny,
            KeyCode::Enter if self.input.text.is_empty() => match self.permission_choice {
                0 => PermissionDecision::AllowOnce,
                1 => PermissionDecision::AllowAlways,
                _ => PermissionDecision::Deny,
            },
            KeyCode::Esc => PermissionDecision::Deny,
            _ => return false,
        };
        self.answer_permission(decision);
        true
    }

    fn answer_permission(&mut self, decision: PermissionDecision) {
        let Some(id) = self.current else { return };
        let Some(conv) = self.convs.get_mut(&id) else { return };
        let Some(req) = conv.permission.take() else { return };
        if let Some(proc) = self.procs.get(&id) {
            proc.answer_permission(&req, decision);
        }
        if decision == PermissionDecision::Deny {
            conv.blocks.push(Block::Notice(format!("已拒绝 {}", req.tool_name)));
        }
        self.permission_choice = 0;
        self.refresh();
    }

    // ───────────── 侧栏 ─────────────

    fn on_sidebar_key(&mut self, key: KeyEvent) {
        let items = self.sidebar_items();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.sidebar_selected = self.sidebar_selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.sidebar_selected = (self.sidebar_selected + 1).min(items.len().saturating_sub(1))
            }
            KeyCode::Home | KeyCode::Char('g') => self.sidebar_selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.sidebar_selected = items.len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(item) = items.get(self.sidebar_selected).copied() {
                    self.activate(item);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                // 收起所在项目并选中项目行
                if let Some(i) = self.expanded {
                    self.expanded = None;
                    self.sessions.clear();
                    if let Some(pos) = self.sidebar_items().iter().position(|it| *it == SidebarItem::Project(i)) {
                        self.sidebar_selected = pos;
                    }
                }
            }
            KeyCode::Char('n') => {
                let project = match items.get(self.sidebar_selected) {
                    Some(SidebarItem::Project(i)) => Some(*i),
                    Some(SidebarItem::Session(_)) => self.expanded,
                    _ => None,
                };
                self.new_conversation(project);
            }
            KeyCode::Char('a') => self.overlay = Some(Overlay::AddProject { input: String::new() }),
            KeyCode::Char('d') => {
                if let Some(SidebarItem::Project(i)) = items.get(self.sidebar_selected) {
                    self.remove_project(*i);
                }
            }
            KeyCode::Char('/') => self.open_switcher(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Char('i') => self.focus = Focus::Input,
            KeyCode::Esc => self.interrupt_current(),
            _ => {}
        }
    }

    fn activate(&mut self, item: SidebarItem) {
        match item {
            SidebarItem::NewChat => self.new_conversation(None),
            SidebarItem::Chat(i) => {
                if let Some(info) = self.chats.get(i).cloned() {
                    self.open_session(&info, None);
                }
            }
            SidebarItem::Project(i) => self.toggle_project(i),
            SidebarItem::Session(i) => {
                let project = self.expanded.and_then(|p| self.projects.get(p)).map(|p| p.path.clone());
                if let Some(info) = self.sessions.get(i).cloned() {
                    self.open_session(&info, project);
                }
            }
        }
    }

    fn toggle_project(&mut self, i: usize) {
        if self.expanded == Some(i) {
            self.expanded = None;
            self.sessions.clear();
        } else {
            self.expanded = Some(i);
            self.sessions.clear();
            self.refresh();
            if let Some(pos) = self.sidebar_items().iter().position(|it| *it == SidebarItem::Project(i)) {
                self.sidebar_selected = pos;
            }
        }
    }

    fn remove_project(&mut self, i: usize) {
        if self.flash.as_deref() != Some(CONFIRM_REMOVE) {
            self.flash = Some(CONFIRM_REMOVE.into());
            return;
        }
        self.flash = None;
        let Some(path) = self.projects.get(i).map(|p| p.path.clone()) else { return };
        match store::remove_project(&self.projects, &path) {
            Ok(list) => {
                self.projects = list;
                self.expanded = None;
                self.sessions.clear();
                self.refresh();
            }
            Err(e) => self.flash = Some(e.to_string()),
        }
    }

    fn on_panel_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                self.panel_tab = match self.panel_tab {
                    PanelTab::Changes => PanelTab::Todos,
                    PanelTab::Todos => PanelTab::Changes,
                }
            }
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Esc => self.interrupt_current(),
            _ => {}
        }
    }

    // ───────────── 对话 ─────────────

    fn alloc_conv(&mut self, cwd: PathBuf, project: Option<PathBuf>) -> ConvId {
        let id = self.next_conv;
        self.next_conv += 1;
        self.convs.insert(id, Conversation::new(id, cwd, project));
        id
    }

    /// 新建对话：project 为 None 时在临时目录里开一个不关联项目的对话
    fn new_conversation(&mut self, project: Option<usize>) {
        let (cwd, project_path) = match project.and_then(|i| self.projects.get(i)) {
            Some(p) => (p.path.clone(), Some(p.path.clone())),
            None => match store::new_chat_dir() {
                Ok(dir) => (dir, None),
                Err(e) => {
                    self.flash = Some(format!("无法创建对话目录：{e}"));
                    return;
                }
            },
        };
        let id = self.alloc_conv(cwd, project_path);
        self.show(id);
    }

    fn show(&mut self, id: ConvId) {
        self.current = Some(id);
        self.chat_scroll = 0;
        self.permission_choice = 0;
        self.focus = Focus::Input;
        self.refresh_changes();
    }

    fn open_session(&mut self, info: &SessionInfo, project: Option<PathBuf>) {
        // 已经打开过的直接切过去
        let existing = info.conv.filter(|id| self.convs.contains_key(id)).or_else(|| {
            self.convs
                .values()
                .find(|c| !info.session_id.is_empty() && c.session_id.as_deref() == Some(info.session_id.as_str()))
                .map(|c| c.id)
        });
        if let Some(id) = existing {
            self.show(id);
            return;
        }
        let id = self.alloc_conv(info.cwd.clone(), project);
        let conv = self.convs.get_mut(&id).expect("刚插入");
        conv.session_id = Some(info.session_id.clone()).filter(|s| !s.is_empty());
        conv.title = info.title.clone();
        if let Some(path) = &info.transcript {
            match sessions::load_transcript(path) {
                Ok(t) => {
                    conv.blocks = t.blocks;
                    conv.todos = t.todos;
                    if !t.title.is_empty() {
                        conv.title = t.title;
                    }
                }
                Err(e) => conv.blocks.push(Block::Notice(format!("读取会话记录失败：{e}"))),
            }
        }
        self.show(id);
        self.refresh();
    }

    fn on_input_key(&mut self, key: KeyEvent, ctrl: bool, alt: bool, tx: &Tx) {
        let input = &mut self.input;
        let len = input.text.chars().count();
        match key.code {
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => self.input_insert('\n'),
            KeyCode::Char('j') if ctrl => self.input_insert('\n'),
            KeyCode::Enter => self.send(tx),
            KeyCode::Esc => {
                if self.current_conv().is_some_and(|c| c.busy) {
                    self.interrupt_current();
                } else if self.sidebar_visible {
                    self.focus = Focus::Sidebar;
                }
            }
            KeyCode::Char('u') if ctrl => {
                let byte = char_to_byte(&input.text, input.cursor);
                input.text.replace_range(..byte, "");
                input.cursor = 0;
            }
            KeyCode::Char('a') if ctrl => input.cursor = 0,
            KeyCode::Char('e') if ctrl => input.cursor = len,
            KeyCode::Char(ch) if !ctrl => self.input_insert(ch),
            KeyCode::Backspace => {
                if input.cursor > 0 {
                    let start = char_to_byte(&input.text, input.cursor - 1);
                    let end = char_to_byte(&input.text, input.cursor);
                    input.text.replace_range(start..end, "");
                    input.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if input.cursor < len {
                    let start = char_to_byte(&input.text, input.cursor);
                    let end = char_to_byte(&input.text, input.cursor + 1);
                    input.text.replace_range(start..end, "");
                }
            }
            KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
            KeyCode::Right => input.cursor = (input.cursor + 1).min(len),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = len,
            KeyCode::Up if input.text.is_empty() => self.scroll(1),
            KeyCode::Down if input.text.is_empty() => self.scroll(-1),
            _ => {}
        }
    }

    fn input_insert(&mut self, ch: char) {
        let byte = char_to_byte(&self.input.text, self.input.cursor);
        self.input.text.insert(byte, ch);
        self.input.cursor += 1;
    }

    fn send(&mut self, tx: &Tx) {
        let text = self.input.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.current.is_none() {
            self.new_conversation(None);
        }
        let Some(id) = self.current else { return };

        if !self.procs.contains_key(&id) {
            let conv = &self.convs[&id];
            let opts = SpawnOptions { resume: conv.session_id.clone(), extra_args: self.claude_args.clone() };
            match ClaudeProcess::spawn(id, &conv.cwd, &opts, tx.clone()) {
                Ok(p) => {
                    self.procs.insert(id, p);
                }
                Err(e) => {
                    self.flash = Some(format!("无法启动 claude：{e}"));
                    return;
                }
            }
        }

        let conv = self.convs.get_mut(&id).expect("当前对话存在");
        conv.blocks.push(Block::User(text.clone()));
        if conv.title == "新对话" {
            conv.title = truncate(text.lines().next().unwrap_or(&text), TITLE_CHARS);
        }
        conv.busy = true;
        conv.error = None;
        conv.turn_started.get_or_insert_with(Instant::now);
        self.procs[&id].send_user(&text);

        self.input = InputState::default();
        self.chat_scroll = 0;
        self.refresh();
        // 新对话第一次出现在侧栏时选中它
        let pos = self.sidebar_items().iter().position(|it| match *it {
            SidebarItem::Chat(i) => self.chats[i].conv == Some(id),
            SidebarItem::Session(i) => self.sessions[i].conv == Some(id),
            _ => false,
        });
        if let Some(pos) = pos {
            self.sidebar_selected = pos;
        }
    }

    fn interrupt_current(&mut self) {
        let Some(id) = self.current else { return };
        if !self.convs.get(&id).is_some_and(|c| c.busy) {
            return;
        }
        // 中断后 claude 会发一条 aborted 的 result，protocol 据此加“已中断”提示
        if let Some(proc) = self.procs.get(&id) {
            proc.interrupt();
        }
    }

    // ───────────── 覆盖层 ─────────────

    fn open_switcher(&mut self) {
        self.build_switch_index();
        self.overlay = Some(Overlay::Switcher { query: String::new(), selected: 0 });
    }

    /// 读所有项目的会话，建 Ctrl+K 的搜索索引
    fn build_switch_index(&mut self) {
        let mut index = Vec::new();
        for (i, info) in self.chats.iter().enumerate() {
            index.push(SwitchEntry {
                label: info.title.clone(),
                group: "对话".into(),
                status: info.status,
                updated: info.updated,
                target: SwitchTarget::Chat(i),
            });
        }
        for (i, p) in self.projects.iter().enumerate() {
            let list = if self.expanded == Some(i) { self.sessions.clone() } else { sessions::list_sessions(&p.path, true) };
            let newest = list.iter().map(|s| s.updated).max().unwrap_or(SystemTime::UNIX_EPOCH);
            index.push(SwitchEntry {
                label: p.name.clone(),
                group: "项目".into(),
                status: SessionStatus::Idle,
                updated: newest,
                target: SwitchTarget::Project(i),
            });
            for s in list {
                let status = s.conv.and_then(|id| self.convs.get(&id)).map_or(s.status, |c| c.status());
                index.push(SwitchEntry {
                    label: s.title.clone(),
                    group: p.name.clone(),
                    status,
                    updated: s.updated,
                    target: SwitchTarget::Session { project: i, session_id: s.session_id.clone() },
                });
            }
        }
        self.switch_index = index;
    }

    fn on_overlay_key(&mut self, key: KeyEvent, _tx: &Tx) {
        let Some(overlay) = self.overlay.as_mut() else { return };
        match overlay {
            Overlay::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.overlay = None;
                }
            }
            Overlay::AddProject { input } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let raw = input.clone();
                    match store::add_project(&self.projects, &raw) {
                        Ok(list) => {
                            let added = list.iter().find(|p| !self.projects.contains(p)).map(|p| p.path.clone());
                            self.projects = list;
                            self.overlay = None;
                            if let Some(i) = self.projects.iter().position(|p| Some(&p.path) == added.as_ref()) {
                                self.expanded = None;
                                self.toggle_project(i);
                            }
                            self.focus = Focus::Sidebar;
                        }
                        Err(e) => self.flash = Some(e.to_string()),
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => input.push(ch),
                _ => {}
            },
            Overlay::Switcher { query, selected } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => *selected += 1,
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => *selected = selected.saturating_sub(1),
                KeyCode::Backspace => {
                    query.pop();
                    *selected = 0;
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    query.push(ch);
                    *selected = 0;
                }
                KeyCode::Enter => {
                    let sel = *selected;
                    self.open_switcher_match(sel);
                }
                _ => {}
            },
        }
        // 选中项不超过结果数
        if let Some(Overlay::Switcher { query, selected }) = &self.overlay {
            let n = self.switcher_matches(query).len();
            let clamped = (*selected).min(n.saturating_sub(1));
            if let Some(Overlay::Switcher { selected, .. }) = self.overlay.as_mut() {
                *selected = clamped;
            }
        }
    }

    /// 关掉 Ctrl+K 并打开第 i 条结果
    fn open_switcher_match(&mut self, i: usize) {
        let Some(Overlay::Switcher { query, .. }) = &self.overlay else { return };
        let target = {
            let hits = self.switcher_matches(query);
            hits.get(i.min(hits.len().saturating_sub(1))).map(|e| e.target.clone())
        };
        self.overlay = None;
        if let Some(t) = target {
            self.jump(t);
        }
    }

    fn jump(&mut self, target: SwitchTarget) {
        match target {
            SwitchTarget::Chat(i) => {
                if let Some(info) = self.chats.get(i).cloned() {
                    self.select_item(SidebarItem::Chat(i));
                    self.open_session(&info, None);
                }
            }
            SwitchTarget::Project(i) => {
                if self.expanded != Some(i) {
                    self.toggle_project(i);
                }
                self.select_item(SidebarItem::Project(i));
                self.focus = Focus::Sidebar;
            }
            SwitchTarget::Session { project, session_id } => {
                if self.expanded != Some(project) {
                    self.toggle_project(project);
                }
                if let Some(i) = self.sessions.iter().position(|s| s.session_id == session_id) {
                    self.select_item(SidebarItem::Session(i));
                    let info = self.sessions[i].clone();
                    let path = self.projects.get(project).map(|p| p.path.clone());
                    self.open_session(&info, path);
                }
            }
        }
    }

    fn select_item(&mut self, item: SidebarItem) {
        if let Some(pos) = self.sidebar_items().iter().position(|it| *it == item) {
            self.sidebar_selected = pos;
        }
    }
}

fn status_rank(s: SessionStatus) -> u8 {
    match s {
        SessionStatus::Working => 0,
        SessionStatus::Blocked => 1,
        SessionStatus::Idle => 2,
        SessionStatus::Failed => 3,
    }
}

fn session_key(s: &SessionInfo) -> String {
    match s.conv {
        Some(id) if s.session_id.is_empty() => format!("c:{id}"),
        _ => format!("s:{}", s.session_id),
    }
}

fn char_to_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map_or(s.len(), |(b, _)| b)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

fn last_line(s: &str) -> &str {
    s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(s)
}
