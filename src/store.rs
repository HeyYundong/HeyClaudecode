use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub fn home_dir() -> PathBuf {
	std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
	match std::env::var_os("XDG_CONFIG_HOME") {
		Some(base) if !base.is_empty() => PathBuf::from(base).join("heyclaude"),
		_ => home_dir().join(".config").join("heyclaude"),
	}
}

fn projects_file() -> PathBuf {
	config_dir().join("projects.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
	pub name: String,
	pub path: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub added_at: Option<String>,
	// 手动加进 projects.json 的其他字段原样保留
	#[serde(flatten)]
	pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct ProjectsFile {
	#[serde(default)]
	projects: Vec<Project>,
}

// 像 path.resolve：相对路径接到当前目录后面，去掉 . 和 ..，不解析符号链接
fn resolve(p: &Path) -> PathBuf {
	let joined = if p.is_absolute() {
		p.to_path_buf()
	} else {
		std::env::current_dir().unwrap_or_default().join(p)
	};
	let mut out = PathBuf::from("/");
	for c in joined.components() {
		match c {
			Component::ParentDir => {
				out.pop();
			}
			Component::Normal(n) => out.push(n),
			_ => {}
		}
	}
	out
}

pub fn expand_path(input: &str) -> PathBuf {
	let p = input.trim();
	if p == "~" {
		return resolve(&home_dir());
	}
	if let Some(rest) = p.strip_prefix("~/") {
		return resolve(&home_dir().join(rest));
	}
	resolve(Path::new(p))
}

// 按名字排序，不区分大小写，数字按数值比较（project2 在 project10 前面）
pub fn sort_projects(projects: &mut [Project]) {
	projects.sort_by(|a, b| natural_cmp(&a.name, &b.name));
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
	let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
	loop {
		match (a.peek().copied(), b.peek().copied()) {
			(None, None) => return std::cmp::Ordering::Equal,
			(None, _) => return std::cmp::Ordering::Less,
			(_, None) => return std::cmp::Ordering::Greater,
			(Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
				let mut na = String::new();
				while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
					na.push(c);
					a.next();
				}
				let mut nb = String::new();
				while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
					nb.push(c);
					b.next();
				}
				let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
				let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
				if ord != std::cmp::Ordering::Equal {
					return ord;
				}
			}
			(Some(x), Some(y)) => {
				let ord = x.to_lowercase().cmp(y.to_lowercase());
				if ord != std::cmp::Ordering::Equal {
					return ord;
				}
				a.next();
				b.next();
			}
		}
	}
}

