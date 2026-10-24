//! 读 Claude Code 存在 ~/.claude/projects 下的会话记录（只读）。
//!
//! 这是 Claude Code 的内部存储，不是公开接口：格式对不上的行直接跳过，不报错。

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{Block, SessionInfo, SessionStatus, Todo, TodoStatus, ToolResult};

/// 列表里找标题时只读文件开头和末尾各这么多字节
const SCAN_BYTES: u64 = 64 * 1024;
const TITLE_CHARS: usize = 40;
/// Claude Code 编码后的目录名超过这个长度会截断再加哈希
const MAX_ENCODED_LEN: usize = 200;
const EMPTY_TITLE: &str = "(空会话)";

fn claude_projects_dir() -> PathBuf {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")).join(".claude"),
    };
    base.join("projects")
}

/// 和 Claude Code 一致：每个非 ASCII 字母数字的字符换成 '-'。
/// 它按 JS 字符串（UTF-16 码元）替换，所以 BMP 以外的字符（如 emoji）会变成两个 '-'
fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .flat_map(|c| {
            let n = if c.is_ascii_alphanumeric() { 1 } else { c.len_utf16() };
            std::iter::repeat_n(if c.is_ascii_alphanumeric() { c } else { '-' }, n)
        })
        .collect()
}

/// cwd 对应的记录目录：projects/<编码后的 cwd>
fn transcript_dir_in(projects: &Path, cwd: &Path) -> PathBuf {
    let enc = encode_cwd(cwd);
    if enc.len() <= MAX_ENCODED_LEN {
        return projects.join(enc);
    }
    // 超长路径的目录名是“前 200 个字符-哈希”，哈希不复现，按前缀找已有目录
    let prefix = format!("{}-", &enc[..MAX_ENCODED_LEN]);
    fs::read_dir(projects)
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.starts_with(&prefix))
        })
        .map(|n| projects.join(n))
        .unwrap_or_else(|| projects.join(&enc[..MAX_ENCODED_LEN]))
}

/// 某个目录下的所有会话，新的在前。include_worktrees 为 true 时也包括 <cwd>/.claude/worktrees/* 下的会话
pub fn list_sessions(cwd: &Path, include_worktrees: bool) -> Vec<SessionInfo> {
    list_sessions_in(&claude_projects_dir(), cwd, include_worktrees)
}

/// 所有不关联项目的对话（chats_root 下每个子目录的会话），新的在前
pub fn list_chats() -> Vec<SessionInfo> {
    list_chats_in(&claude_projects_dir(), &super::store::chats_root())
}

fn list_chats_in(projects: &Path, root: &Path) -> Vec<SessionInfo> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<SessionInfo> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .flat_map(|e| list_sessions_in(projects, &e.path(), false))
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out
}

fn list_sessions_in(projects: &Path, cwd: &Path, include_worktrees: bool) -> Vec<SessionInfo> {
    let main = transcript_dir_in(projects, cwd);
    let mut dirs = vec![main];
    if include_worktrees {
        let prefix = format!("{}--claude-worktrees-", encode_cwd(cwd));
        if let Ok(rd) = fs::read_dir(projects) {
            dirs.extend(
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                    .map(|e| e.path()),
            );
        }
    }
    let mut out: Vec<SessionInfo> = dirs
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| session_info(&e.path(), cwd))
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out
}

