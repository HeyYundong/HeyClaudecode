use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::input::Input;
use crate::sessions::{Counts, Groups, STATUS_ORDER, Session, Status, group_sessions, reactivated, stats_for, suggest_paths};
use crate::store::{self, Confirmed, Project};
use crate::text::{self, fit, info, name_column, pad_start, relative_time, truncate, where_in, width, with_suffix};
use crate::worker::{self, Msg};

// 每个分组默认只列最新的几个，多出来的折叠成一行，光标移上去按 Enter 展开或收起
pub const RECENT: usize = 5;
// 多位序号：两次数字键间隔超过这个时间就按已输入的序号进入
const DIGIT_WAIT: Duration = Duration::from_millis(600);
// 返回上一级总是立刻生效，连按两下 Ctrl+Z 就能从会话回到项目列表。
// 只有“在项目列表里按 Ctrl+Z 退出”要和上一次返回隔开这么久，免得连按时一路退出 HeyClaude
const QUIT_GAP: Duration = Duration::from_millis(500);
// 任务状态文件变化后等一会儿再刷新，把一串变化合并成一次
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(300);
// 两次刷新之间至少隔这么久
const MIN_FETCH_GAP: Duration = Duration::from_secs(2);
// 没有收到变化通知时也定时刷新，兜底（比如监听不到交互会话的变化）
const FALLBACK_REFRESH: Duration = Duration::from_secs(30);
// 失败原因那一列的宽度（“已停止”加两格空）；项目里没有失败任务时不留这一列
const TAG_WIDTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
	// 默认：项目列表 → 按状态分组的任务列表
	Grouped,
	// hcc --tabs：一个界面里用标签切换项目和状态
	Tabs,
}

pub enum Action {
	Quit,
	Attach(Project, Session),
}

struct AddState {
	value: String,
	candidates: Vec<String>,
	// None = 输入框
	selected: Option<usize>,
	error: Option<String>,
}

enum Screen {
	List,
	Add(AddState),
	Delete,
	Sessions,
	NewTask(Input),
}

// 任务列表的状态：标签、展开的分组、光标、撤销栈
#[derive(Default)]
struct View {
	// tabs 布局当前的状态标签；None 表示还没手动选过，用默认的
	tab_choice: Option<Status>,
	expanded: HashSet<Status>,
	asking: bool,
	cursor: usize,
	// 每次确认的一批会话 ID
	undo: Vec<Vec<String>>,
	// 从会话回来时，下次数据到了以后把光标放回这个任务上
	restore_id: Option<String>,
}

// 可以选中的行，按显示顺序：任务，以及超过 RECENT 个的分组末尾的折叠行
enum Item<'a> {
	Session(&'a Session, Status),
	Fold { group: Status, open: bool, hidden: usize },
}

impl Item<'_> {
	fn group(&self) -> Status {
		match self {
			Item::Session(_, g) => *g,
			Item::Fold { group, .. } => *group,
		}
	}
}

type HelpSection = (&'static str, Vec<(&'static str, String)>);

pub struct App {
	layout: Layout,
	projects: Vec<Project>,
	missing: HashSet<String>,
	cursor: usize,
	screen: Screen,
	sessions: Option<Vec<Session>>,
	loading: bool,
	refetch: bool,
	last_fetch: Option<Instant>,
	fetch_due: Option<Instant>,
	message: Option<(String, Color)>,
	help: Option<(&'static str, Vec<HelpSection>)>,
	back_at: Option<Instant>,
	confirmed: Confirmed,
	view: View,
	digits: String,
	digit_due: Option<Instant>,
	forwarded: Vec<String>,
	fetch_tx: Sender<Confirmed>,
	msg_tx: Sender<Msg>,
	action: Option<Action>,
}

fn dim() -> Style {
	Style::default().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
	Style::default().add_modifier(Modifier::BOLD)
}

fn red() -> Style {
	Style::default().fg(Color::Red)
}

// 五种状态的颜色；完成变灰
fn status_style(s: Status) -> Style {
	match s {
		Status::Working => Style::default().fg(Color::Yellow),
		Status::Blocked => Style::default().fg(Color::Magenta),
		Status::Pending => Style::default().fg(Color::Cyan),
		Status::Failed => Style::default().fg(Color::Red),
		Status::Confirmed | Status::Idle => dim(),
	}
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
	Span::styled(text.into(), style)
}

fn line_width(line: &Line) -> usize {
	line.spans.iter().map(|s| width(&s.content)).sum()
}

// 一个状态的数量文字：不为零时用状态色，为零时变灰
fn count_span(s: Status, n: usize, num_width: usize) -> Span<'static> {
	let style = if n > 0 { status_style(s) } else { dim() };
	span(format!("{} {}", pad_start(&n.to_string(), num_width), info(s).label), style)
}

// “3 进行中 · 1 需要输入 · 0 待确认 · 41 完成 · 0 失败”：数字在前，五种状态都列，为零的变灰
fn status_counts(counts: &Counts) -> Vec<Span<'static>> {
	let mut spans = Vec::new();
	for (i, s) in STATUS_ORDER.into_iter().enumerate() {
		if i > 0 {
			spans.push(span(" · ", dim()));
		}
		spans.push(count_span(s, counts.get(s), 0));
	}
	spans
}

// 列表行里的五列数量：每种状态固定一列，数字右对齐，这样各行上下对齐
fn status_columns(counts: &Counts, num_width: usize) -> Vec<Span<'static>> {
	STATUS_ORDER
		.into_iter()
		.map(|s| {
			let col = num_width + 1 + width(info(s).label) + 3;
			let sp = count_span(s, counts.get(s), num_width);
			let padded = fit(&sp.content, col);
			Span::styled(padded, sp.style)
		})
		.collect()
}

// 顶部一行：左边是位置，右边是概况；下面空一行
fn top_bar(left: Vec<Span<'static>>, right: Vec<Span<'static>>, inner: usize) -> Vec<Line<'static>> {
	let mut line = Line::from(left);
	if !right.is_empty() {
		let right = Line::from(right);
		let gap = inner.saturating_sub(line_width(&line) + line_width(&right)).max(2);
		line.spans.push(span(" ".repeat(gap), Style::default()));
		line.spans.extend(right.spans);
	}
	vec![line, Line::default()]
}

