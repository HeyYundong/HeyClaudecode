// hagy：用同一套界面管理 Antigravity CLI（agy）的任务。
//
// agy 没有 claude 那样的 `agents --json` 和 `--bg`，所以：
//   启动任务  在项目目录里后台跑 `agy -p <内容> --output-format json`，脱离终端，
//             HeyClaude 退出也不影响。任务记录（目录、内容、进程号、输出、退出码）
//             放在 HeyClaude 自己的数据目录 agy-jobs/<任务>/ 里
//   读会话    自己的任务记录 + agy 的会话缓存 cache/conversation_metadata.json（标题、目录），
//             以及 presence/<会话>.lock：agy 打开一个会话时一直锁着这个文件，
//             从 /proc/locks 能看出谁锁着，锁着就是进行中
//   打开会话  `agy --conversation <ID>`
// agy 的数据目录是它的内部存储，不是公开接口：只读不写，读不到就当没有。
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sessions::Session;
use crate::store::{self, home_dir};

// agy 的数据目录
pub fn agy_dir() -> PathBuf {
	match std::env::var_os("HAGY_AGY_DIR") {
		Some(dir) if !dir.is_empty() => PathBuf::from(dir),
		_ => home_dir().join(".gemini").join("antigravity-cli"),
	}
}

pub fn presence_dir() -> PathBuf {
	agy_dir().join("presence")
}

pub fn cache_dir() -> PathBuf {
	agy_dir().join("cache")
}

// hagy 启动的任务记录
pub fn jobs_dir() -> PathBuf {
	store::data_dir().join("agy-jobs")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
	cwd: String,
	prompt: String,
	// 毫秒
	started_at: f64,
	// 后台 sh 的进程号，agy 是它的子进程
	pid: u32,
}

// agy -p --output-format json 的结果，只取用得到的字段。出错时退出码也是 0，要看 status
#[derive(Debug, Default, Deserialize)]
struct Output {
	#[serde(default)]
	conversation_id: String,
	#[serde(default)]
	status: String,
}

fn parse_output(text: &str) -> Option<Output> {
	// 正常只有一行 JSON；保险起见从后往前找第一行能解析的
	text.lines()
		.rev()
		.filter(|l| l.trim_start().starts_with('{'))
		.find_map(|l| serde_json::from_str(l).ok())
}

// /proc/locks 里被持有的锁：设备:inode → 持有者进程号。等锁的行（带 ->）不算
fn held_locks(text: &str) -> HashMap<String, u32> {
	let mut held = HashMap::new();
	for line in text.lines() {
		let fields: Vec<&str> = line.split_whitespace().collect();
		if fields.contains(&"->") || fields.len() < 6 {
			continue;
		}
		if let Ok(pid) = fields[4].parse() {
			held.insert(fields[5].to_string(), pid);
		}
	}
	held
}

// 和 /proc/locks 一样的写法：主设备号:次设备号（两位十六进制）:inode
fn lock_key(meta: &fs::Metadata) -> String {
	let dev = meta.dev();
	format!("{:02x}:{:02x}:{}", libc::major(dev), libc::minor(dev), meta.ino())
}

// 现在被打开着的会话：会话 ID → 打开它的 agy 进程号
fn open_conversations() -> HashMap<String, u32> {
	let held = held_locks(&fs::read_to_string("/proc/locks").unwrap_or_default());
	let mut open = HashMap::new();
	for entry in fs::read_dir(presence_dir()).into_iter().flatten().flatten() {
		let path = entry.path();
		if path.extension().is_none_or(|e| e != "lock") {
			continue;
		}
		let (Some(id), Ok(meta)) = (path.file_stem().and_then(|s| s.to_str()), entry.metadata()) else {
			continue;
		};
		if let Some(&pid) = held.get(&lock_key(&meta)) {
			open.insert(id.to_string(), pid);
		}
	}
	open
}

// /proc/<pid>/stat 里的父进程号和状态；进程名里可能有空格和括号，从最后一个 ) 后面开始数
fn proc_stat(pid: u32) -> Option<(char, u32)> {
	let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
	let rest = &text[text.rfind(')')? + 1..];
	let mut fields = rest.split_whitespace();
	let state = fields.next()?.chars().next()?;
	let ppid = fields.next()?.parse().ok()?;
	Some((state, ppid))
}

