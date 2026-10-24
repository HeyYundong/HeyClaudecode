use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use notify::{EventKind, RecursiveMode, Watcher};

use crate::sessions::{Session, fetch_sessions};
use crate::store::Confirmed;
use crate::transcripts::{Transcripts, claude_dir};

// 后台线程发回界面线程的消息
pub enum Msg {
	// 一次刷新的结果；revived 是确认之后又有过新对话的会话
	Fetched {
		result: Result<Vec<Session>, String>,
		revived: Vec<String>,
	},
	// claude --bg 启动新任务的结果
	Dispatched(Result<String, String>),
	// Claude Code 的任务状态文件有变化
	Changed,
}

// 刷新线程：收到请求就调用一次 claude agents，再检查对话记录。
// 排队的请求合并成一次，所以连着请求也只拉起一个 claude 进程
pub fn spawn_fetcher(tx: Sender<Msg>) -> Sender<Confirmed> {
	let (req_tx, req_rx): (Sender<Confirmed>, Receiver<Confirmed>) = channel();
	thread::spawn(move || {
		let mut transcripts = Transcripts::default();
		while let Ok(mut confirmed) = req_rx.recv() {
			while let Ok(newer) = req_rx.try_recv() {
				confirmed = newer;
			}
			let result = fetch_sessions();
			let revived = if result.is_ok() {
				transcripts.replied_after_confirm(&confirmed)
			} else {
				Vec::new()
			};
			if tx.send(Msg::Fetched { result, revived }).is_err() {
				break;
			}
		}
	});
	req_tx
}

fn is_state_file(path: &Path) -> bool {
	let in_sessions = path.parent().and_then(Path::file_name).is_some_and(|n| n == "sessions");
	let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
	name == "state.json" || (in_sessions && name.ends_with(".json"))
}

// 监听 ~/.claude/jobs/*/state.json（后台任务状态）和 ~/.claude/sessions（进程状态）。
// 这两处是 Claude Code 的内部文件，只用来知道“该刷新了”，内容仍然从 claude agents 读。
// 不递归监听 jobs：每个任务的 tmp 目录里可能有成千上万个文件。新任务目录出现时再单独加上。
// 监听在自己的线程里建立，不耽误界面启动；失败也没关系，界面还会定时刷新
pub fn spawn_watcher(tx: Sender<Msg>) {
	thread::spawn(move || {
		let (event_tx, event_rx) = channel::<notify::Result<notify::Event>>();
		let Ok(mut watcher) = notify::recommended_watcher(event_tx) else {
			return;
		};
		let base = claude_dir();
		let jobs = base.join("jobs");
		let _ = watcher.watch(&base.join("sessions"), RecursiveMode::NonRecursive);
		let _ = watcher.watch(&jobs, RecursiveMode::NonRecursive);
		for entry in std::fs::read_dir(&jobs).into_iter().flatten().flatten() {
			if entry.file_type().is_ok_and(|t| t.is_dir()) {
				let _ = watcher.watch(&entry.path(), RecursiveMode::NonRecursive);
			}
		}
		for event in event_rx {
			let Ok(event) = event else { continue };
			if matches!(event.kind, EventKind::Access(_)) {
				continue;
			}
			for path in &event.paths {
				if matches!(event.kind, EventKind::Create(_)) && path.parent() == Some(jobs.as_path()) && path.is_dir() {
					let _ = watcher.watch(path, RecursiveMode::NonRecursive);
					let _ = tx.send(Msg::Changed);
				}
			}
			if event.paths.iter().any(|p| is_state_file(p)) && tx.send(Msg::Changed).is_err() {
				break;
			}
		}
	});
}

// 用 claude --bg 在项目目录里启动后台任务，返回它打印的 ID
pub fn dispatch_task(tx: Sender<Msg>, cwd: String, prompt: String, forwarded: Vec<String>) {
	thread::spawn(move || {
		let output = Command::new("claude")
			.args(&forwarded)
			.arg("--bg")
			.arg(&prompt)
			.current_dir(&cwd)
			.stdin(Stdio::null())
			.output();
		let last_line = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().lines().last().unwrap_or("").to_string();
		let result = match output {
			Err(e) => Err(e.to_string()),
			Ok(out) if !out.status.success() => {
				let err = last_line(&out.stderr);
				Err(if err.is_empty() {
					format!("退出码 {}", out.status.code().unwrap_or(-1))
				} else {
					err
				})
			}
			Ok(out) => Ok(last_line(&out.stdout)),
		};
		let _ = tx.send(Msg::Dispatched(result));
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn state_files() {
		assert!(is_state_file(Path::new("/h/.claude/jobs/abc/state.json")));
		assert!(is_state_file(Path::new("/h/.claude/sessions/123.json")));
		assert!(!is_state_file(Path::new("/h/.claude/jobs/abc/timeline.jsonl")));
		assert!(!is_state_file(Path::new("/h/.claude/jobs/abc/tmp/x.json")));
	}
}