// 底部：一行提示消息，再一行常用按键
fn key_hints(keys: &[(&str, &str)], message: &Option<(String, Color)>) -> Vec<Line<'static>> {
	let mut lines = vec![Line::default()];
	if let Some((text, color)) = message {
		lines.push(Line::from(span(text.clone(), Style::default().fg(*color))));
	}
	let joined = keys.iter().map(|(k, l)| format!("{k} {l}")).collect::<Vec<_>>().join("    ");
	lines.push(Line::from(span(joined, dim())));
	lines
}

// 一行放不下时换行，项和项之间隔 gap 格
fn wrap_items(items: Vec<Span<'static>>, gap: usize, inner: usize) -> Vec<Line<'static>> {
	let mut lines: Vec<Line<'static>> = vec![Line::default()];
	for item in items {
		let w = width(&item.content);
		let current = lines.last_mut().unwrap();
		let used = line_width(current);
		if used > 0 && used + gap + w > inner {
			lines.push(Line::from(item));
		} else {
			if used > 0 {
				current.spans.push(span(" ".repeat(gap), Style::default()));
			}
			current.spans.push(item);
		}
	}
	lines
}

fn plain_char(key: &KeyEvent) -> Option<char> {
	match key.code {
		KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => Some(c),
		_ => None,
	}
}

fn is_back(key: &KeyEvent) -> bool {
	key.code == KeyCode::Esc || (key.code == KeyCode::Char('z') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
	key.code == KeyCode::Char(c) && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_up(key: &KeyEvent) -> bool {
	key.code == KeyCode::Up || plain_char(key) == Some('k')
}

fn is_down(key: &KeyEvent) -> bool {
	key.code == KeyCode::Down || plain_char(key) == Some('j')
}

fn is_open(key: &KeyEvent) -> bool {
	key.code == KeyCode::Enter || plain_char(key) == Some('o')
}

// 过滤掉粘贴进来的换行等控制字符
fn printable(text: &str) -> String {
	text.chars().filter(|c| !c.is_control()).collect()
}

fn list_help() -> Vec<HelpSection> {
	vec![(
		"项目列表",
		vec![
			("↑ ↓ / k j", "选择项目".into()),
			("Enter", "进入项目".into()),
			("1–9…", "按序号直接进入（超过 9 个可以连按多位）".into()),
			("a", "添加项目".into()),
			("d", "从列表移除项目（目录和会话不动）".into()),
			("r", "刷新（任务状态一变就会自动刷新）".into()),
			("Ctrl+Z / q", "退出".into()),
		],
	)]
}

fn session_help(layout: Layout) -> Vec<HelpSection> {
	let mut sections = vec![(
		"任务",
		vec![
			("↑ ↓ / k j", "选择任务".into()),
			("Enter / o", "打开会话（claude attach），在会话里按 Ctrl+Z 回来".into()),
			(
				"Enter",
				format!("在“还有 N 个”那一行上：展开或收起这一组（默认只显示最新 {RECENT} 个）"),
			),
			("c", "确认待确认的任务，它变成完成；在完成的任务上按则取消确认".into()),
			("C", "确认这个项目下全部待确认任务".into()),
			("u", "撤销最近一次确认".into()),
			("n", "新任务：在项目目录里用 claude --bg 启动".into()),
			("r", "刷新".into()),
		],
	)];
	if layout == Layout::Tabs {
		sections.push(("切换", vec![("← →", "切换项目".into()), ("Tab / Shift+Tab", "切换状态".into())]));
	}
	let back = if layout == Layout::Tabs {
		"回到项目列表"
	} else {
		"回到项目列表（连按两下就从会话回到项目列表）"
	};
	sections.push(("返回", vec![("Ctrl+Z / Esc", back.into())]));
	sections
}

fn session_keys(layout: Layout) -> [(&'static str, &'static str); 5] {
	[
		("Enter", "打开"),
		("c", "确认"),
		("n", "新任务"),
		("Ctrl+Z", if layout == Layout::Tabs { "项目列表" } else { "返回" }),
		("?", "全部按键"),
	]
}

fn fail_tag(s: &Session) -> &'static str {
	if s.state.as_deref() == Some("stopped") {
		"已停止"
	} else {
		"出错"
	}
}

// 默认标签：先看需要你处理的
fn default_tab(counts: &Counts) -> Status {
	[Status::Blocked, Status::Pending, Status::Failed, Status::Working, Status::Confirmed]
		.into_iter()
		.find(|&s| counts.get(s) > 0)
		.unwrap_or(Status::Working)
}

fn items<'a>(groups: &'a Groups, visible: &[Status], expanded: &HashSet<Status>) -> Vec<Item<'a>> {
	let mut items = Vec::new();
	for &g in visible {
		let all = groups.get(g);
		let foldable = all.len() > RECENT;
		let open = expanded.contains(&g);
		let shown = if foldable && !open { &all[..RECENT] } else { all };
		items.extend(shown.iter().map(|s| Item::Session(s, g)));
		if foldable {
			items.push(Item::Fold {
				group: g,
				open,
				hidden: all.len() - RECENT,
			});
		}
	}
	items
}

// 光标保持在可见窗口内：返回窗口的起点
fn window_start(cursor: usize, visible: usize, total: usize) -> usize {
	(cursor + 1).saturating_sub(visible).min(total.saturating_sub(visible))
}

impl App {
	pub fn new(layout: Layout, projects: Vec<Project>, forwarded: Vec<String>, fetch_tx: Sender<Confirmed>, msg_tx: Sender<Msg>) -> App {
		let (confirmed, message) = match store::load_reviews() {
			Ok(c) => (c, None),
			Err(e) => (Confirmed::new(), Some((e, Color::Red))),
		};
		let screen = if !projects.is_empty() && layout == Layout::Tabs {
			Screen::Sessions
		} else {
			Screen::List
		};
		let mut app = App {
			layout,
			projects: Vec::new(),
			missing: HashSet::new(),
			cursor: 0,
			screen,
			sessions: None,
			loading: false,
			refetch: false,
			last_fetch: None,
			fetch_due: None,
			message,
			help: None,
			back_at: None,
			confirmed,
			view: View::default(),
			digits: String::new(),
			digit_due: None,
			forwarded,
			fetch_tx,
			msg_tx,
			action: None,
		};
		app.set_projects(projects);
		app.refresh();
		app
	}

	pub fn has_action(&self) -> bool {
		self.action.is_some()
	}