/// 只读文件头尾各一段：头里找 cwd 和第一条用户消息，尾里找最新的自定义/AI 标题。
/// 都找不到（空文件、没有用户消息）返回 None
fn session_info(path: &Path, cwd: &Path) -> Option<SessionInfo> {
    let meta = fs::metadata(path).ok()?;
    let size = meta.len();
    if size == 0 {
        return None;
    }
    let session_id = path.file_stem()?.to_string_lossy().into_owned();
    let mut f = File::open(path).ok()?;

    let mut titles = Titles::default();
    let mut rec_cwd: Option<PathBuf> = None;
    for e in read_chunk(&mut f, 0, size.min(SCAN_BYTES)) {
        if rec_cwd.is_none() {
            rec_cwd = e.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        titles.see(&e);
    }
    if size > SCAN_BYTES {
        // 头尾重叠时只读没读过的部分
        let start = (size - SCAN_BYTES).max(SCAN_BYTES);
        for e in read_chunk(&mut f, start, size - start) {
            titles.see(&e);
        }
    }
    let title = titles.best()?;
    Some(SessionInfo {
        session_id,
        title,
        cwd: rec_cwd.unwrap_or_else(|| cwd.to_path_buf()),
        transcript: Some(path.to_path_buf()),
        updated: meta.modified().ok()?,
        status: SessionStatus::Idle,
        conv: None,
    })
}

/// 读 [start, start+len) 并逐行解析；截断处不完整的行解析失败自然跳过
fn read_chunk(f: &mut File, start: u64, len: u64) -> Vec<Value> {
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.take(len).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    String::from_utf8_lossy(&buf).lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// 标题候选：最后一次 /rename 的标题 > 最后一次 AI 生成的标题 > 第一条用户消息
#[derive(Default)]
struct Titles {
    custom: Option<String>,
    ai: Option<String>,
    first_user: Option<String>,
}

impl Titles {
    fn see(&mut self, e: &Value) {
        let field = |k: &str| e.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        match e.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                if let Some(t) = field("customTitle") {
                    self.custom = Some(t.to_string());
                }
            }
            Some("ai-title") => {
                if let Some(t) = field("aiTitle") {
                    self.ai = Some(t.to_string());
                }
            }
            Some("user") if self.first_user.is_none() => {
                if let UserMsg::Text(t) = user_msg(e) {
                    self.first_user = Some(t);
                }
            }
            _ => {}
        }
    }

    fn best(&self) -> Option<String> {
        let t = self.custom.as_ref().or(self.ai.as_ref()).or(self.first_user.as_ref())?;
        Some(truncate_title(t))
    }
}

/// 空白折叠成单个空格，超过 40 个字符时截成 39 个加 “…”
fn truncate_title(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TITLE_CHARS {
        return flat;
    }
    let mut t: String = flat.chars().take(TITLE_CHARS - 1).collect();
    t.push('…');
    t
}

enum UserMsg {
    /// 你真正输入的文字（已去掉 system-reminder）
    Text(String),
    /// [Request interrupted by user…]
    Interrupted,
    /// 上下文压缩后自动插入的摘要
    Compact,
    /// 只有 tool_result，或 meta/附属/命令输出等不算对话的内容
    Skip,
}

fn user_msg(e: &Value) -> UserMsg {
    let flag = |k: &str| e.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("isMeta") || flag("isSidechain") {
        return UserMsg::Skip;
    }
    if flag("isCompactSummary") {
        return UserMsg::Compact;
    }
    let raw = match e.pointer("/message/content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => {
            let parts: Vec<&str> = items
                .iter()
                .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                    Some("text") => b.get("text").and_then(Value::as_str),
                    Some("image") => Some("[图片]"),
                    _ => None,
                })
                .collect();
            parts.join("\n")
        }
        _ => return UserMsg::Skip,
    };
    let text = strip_reminders(&raw);
    let text = text.trim();
    if text.is_empty() {
        return UserMsg::Skip;
    }
    if text.starts_with("[Request interrupted") {
        return UserMsg::Interrupted;
    }
    const WRAPPERS: [&str; 4] = ["<command-", "<local-command-", "<task-notification", "<bash-"];
    if WRAPPERS.iter().any(|w| text.starts_with(w)) {
        return UserMsg::Skip;
    }
    UserMsg::Text(text.to_string())
}