pub fn load_projects() -> Result<Vec<Project>, String> {
	let file = projects_file();
	match fs::read_to_string(&file) {
		Ok(text) => {
			let data: ProjectsFile = serde_json::from_str(&text).map_err(|e| format!("无法读取 {}: {e}", file.display()))?;
			let mut projects = data.projects;
			sort_projects(&mut projects);
			Ok(projects)
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
		Err(e) => Err(format!("无法读取 {}: {e}", file.display())),
	}
}

// 先写临时文件再改名，写到一半不会弄坏原文件
pub fn write_json_atomic(file: &Path, value: &impl Serialize) -> Result<(), String> {
	let dir = file.parent().unwrap_or(Path::new("."));
	fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	let tmp = file.with_extension("json.tmp");
	let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n";
	fs::write(&tmp, text).map_err(|e| e.to_string())?;
	fs::rename(&tmp, file).map_err(|e| e.to_string())
}

pub fn save_projects(projects: &[Project]) -> Result<(), String> {
	write_json_atomic(
		&projects_file(),
		&ProjectsFile {
			projects: projects.to_vec(),
		},
	)
}

// 校验并返回新列表和新加的项目；出错时返回中文说明
pub fn add_project(projects: &[Project], raw_path: &str) -> Result<(Vec<Project>, Project), String> {
	let resolved = if raw_path.trim().is_empty() {
		resolve(&std::env::current_dir().unwrap_or_default())
	} else {
		expand_path(raw_path)
	};
	let shown = resolved.display().to_string();
	let meta = fs::metadata(&resolved).map_err(|_| format!("目录不存在: {shown}"))?;
	if !meta.is_dir() {
		return Err(format!("不是目录: {shown}"));
	}
	if projects.iter().any(|p| p.path == shown) {
		return Err(format!("已经添加过: {shown}"));
	}
	let project = Project {
		name: resolved
			.file_name()
			.map(|n| n.to_string_lossy().into_owned())
			.unwrap_or_else(|| shown.clone()),
		path: shown,
		added_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
		extra: Default::default(),
	};
	let mut next = projects.to_vec();
	next.push(project.clone());
	sort_projects(&mut next);
	Ok((next, project))
}

pub fn remove_project(projects: &[Project], path: &str) -> Vec<Project> {
	projects.iter().filter(|p| p.path != path).cloned().collect()
}

pub fn dir_exists(p: &str) -> bool {
	fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

// 你确认过的会话：{sessionId: 确认时间 ISO 字符串}
// 只存在 HeyClaude 自己的配置目录里，不写 Claude Code 的任何数据
pub type Confirmed = BTreeMap<String, String>;

#[derive(Serialize, Deserialize, Default)]
struct ReviewsFile {
	#[serde(default)]
	confirmed: Confirmed,
}

fn reviews_file() -> PathBuf {
	config_dir().join("reviews.json")
}

pub fn load_reviews() -> Result<Confirmed, String> {
	let file = reviews_file();
	match fs::read_to_string(&file) {
		Ok(text) => serde_json::from_str::<ReviewsFile>(&text)
			.map(|r| r.confirmed)
			.map_err(|e| format!("无法读取 {}: {e}", file.display())),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Confirmed::new()),
		Err(e) => Err(format!("无法读取 {}: {e}", file.display())),
	}
}

pub fn save_reviews(confirmed: &Confirmed) -> Result<(), String> {
	write_json_atomic(
		&reviews_file(),
		&ReviewsFile {
			confirmed: confirmed.clone(),
		},
	)
}

pub fn now_iso() -> String {
	chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn with_confirmed(confirmed: &Confirmed, ids: &[String], at: &str) -> Confirmed {
	let mut next = confirmed.clone();
	for id in ids {
		next.insert(id.clone(), at.to_string());
	}
	next
}

pub fn without_confirmed(confirmed: &Confirmed, ids: &[String]) -> Confirmed {
	let mut next = confirmed.clone();
	for id in ids {
		next.remove(id);
	}
	next
}

fn common_prefix(names: &[String]) -> String {
	let mut prefix: String = names.first().cloned().unwrap_or_default();
	for name in names {
		while !name.starts_with(&prefix) {
			prefix.pop();
		}
	}
	prefix
}

// Tab 补全目录：返回补全后的输入和候选列表
pub fn complete_path(input: &str) -> (String, Vec<String>) {
	let raw = if input.is_empty() { "./" } else { input };
	let ends_with_slash = raw.ends_with('/');
	let expanded = expand_path(raw);
	let (dir, partial) = if ends_with_slash {
		(expanded.clone(), String::new())
	} else {
		(
			expanded.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/")),
			expanded.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
		)
	};
	// 保留用户原来写的前半段（例如 ~/），只替换最后一段
	let head = if ends_with_slash {
		raw.to_string()
	} else {
		raw[..raw.len().saturating_sub(partial.len())].to_string()
	};

	let Ok(entries) = fs::read_dir(&dir) else {
		return (input.to_string(), Vec::new());
	};
	let show_hidden = partial.starts_with('.');
	let mut matches: Vec<String> = entries
		.filter_map(Result::ok)
		.filter(|e| fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false))
		.map(|e| e.file_name().to_string_lossy().into_owned())
		.filter(|name| name.starts_with(&partial) && (show_hidden || !name.starts_with('.')))
		.collect();
	matches.sort();

	match matches.len() {
		0 => (input.to_string(), Vec::new()),
		1 => (format!("{head}{}/", matches[0]), Vec::new()),
		_ => (format!("{head}{}", common_prefix(&matches)), matches),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn project(name: &str) -> Project {
		Project {
			name: name.into(),
			path: format!("/{name}"),
			added_at: None,
			extra: Default::default(),
		}
	}

	#[test]
	fn sorts_naturally_ignoring_case() {
		let mut ps = vec![project("project10"), project("Beta"), project("project2"), project("alpha")];
		sort_projects(&mut ps);
		let names: Vec<_> = ps.iter().map(|p| p.name.as_str()).collect();
		assert_eq!(names, ["alpha", "Beta", "project2", "project10"]);
	}

	#[test]
	fn expands_and_normalizes() {
		assert_eq!(expand_path("/a/b/../c/./d"), PathBuf::from("/a/c/d"));
		assert_eq!(expand_path("~/x"), home_dir().join("x"));
	}

	#[test]
	fn completes_directories() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path().display().to_string();
		fs::create_dir(dir.path().join("alpha")).unwrap();
		fs::create_dir(dir.path().join("alps")).unwrap();
		fs::create_dir(dir.path().join("beta")).unwrap();
		fs::create_dir(dir.path().join(".hidden")).unwrap();
		fs::write(dir.path().join("alfile"), "").unwrap();
		let (value, candidates) = complete_path(&format!("{base}/al"));
		assert_eq!(value, format!("{base}/alp"));
		assert_eq!(candidates, ["alpha", "alps"]);
		assert_eq!(complete_path(&format!("{base}/b")).0, format!("{base}/beta/"));
		assert_eq!(complete_path(&format!("{base}/.h")).0, format!("{base}/.hidden/"));
		let (_, all) = complete_path(&format!("{base}/"));
		assert_eq!(all, ["alpha", "alps", "beta"]);
	}

	#[test]
	fn keeps_unknown_project_fields() {
		let text = r#"{"projects":[{"name":"a","path":"/a","color":"red"}]}"#;
		let data: ProjectsFile = serde_json::from_str(text).unwrap();
		let out = serde_json::to_string(&data).unwrap();
		assert!(out.contains(r#""color":"red""#));
	}
}