	pub fn take_action(&mut self) -> Option<Action> {
		self.action.take()
	}

	fn set_projects(&mut self, projects: Vec<Project>) {
		self.missing = projects
			.iter()
			.filter(|p| !store::dir_exists(&p.path))
			.map(|p| p.path.clone())
			.collect();
		self.projects = projects;
		self.cursor = self.cursor.min(self.projects.len().saturating_sub(1));
	}

	fn current(&self) -> Option<&Project> {
		self.projects.get(self.cursor)
	}

	fn set_message(&mut self, text: impl Into<String>, color: Color) {
		self.message = Some((text.into(), color));
	}

	// 返回上一级（Ctrl+Z / Esc）都经过这里，记下时间
	fn go_back(&mut self) {
		self.back_at = Some(Instant::now());
	}

	// 在项目列表里按 Ctrl+Z / Esc 退出：刚返回过（包括刚从会话回来）就不退，
	// 这一下算作连按里多出来的。按住不放时也一直不退
	fn may_quit(&mut self) -> bool {
		let quit = self.back_at.is_none_or(|at| Instant::now() >= at + QUIT_GAP);
		self.go_back();
		quit
	}

	pub fn refresh(&mut self) {
		self.fetch_due = None;
		if self.loading {
			self.refetch = true;
			return;
		}
		self.loading = true;
		self.last_fetch = Some(Instant::now());
		let _ = self.fetch_tx.send(self.confirmed.clone());
	}

	fn save_confirmed(&mut self, next: Confirmed) -> bool {
		if let Err(e) = store::save_reviews(&next) {
			self.set_message(format!("保存确认记录失败：{e}"), Color::Red);
			return false;
		}
		self.confirmed = next;
		true
	}

	fn persist(&mut self, next: Vec<Project>) -> bool {
		match store::save_projects(&next) {
			Ok(()) => {
				self.set_projects(next);
				true
			}
			Err(e) => {
				self.set_message(format!("保存失败：{e}"), Color::Red);
				false
			}
		}
	}

	pub fn handle_msg(&mut self, msg: Msg) {
		match msg {
			Msg::Fetched { result, revived } => {
				self.loading = false;
				match result {
					Err(e) => self.set_message(format!("读取会话失败：{e}"), Color::Red),
					Ok(sessions) => {
						// 完成的任务又有了新进展，取消确认，让它重新走一遍待确认：
						// 一是此刻又在进行中或需要输入；二是确认之后又有过新对话（即使已经答完了，
						// 比如你在打开的会话里回复、Claude 在你回到 HeyClaude 之前就答完）
						let mut ids: Vec<String> = reactivated(&sessions, &self.confirmed);
						ids.extend(revived.into_iter().filter(|id| self.confirmed.contains_key(id)));
						if !ids.is_empty() {
							let next = store::without_confirmed(&self.confirmed, &ids);
							self.save_confirmed(next);
						}
						self.sessions = Some(sessions);
						self.missing = self
							.projects
							.iter()
							.filter(|p| !store::dir_exists(&p.path))
							.map(|p| p.path.clone())
							.collect();
						self.restore_cursor();
					}
				}
				if std::mem::take(&mut self.refetch) {
					self.refresh();
				}
			}
			Msg::Dispatched(Err(e)) => self.set_message(format!("启动失败：{e}"), Color::Red),
			Msg::Dispatched(Ok(out)) => {
				self.set_message(format!("已启动新任务 {out}"), Color::Green);
				self.refresh();
			}
			Msg::Changed => {
				if self.fetch_due.is_none() {
					let now = Instant::now();
					let earliest = self.last_fetch.map_or(now, |t| t + MIN_FETCH_GAP);
					self.fetch_due = Some((now + CHANGE_DEBOUNCE).max(earliest));
				}
			}
		}
	}

	// 定时的事：多位序号、合并后的刷新、兜底刷新。返回 true 表示需要重画
	pub fn tick(&mut self) -> bool {
		let now = Instant::now();
		let mut changed = false;
		if self.digit_due.is_some_and(|t| now >= t) {
			self.digit_due = None;
			let n: usize = self.digits.parse().unwrap_or(0);
			self.digits.clear();
			self.open_sessions(n.saturating_sub(1));
			changed = true;
		}
		let fallback = self.last_fetch.is_none_or(|t| now >= t + FALLBACK_REFRESH);
		if self.fetch_due.is_some_and(|t| now >= t) || (fallback && !self.loading) {
			self.refresh();
		}
		changed
	}

	// 从会话回来：回到这个项目的任务列表，光标和状态标签停在刚才的位置
	pub fn returned(&mut self, project_path: &str, session_id: &str, message: Option<(String, Color)>) {
		match store::load_projects() {
			Ok(projects) => self.set_projects(projects),
			Err(e) => self.set_message(e, Color::Red),
		}
		self.cursor = self.projects.iter().position(|p| p.path == project_path).unwrap_or(0);
		if !self.projects.is_empty() {
			self.screen = Screen::Sessions;
		}
		self.view.restore_id = Some(session_id.to_string());
		self.restore_cursor();
		self.view.restore_id = Some(session_id.to_string());
		if let Some(m) = message {
			self.message = Some(m);
		}
		self.go_back();
		self.refresh();
	}

	fn restore_cursor(&mut self) {
		let Some(id) = &self.view.restore_id else { return };
		let Some((_, list, _)) = self.session_items() else { return };
		let found = list.iter().position(|(sid, _)| sid == id);
		self.view.restore_id = None;
		if let Some(i) = found {
			self.view.cursor = i;
		}
	}

	fn groups(&self) -> Option<Groups> {
		let project = self.current()?;
		Some(group_sessions(self.sessions.as_deref()?, &project.path, &self.confirmed))
	}

	fn tab(&self, groups: &Groups) -> Status {
		self.view.tab_choice.unwrap_or_else(|| default_tab(&groups.counts()))
	}

	fn session_items(&self) -> Option<(Groups, Vec<(String, Status)>, Status)> {
		// 只返回会话 ID 和分组，避免借用 groups
		let groups = self.groups()?;
		let tab = self.tab(&groups);
		let visible: Vec<Status> = if self.layout == Layout::Tabs {
			vec![tab]
		} else {
			STATUS_ORDER.to_vec()
		};
		let list = items(&groups, &visible, &self.view.expanded)
			.iter()
			.map(|it| match it {
				Item::Session(s, g) => (s.session_id.clone(), *g),
				Item::Fold { group, .. } => (String::new(), *group),
			})
			.collect();
		Some((groups, list, tab))
	}

