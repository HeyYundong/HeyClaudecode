use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

// 单行输入框：光标可以左右移动，在光标处插入和删除；太长时折成多行显示
#[derive(Default)]
pub struct Input {
	// 按字符存，中文和 emoji 也是一个字符一格光标
	chars: Vec<char>,
	pos: usize,
}

impl Input {
	pub fn value(&self) -> String {
		self.chars.iter().collect()
	}

	pub fn insert(&mut self, text: &str) {
		let text: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
		let n = text.len();
		self.chars.splice(self.pos..self.pos, text);
		self.pos += n;
	}

	// 处理编辑按键，返回 false 表示不是编辑键（Enter、Ctrl+Z 等交给调用方）
	pub fn key(&mut self, key: &KeyEvent) -> bool {
		let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
		match key.code {
			KeyCode::Left => self.pos = self.pos.saturating_sub(1),
			KeyCode::Right => self.pos = (self.pos + 1).min(self.chars.len()),
			KeyCode::Home => self.pos = 0,
			KeyCode::End => self.pos = self.chars.len(),
			KeyCode::Char('a') if ctrl => self.pos = 0,
			KeyCode::Char('e') if ctrl => self.pos = self.chars.len(),
			KeyCode::Char('u') if ctrl => *self = Self::default(),
			KeyCode::Backspace => {
				if self.pos > 0 {
					self.pos -= 1;
					self.chars.remove(self.pos);
				}
			}
			KeyCode::Delete => {
				if self.pos < self.chars.len() {
					self.chars.remove(self.pos);
				}
			}
			KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
				if !c.is_control() {
					self.insert(&c.to_string());
				}
			}
			_ => return false,
		}
		true
	}

	// 显示成若干行：第一行以 prompt 开头，续行用空格缩进到同一列；光标是反色的那个字符。
	// 按显示宽度折行，中文占两格；光标在末尾时它自己也占一格
	pub fn lines(&self, prompt: Span<'static>, placeholder: &str, cols: usize) -> Vec<Line<'static>> {
		let indent = prompt.width();
		let cols = cols.saturating_sub(indent).max(2);
		let cursor = Style::default().add_modifier(Modifier::REVERSED);
		let mut rows: Vec<Vec<Span<'static>>> = vec![vec![]];
		let mut used = 0;
		let mut text = String::new();
		// 光标在末尾时多画一个空格当光标
		let tail = (self.pos == self.chars.len()).then_some(' ');
		for (i, ch) in self.chars.iter().copied().chain(tail).enumerate() {
			let w = ch.width().unwrap_or(0);
			if used + w > cols {
				rows.last_mut().unwrap().push(Span::raw(std::mem::take(&mut text)));
				rows.push(vec![]);
				used = 0;
			}
			used += w;
			if i == self.pos {
				rows.last_mut().unwrap().push(Span::raw(std::mem::take(&mut text)));
				rows.last_mut().unwrap().push(Span::styled(ch.to_string(), cursor));
			} else {
				text.push(ch);
			}
		}
		rows.last_mut().unwrap().push(Span::raw(text));
		if self.chars.is_empty() && !placeholder.is_empty() {
			rows[0].push(Span::styled(placeholder.to_string(), Style::default().add_modifier(Modifier::DIM)));
		}
		rows.into_iter()
			.enumerate()
			.map(|(i, mut row)| {
				row.insert(0, if i == 0 { prompt.clone() } else { Span::raw(" ".repeat(indent)) });
				Line::from(row)
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ratatui::crossterm::event::KeyEvent;

	fn press(input: &mut Input, code: KeyCode) {
		input.key(&KeyEvent::new(code, KeyModifiers::NONE));
	}

	fn plain(lines: &[Line]) -> Vec<String> {
		lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
	}

	fn cursor_at(lines: &[Line]) -> Option<(usize, String)> {
		lines.iter().enumerate().find_map(|(i, l)| {
			l.spans
				.iter()
				.find(|s| s.style.add_modifier.contains(Modifier::REVERSED))
				.map(|s| (i, s.content.to_string()))
		})
	}

	#[test]
	fn edits_at_cursor() {
		let mut input = Input::default();
		input.insert("修复bug");
		press(&mut input, KeyCode::Left);
		press(&mut input, KeyCode::Left);
		press(&mut input, KeyCode::Left);
		input.insert("登录");
		assert_eq!(input.value(), "修复登录bug");
		press(&mut input, KeyCode::Backspace);
		assert_eq!(input.value(), "修复登bug");
		press(&mut input, KeyCode::Delete);
		assert_eq!(input.value(), "修复登ug");
		press(&mut input, KeyCode::Home);
		press(&mut input, KeyCode::Backspace);
		assert_eq!(input.value(), "修复登ug");
		press(&mut input, KeyCode::End);
		press(&mut input, KeyCode::Right);
		input.insert("!");
		assert_eq!(input.value(), "修复登ug!");
	}

	#[test]
	fn wraps_by_display_width() {
		let mut input = Input::default();
		input.insert("一二三四五六");
		// 提示符两格，正文每行 6 格：三个汉字一行，末尾的光标折到第三行
		let lines = input.lines(Span::raw("❯ "), "", 8);
		assert_eq!(plain(&lines), ["❯ 一二三", "  四五六", "   "]);
		assert_eq!(cursor_at(&lines), Some((2, " ".into())));
		press(&mut input, KeyCode::Home);
		let lines = input.lines(Span::raw("❯ "), "", 8);
		assert_eq!(plain(&lines), ["❯ 一二三", "  四五六"]);
		assert_eq!(cursor_at(&lines), Some((0, "一".into())));
	}

	#[test]
	fn shows_placeholder_after_cursor() {
		let lines = Input::default().lines(Span::raw("❯ "), "要做什么？", 40);
		assert_eq!(plain(&lines), ["❯  要做什么？"]);
		assert_eq!(cursor_at(&lines), Some((0, " ".into())));
	}
}
