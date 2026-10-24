use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::store::{Confirmed, home_dir};

// 读 Claude Code 存在本机的对话记录，判断一个会话在某个时间之后有没有新的对话。
// 这是 Claude Code 的内部存储，不是公开接口：只读不写；找不到文件或格式变了就当作“不知道”，
// 退回到只看 claude agents --json 的逻辑。

pub fn claude_dir() -> PathBuf {
	match std::env::var_os("CLAUDE_CONFIG_DIR") {
		Some(base) if !base.is_empty() => PathBuf::from(base),
		_ => home_dir().join(".claude"),
	}
}

fn projects_dir() -> PathBuf {
	claude_dir().join("projects")
}

// 只有你发的消息和 Claude 的回复算对话；斜杠命令、命令输出和附属记录不算
fn is_conversation(entry: &serde_json::Value) -> bool {
	if entry.get("timestamp").and_then(|t| t.as_str()).is_none_or(str::is_empty) {
		return false;
	}
	if entry.get("isMeta").and_then(|m| m.as_bool()).unwrap_or(false) {
		return false;
	}
	match entry.get("type").and_then(|t| t.as_str()) {
		Some("assistant") => true,
		Some("user") => {
			let content = entry.pointer("/message/content");
			let text = match content {
				Some(serde_json::Value::String(s)) => s.as_str(),
				Some(serde_json::Value::Array(items))
					if items.first().and_then(|i| i.get("type")).and_then(|t| t.as_str()) == Some("text") =>
				{
					items[0].get("text").and_then(|t| t.as_str()).unwrap_or("")
				}
				_ => "",
			};
			let t = text.trim_start();
			!(t.starts_with("<command-name") || t.starts_with("<local-command"))
		}
		_ => false,
	}
}

const TAIL_BYTES: u64 = 256 * 1024;

fn parse_time_ms(s: &str) -> Option<i64> {
	chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
}

// 记录文件里最后一条对话的时间（毫秒），读不到返回 None。只读文件末尾一段
fn last_conversation_time(file: &Path) -> Option<i64> {
	let mut f = File::open(file).ok()?;
	let size = f.metadata().ok()?.len();
	let length = size.min(TAIL_BYTES);
	f.seek(SeekFrom::Start(size - length)).ok()?;
	let mut buffer = Vec::with_capacity(length as usize);
	f.take(length).read_to_end(&mut buffer).ok()?;
	let text = String::from_utf8_lossy(&buffer);
	// 从头截断的第一行可能不完整，倒着找，解析失败的跳过
	for line in text.split('\n').rev() {
		if line.is_empty() {
			continue;
		}
		let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
			continue;
		};
		if is_conversation(&entry) {
			return parse_time_ms(entry["timestamp"].as_str()?);
		}
	}
	None
}

// 会话 ID → 记录文件路径。文件位置不会变，找到一次就记住
#[derive(Default)]
pub struct Transcripts {
	paths: HashMap<String, PathBuf>,
	// 上次读过的文件修改时间和读到的结果，文件没变就不再读
	seen: HashMap<PathBuf, (std::time::SystemTime, Option<i64>)>,
}

impl Transcripts {
	fn find(&mut self, session_id: &str) -> Option<PathBuf> {
		if let Some(p) = self.paths.get(session_id)
			&& p.exists()
		{
			return Some(p.clone());
		}
		let dir = projects_dir();
		let name = format!("{session_id}.jsonl");
		for d in fs::read_dir(&dir).ok()?.filter_map(Result::ok) {
			if !d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
				continue;
			}
			let file = d.path().join(&name);
			if file.exists() {
				self.paths.insert(session_id.to_string(), file.clone());
				return Some(file);
			}
		}
		None
	}

	// 确认之后又有新对话的会话 ID。先看文件修改时间，没晚于确认时间的直接跳过，不读内容
	pub fn replied_after_confirm(&mut self, confirmed: &Confirmed) -> Vec<String> {
		let mut ids = Vec::new();
		for (session_id, at) in confirmed {
			let Some(confirmed_at) = parse_time_ms(at) else { continue };
			let Some(file) = self.find(session_id) else { continue };
			let Ok(mtime) = fs::metadata(&file).and_then(|m| m.modified()) else {
				continue;
			};
			let mtime_ms = mtime
				.duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_millis() as i64)
				.unwrap_or(0);
			if mtime_ms <= confirmed_at {
				continue;
			}
			let last = match self.seen.get(&file) {
				Some((m, last)) if *m == mtime => *last,
				_ => {
					let last = last_conversation_time(&file);
					self.seen.insert(file.clone(), (mtime, last));
					last
				}
			};
			if last.is_some_and(|t| t > confirmed_at) {
				ids.push(session_id.clone());
			}
		}
		ids
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn conversation_entries() {
		assert!(is_conversation(&json!({"type": "assistant", "timestamp": "t"})));
		assert!(is_conversation(
			&json!({"type": "user", "timestamp": "t", "message": {"content": "hi"}})
		));
		assert!(is_conversation(
			&json!({"type": "user", "timestamp": "t", "message": {"content": [{"type": "text", "text": "hi"}]}})
		));
		assert!(!is_conversation(
			&json!({"type": "user", "timestamp": "t", "message": {"content": "<command-name>/status"}})
		));
		assert!(!is_conversation(
			&json!({"type": "user", "timestamp": "t", "message": {"content": " <local-command-stdout>"}})
		));
		assert!(!is_conversation(&json!({"type": "assistant", "timestamp": "t", "isMeta": true})));
		assert!(!is_conversation(&json!({"type": "assistant"})));
		assert!(!is_conversation(&json!({"type": "summary", "timestamp": "t"})));
	}

	#[test]
	fn finds_last_conversation_skipping_commands() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("s.jsonl");
		let lines = [
			json!({"type": "user", "timestamp": "2026-01-01T00:00:00Z", "message": {"content": "hi"}}).to_string(),
			json!({"type": "assistant", "timestamp": "2026-01-01T00:01:00Z"}).to_string(),
			json!({"type": "user", "timestamp": "2026-01-01T00:05:00Z", "message": {"content": "<command-name>/status"}}).to_string(),
			"{broken".to_string(),
		];
		fs::write(&file, lines.join("\n") + "\n").unwrap();
		assert_eq!(last_conversation_time(&file), parse_time_ms("2026-01-01T00:01:00Z"));
	}
}