	fn open_sessions(&mut self, index: usize) {
		if index >= self.projects.len() {
			return;
		}
		self.cursor = index;
		self.view = View::default();
		self.screen = Screen::Sessions;
	}

	fn clear_digits(&mut self) {
		self.digits.clear();
		self.digit_due = None;
	}

	// 数字键按序号进入；序号还可能更长时（比如有 12 个项目时按了 1）稍等下一位
	fn type_digit(&mut self, digit: char) {
		let digits = format!("{}{digit}", self.digits);
		let n: usize = digits.parse().unwrap_or(0);
		if n < 1 || n > self.projects.len() {
			self.clear_digits();
			return self.set_message(format!("没有序号 {digits} 的项目"), Color::Yellow);
		}
		self.cursor = n - 1;
		if n * 10 > self.projects.len() {
			self.clear_digits();
			return self.open_sessions(n - 1);
		}
		self.digits = digits;
		self.digit_due = Some(Instant::now() + DIGIT_WAIT);
	}

	pub fn handle_event(&mut self, event: Event) {
		match event {
			Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
			Event::Paste(text) => self.handle_paste(&text),
			_ => {}
		}
	}

	fn handle_paste(&mut self, text: &str) {
		let text = printable(text);
		match &mut self.screen {
			Screen::Add(add) => {
				let next = format!("{}{text}", add.value);
				add_edit(add, next);
			}
			Screen::NewTask(input) => input.insert(&text),
			_ => {}
		}
	}

	fn handle_key(&mut self, key: KeyEvent) {
		// ? 打开的按键说明，按任意键关闭
		if self.help.is_some() {
			self.help = None;
			return;
		}
		match self.screen {
			Screen::List => self.list_key(key),
			Screen::Delete => self.delete_key(key),
			Screen::Add(_) => self.add_key(key),
			Screen::NewTask(_) => self.new_task_key(key),
			Screen::Sessions => {
				if self.current().is_some() {
					self.sessions_key(key)
				} else {
					self.screen = Screen::List;
				}
			}
		}
	}

	fn list_key(&mut self, key: KeyEvent) {
		self.message = None;
		let ch = plain_char(&key);
		if let Some(d) = ch.filter(char::is_ascii_digit) {
			return self.type_digit(d);
		}
		self.clear_digits();
		if ch == Some('q') {
			self.action = Some(Action::Quit);
		} else if is_back(&key) {
			if self.may_quit() {
				self.action = Some(Action::Quit);
			}
		} else if is_up(&key) {
			self.cursor = self.cursor.saturating_sub(1);
		} else if is_down(&key) {
			self.cursor = (self.cursor + 1).min(self.projects.len().saturating_sub(1));
		} else if ch == Some('a') {
			self.screen = Screen::Add(AddState {
				value: String::new(),
				candidates: Vec::new(),
				selected: None,
				error: None,
			});
		} else if ch == Some('r') {
			self.refresh();
		} else if ch == Some('?') {
			self.help = Some(("全部按键", list_help()));
		} else if self.current().is_none() {
		} else if ch == Some('d') {
			self.screen = Screen::Delete;
		} else if key.code == KeyCode::Enter {
			self.open_sessions(self.cursor);
		}
	}

	fn delete_key(&mut self, key: KeyEvent) {
		if matches!(plain_char(&key), Some('y' | 'Y'))
			&& let Some(current) = self.current().cloned()
		{
			let len = self.projects.len();
			if self.persist(store::remove_project(&self.projects, &current.path)) {
				self.set_message(format!("已移除 {}（目录和会话都没有动）", current.name), Color::Green);
				self.cursor = self.cursor.min(len.saturating_sub(2));
			}
		}
		self.screen = Screen::List;
	}

	fn suggestions(&self, query: &str) -> Vec<(String, usize)> {
		let Some(sessions) = &self.sessions else { return Vec::new() };
		let added: Vec<String> = self.projects.iter().map(|p| p.path.clone()).collect();
		let q = query.trim().to_lowercase();
		suggest_paths(sessions, &added)
			.into_iter()
			.filter(|(p, _)| q.is_empty() || p.to_lowercase().contains(&q))
			.take(8)
			.collect()
	}

	fn add_key(&mut self, key: KeyEvent) {
		let Screen::Add(add) = &self.screen else { return };
		let suggestions = self.suggestions(&add.value);
		let (value, selected) = (add.value.clone(), add.selected);
		if is_back(&key) {
			self.go_back();
			self.screen = Screen::List;
			return;
		}
		if key.code == KeyCode::Enter {
			let raw = selected.and_then(|i| suggestions.get(i)).map_or(value, |s| s.0.clone());
			match store::add_project(&self.projects, &raw) {
				Ok((next, project)) => {
					if self.persist(next) {
						self.cursor = self.projects.iter().position(|p| p.path == project.path).unwrap_or(0);
						self.set_message(format!("已添加 {}", project.name), Color::Green);
					}
					self.screen = Screen::List;
				}
				Err(e) => {
					if let Screen::Add(add) = &mut self.screen {
						add.error = Some(e);
					}
				}
			}
			return;
		}
		let Screen::Add(add) = &mut self.screen else { return };
		match key.code {
			KeyCode::Up => add.selected = selected.and_then(|i| i.checked_sub(1)),
			KeyCode::Down if !suggestions.is_empty() => {
				add.selected = Some(selected.map_or(0, |i| (i + 1).min(suggestions.len() - 1)));
			}
			KeyCode::Tab => {
				if let Some(s) = selected.and_then(|i| suggestions.get(i)) {
					add_edit(add, s.0.clone());
				} else {
					let (v, candidates) = store::complete_path(&value);
					add.value = v;
					add.candidates = candidates;
					add.error = None;
				}
			}
			KeyCode::Backspace | KeyCode::Delete => {
				let mut v = value;
				v.pop();
				add_edit(add, v);
			}
			_ if is_ctrl(&key, 'u') => add_edit(add, String::new()),
			_ => {
				if let Some(c) = plain_char(&key).filter(|c| !c.is_control()) {
					add_edit(add, format!("{value}{c}"));
				}
			}
		}
	}

