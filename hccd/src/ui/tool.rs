//! 工具调用的摘要、Edit 的 diff 等，对话区和权限框共用。

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::text::{self, pad, truncate};
use super::theme::*;

fn str_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

/// 一行摘要：Bash→command，Read/Write/Edit→路径，Grep/Glob→pattern…
pub fn summary(name: &str, input: &Value, cwd: &Path) -> String {
    let s = match name {
        "Bash" => str_field(input, "command").map(|c| c.lines().next().unwrap_or("").to_string()),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => str_field(input, "file_path")
            .or_else(|| str_field(input, "notebook_path"))
            .map(|p| rel_path(p, cwd)),
        "Grep" | "Glob" => str_field(input, "pattern").map(|p| match str_field(input, "path") {
            Some(path) => format!("{p}  {}", rel_path(path, cwd)),
            None => p.to_string(),
        }),
        "WebSearch" => str_field(input, "query").map(str::to_string),
        "WebFetch" => str_field(input, "url").map(str::to_string),
        "Task" | "Agent" => str_field(input, "description").map(str::to_string),
        "TodoWrite" => input.get("todos").and_then(Value::as_array).map(|a| format!("{} 项待办", a.len())),
        _ => None,
    };
    s.or_else(|| {
        input
            .as_object()
            .and_then(|o| o.values().find_map(Value::as_str))
            .map(|v| v.lines().next().unwrap_or("").to_string())
    })
    .unwrap_or_default()
}

pub fn is_edit(name: &str) -> bool {
    matches!(name, "Edit" | "MultiEdit")
}

/// Edit / MultiEdit 的 (old, new) 对
fn edit_pairs(input: &Value) -> Vec<(String, String)> {
    let pair = |v: &Value| {
        Some((
            v.get("old_string")?.as_str()?.to_string(),
            v.get("new_string")?.as_str()?.to_string(),
        ))
    };
    if let Some(edits) = input.get("edits").and_then(Value::as_array) {
        edits.iter().filter_map(pair).collect()
    } else {
        pair(input).into_iter().collect()
    }
}

enum DiffLine {
    Ctx(String),
    Del(String),
    Add(String),
}

fn diff_lines(input: &Value) -> Vec<DiffLine> {
    let mut out = Vec::new();
    for (n, (old, new)) in edit_pairs(input).iter().enumerate() {
        if n > 0 {
            out.push(DiffLine::Ctx("…".into()));
        }
        let a: Vec<&str> = old.split('\n').collect();
        let b: Vec<&str> = new.split('\n').collect();
        // 去掉首尾相同的行，各留一行上下文
        let mut p = 0;
        while p < a.len() && p < b.len() && a[p] == b[p] {
            p += 1;
        }
        let mut s = 0;
        while s < a.len() - p && s < b.len() - p && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
            s += 1;
        }
        if p > 0 {
            out.push(DiffLine::Ctx(a[p - 1].to_string()));
        }
        out.extend(a[p..a.len() - s].iter().map(|l| DiffLine::Del(l.to_string())));
        out.extend(b[p..b.len() - s].iter().map(|l| DiffLine::Add(l.to_string())));
        if s > 0 {
            out.push(DiffLine::Ctx(a[a.len() - s].to_string()));
        }
    }
    out
}

/// 增删行数
pub fn diff_stats(input: &Value) -> (usize, usize) {
    let lines = diff_lines(input);
    let add = lines.iter().filter(|l| matches!(l, DiffLine::Add(_))).count();
    let del = lines.iter().filter(|l| matches!(l, DiffLine::Del(_))).count();
    (add, del)
}

/// 画成带底色的 diff 行，最多 max 行。indent 是左侧缩进，width 是总宽度
pub fn diff_view(input: &Value, indent: usize, width: usize, max: usize) -> Vec<Line<'static>> {
    let lines = diff_lines(input);
    let inner = width.saturating_sub(indent + 1);
    let mut out = Vec::new();
    for l in lines.iter().take(max) {
        let (mark, body, style) = match l {
            DiffLine::Ctx(s) => ("  ", s, dim()),
            DiffLine::Del(s) => ("- ", s, Style::new().fg(DEL_FG).bg(DEL_BG)),
            DiffLine::Add(s) => ("+ ", s, Style::new().fg(ADD_FG).bg(ADD_BG)),
        };
        let body = truncate(&text::clean(body), inner.saturating_sub(2));
        let mut line = Line::from(vec![Span::raw(" ".repeat(indent)), Span::styled(format!("{mark}{body}"), style)]);
        pad(&mut line, indent + inner, style);
        out.push(line);
    }
    if lines.len() > max {
        out.push(Line::from(vec![
            Span::raw(" ".repeat(indent)),
            Span::styled(format!("… 另外 {} 行", lines.len() - max), dim()),
        ]));
    }
    out
}

/// 权限框标题
pub fn permission_title(name: &str) -> String {
    match name {
        "Bash" => "Claude 想运行这条命令".into(),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "Claude 想修改文件".into(),
        "Read" => "Claude 想读取文件".into(),
        "Grep" | "Glob" => "Claude 想搜索文件".into(),
        "WebFetch" => "Claude 想访问网页".into(),
        "WebSearch" => "Claude 想搜索网络".into(),
        "Task" | "Agent" => "Claude 想启动子任务".into(),
        n => format!("Claude 想使用工具 {n}"),
    }
}

/// 结果的行数（去掉末尾空行）
pub fn line_count(s: &str) -> usize {
    s.trim_end_matches('\n').lines().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summaries() {
        let cwd = Path::new("/p");
        assert_eq!(summary("Bash", &json!({"command": "ls -la\npwd"}), cwd), "ls -la");
        assert_eq!(summary("Read", &json!({"file_path": "/p/src/a.rs"}), cwd), "src/a.rs");
        assert_eq!(summary("WebFetch", &json!({"url": "https://x"}), cwd), "https://x");
        assert_eq!(summary("Foo", &json!({"n": 1, "x": "first"}), cwd), "first");
    }

    #[test]
    fn diff() {
        let input = json!({"old_string": "a\nb\nc", "new_string": "a\nB\nB2\nc"});
        assert_eq!(diff_stats(&input), (2, 1));
        let lines = diff_view(&input, 4, 40, 12);
        assert_eq!(lines.len(), 5);
    }
}