/// 去掉 Claude Code 注入的 <system-reminder>…</system-reminder> 段
fn strip_reminders(s: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&rest[..i]);
        match rest[i..].find(CLOSE) {
            Some(j) => rest = &rest[i + j + CLOSE.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

pub struct Transcript {
    pub blocks: Vec<Block>,
    pub todos: Vec<Todo>,
    pub title: String,
}

pub fn load_transcript(path: &Path) -> anyhow::Result<Transcript> {
    let text = fs::read(path)?;
    let text = String::from_utf8_lossy(&text);
    let mut blocks: Vec<Block> = Vec::new();
    let mut todos: Vec<Todo> = Vec::new();
    let mut titles = Titles::default();
    // tool_use_id → blocks 下标，用来回填结果
    let mut tools: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
        titles.see(&e);
        if e.get("isSidechain").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        match e.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let Some(items) = e.pointer("/message/content").and_then(Value::as_array) else { continue };
                for b in items {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                                blocks.push(Block::Text(t.to_string()));
                            }
                        }
                        Some("tool_use") => {
                            let id = b.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = b.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            if let Some(t) = parse_todos(&name, &input) {
                                todos = t;
                            }
                            tools.insert(id.clone(), blocks.len());
                            blocks.push(Block::Tool { id, name, input, result: None });
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                if let Some(items) = e.pointer("/message/content").and_then(Value::as_array) {
                    for b in items.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                        let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
                        if let Some(Block::Tool { result, .. }) = tools.get(id).and_then(|&i| blocks.get_mut(i)) {
                            *result = Some(ToolResult {
                                text: result_text(b.get("content")),
                                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                            });
                        }
                    }
                }
                match user_msg(&e) {
                    UserMsg::Text(t) => blocks.push(Block::User(t)),
                    UserMsg::Interrupted => blocks.push(Block::Notice("已中断".into())),
                    UserMsg::Compact => blocks.push(Block::Notice("之前的对话已压缩".into())),
                    UserMsg::Skip => {}
                }
            }
            _ => {}
        }
    }
    Ok(Transcript { blocks, todos, title: titles.best().unwrap_or_else(|| EMPTY_TITLE.into()) })
}