	fn new_task_key(&mut self, key: KeyEvent) {
		let Screen::NewTask(input) = &mut self.screen else { return };
		if is_back(&key) {
			self.go_back();
			self.screen = Screen::Sessions;
			return;
		}
		match key.code {
			KeyCode::Enter => {
				let text = input.value().trim().to_string();
				if text.is_empty() {
					self.go_back();
					self.screen = Screen::Sessions;
					return;
				}
				self.screen = Screen::Sessions;
				let Some(project) = self.current().cloned() else { return };
				self.set_message(format!("正在 {} 里启动新任务…", project.name), Color::Yellow);
				worker::dispatch_task(self.msg_tx.clone(), project.path, text, self.forwarded.clone());
			}
			_ => {
				input.key(&key);
			}
		}
	}

	fn confirm(&mut self, ids: Vec<String>, text: String) {
		let next = store::with_confirmed(&self.confirmed, &ids, &store::now_iso());
		if !self.save_confirmed(next) {
			return;
		}
		self.view.undo.push(ids);
		self.set_message(format!("{text}（u 撤销）"), Color::Green);
	}

	fn sessions_key(&mut self, key: KeyEvent) {
		let project = self.current().cloned().expect("sessions screen has a project");
		let Some((groups, list, tab)) = self.session_items() else {
			// 还没有数据时只能返回、刷新、看帮助
			self.message = None;
			if is_back(&key) {
				self.go_back();
				self.screen = Screen::List;
			} else if plain_char(&key) == Some('r') {
				self.refresh();
			} else if plain_char(&key) == Some('?') {
				self.help = Some(("全部按键", session_help(self.layout)));
			}
			return;
		};
		let tabs = self.layout == Layout::Tabs;
		let ch = plain_char(&key);

		if self.view.asking {
			if matches!(ch, Some('y' | 'Y')) {
				let ids: Vec<String> = groups.pending.iter().map(|s| s.session_id.clone()).collect();
				let n = ids.len();
				self.confirm(ids, format!("已确认 {} 下全部 {n} 个待确认任务", project.name));
			}
			self.view.asking = false;
			return;
		}

		self.message = None;
		let clamp = |i: usize| i.min(list.len().saturating_sub(1));
		if is_back(&key) {
			self.go_back();
			self.screen = Screen::List;
			return;
		}
		if ch == Some('?') {
			self.help = Some(("全部按键", session_help(self.layout)));
			return;
		}
		if is_up(&key) {
			self.view.cursor = clamp(self.view.cursor).saturating_sub(1);
			return;
		}
		if is_down(&key) {
			self.view.cursor = clamp(self.view.cursor + 1);
			return;
		}
		if tabs && matches!(key.code, KeyCode::Left | KeyCode::Right) {
			self.view.cursor = 0;
			let n = self.projects.len();
			self.cursor = if key.code == KeyCode::Left {
				(self.cursor + n - 1) % n
			} else {
				(self.cursor + 1) % n
			};
			return;
		}
		if tabs && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
			let i = STATUS_ORDER.iter().position(|&s| s == tab).unwrap_or(0);
			let step = if key.code == KeyCode::BackTab { STATUS_ORDER.len() - 1 } else { 1 };
			self.view.tab_choice = Some(STATUS_ORDER[(i + step) % STATUS_ORDER.len()]);
			self.view.cursor = 0;
			return;
		}
		match ch {
			Some('r') => return self.refresh(),
			Some('n') => {
				self.screen = Screen::NewTask(Input::default());
				return;
			}
			Some('u') => {
				let Some(ids) = self.view.undo.pop() else {
					return self.set_message("没有可以撤销的确认", Color::Yellow);
				};
				let next = store::without_confirmed(&self.confirmed, &ids);
				if self.save_confirmed(next) {
					self.set_message(format!("已撤销 {} 个任务的确认", ids.len()), Color::Green);
				}
				return;
			}
			Some('C') => {
				if groups.pending.is_empty() {
					return self.set_message("没有待确认的任务", Color::Yellow);
				}
				self.view.asking = true;
				return;
			}
			_ => {}
		}
		let index = clamp(self.view.cursor);
		let Some((id, group)) = list.get(index).cloned() else { return };
		if id.is_empty() {
			// 折叠行：展开或收起一组。收起时光标回到折叠行；展开时光标留在原位，正好落在第一个新露出来的任务上
			if is_open(&key) {
				if self.view.expanded.remove(&group) {
					let first = list.iter().position(|(_, g)| *g == group).unwrap_or(0);
					self.view.cursor = first + RECENT;
				} else {
					self.view.expanded.insert(group);
				}
			} else if ch == Some('c') {
				self.set_message("先按 Enter 展开，再选中要确认的任务", Color::Yellow);
			}
			return;
		}
		let Some(session) = groups.get(group).iter().find(|s| s.session_id == id).cloned() else {
			return;
		};
		let name = session.short_name().to_string();
		if is_open(&key) {
			if !session.is_background() {
				return self.set_message("这是交互会话，请在它自己的终端里操作", Color::Yellow);
			}
			self.action = Some(Action::Attach(project, session));
			return;
		}
		if ch == Some('c') {
			match group {
				Status::Pending => self.confirm(vec![id], format!("已确认 {name}")),
				Status::Confirmed => {
					let next = store::without_confirmed(&self.confirmed, &[id]);
					if self.save_confirmed(next) {
						self.set_message(format!("已取消确认 {name}，它回到了待确认"), Color::Green);
					}
				}
				Status::Working => self.set_message("任务还在进行，结束后才能确认", Color::Yellow),
				Status::Blocked => self.set_message("这个任务在等你输入，按 Enter 打开它", Color::Yellow),
				_ => self.set_message("失败的任务不需要确认", Color::Yellow),
			}
		}
	}

	// ---- 画界面 ----

	pub fn draw(&self, frame: &mut Frame) {
		let area = frame.area();
		let (columns, rows) = (area.width as usize, area.height as usize);
		// 四周留白：左右两格，上面一行。根节点比终端窄一列：有的终端写满最后一列会被截断
		let inner = columns.saturating_sub(5);
		let lines = self.lines(columns, rows, inner);
		let rect = Rect {
			x: 2.min(area.width),
			y: 1.min(area.height),
			width: inner as u16,
			height: area.height.saturating_sub(1),
		};
		frame.render_widget(Paragraph::new(lines), rect);
	}

	fn brand() -> Span<'static> {
		span("HeyClaude", red().add_modifier(Modifier::BOLD))
	}

	// 所有页面的左上角都是红色的 HeyClaude，进入下一级时在后面接“› 名字”
	fn crumb(name: &str) -> Vec<Span<'static>> {
		vec![Self::brand(), span(" › ", dim()), span(name.to_string(), bold())]
	}

	fn lines(&self, columns: usize, rows: usize, inner: usize) -> Vec<Line<'static>> {
		if let Some((title, sections)) = &self.help {
			return help_lines(title, sections, inner);
		}
		match &self.screen {
			Screen::Sessions if self.current().is_some() => self.sessions_lines(columns, rows, inner),
			Screen::NewTask(input) if self.current().is_some() => {
				let project = self.current().unwrap();
				let mut lines = top_bar(Self::crumb(&format!("{} › 新任务", project.name)), vec![], inner);
				lines.push(Line::from(span(
					format!("在 {} 里用 claude --bg 启动一个后台任务", project.name),
					dim(),
				)));
				lines.push(Line::default());
				lines.extend(input.lines(span("❯ ", red()), "要 Claude 做什么？", inner));
				lines.extend(key_hints(&[("Enter", "启动"), ("← →", "移动光标"), ("Ctrl+U", "清空"), ("Ctrl+Z", "取消")], &None));
				lines
			}
			Screen::Add(add) => self.add_lines(add, inner),
			_ => self.list_lines(columns, rows, inner),
		}
	}

	fn all_stats(&self) -> Option<Vec<Counts>> {
		let sessions = self.sessions.as_deref()?;
		Some(
			self.projects
				.iter()
				.map(|p| stats_for(sessions, &p.path, &self.confirmed))
				.collect(),
		)
	}

	fn list_lines(&self, columns: usize, rows: usize, inner: usize) -> Vec<Line<'static>> {
		let stats = self.all_stats();
		let mut totals = Counts::default();
		for s in stats.iter().flatten() {
			totals.add(s);
		}
		let mut right = vec![span(format!("{} 个项目", self.projects.len()), dim()), span(" · ", dim())];
		right.extend(status_counts(&totals));
		let mut lines = top_bar(vec![Self::brand()], right, inner);

		if self.projects.is_empty() {
			lines.push(Line::from("还没有项目。"));
			lines.push(Line::from(span("按 a 添加项目目录。", dim())));
		} else {
			// 每个项目一行：序号、名字、五种状态的数量（固定五列，为零也显示）。样式和任务列表的行一致
			let number_width = (self.projects.len().to_string().len() + 2).max(3);
			let name_width = name_column(self.projects.iter().map(|p| p.name.as_str()), columns.saturating_sub(4));
			// 数字列的宽度按最大的数来定，这样各行上下对齐
			let biggest = stats.iter().flatten().map(Counts::max).max().unwrap_or(0);
			let num_width = biggest.to_string().len();
			let visible = rows.saturating_sub(7).max(1);
			let start = window_start(self.cursor, visible, self.projects.len());
			if start > 0 {
				lines.push(Line::from(span(format!("  ↑ 还有 {start} 个"), dim())));
			}
			for (index, project) in self.projects.iter().enumerate().skip(start).take(visible) {
				let selected = index == self.cursor;
				let mut row = vec![
					span(if selected { "❯ " } else { "  " }, red()),
					span(fit(&(index + 1).to_string(), number_width), dim()),
					span(
						fit(&project.name, name_width),
						if selected {
							red().add_modifier(Modifier::BOLD)
						} else {
							Style::default()
						},
					),
				];
				if self.missing.contains(&project.path) {
					row.push(span("目录不存在（按 d 移除）", Style::default().fg(Color::Yellow)));
				} else if let Some(stats) = &stats {
					row.extend(status_columns(&stats[index], num_width));
				} else if self.loading {
					row.push(span("…", dim()));
				}
				lines.push(Line::from(row));
			}
			let rest = self.projects.len().saturating_sub(start + visible);
			if rest > 0 {
				lines.push(Line::from(span(format!("  ↓ 还有 {rest} 个"), dim())));
			}
		}
		if let (Screen::Delete, Some(current)) = (&self.screen, self.current()) {
			lines.push(Line::from(span(
				format!("从列表移除 {}？目录和会话不受影响。(y/N)", current.name),
				Style::default().fg(Color::Yellow),
			)));
		}
		lines.extend(key_hints(
			&[
				("Enter", "进入"),
				("a", "添加"),
				("d", "删除"),
				("Ctrl+Z", "退出"),
				("?", "全部按键"),
			],
			&self.message,
		));
		lines
	}

	fn add_lines(&self, add: &AddState, inner: usize) -> Vec<Line<'static>> {
		let mut lines = top_bar(Self::crumb("添加项目"), vec![], inner);
		lines.push(Line::from(span(" 添加项目", bold())));
		lines.push(Line::default());
		let on_input = add.selected.is_none();
		let mut input = vec![span(if on_input { " ❯ " } else { "   " }, red()), span("路径: ", Style::default())];
		if add.value.is_empty() {
			let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
			input.push(span(format!("留空 = 当前目录 ({cwd})"), dim()));
		} else {
			input.push(span(add.value.clone(), Style::default()));
		}
		if on_input {
			input.push(span(" ", Style::default().add_modifier(Modifier::REVERSED)));
		}
		lines.push(Line::from(input));
		if !add.candidates.is_empty() {
			let mut text = add.candidates.iter().take(20).cloned().collect::<Vec<_>>().join("  ");
			if add.candidates.len() > 20 {
				text.push_str("  …");
			}
			lines.push(Line::from(vec![
				span(" ".repeat(11), Style::default()),
				span(truncate(&text, inner.saturating_sub(11)), dim()),
			]));
		}
		if let Some(error) = &add.error {
			lines.push(Line::from(span(format!("   {error}"), red())));
		}
		let suggestions = self.suggestions(&add.value);
		if !suggestions.is_empty() {
			lines.push(Line::default());
			lines.push(Line::from(span("   有会话记录、还没添加的目录：", dim())));
			for (i, (path, count)) in suggestions.iter().enumerate() {
				let selected = add.selected == Some(i);
				lines.push(Line::from(vec![
					span(if selected { " ❯ " } else { "   " }, red()),
					span(path.clone(), if selected { bold() } else { Style::default() }),
					span(format!("  ({count} 个会话)"), dim()),
				]));
			}
		}
		lines.extend(key_hints(
			&[("Tab", "补全/选用"), ("↑↓", "选择建议"), ("Enter", "添加"), ("Ctrl+Z", "取消")],
			&None,
		));
		lines
	}

	fn sessions_lines(&self, columns: usize, rows: usize, inner: usize) -> Vec<Line<'static>> {
		let project = self.current().unwrap();
		let tabs = self.layout == Layout::Tabs;
		let groups = self.groups();
		let mut lines = Vec::new();
		if !tabs {
			let right = groups.as_ref().map(|g| status_counts(&g.counts())).unwrap_or_default();
			lines.extend(top_bar(Self::crumb(&project.name), right, inner));
		}
		match &groups {
			None => {
				let text = if self.loading {
					"读取会话中…"
				} else {
					"没有会话数据，按 r 重试"
				};
				lines.push(Line::from(span(text, dim())));
			}
			Some(groups) if tabs => lines.extend(self.tabs_body(groups, columns, rows, inner)),
			Some(groups) => lines.extend(self.grouped_body(groups, columns, rows, inner)),
		}
		if let Some(groups) = &groups
			&& self.view.asking
		{
			lines.push(Line::default());
			let text = format!("确认 {} 下全部 {} 个待确认任务？(y/N)", project.name, groups.pending.len());
			lines.push(Line::from(span(text, Style::default().fg(Color::Yellow))));
		}
		lines.extend(key_hints(&session_keys(self.layout), &self.message));
		lines
	}

	fn fold_line(group_open: bool, hidden: usize, selected: bool) -> Line<'static> {
		let text = if group_open {
			format!("收起，只显示最新 {RECENT} 个")
		} else {
			format!("还有 {hidden} 个，Enter 展开")
		};
		Line::from(vec![
			span(if selected { "❯ " } else { "  " }, red()),
			span(if group_open { "▴  " } else { "▾  " }, dim()),
			span(text, if selected { red().add_modifier(Modifier::BOLD) } else { dim() }),
		])
	}

	// 默认布局：按状态分组，组之间空一行
	fn grouped_body(&self, groups: &Groups, columns: usize, rows: usize, _inner: usize) -> Vec<Line<'static>> {
		let list = items(groups, &STATUS_ORDER, &self.view.expanded);
		let cursor = self.view.cursor.min(list.len().saturating_sub(1));
		let names: Vec<String> = list
			.iter()
			.filter_map(|i| {
				if let Item::Session(s, _) = i {
					Some(s.display_name())
				} else {
					None
				}
			})
			.collect();
		let name_width = name_column(names.iter().map(String::as_str), columns.saturating_sub(4));
		let tag_width = if groups.failed.is_empty() { 0 } else { TAG_WIDTH };
		let num_width = groups.counts().max().to_string().len();
		let now = text::now_ms();

		// 先排好所有行，记下每个可选行在第几行
		let mut out: Vec<(Line<'static>, Option<usize>)> = Vec::new();
		let mut index = 0;
		for g in STATUS_ORDER {
			let all = groups.get(g);
			if all.is_empty() {
				continue;
			}
			if !out.is_empty() {
				out.push((Line::from(" "), None));
			}
			let foldable = all.len() > RECENT;
			let open = self.view.expanded.contains(&g);
			// 分组标题，写法和状态数量一样：“ 3 待确认”。缩进到图标那一列，数字右对齐，各组的状态名上下对齐
			let mut header = vec![
				span("  ", Style::default()),
				span(
					format!("{} {}", pad_start(&all.len().to_string(), num_width), info(g).label),
					status_style(g).add_modifier(Modifier::BOLD),
				),
			];
			if foldable && !open {
				header.push(span(format!(" · 最新 {RECENT} 个"), dim()));
			}
			out.push((Line::from(header), None));
			while index < list.len() && list[index].group() == g {
				let selected = index == cursor;
				let line = match &list[index] {
					Item::Fold { open, hidden, .. } => Self::fold_line(*open, *hidden, selected),
					Item::Session(s, _) => {
						let dimmed = g == Status::Confirmed;
						let mut name_style = Style::default();
						if selected {
							name_style = red().add_modifier(Modifier::BOLD);
						} else if dimmed {
							name_style = dim();
						}
						let mut row = vec![
							span(if selected { "❯ " } else { "  " }, red()),
							span(format!("{}  ", info(g).icon), status_style(g)),
							span(fit(&s.display_name(), name_width), name_style),
						];
						if tag_width > 0 {
							let tag = if g == Status::Failed { fail_tag(s) } else { "" };
							row.push(span(
								fit(tag, tag_width),
								if g == Status::Failed {
									red().add_modifier(Modifier::DIM)
								} else {
									dim()
								},
							));
						}
						row.push(span(relative_time(s.started(), now), dim()));
						Line::from(row)
					}
				};
				out.push((line, Some(index)));
				index += 1;
			}
		}

		let mut lines = Vec::new();
		if out.is_empty() {
			lines.push(Line::from(span("这个项目还没有后台任务，按 n 启动一个", dim())));
			return lines;
		}
		let visible = rows.saturating_sub(9).max(4);
		let cursor_line = out.iter().position(|(_, i)| *i == Some(cursor)).unwrap_or(0);
		let start = window_start(cursor_line, visible, out.len());
		if start > 0 {
			lines.push(Line::from(span(format!("  ↑ 还有 {start} 行"), dim())));
		}
		let total = out.len();
		lines.extend(out.into_iter().skip(start).take(visible).map(|(l, _)| l));
		let rest = total.saturating_sub(start + visible);
		if rest > 0 {
			lines.push(Line::from(span(format!("  ↓ 还有 {rest} 行"), dim())));
		}
		lines
	}

	// --tabs 布局：第一行项目，第二行状态标签，下面是这个状态的任务，每个两行
	fn tabs_body(&self, groups: &Groups, columns: usize, rows: usize, inner: usize) -> Vec<Line<'static>> {
		let project = self.current().unwrap();
		let counts = groups.counts();
		let tab = self.tab(groups);
		let mut project_items = vec![Self::brand()];
		for (i, p) in self.projects.iter().enumerate() {
			let style = if i == self.cursor {
				bold().add_modifier(Modifier::UNDERLINED)
			} else {
				dim()
			};
			project_items.push(span(p.name.clone(), style));
		}
		project_items.push(span("← →", dim()));
		let mut lines = wrap_items(project_items, 3, inner);
		lines.push(Line::default());

		let mut tab_items = Vec::new();
		for s in STATUS_ORDER {
			let selected = s == tab;
			let n = counts.get(s);
			let mut style = status_style(s);
			if selected {
				style = style
					.remove_modifier(Modifier::DIM)
					.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
			} else if n == 0 {
				style = style.add_modifier(Modifier::DIM);
			}
			tab_items.push(span(format!("{n} {}", info(s).label), style));
		}
		tab_items.push(span("Tab", dim()));
		lines.extend(wrap_items(tab_items, 3, inner));
		lines.push(Line::from(span("─".repeat(columns.saturating_sub(5).max(10)), dim())));
		lines.push(Line::default());

		let list = items(groups, &[tab], &self.view.expanded);
		if list.is_empty() {
			lines.push(Line::from(span(format!("没有{}的任务", info(tab).label), dim())));
		}
		let visible = (rows.saturating_sub(10) / 3).max(1);
		let cursor = self.view.cursor.min(list.len().saturating_sub(1));
		let start = window_start(cursor, visible, list.len());
		if start > 0 {
			lines.push(Line::from(span(format!("  ↑ 还有 {start} 个"), dim())));
		}
		let now = text::now_ms();
		for (i, item) in list.iter().enumerate().skip(start).take(visible) {
			let selected = i == cursor;
			match item {
				Item::Fold { open, hidden, .. } => lines.push(Self::fold_line(*open, *hidden, selected)),
				Item::Session(s, g) => {
					let dimmed = *g == Status::Confirmed;
					let mut style = Style::default();
					if !dimmed || selected {
						style = style.add_modifier(Modifier::BOLD);
					}
					if dimmed && !selected {
						style = style.add_modifier(Modifier::DIM);
					}
					lines.push(Line::from(vec![
						span(if selected { "❯ " } else { "  " }, red()),
						span(format!("{}  ", info(*g).icon), status_style(*g)),
						span(truncate(&s.display_name(), inner.saturating_sub(5)), style),
					]));
					let when = match (g, self.confirmed.get(&s.session_id)) {
						(Status::Confirmed, Some(at)) => {
							let t = chrono::DateTime::parse_from_rfc3339(at)
								.map(|t| t.timestamp_millis() as f64)
								.unwrap_or(f64::NAN);
							with_suffix(&relative_time(t, now), "确认")
						}
						_ => with_suffix(&relative_time(s.started(), now), "开始"),
					};
					let mut meta = Vec::new();
					if *g == Status::Failed {
						meta.push(fail_tag(s).to_string());
					}
					meta.push(when);
					let place = where_in(s, &project.path);
					if !place.is_empty() {
						meta.push(place);
					}
					lines.push(Line::from(vec![
						span("     ", Style::default()),
						span(truncate(&meta.join(" · "), inner.saturating_sub(5)), dim()),
					]));
				}
			}
			lines.push(Line::default());
		}
		let rest = list.len().saturating_sub(start + visible);
		if rest > 0 {
			lines.push(Line::from(span(format!("  ↓ 还有 {rest} 个"), dim())));
		}
		lines
	}
}

