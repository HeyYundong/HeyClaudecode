use chrono::{Datelike, Local, TimeZone, Timelike};
use unicode_width::UnicodeWidthStr;

use crate::sessions::{Session, Status};

// 终端里的显示宽度：中日韩文字和全角符号占两格
pub fn width(text: &str) -> usize {
	text.width()
}

// 超出宽度时截断并以 … 结尾
pub fn truncate(text: &str, max: usize) -> String {
	if width(text) <= max {
		return text.to_string();
	}
	if max == 0 {
		return String::new();
	}
	let mut out = String::new();
	let mut used = 0;
	for ch in text.chars() {
		let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
		if used + w > max - 1 {
			break;
		}
		out.push(ch);
		used += w;
	}
	out.push('…');
	out
}

// 截断后用空格补到固定宽度
pub fn fit(text: &str, cols: usize) -> String {
	let t = truncate(text, cols);
	let pad = cols.saturating_sub(width(&t));
	t + &" ".repeat(pad)
}

pub fn pad_start(text: &str, cols: usize) -> String {
	" ".repeat(cols.saturating_sub(width(text))) + text
}

// 列表里名字一栏的宽度：项目列表和任务列表用同一个算法，右侧信息紧跟在名字后面
pub fn name_column<'a>(names: impl Iterator<Item = &'a str>, columns: usize) -> usize {
	let longest = names.map(width).max().unwrap_or(0);
	(columns as i64 - 30).min(40).min(longest as i64 + 3).max(12) as usize
}

pub struct StatusInfo {
	pub icon: &'static str,
	pub label: &'static str,
}

// 「待确认」是 HeyClaude 加的：Claude 说做完了，你还没确认
pub fn info(s: Status) -> StatusInfo {
	let (icon, label) = match s {
		Status::Working => ("◌", "进行中"),
		Status::Blocked => ("●", "需要输入"),
		Status::Pending => ("◆", "待确认"),
		Status::Confirmed => ("✓", "完成"),
		Status::Failed => ("✗", "失败"),
		Status::Idle => (" ", "空闲"),
	};
	StatusInfo { icon, label }
}

// “昨天 18:20 开始”要空格，“12 分钟前开始”不要
pub fn with_suffix(text: &str, suffix: &str) -> String {
	if text.chars().last().is_some_and(|c| c.is_ascii_digit()) {
		format!("{text} {suffix}")
	} else {
		format!("{text}{suffix}")
	}
}

// 刚刚 / 10 分钟前 / 3 小时前 / 昨天 18:20 / 10月3日
pub fn relative_time(time_ms: f64, now_ms: f64) -> String {
	if !time_ms.is_finite() || time_ms <= 0.0 {
		return String::new();
	}
	let minutes = ((now_ms - time_ms).max(0.0) / 60_000.0).floor() as i64;
	if minutes < 1 {
		return "刚刚".into();
	}
	if minutes < 60 {
		return format!("{minutes} 分钟前");
	}
	let (Some(date), Some(now)) = (
		Local.timestamp_millis_opt(time_ms as i64).single(),
		Local.timestamp_millis_opt(now_ms as i64).single(),
	) else {
		return String::new();
	};
	if minutes < 24 * 60 && date.date_naive() == now.date_naive() {
		return format!("{} 小时前", minutes / 60);
	}
	if now.date_naive().pred_opt() == Some(date.date_naive()) {
		return format!("昨天 {:02}:{:02}", date.hour(), date.minute());
	}
	// 更早的只显示日期，跨年时带上年份
	let year = if date.year() == now.year() {
		String::new()
	} else {
		format!("{}年", date.year())
	};
	format!("{year}{}月{}日", date.month(), date.day())
}

pub fn now_ms() -> f64 {
	chrono::Utc::now().timestamp_millis() as f64
}

// 会话在项目里的位置：worktree 名字或子目录；就在项目目录里时不显示
pub fn where_in(session: &Session, project_path: &str) -> String {
	let cwd = session.cwd();
	if cwd == project_path {
		return String::new();
	}
	if let Some(i) = cwd.find("/.claude/worktrees/") {
		let rest = &cwd[i + "/.claude/worktrees/".len()..];
		let name = rest.split('/').next().unwrap_or("");
		if !name.is_empty() {
			return format!("worktree: {name}");
		}
	}
	match cwd.strip_prefix(&format!("{project_path}/")) {
		Some(rest) => rest.to_string(),
		None => cwd.to_string(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> f64 {
		Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap().timestamp_millis() as f64
	}

	#[test]
	fn relative_times() {
		let now = local(2026, 10, 24, 18, 0);
		assert_eq!(relative_time(now - 30_000.0, now), "刚刚");
		assert_eq!(relative_time(now - 25.0 * 60_000.0, now), "25 分钟前");
		assert_eq!(relative_time(local(2026, 10, 24, 9, 30), now), "8 小时前");
		assert_eq!(relative_time(local(2026, 10, 23, 18, 20), now), "昨天 18:20");
		assert_eq!(relative_time(local(2026, 10, 3, 1, 0), now), "10月3日");
		assert_eq!(relative_time(local(2025, 12, 31, 1, 0), now), "2025年12月31日");
		assert_eq!(relative_time(f64::NAN, now), "");
	}

	#[test]
	fn suffixes() {
		assert_eq!(with_suffix("昨天 18:20", "开始"), "昨天 18:20 开始");
		assert_eq!(with_suffix("12 分钟前", "开始"), "12 分钟前开始");
	}

	#[test]
	fn widths_and_truncation() {
		assert_eq!(width("进行中"), 6);
		assert_eq!(width("◌"), 1);
		assert_eq!(truncate("abcdef", 4), "abc…");
		assert_eq!(truncate("中文名字", 5), "中文…");
		assert_eq!(fit("ab", 4), "ab  ");
		assert_eq!(pad_start("7", 3), "  7");
		assert_eq!(name_column(["short"].into_iter(), 120), 12);
		assert_eq!(name_column(["a-very-long-session-name-here"].into_iter(), 120), 32);
		assert_eq!(name_column(["a-very-long-session-name-here"].into_iter(), 50), 20);
	}

	#[test]
	fn where_in_project() {
		let mut s = Session {
			cwd: Some("/p".into()),
			..Default::default()
		};
		assert_eq!(where_in(&s, "/p"), "");
		s.cwd = Some("/p/.claude/worktrees/readme".into());
		assert_eq!(where_in(&s, "/p"), "worktree: readme");
		s.cwd = Some("/p/sub/dir".into());
		assert_eq!(where_in(&s, "/p"), "sub/dir");
	}
}
