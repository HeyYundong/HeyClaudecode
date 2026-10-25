use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::store::Confirmed;

// `claude agents --json --all` 里的一个会话，只取用得到的字段
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
	#[serde(default)]
	pub id: String,
	#[serde(default)]
	pub session_id: String,
	#[serde(default)]
	pub cwd: Option<String>,
	#[serde(default)]
	pub kind: Option<String>,
	#[serde(default)]
	pub started_at: Option<f64>,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub state: Option<String>,
	#[serde(default)]
	pub status: Option<String>,
	// 你在 HeyClaude 里改过的标题（存在 titles.json），优先于 Claude Code 给的名字
	#[serde(skip)]
	pub title: Option<String>,
}

impl Session {
	pub fn is_background(&self) -> bool {
		self.kind.as_deref() == Some("background")
	}

	pub fn started(&self) -> f64 {
		self.started_at.unwrap_or(0.0)
	}

	pub fn short_name(&self) -> &str {
		if let Some(t) = self.title.as_deref().filter(|t| !t.is_empty()) {
			return t;
		}
		match self.name.as_deref() {
			Some(n) if !n.is_empty() => n,
			_ if !self.id.is_empty() => &self.id,
			_ => &self.session_id,
		}
	}

	// 列表里的名字：交互会话后面标出来
	pub fn display_name(&self) -> String {
		let name = self.short_name();
		if self.is_background() {
			name.to_string()
		} else {
			format!("{name}（交互）")
		}
	}

	pub fn cwd(&self) -> &str {
		self.cwd.as_deref().unwrap_or("")
	}
}

const AGENTS_TIMEOUT: Duration = Duration::from_secs(20);

// 读取 `claude agents --json --all` 的会话列表
pub fn fetch_sessions() -> Result<Vec<Session>, String> {
	let mut child = Command::new("claude")
		.args(["agents", "--json", "--all"])
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.map_err(|e| {
			if e.kind() == std::io::ErrorKind::NotFound {
				"找不到 claude 命令".to_string()
			} else {
				e.to_string()
			}
		})?;
	let mut stdout = child.stdout.take().expect("stdout is piped");
	let reader = std::thread::spawn(move || {
		let mut buf = Vec::new();
		let _ = stdout.read_to_end(&mut buf);
		buf
	});
	let start = Instant::now();
	let status = loop {
		match child.try_wait() {
			Ok(Some(status)) => break status,
			Ok(None) if start.elapsed() > AGENTS_TIMEOUT => {
				let _ = child.kill();
				let _ = child.wait();
				return Err("claude agents 超时".to_string());
			}
			Ok(None) => std::thread::sleep(Duration::from_millis(20)),
			Err(e) => return Err(e.to_string()),
		}
	};
	let out = reader.join().unwrap_or_default();
	if !status.success() {
		let mut err = String::new();
		if let Some(mut stderr) = child.stderr.take() {
			let _ = stderr.read_to_string(&mut err);
		}
		let line = err.trim().lines().last().unwrap_or("").to_string();
		return Err(if line.is_empty() {
			format!("claude agents 退出码 {}", status.code().unwrap_or(-1))
		} else {
			line
		});
	}
	parse_sessions(&out)
}

pub fn parse_sessions(bytes: &[u8]) -> Result<Vec<Session>, String> {
	let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("无法解析会话列表: {e}"))?;
	let serde_json::Value::Array(items) = value else {
		return Ok(Vec::new());
	};
	// 单个条目格式不对就跳过，不影响其他会话
	Ok(items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

// 会话归类：
//   Working   进行中
//   Blocked   需要输入
//   Pending   待确认：Claude 说做完了，你还没确认
//   Confirmed 完成：你确认过
//   Failed    失败：出错（failed）或被停止（stopped），不需要确认
//   Idle      空闲的交互会话（不需要处理，不统计）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
	Working,
	Blocked,
	Pending,
	Confirmed,
	Failed,
	Idle,
}

// 五种状态，按这个顺序显示
pub const STATUS_ORDER: [Status; 5] = [Status::Working, Status::Blocked, Status::Pending, Status::Confirmed, Status::Failed];

// 后台任务只看 Claude Code 记录的 state。进程的 status（busy / idle / waiting）不作数：
// 它可能长时间卡在 busy，而你回复一个结束的任务时 state 本身会变回 working。
// 交互会话没有 state，只能看 status 是不是 busy。
// 其余结束状态（包括 Claude Code 以后新增的）都按“做完了”处理，进待确认
pub fn classify(session: &Session, confirmed: &Confirmed) -> Status {
	if !session.is_background() {
		return if session.status.as_deref() == Some("busy") {
			Status::Working
		} else {
			Status::Idle
		};
	}
	match session.state.as_deref() {
		Some("working") => Status::Working,
		Some("blocked") => Status::Blocked,
		Some("failed") | Some("stopped") => Status::Failed,
		_ if confirmed.contains_key(&session.session_id) => Status::Confirmed,
		_ => Status::Pending,
	}
}