fn add_edit(add: &mut AddState, next: String) {
	add.value = next;
	add.selected = None;
	add.candidates.clear();
	add.error = None;
}

// ? 打开的全部按键说明
fn help_lines(title: &str, sections: &[HelpSection], inner: usize) -> Vec<Line<'static>> {
	let mut lines = top_bar(vec![span(title.to_string(), bold())], vec![span("按任意键返回", dim())], inner);
	let key_width = sections
		.iter()
		.flat_map(|(_, keys)| keys.iter().map(|(k, _)| width(k)))
		.max()
		.unwrap_or(0)
		+ 4;
	for (section, keys) in sections {
		lines.push(Line::from(span(section.to_string(), bold())));
		for (k, desc) in keys {
			lines.push(Line::from(vec![
				span(fit(&format!("  {k}"), key_width), Style::default().fg(Color::Cyan)),
				span(desc.clone(), Style::default()),
			]));
		}
		lines.push(Line::default());
	}
	lines
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sessions(n: usize, state: &str) -> Vec<Session> {
		(0..n)
			.map(|i| Session {
				id: format!("s{i}"),
				session_id: format!("s{i}-full"),
				cwd: Some("/p".into()),
				kind: Some("background".into()),
				started_at: Some(i as f64),
				name: Some(format!("s{i}")),
				state: Some(state.into()),
				status: None,
			})
			.collect()
	}

	#[test]
	fn folds_groups_over_recent() {
		let mut all = sessions(7, "done");
		all.extend(sessions(2, "working").into_iter().map(|mut s| {
			s.session_id += "-w";
			s
		}));
		let groups = group_sessions(&all, "/p", &Confirmed::new());
		let list = items(&groups, &STATUS_ORDER, &HashSet::new());
		// 2 个进行中，5 个待确认，一行折叠
		assert_eq!(list.len(), 2 + RECENT + 1);
		assert!(matches!(
			list.last(),
			Some(Item::Fold {
				hidden: 2,
				open: false,
				..
			})
		));
		let open = items(&groups, &STATUS_ORDER, &HashSet::from([Status::Pending]));
		assert_eq!(open.len(), 2 + 7 + 1);
	}

	#[test]
	fn default_tab_prefers_attention() {
		let counts = Counts {
			working: 1,
			pending: 2,
			..Default::default()
		};
		assert_eq!(default_tab(&counts), Status::Pending);
		assert_eq!(default_tab(&Counts::default()), Status::Working);
	}

	#[test]
	fn windows_follow_cursor() {
		assert_eq!(window_start(0, 5, 20), 0);
		assert_eq!(window_start(7, 5, 20), 3);
		assert_eq!(window_start(19, 5, 20), 15);
		assert_eq!(window_start(3, 10, 4), 0);
	}

	#[test]
	fn wraps_items() {
		let items = vec![
			span("aaaa", Style::default()),
			span("bbbb", Style::default()),
			span("cc", Style::default()),
		];
		let lines = wrap_items(items, 3, 12);
		assert_eq!(lines.len(), 2);
		assert_eq!(line_width(&lines[0]), 11);
	}
}