/// tool_result 的 content：字符串，或 [{type:text,text}] 数组（图片记作 [图片]）
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text") => b.get("text").and_then(Value::as_str),
                Some("image") => Some("[图片]"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// TodoWrite 的 input.todos；桌面版的 update_plan 工具 input.items 结构相同，也认
fn parse_todos(name: &str, input: &Value) -> Option<Vec<Todo>> {
    let list = match name {
        "TodoWrite" => input.get("todos"),
        n if n.ends_with("__update_plan") => input.get("items"),
        _ => None,
    }?
    .as_array()?;
    Some(
        list.iter()
            .filter_map(|t| {
                let status = match t.get("status")?.as_str()? {
                    "pending" => TodoStatus::Pending,
                    "in_progress" => TodoStatus::InProgress,
                    "completed" => TodoStatus::Completed,
                    _ => return None,
                };
                Some(Todo { content: t.get("content")?.as_str()?.to_string(), status })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("hccd-sessions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_jsonl(path: &Path, entries: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut s: String = entries.iter().map(|e| format!("{e}\n")).collect();
        s.push_str("{not json\n");
        fs::write(path, s).unwrap();
    }

    fn user(content: Value) -> Value {
        json!({"type":"user","isSidechain":false,"cwd":"/real/cwd","message":{"role":"user","content":content}})
    }

    fn assistant(content: Value) -> Value {
        json!({"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":content}})
    }

    #[test]
    fn encoding() {
        assert_eq!(encode_cwd(Path::new("/home/yundong.zhang/.claude/jobs")), "-home-yundong-zhang--claude-jobs");
        assert_eq!(
            encode_cwd(Path::new("/mnt/a/HeyClaudecode/.claude/worktrees/tui-design")),
            "-mnt-a-HeyClaudecode--claude-worktrees-tui-design"
        );
        assert_eq!(encode_cwd(Path::new("/a b_c/项目😀")), "-a-b-c-----");
        let p = Path::new("/projects");
        assert_eq!(transcript_dir_in(p, Path::new("/x/y")), PathBuf::from("/projects/-x-y"));
    }

    #[test]
    fn long_path_finds_hashed_dir() {
        let root = temp_dir();
        let cwd = PathBuf::from(format!("/{}", "a".repeat(250)));
        let enc = encode_cwd(&cwd);
        let hashed = root.join(format!("{}-abc123", &enc[..MAX_ENCODED_LEN]));
        fs::create_dir_all(&hashed).unwrap();
        assert_eq!(transcript_dir_in(&root, &cwd), hashed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn titles() {
        assert_eq!(truncate_title("  a \n b  "), "a b");
        let long = "字".repeat(50);
        let t = truncate_title(&long);
        assert_eq!(t.chars().count(), 40);
        assert!(t.ends_with('…'));
        assert_eq!(strip_reminders("<system-reminder>x</system-reminder>\nhi<system-reminder>y"), "\nhi");
    }

    #[test]
    fn list_and_titles() {
        let projects = temp_dir();
        let cwd = Path::new("/p/proj");
        let dir = projects.join("-p-proj");
        // 跳过命令、meta、tool_result、system-reminder 后取第一条
        write_jsonl(
            &dir.join("s1.jsonl"),
            &[
                user(json!("<command-name>/model</command-name>")),
                json!({"type":"user","isMeta":true,"message":{"content":"meta"}}),
                json!({"type":"user","isSidechain":true,"message":{"content":"side"}}),
                user(json!([{"type":"tool_result","tool_use_id":"x","content":"r"}])),
                user(json!("<system-reminder>注入</system-reminder>\n\n  第一条   真正的消息 ")),
                user(json!("第二条")),
            ],
        );
        // 自定义标题优先于 AI 标题，取最后一次
        write_jsonl(
            &dir.join("s2.jsonl"),
            &[
                user(json!("hello")),
                json!({"type":"ai-title","aiTitle":"ai title"}),
                json!({"type":"custom-title","customTitle":"old"}),
                json!({"type":"custom-title","customTitle":"改过的名字"}),
            ],
        );
        write_jsonl(&dir.join("s3.jsonl"), &[json!({"type":"user","message":{"content":"[Request interrupted by user]"}}), json!({"type":"ai-title","aiTitle":"only ai"})]);
        // 空文件和没有用户消息的跳过
        fs::write(dir.join("empty.jsonl"), "").unwrap();
        write_jsonl(&dir.join("none.jsonl"), &[assistant(json!([{"type":"text","text":"hi"}]))]);
        fs::write(dir.join("notes.txt"), "x").unwrap();
        // worktree 目录
        write_jsonl(&projects.join("-p-proj--claude-worktrees-wt").join("w1.jsonl"), &[user(json!("wt msg"))]);
        write_jsonl(&projects.join("-p-proj-other").join("o1.jsonl"), &[user(json!("other"))]);

        let list = list_sessions_in(&projects, cwd, false);
        let mut got: Vec<(String, String)> = list.iter().map(|s| (s.session_id.clone(), s.title.clone())).collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("s1".to_string(), "第一条 真正的消息".to_string()),
                ("s2".into(), "改过的名字".into()),
                ("s3".into(), "only ai".into()),
            ]
        );
        let s1 = list.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!(s1.cwd, PathBuf::from("/real/cwd"));
        assert_eq!(s1.status, SessionStatus::Idle);
        assert_eq!(s1.transcript.as_deref(), Some(dir.join("s1.jsonl").as_path()));
        let s3 = list.iter().find(|s| s.session_id == "s3").unwrap();
        assert_eq!(s3.cwd, cwd);

        let all = list_sessions_in(&projects, cwd, true);
        assert_eq!(all.len(), 4);
        assert!(all.iter().any(|s| s.session_id == "w1"));
        assert!(list_sessions_in(&projects, Path::new("/nope"), true).is_empty());
        fs::remove_dir_all(projects).unwrap();
    }

    #[test]
    fn big_file_title_from_tail() {
        let projects = temp_dir();
        let pad = "x".repeat(1000);
        let mut entries = vec![user(json!("开头的消息"))];
        entries.extend((0..200).map(|_| assistant(json!([{"type":"text","text":pad}]))));
        entries.push(json!({"type":"custom-title","customTitle":"尾部标题"}));
        write_jsonl(&projects.join("-c").join("big.jsonl"), &entries);
        let list = list_sessions_in(&projects, Path::new("/c"), false);
        assert_eq!(list[0].title, "尾部标题");
        fs::remove_dir_all(projects).unwrap();
    }

    #[test]
    fn chats() {
        let projects = temp_dir();
        let root = temp_dir();
        for (i, id) in ["a1", "b2"].iter().enumerate() {
            let chat = root.join(id);
            fs::create_dir_all(&chat).unwrap();
            write_jsonl(&transcript_dir_in(&projects, &chat).join(format!("{i}.jsonl")), &[user(json!(*id))]);
        }
        fs::create_dir_all(root.join("empty")).unwrap();
        let list = list_chats_in(&projects, &root);
        assert_eq!(list.len(), 2);
        fs::remove_dir_all(projects).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transcript() {
        let dir = temp_dir();
        let path = dir.join("t.jsonl");
        write_jsonl(
            &path,
            &[
                user(json!("<local-command-stdout>x</local-command-stdout>")),
                user(json!([{"type":"text","text":"请修改"},{"type":"image","source":{}}])),
                assistant(json!([{"type":"thinking","thinking":"hmm"}])),
                assistant(json!([{"type":"text","text":"好的"}])),
                assistant(json!([{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}}])),
                assistant(json!([{"type":"tool_use","id":"t2","name":"TodoWrite","input":{"todos":[
                    {"content":"一","status":"completed","activeForm":"x"},
                    {"content":"二","status":"in_progress"}]}}])),
                json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"side"}]}}),
                user(json!([{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"文件内容"}]}])),
                user(json!([{"type":"tool_result","tool_use_id":"t2","content":"ok","is_error":true}])),
                assistant(json!([{"type":"tool_use","id":"t3","name":"TodoWrite","input":{"todos":[
                    {"content":"一","status":"completed"},{"content":"二","status":"pending"}]}}])),
                user(json!([{"type":"text","text":"[Request interrupted by user for tool use]"}])),
                json!({"type":"user","isCompactSummary":true,"message":{"content":"summary"}}),
                json!({"type":"system","content":"x"}),
            ],
        );
        let t = load_transcript(&path).unwrap();
        assert_eq!(t.title, "请修改 [图片]");
        assert_eq!(t.blocks.len(), 7);
        assert_eq!(t.blocks[0], Block::User("请修改\n[图片]".into()));
        assert_eq!(t.blocks[1], Block::Text("好的".into()));
        assert_eq!(
            t.blocks[2],
            Block::Tool {
                id: "t1".into(),
                name: "Read".into(),
                input: json!({"file_path":"/a"}),
                result: Some(ToolResult { text: "文件内容".into(), is_error: false }),
            }
        );
        let Block::Tool { result, .. } = &t.blocks[3] else { panic!() };
        assert_eq!(result.as_ref().unwrap(), &ToolResult { text: "ok".into(), is_error: true });
        let Block::Tool { result, .. } = &t.blocks[4] else { panic!() };
        assert!(result.is_none());
        assert_eq!(t.blocks[5], Block::Notice("已中断".into()));
        assert_eq!(t.blocks[6], Block::Notice("之前的对话已压缩".into()));
        assert_eq!(
            t.todos,
            [
                Todo { content: "一".into(), status: TodoStatus::Completed },
                Todo { content: "二".into(), status: TodoStatus::Pending },
            ]
        );
        fs::write(&path, "").unwrap();
        assert_eq!(load_transcript(&path).unwrap().title, EMPTY_TITLE);
        assert!(load_transcript(&dir.join("missing.jsonl")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