// 与 `claude agents --cwd` 一致：目录本身及其子目录（含 .claude/worktrees）都算
pub fn is_under(cwd: &str, project_path: &str) -> bool {
	if cwd.is_empty() {
		return false;
	}
	if cwd == project_path {
		return true;
	}
	let prefix = if project_path.ends_with('/') {
		project_path.to_string()
	} else {
		format!("{project_path}/")
	};
	cwd.starts_with(&prefix)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
	pub working: usize,
	pub blocked: usize,
	pub pending: usize,
	pub confirmed: usize,
	pub failed: usize,
}

impl Counts {
	pub fn get(&self, s: Status) -> usize {
		match s {
			Status::Working => self.working,
			Status::Blocked => self.blocked,
			Status::Pending => self.pending,
			Status::Confirmed => self.confirmed,
			Status::Failed => self.failed,
			Status::Idle => 0,
		}
	}

	fn bump(&mut self, s: Status) {
		match s {
			Status::Working => self.working += 1,
			Status::Blocked => self.blocked += 1,
			Status::Pending => self.pending += 1,
			Status::Confirmed => self.confirmed += 1,
			Status::Failed => self.failed += 1,
			Status::Idle => {}
		}
	}

	pub fn add(&mut self, other: &Counts) {
		self.working += other.working;
		self.blocked += other.blocked;
		self.pending += other.pending;
		self.confirmed += other.confirmed;
		self.failed += other.failed;
	}

	pub fn max(&self) -> usize {
		STATUS_ORDER.iter().map(|&s| self.get(s)).max().unwrap_or(0)
	}
}

pub fn stats_for(sessions: &[Session], project_path: &str, confirmed: &Confirmed) -> Counts {
	let mut counts = Counts::default();
	for s in sessions.iter().filter(|s| is_under(s.cwd(), project_path)) {
		counts.bump(classify(s, confirmed));
	}
	counts
}

// 已确认但又开始运行或在等你回复的会话：它有了新进展，需要重新审阅
pub fn reactivated(sessions: &[Session], confirmed: &Confirmed) -> Vec<String> {
	let none = Confirmed::new();
	sessions
		.iter()
		.filter(|s| confirmed.contains_key(&s.session_id) && matches!(classify(s, &none), Status::Working | Status::Blocked))
		.map(|s| s.session_id.clone())
		.collect()
}

// 按状态分组的会话；完成的按确认时间倒序，其余按开始时间倒序
#[derive(Debug, Default, Clone)]
pub struct Groups {
	pub working: Vec<Session>,
	pub blocked: Vec<Session>,
	pub pending: Vec<Session>,
	pub confirmed: Vec<Session>,
	pub failed: Vec<Session>,
}

impl Groups {
	pub fn get(&self, s: Status) -> &[Session] {
		match s {
			Status::Working => &self.working,
			Status::Blocked => &self.blocked,
			Status::Pending => &self.pending,
			Status::Confirmed => &self.confirmed,
			Status::Failed => &self.failed,
			Status::Idle => &[],
		}
	}

	fn get_mut(&mut self, s: Status) -> Option<&mut Vec<Session>> {
		match s {
			Status::Working => Some(&mut self.working),
			Status::Blocked => Some(&mut self.blocked),
			Status::Pending => Some(&mut self.pending),
			Status::Confirmed => Some(&mut self.confirmed),
			Status::Failed => Some(&mut self.failed),
			Status::Idle => None,
		}
	}

	pub fn counts(&self) -> Counts {
		Counts {
			working: self.working.len(),
			blocked: self.blocked.len(),
			pending: self.pending.len(),
			confirmed: self.confirmed.len(),
			failed: self.failed.len(),
		}
	}
}

pub fn group_sessions(sessions: &[Session], project_path: &str, confirmed: &Confirmed) -> Groups {
	let mut groups = Groups::default();
	for s in sessions.iter().filter(|s| is_under(s.cwd(), project_path)) {
		if let Some(g) = groups.get_mut(classify(s, confirmed)) {
			g.push(s.clone());
		}
	}
	let newest = |a: &Session, b: &Session| b.started().total_cmp(&a.started());
	for s in [Status::Working, Status::Blocked, Status::Pending, Status::Failed] {
		groups.get_mut(s).unwrap().sort_by(newest);
	}
	// 一起确认的（确认时间相同）再按开始时间排
	let at = |s: &Session| confirmed.get(&s.session_id).cloned().unwrap_or_default();
	groups.confirmed.sort_by(|a, b| at(b).cmp(&at(a)).then_with(|| newest(a, b)));
	groups
}

// worktree 会话归到它所属的项目目录
fn project_root(cwd: &str) -> &str {
	match cwd.find("/.claude/worktrees/") {
		Some(i) => &cwd[..i],
		None => cwd,
	}
}