// 任务的后台 sh 还在跑：进程在、不是僵尸，而且命令行里有这个任务的目录（防止进程号被复用）
fn job_alive(pid: u32, dir: &Path) -> bool {
	if !matches!(proc_stat(pid), Some((state, _)) if state != 'Z') {
		return false;
	}
	let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
	let dir = dir.as_os_str().as_encoded_bytes();
	cmdline.windows(dir.len()).any(|w| w == dir)
}

fn first_line(text: &str) -> String {
	text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

fn short_id(id: &str) -> String {
	id.chars().take(8).collect()
}

fn parse_time_ms(s: &str) -> Option<f64> {
	let t = chrono::DateTime::parse_from_rfc3339(s).ok()?.timestamp_millis();
	// 没填的时间是 0001-01-01
	(t > 0).then_some(t as f64)
}

// file:///a/b%20c → /a/b c
fn uri_to_path(uri: &str) -> Option<String> {
	let rest = uri.strip_prefix("file://")?;
	let bytes = rest.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%'
			&& let Some(Ok(b)) = rest.get(i + 1..i + 3).map(|h| u8::from_str_radix(h, 16))
		{
			out.push(b);
			i += 3;
			continue;
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8(out).ok()
}

// agy 缓存里的会话：标题、目录、更新时间
struct Known {
	title: String,
	cwd: Option<String>,
	updated: Option<f64>,
	steps: u64,
}

fn known_conversations(text: &str) -> HashMap<String, Known> {
	let mut known = HashMap::new();
	let Ok(value) = serde_json::from_str::<Value>(text) else {
		return known;
	};
	let Some(map) = value.get("conversations").and_then(Value::as_object) else {
		return known;
	};
	for (id, entry) in map {
		let Some(s) = entry.get("summary") else { continue };
		// 只要 CLI 自己的会话：内部会话不算，桌面版的会话 agy --conversation 打不开
		if s.get("Internal").and_then(Value::as_bool).unwrap_or(false)
			|| s.get("AppDataDir").and_then(Value::as_str) != Some("antigravity-cli")
		{
			continue;
		}
		let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
		let title = Some(text("Title"))
			.filter(|t| !t.is_empty())
			.unwrap_or_else(|| first_line(&text("Preview")));
		let cwd = s
			.get("WorkspaceURIs")
			.and_then(Value::as_array)
			.and_then(|a| a.first())
			.and_then(Value::as_str)
			.and_then(uri_to_path);
		let updated =
			parse_time_ms(&text("UpdatedAt")).or_else(|| entry.get("last_modified_time").and_then(Value::as_str).and_then(parse_time_ms));
		let steps = s.get("NumSteps").and_then(Value::as_u64).unwrap_or(0);
		known.insert(
			id.clone(),
			Known {
				title,
				cwd,
				updated,
				steps,
			},
		);
	}
	known
}

fn session(id: &str, cwd: Option<String>, started: Option<f64>, name: String, state: &str) -> Session {
	Session {
		id: short_id(id),
		session_id: id.to_string(),
		cwd,
		kind: Some("background".into()),
		started_at: started,
		name: Some(name).filter(|n| !n.is_empty()),
		state: Some(state.into()),
		status: None,
		title: None,
	}
}

pub fn fetch_sessions() -> Result<Vec<Session>, String> {
	let open = open_conversations();
	let known = known_conversations(&fs::read_to_string(cache_dir().join("conversation_metadata.json")).unwrap_or_default());
	let mut sessions = Vec::new();
	let mut seen = HashSet::new();

	// hagy 启动的任务
	for entry in fs::read_dir(jobs_dir()).into_iter().flatten().flatten() {
		let dir = entry.path();
		let Some(job) = fs::read_to_string(dir.join("job.json"))
			.ok()
			.and_then(|t| serde_json::from_str::<Job>(&t).ok())
		else {
			continue;
		};
		let exit = fs::read_to_string(dir.join("exit")).ok();
		let output = fs::read_to_string(dir.join("out.json")).ok().and_then(|t| parse_output(&t));
		// 跑完了从输出里拿会话 ID；还在跑就找 sh 的子进程锁着的会话
		let conversation = output
			.as_ref()
			.map(|o| o.conversation_id.clone())
			.filter(|id| !id.is_empty())
			.or_else(|| {
				open.iter()
					.find(|&(_, &pid)| pid == job.pid || proc_stat(pid).is_some_and(|(_, ppid)| ppid == job.pid))
					.map(|(id, _)| id.clone())
			});
		let state = if conversation.as_ref().is_some_and(|id| open.contains_key(id)) {
			"working"
		} else if exit.is_some() {
			match &output {
				Some(o) if o.status == "SUCCESS" => "done",
				_ => "failed",
			}
		} else if job_alive(job.pid, &dir) {
			"working"
		} else {
			// 没写下退出码就没了：被杀掉了
			"stopped"
		};
		let job_id = format!("job:{}", entry.file_name().to_string_lossy());
		let id = conversation.unwrap_or(job_id);
		let info = known.get(&id);
		let name = info
			.map(|k| k.title.clone())
			.filter(|t| !t.is_empty())
			.unwrap_or_else(|| first_line(&job.prompt));
		let mut s = session(&id, Some(job.cwd), Some(job.started_at), name, state);
		if id.starts_with("job:") {
			s.id = String::new();
		}
		seen.insert(id);
		sessions.push(s);
	}

	// agy 里直接开的会话
	for (id, k) in &known {
		if seen.contains(id) || (k.steps == 0 && !open.contains_key(id)) {
			continue;
		}
		seen.insert(id.clone());
		let state = if open.contains_key(id) { "working" } else { "done" };
		let mut s = session(id, k.cwd.clone(), k.updated, k.title.clone(), state);
		if open.contains_key(id) {
			s.kind = Some("interactive".into());
			s.status = Some("busy".into());
		}
		sessions.push(s);
	}

	// 刚开、缓存里还没有的会话：目录看打开它的进程在哪
	for (id, &pid) in &open {
		if seen.contains(id) {
			continue;
		}
		let cwd = fs::read_link(format!("/proc/{pid}/cwd")).ok().map(|p| p.display().to_string());
		let started = fs::metadata(presence_dir().join(format!("{id}.lock")))
			.ok()
			.and_then(|m| m.modified().ok())
			.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
			.map(|d| d.as_millis() as f64);
		let mut s = session(id, cwd, started, String::new(), "working");
		s.kind = Some("interactive".into());
		s.status = Some("busy".into());
		sessions.push(s);
	}
	Ok(sessions)
}

// 后台 sh：在任务目录里记下 agy 的输出和退出码。$1 是任务目录，后面是给 agy 的参数
const RUNNER: &str = r#"d=$1; shift; agy "$@" > "$d/out.json" 2> "$d/err.txt"; echo $? > "$d/exit""#;

fn new_job_id() -> String {
	let now = chrono::Local::now();
	format!(
		"{}-{:04x}",
		now.format("%Y%m%d-%H%M%S"),
		(now.timestamp_subsec_nanos() ^ std::process::id()) & 0xffff
	)
}

// 在项目目录里后台启动 agy -p，返回任务的短 ID
pub fn dispatch(cwd: &str, prompt: &str, args: &[String]) -> Result<String, String> {
	let id = new_job_id();
	let dir = jobs_dir().join(&id);
	fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}: {e}", dir.display()))?;
	let mut cmd = Command::new("sh");
	cmd.arg("-c")
		.arg(RUNNER)
		.arg("sh")
		.arg(&dir)
		.args(args)
		.args(["--output-format", "json", "-p", prompt])
		.current_dir(cwd)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	// 自己一个会话：不受 HeyClaude 所在终端的挂断和 Ctrl+C 影响，HeyClaude 退出后照样跑完
	unsafe {
		cmd.pre_exec(|| {
			if libc::setsid() == -1 {
				return Err(std::io::Error::last_os_error());
			}
			Ok(())
		});
	}
	let mut child = cmd.spawn().map_err(|e| {
		let _ = fs::remove_dir_all(&dir);
		e.to_string()
	})?;
	let job = Job {
		cwd: cwd.to_string(),
		prompt: prompt.to_string(),
		started_at: crate::text::now_ms(),
		pid: child.id(),
	};
	store::write_json_atomic(&dir.join("job.json"), &job)?;
	// 跑完时回收，免得留下僵尸进程
	std::thread::spawn(move || child.wait());
	Ok(id)
}

// 新建任务时默认的模型：启动参数 → agy 的设置。设置里存的是显示名（Gemini 4 Neon (XHigh)），
// 换成 --model 认的名字（gemini-4-neon），括号里的是思考强度
fn settings_model() -> Option<String> {
	let text = fs::read_to_string(agy_dir().join("settings.json")).ok()?;
	let v: Value = serde_json::from_str(&text).ok()?;
	v.get("model")?.as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

fn split_display(name: &str) -> (String, Option<String>) {
	let (base, effort) = match name.rfind('(') {
		Some(i) if name.ends_with(')') => (&name[..i], Some(name[i + 1..name.len() - 1].trim().to_lowercase())),
		_ => (name, None),
	};
	let id = base.split_whitespace().collect::<Vec<_>>().join("-").to_lowercase();
	(id, effort)
}

pub fn default_model() -> Option<String> {
	settings_model().map(|m| split_display(&m).0)
}

pub fn default_effort(model: Option<&str>) -> Option<String> {
	let (id, effort) = split_display(&settings_model()?);
	// 换了别的模型就不知道它默认用多大的强度了
	match model {
		Some(m) if m != id => None,
		_ => effort,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_proc_locks() {
		let text = "1: FLOCK  ADVISORY  WRITE 32753 08:02:29887407 0 EOF\n\
			2: -> FLOCK  ADVISORY  WRITE 99 08:02:29887407 0 EOF\n\
			3: POSIX  ADVISORY  READ 2479 08:02:23330831 128 128\n";
		let held = held_locks(text);
		assert_eq!(held.get("08:02:29887407"), Some(&32753));
		assert_eq!(held.get("08:02:23330831"), Some(&2479));
		assert_eq!(held.len(), 2);
	}

	#[test]
	fn reads_output() {
		let ok = r#"{"conversation_id":"abc","status":"SUCCESS","response":"DONE\n"}"#;
		let o = parse_output(ok).unwrap();
		assert_eq!((o.conversation_id.as_str(), o.status.as_str()), ("abc", "SUCCESS"));
		let err = "warning\n{\"conversation_id\":\"\",\"status\":\"ERROR\",\"error\":\"bad\"}\n";
		assert_eq!(parse_output(err).unwrap().status, "ERROR");
		assert!(parse_output("").is_none());
	}

	#[test]
	fn decodes_uris() {
		assert_eq!(uri_to_path("file:///a/b%20c").as_deref(), Some("/a/b c"));
		assert_eq!(uri_to_path("file:///a/%E4%B8%AD").as_deref(), Some("/a/中"));
		assert_eq!(uri_to_path("/a"), None);
	}

	#[test]
	fn reads_cached_conversations() {
		let text = r#"{"conversations":{
			"a":{"summary":{"Title":"Fix it","WorkspaceURIs":["file:///p"],"UpdatedAt":"2026-11-24T03:34:22Z","NumSteps":3,"AppDataDir":"antigravity-cli","Internal":false}},
			"b":{"summary":{"Title":"","Preview":"\nhello\nmore","WorkspaceURIs":[],"UpdatedAt":"0001-01-01T00:00:00Z","NumSteps":0,"AppDataDir":"antigravity-cli"},"last_modified_time":"2026-11-24T06:40:00Z"},
			"c":{"summary":{"Title":"desktop","AppDataDir":"antigravity"}},
			"d":{"is_internal":true}
		}}"#;
		let k = known_conversations(text);
		assert_eq!(k.len(), 2);
		assert_eq!(k["a"].title, "Fix it");
		assert_eq!(k["a"].cwd.as_deref(), Some("/p"));
		assert_eq!(k["a"].steps, 3);
		assert_eq!(k["b"].title, "hello");
		assert_eq!(k["b"].cwd, None);
		assert!(k["b"].updated.is_some());
	}

	#[test]
	fn display_names_to_model_ids() {
		assert_eq!(
			split_display("Gemini 4 Neon (XHigh)"),
			("gemini-4-neon".into(), Some("xhigh".into()))
		);
		assert_eq!(split_display("gemini-3.8-flash"), ("gemini-3.8-flash".into(), None));
	}
}
