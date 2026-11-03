// 新建任务时选的模型和思考强度。
// 默认值按 Claude Code 的规则找：hcc 的启动参数 → 环境变量（模型）→ 项目设置 → 用户设置
use std::path::Path;

use serde_json::Value;

use crate::store;

pub const MODELS: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

// 启动参数里 --name value 或 --name=value 的值
fn flag(args: &[String], name: &str) -> Option<String> {
	let prefix = format!("{name}=");
	let mut it = args.iter();
	while let Some(a) = it.next() {
		if a == name {
			return it.next().cloned();
		}
		if let Some(v) = a.strip_prefix(&prefix) {
			return Some(v.to_string());
		}
	}
	None
}

// 去掉 --name value / --name=value
fn strip(args: &[String], name: &str) -> Vec<String> {
	let prefix = format!("{name}=");
	let mut out = Vec::new();
	let mut it = args.iter();
	while let Some(a) = it.next() {
		if a == name {
			it.next();
		} else if !a.starts_with(&prefix) {
			out.push(a.clone());
		}
	}
	out
}

fn env(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

// 优先级从高到低：项目的 settings.local.json、settings.json，用户的 settings.json
fn settings(cwd: &str) -> Vec<Value> {
	let project = Path::new(cwd).join(".claude");
	[
		project.join("settings.local.json"),
		project.join("settings.json"),
		store::home_dir().join(".claude/settings.json"),
	]
	.iter()
	.filter_map(|p| std::fs::read_to_string(p).ok())
	.filter_map(|t| serde_json::from_str(&t).ok())
	.collect()
}

fn str_at(v: &Value, key: &str) -> Option<String> {
	v.get(key)?.as_str().map(str::to_string).filter(|s| !s.is_empty())
}

// 不指定时 claude 会用的模型；找不到就是 None（账号默认）
pub fn default_model(forwarded: &[String], cwd: &str) -> Option<String> {
	flag(forwarded, "--model")
		.or_else(|| env("ANTHROPIC_MODEL"))
		.or_else(|| settings(cwd).iter().find_map(|s| str_at(s, "model")))
}

// modelSettings 按完整模型名记，比如 claude-opus-5-5；别名 opus 就找名字里带 opus 的
fn model_effort(s: &Value, model: Option<&str>) -> Option<String> {
	let model = model?;
	let map = s.get("modelSettings")?.as_object()?;
	map.iter()
		.filter(|(k, _)| k.as_str() == model || k.contains(model))
		.find_map(|(_, v)| str_at(v, "effortLevel"))
}

// 不指定时用的思考强度：跟着选中的模型走（每个模型可以有自己的默认强度）
pub fn default_effort(forwarded: &[String], cwd: &str, model: Option<&str>) -> Option<String> {
	if let Some(e) = flag(forwarded, "--effort") {
		return Some(e);
	}
	let all = settings(cwd);
	all.iter()
		.find_map(|s| model_effort(s, model))
		.or_else(|| all.iter().find_map(|s| str_at(s, "effortLevel")))
}

// 新建任务界面上的选择；None 表示用默认
#[derive(Default, Clone, Copy)]
pub struct Choice {
	pub model: Option<usize>,
	pub effort: Option<usize>,
}

// 转一圈的顺序：默认值 → 列表里排在默认值后面的 → 绕回前面的 → 回到默认值。
// 和默认值一样的那项跳过，免得同一个名字出现两次
fn cycle(current: Option<usize>, list: &[&str], default: Option<&str>) -> Option<usize> {
	let at = default.and_then(|d| list.iter().position(|&x| x == d));
	let start = at.map_or(0, |i| i + 1);
	let order: Vec<usize> = (0..list.len())
		.map(|k| (start + k) % list.len())
		.filter(|&i| Some(i) != at)
		.collect();
	match current.and_then(|c| order.iter().position(|&i| i == c)) {
		None if current.is_none() => order.first().copied(),
		None => None,
		Some(k) => order.get(k + 1).copied(),
	}
}

impl Choice {
	pub fn next_model(&mut self, default: Option<&str>) {
		self.model = cycle(self.model, &MODELS, default);
	}

	pub fn next_effort(&mut self, default: Option<&str>) {
		self.effort = cycle(self.effort, &EFFORTS, default);
	}

	pub fn model(&self) -> Option<&'static str> {
		self.model.map(|i| MODELS[i])
	}

	pub fn effort(&self) -> Option<&'static str> {
		self.effort.map(|i| EFFORTS[i])
	}

	// 传给 claude --bg 的参数：选了的就替换掉启动参数里的同名参数
	pub fn args(&self, forwarded: &[String]) -> Vec<String> {
		let mut args = forwarded.to_vec();
		if let Some(m) = self.model() {
			args = strip(&args, "--model");
			args.extend(["--model".to_string(), m.to_string()]);
		}
		if let Some(e) = self.effort() {
			args = strip(&args, "--effort");
			args.extend(["--effort".to_string(), e.to_string()]);
		}
		args
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn v(s: &[&str]) -> Vec<String> {
		s.iter().map(|x| x.to_string()).collect()
	}

	#[test]
	fn flags() {
		assert_eq!(flag(&v(&["--model", "opus"]), "--model").as_deref(), Some("opus"));
		assert_eq!(flag(&v(&["--effort=high"]), "--effort").as_deref(), Some("high"));
		assert_eq!(flag(&v(&["--x"]), "--model"), None);
	}

	#[test]
	fn args_replace_forwarded() {
		let fwd = v(&["--model", "opus", "--effort=high", "--verbose"]);
		let c = Choice::default();
		assert_eq!(c.args(&fwd), fwd);
		let c = Choice {
			model: Some(2),
			effort: None,
		};
		assert_eq!(c.args(&fwd), v(&["--effort=high", "--verbose", "--model", "sonnet"]));
		let c = Choice {
			model: None,
			effort: Some(4),
		};
		assert_eq!(c.args(&fwd), v(&["--model", "opus", "--verbose", "--effort", "max"]));
	}

	#[test]
	fn cycles_back_to_default() {
		let mut c = Choice::default();
		for _ in 0..MODELS.len() {
			c.next_model(None);
		}
		assert_eq!(c.model(), Some("haiku"));
		c.next_model(None);
		assert_eq!(c.model(), None);
	}

	#[test]
	fn cycle_skips_default() {
		let mut seen = Vec::new();
		let mut c = Choice::default();
		loop {
			c.next_model(Some("opus"));
			match c.model() {
				Some(m) => seen.push(m),
				None => break,
			}
		}
		assert_eq!(seen, ["sonnet", "haiku", "fable"]);
	}

	#[test]
	fn effort_by_model() {
		let s: Value = serde_json::from_str(r#"{"modelSettings":{"claude-opus-5-5":{"effortLevel":"medium"}}}"#).unwrap();
		assert_eq!(model_effort(&s, Some("opus")).as_deref(), Some("medium"));
		assert_eq!(model_effort(&s, Some("sonnet")), None);
	}
}