// 有会话记录但还没添加的目录，按会话数倒序
pub fn suggest_paths(sessions: &[Session], added: &[String]) -> Vec<(String, usize)> {
	let added: HashSet<&str> = added.iter().map(String::as_str).collect();
	let mut counts: HashMap<&str, usize> = HashMap::new();
	for s in sessions {
		if s.cwd().is_empty() {
			continue;
		}
		let root = project_root(s.cwd());
		if added.contains(root) {
			continue;
		}
		*counts.entry(root).or_default() += 1;
	}
	let mut list: Vec<(String, usize)> = counts.into_iter().map(|(p, n)| (p.to_string(), n)).collect();
	list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
	list
}

#[cfg(test)]
mod tests {
	use super::*;

	fn bg(id: &str, cwd: &str, state: &str, started: f64) -> Session {
		Session {
			id: id.into(),
			session_id: format!("{id}-full"),
			cwd: Some(cwd.into()),
			kind: Some("background".into()),
			started_at: Some(started),
			name: Some(id.into()),
			state: Some(state.into()),
			status: None,
			title: None,
		}
	}

	#[test]
	fn classifies_by_state_not_status() {
		let mut confirmed = Confirmed::new();
		let mut s = bg("a", "/p", "done", 0.0);
		s.status = Some("busy".into());
		assert_eq!(classify(&s, &confirmed), Status::Pending);
		confirmed.insert("a-full".into(), "2026-01-01T00:00:00Z".into());
		assert_eq!(classify(&s, &confirmed), Status::Confirmed);
		assert_eq!(classify(&bg("b", "/p", "working", 0.0), &confirmed), Status::Working);
		assert_eq!(classify(&bg("b", "/p", "blocked", 0.0), &confirmed), Status::Blocked);
		assert_eq!(classify(&bg("b", "/p", "stopped", 0.0), &confirmed), Status::Failed);
		assert_eq!(classify(&bg("b", "/p", "failed", 0.0), &confirmed), Status::Failed);
		assert_eq!(classify(&bg("b", "/p", "someday-new", 0.0), &confirmed), Status::Pending);
		let interactive = Session {
			kind: Some("interactive".into()),
			status: Some("busy".into()),
			..Default::default()
		};
		assert_eq!(classify(&interactive, &confirmed), Status::Working);
		let idle = Session {
			kind: Some("interactive".into()),
			status: Some("idle".into()),
			..Default::default()
		};
		assert_eq!(classify(&idle, &confirmed), Status::Idle);
	}

	#[test]
	fn under_matches_subdirs_only() {
		assert!(is_under("/a/b", "/a/b"));
		assert!(is_under("/a/b/.claude/worktrees/x", "/a/b"));
		assert!(!is_under("/a/bc", "/a/b"));
		assert!(!is_under("", "/a/b"));
		assert!(is_under("/a/b/c", "/a/b/"));
	}

	#[test]
	fn groups_sort_newest_and_confirmed_by_time() {
		let mut confirmed = Confirmed::new();
		confirmed.insert("x-full".into(), "2026-01-02T00:00:00Z".into());
		confirmed.insert("y-full".into(), "2026-01-03T00:00:00Z".into());
		confirmed.insert("z-full".into(), "2026-01-03T00:00:00Z".into());
		let sessions = vec![
			bg("old", "/p", "done", 1.0),
			bg("new", "/p", "done", 5.0),
			bg("x", "/p", "done", 9.0),
			bg("y", "/p", "done", 1.0),
			bg("z", "/p/sub", "done", 2.0),
			bg("other", "/q", "done", 3.0),
		];
		let g = group_sessions(&sessions, "/p", &confirmed);
		assert_eq!(g.pending.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
		assert_eq!(g.confirmed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["z", "y", "x"]);
		assert_eq!(stats_for(&sessions, "/p", &confirmed).confirmed, 3);
	}

	#[test]
	fn reactivated_only_confirmed_and_running() {
		let mut confirmed = Confirmed::new();
		confirmed.insert("a-full".into(), "t".into());
		confirmed.insert("b-full".into(), "t".into());
		let sessions = vec![
			bg("a", "/p", "working", 0.0),
			bg("b", "/p", "done", 0.0),
			bg("c", "/p", "blocked", 0.0),
		];
		assert_eq!(reactivated(&sessions, &confirmed), ["a-full"]);
	}

	#[test]
	fn suggests_unadded_roots() {
		let sessions = vec![
			bg("a", "/p", "done", 0.0),
			bg("b", "/q/.claude/worktrees/w", "done", 0.0),
			bg("c", "/q", "done", 0.0),
			bg("d", "/r", "done", 0.0),
		];
		assert_eq!(suggest_paths(&sessions, &["/p".into()]), vec![("/q".into(), 2), ("/r".into(), 1)]);
	}

	#[test]
	fn parses_and_skips_bad_entries() {
		let json = br#"[{"id":"a","sessionId":"a1","kind":"background","startedAt":1,"state":"done","cwd":"/p"},{"id":5}]"#;
		let s = parse_sessions(json).unwrap();
		assert_eq!(s.len(), 1);
		assert!(parse_sessions(b"nope").is_err());
	}
}
