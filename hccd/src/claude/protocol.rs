//! 解析 stream-json 消息并更新对话状态。

use serde_json::Value;

use crate::model::{Block, Conversation, PermissionRequest, Todo, TodoStatus, ToolResult};

/// 标题最多取第一条用户消息的这么多个字符
const TITLE_CHARS: usize = 40;

/// 把 claude 输出的一行消息应用到对话上
pub fn apply(conv: &mut Conversation, msg: &Value) {
    match msg["type"].as_str() {
        Some("system") => apply_system(conv, msg),
        Some("stream_event") => {
            if is_main(msg) {
                let ev = &msg["event"];
                if ev["type"] == "content_block_delta"
                    && ev["delta"]["type"] == "text_delta"
                    && let Some(t) = ev["delta"]["text"].as_str()
                {
                    conv.streaming.push_str(t);
                }
            }
        }
        Some("assistant") if is_main(msg) => apply_assistant(conv, &msg["message"]),
        Some("user") if is_main(msg) => apply_tool_results(conv, &msg["message"]["content"]),
        Some("control_request") => {
            let req = &msg["request"];
            if req["subtype"] == "can_use_tool" {
                conv.permission = Some(PermissionRequest {
                    request_id: str_of(&msg["request_id"]),
                    tool_name: str_of(&req["tool_name"]),
                    input: req["input"].clone(),
                    description: req["description"].as_str().filter(|s| !s.is_empty()).map(String::from),
                    suggestions: req["permission_suggestions"].clone(),
                });
            }
        }
        // claude 撤回了还没回答的权限请求（比如这一轮被中断）
        Some("control_cancel_request") => {
            if conv.permission.as_ref().is_some_and(|p| msg["request_id"] == p.request_id.as_str()) {
                conv.permission = None;
            }
        }
        Some("result") => apply_result(conv, msg),
        _ => {}
    }
    update_title(conv);
}

/// parent_tool_use_id 非空的消息属于子 agent
fn is_main(msg: &Value) -> bool {
    msg["parent_tool_use_id"].is_null()
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn apply_system(conv: &mut Conversation, msg: &Value) {
    match msg["subtype"].as_str() {
        Some("init") => {
            if let Some(id) = msg["session_id"].as_str() {
                conv.session_id = Some(id.to_string());
            }
            if let Some(m) = msg["model"].as_str() {
                conv.model = Some(m.to_string());
            }
        }
        Some("permission_denied") => {
            let tool = msg["tool_name"].as_str().unwrap_or("工具");
            let text = match msg["message"].as_str().filter(|s| !s.is_empty()) {
                Some(m) => format!("{tool} 被拒绝：{m}"),
                None => format!("{tool} 被拒绝"),
            };
            conv.blocks.push(Block::Notice(text));
        }
        _ => {}
    }
}

fn apply_assistant(conv: &mut Conversation, message: &Value) {
    let u = &message["usage"];
    if u.is_object() {
        conv.usage.context_tokens = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .iter()
            .map(|k| u[k].as_u64().unwrap_or(0))
            .sum();
    }
    let Some(content) = message["content"].as_array() else { return };
    for item in content {
        match item["type"].as_str() {
            Some("text") => {
                let text = item["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    conv.blocks.push(Block::Text(text.to_string()));
                }
                conv.streaming.clear();
            }
            Some("tool_use") => {
                let id = str_of(&item["id"]);
                // 同一个块不重复加入
                if conv.blocks.iter().any(|b| matches!(b, Block::Tool { id: i, .. } if *i == id)) {
                    continue;
                }
                let name = str_of(&item["name"]);
                let input = item["input"].clone();
                update_todos(conv, &name, &input);
                conv.blocks.push(Block::Tool { id, name, input, result: None });
            }
            _ => {}
        }
    }
}

fn parse_status(v: &Value) -> Option<TodoStatus> {
    match v.as_str()? {
        "pending" => Some(TodoStatus::Pending),
        "in_progress" => Some(TodoStatus::InProgress),
        "completed" => Some(TodoStatus::Completed),
        _ => None,
    }
}

/// TodoWrite 每次给出完整列表；TaskCreate/TaskUpdate 逐条增改，taskId 是创建顺序（从 1 起）
fn update_todos(conv: &mut Conversation, name: &str, input: &Value) {
    match name {
        "TodoWrite" => {
            if let Some(list) = input["todos"].as_array() {
                conv.todos = list
                    .iter()
                    .map(|t| Todo {
                        content: str_of(&t["content"]),
                        status: parse_status(&t["status"]).unwrap_or(TodoStatus::Pending),
                    })
                    .collect();
            }
        }
        "TaskCreate" => {
            let content = input["subject"].as_str().or(input["description"].as_str()).unwrap_or_default();
            conv.todos.push(Todo { content: content.to_string(), status: TodoStatus::Pending });
        }
        "TaskUpdate" => {
            let id = match &input["taskId"] {
                Value::String(s) => s.trim().parse::<usize>().ok(),
                v => v.as_u64().map(|n| n as usize),
            };
            if let Some(todo) = id.and_then(|i| i.checked_sub(1)).and_then(|i| conv.todos.get_mut(i)) {
                if let Some(s) = parse_status(&input["status"]) {
                    todo.status = s;
                }
                if let Some(subject) = input["subject"].as_str() {
                    todo.content = subject.to_string();
                }
            }
        }
        _ => {}
    }
}

fn apply_tool_results(conv: &mut Conversation, content: &Value) {
    // 字符串 content 是回放的用户文字，忽略
    let Some(items) = content.as_array() else { return };
    for item in items.iter().filter(|i| i["type"] == "tool_result") {
        let id = item["tool_use_id"].as_str().unwrap_or_default();
        let text = match &item["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        let is_error = item["is_error"].as_bool().unwrap_or(false);
        if let Some(Block::Tool { result, .. }) =
            conv.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if i == id))
        {
            *result = Some(ToolResult { text, is_error });
        }
    }
}

fn apply_result(conv: &mut Conversation, msg: &Value) {
    conv.busy = false;
    conv.turn_started = None;
    conv.permission = None;
    if !conv.streaming.is_empty() {
        let text = std::mem::take(&mut conv.streaming);
        conv.blocks.push(Block::Text(text));
    }

    // total_cost_usd 是这个进程启动以来的累计值，--resume 重启后会从 0 算起，
    // 所以优先累加本轮的 model_usage_since_last_result
    match msg["model_usage_since_last_result"].as_object() {
        Some(per_turn) => {
            conv.usage.cost_usd += per_turn.values().filter_map(|m| m["costUSD"].as_f64()).sum::<f64>();
        }
        None => {
            if let Some(total) = msg["total_cost_usd"].as_f64() {
                conv.usage.cost_usd = conv.usage.cost_usd.max(total);
            }
        }
    }

    if let Some(models) = msg["modelUsage"].as_object() {
        let current = conv.model.as_deref().and_then(|m| models.get(m));
        let window = current
            .and_then(|m| m["contextWindow"].as_u64())
            .or_else(|| models.values().filter_map(|m| m["contextWindow"].as_u64()).max());
        if window.is_some() {
            conv.usage.context_window = window;
        }
    }

    // 被 interrupt 打断时 is_error 为 true、terminal_reason 为 aborted_*，这不算出错
    let aborted = msg["terminal_reason"].as_str().is_some_and(|r| r.starts_with("aborted"));
    if aborted {
        conv.blocks.push(Block::Notice("已中断".into()));
        conv.error = None;
    } else if msg["is_error"].as_bool().unwrap_or(false) {
        let detail = msg["result"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                msg["errors"].as_array().map(|e| e.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("；"))
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| str_of(&msg["subtype"]));
        conv.blocks.push(Block::Notice(format!("出错了：{detail}")));
        conv.error = Some(detail);
    } else {
        conv.error = None;
    }
}

/// 标题还是默认值时，用第一条用户消息
fn update_title(conv: &mut Conversation) {
    if conv.title != "新对话" {
        return;
    }
    if let Some(text) = conv.blocks.iter().find_map(|b| match b {
        Block::User(t) => Some(t),
        _ => None,
    }) {
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        if line.is_empty() {
            return;
        }
        let mut title: String = line.chars().take(TITLE_CHARS).collect();
        if line.chars().count() > TITLE_CHARS {
            title.push('…');
        }
        conv.title = title;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn conv() -> Conversation {
        let mut c = Conversation::new(1, PathBuf::from("/tmp"), None);
        c.busy = true;
        c.turn_started = Some(std::time::Instant::now());
        c
    }

    fn assistant(content: Value) -> Value {
        json!({
            "type": "assistant",
            "message": {
                "model": "claude-haiku-5-5", "id": "msg_1", "type": "message", "role": "assistant",
                "content": content,
                "usage": {"input_tokens": 2, "cache_creation_input_tokens": 9805, "cache_read_input_tokens": 11335, "output_tokens": 16}
            },
            "parent_tool_use_id": null,
            "session_id": "s1"
        })
    }

    fn delta(text: &str) -> Value {
        json!({
            "type": "stream_event",
            "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}},
            "parent_tool_use_id": null,
            "session_id": "s1"
        })
    }

    #[test]
    fn init_sets_session_and_model() {
        let mut c = conv();
        apply(&mut c, &json!({"type": "system", "subtype": "init", "session_id": "abc", "model": "claude-haiku-5-5", "cwd": "/tmp", "permissionMode": "default"}));
        assert_eq!(c.session_id.as_deref(), Some("abc"));
        assert_eq!(c.model.as_deref(), Some("claude-haiku-5-5"));
    }

    #[test]
    fn unknown_messages_are_ignored() {
        let mut c = conv();
        apply(&mut c, &json!({"type": "system", "subtype": "status", "status": "requesting"}));
        apply(&mut c, &json!({"type": "system", "subtype": "compact_boundary"}));
        apply(&mut c, &json!({"type": "rate_limit_event", "rate_limit_info": {}}));
        apply(&mut c, &json!({"type": "control_response", "response": {"subtype": "success", "request_id": "x"}}));
        assert!(c.blocks.is_empty());
        assert!(c.busy);
    }

    #[test]
    fn text_delta_then_assistant_text() {
        let mut c = conv();
        apply(&mut c, &delta("你好"));
        apply(&mut c, &delta("，世界"));
        apply(&mut c, &json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "嗯"}}, "parent_tool_use_id": null}));
        assert_eq!(c.streaming, "你好，世界");
        apply(&mut c, &assistant(json!([{"type": "text", "text": "你好，世界"}])));
        assert!(c.streaming.is_empty());
        assert_eq!(c.blocks, vec![Block::Text("你好，世界".into())]);
        assert_eq!(c.usage.context_tokens, 2 + 9805 + 11335);
    }

    #[test]
    fn thinking_is_ignored() {
        let mut c = conv();
        apply(&mut c, &assistant(json!([{"type": "thinking", "thinking": "", "signature": "xx"}])));
        assert!(c.blocks.is_empty());
    }

    #[test]
    fn tool_use_and_result() {
        let mut c = conv();
        let tool = json!([{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}]);
        apply(&mut c, &assistant(tool.clone()));
        // 重复的块不再加入
        apply(&mut c, &assistant(tool));
        assert_eq!(c.blocks.len(), 1);
        apply(&mut c, &json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "a.txt\nb.txt", "is_error": false}]},
            "parent_tool_use_id": null, "session_id": "s1"
        }));
        let Block::Tool { name, input, result, .. } = &c.blocks[0] else { panic!() };
        assert_eq!(name, "Bash");
        assert_eq!(input["command"], "ls");
        assert_eq!(result, &Some(ToolResult { text: "a.txt\nb.txt".into(), is_error: false }));
    }

    #[test]
    fn tool_result_array_content() {
        let mut c = conv();
        apply(&mut c, &assistant(json!([{"type": "tool_use", "id": "t2", "name": "Read", "input": {}}])));
        apply(&mut c, &json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t2", "is_error": true,
                "content": [{"type": "text", "text": "第一行"}, {"type": "tool_reference", "tool_name": "X"}, {"type": "text", "text": "第二行"}]}]},
            "parent_tool_use_id": null
        }));
        let Block::Tool { result, .. } = &c.blocks[0] else { panic!() };
        assert_eq!(result, &Some(ToolResult { text: "第一行\n第二行".into(), is_error: true }));
    }

    #[test]
    fn replayed_user_text_ignored() {
        let mut c = conv();
        apply(&mut c, &json!({"type": "user", "message": {"role": "user", "content": "hello"}, "parent_tool_use_id": null}));
        apply(&mut c, &json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "hello"}]}, "parent_tool_use_id": null}));
        assert!(c.blocks.is_empty());
    }

    #[test]
    fn subagent_messages_skipped() {
        let mut c = conv();
        let mut m = assistant(json!([{"type": "text", "text": "子 agent"}]));
        m["parent_tool_use_id"] = json!("toolu_task");
        apply(&mut c, &m);
        let mut d = delta("子");
        d["parent_tool_use_id"] = json!("toolu_task");
        apply(&mut c, &d);
        assert!(c.blocks.is_empty());
        assert!(c.streaming.is_empty());
        assert_eq!(c.usage.context_tokens, 0);
    }

    #[test]
    fn todo_write() {
        let mut c = conv();
        apply(&mut c, &assistant(json!([{"type": "tool_use", "id": "t", "name": "TodoWrite", "input": {"todos": [
            {"content": "alpha", "status": "completed", "activeForm": "Doing alpha"},
            {"content": "beta", "status": "in_progress", "activeForm": "Doing beta"},
            {"content": "gamma", "status": "pending", "activeForm": "Doing gamma"}
        ]}}])));
        assert_eq!(c.todos, vec![
            Todo { content: "alpha".into(), status: TodoStatus::Completed },
            Todo { content: "beta".into(), status: TodoStatus::InProgress },
            Todo { content: "gamma".into(), status: TodoStatus::Pending },
        ]);
    }

    #[test]
    fn task_create_update() {
        let mut c = conv();
        apply(&mut c, &assistant(json!([
            {"type": "tool_use", "id": "a", "name": "TaskCreate", "input": {"subject": "alpha", "description": "do alpha"}},
            {"type": "tool_use", "id": "b", "name": "TaskCreate", "input": {"description": "beta"}},
            {"type": "tool_use", "id": "c", "name": "TaskUpdate", "input": {"taskId": "2", "status": "in_progress"}},
            {"type": "tool_use", "id": "d", "name": "TaskUpdate", "input": {"taskId": 1, "status": "completed"}},
            {"type": "tool_use", "id": "e", "name": "TaskUpdate", "input": {"taskId": "9", "status": "completed"}}
        ])));
        assert_eq!(c.todos, vec![
            Todo { content: "alpha".into(), status: TodoStatus::Completed },
            Todo { content: "beta".into(), status: TodoStatus::InProgress },
        ]);
    }

    #[test]
    fn can_use_tool_and_cancel() {
        let mut c = conv();
        apply(&mut c, &json!({
            "type": "control_request", "request_id": "req-1",
            "request": {"subtype": "can_use_tool", "tool_name": "Bash", "display_name": "Bash",
                "input": {"command": "echo hi > out.txt", "description": "Write hi"},
                "description": "Write \"hi\" into out.txt",
                "permission_suggestions": [{"type": "addRules", "rules": [{"toolName": "Bash"}], "behavior": "allow", "destination": "session"}],
                "blocked_path": "/tmp/out.txt", "tool_use_id": "toolu_x"}
        }));
        let p = c.permission.as_ref().unwrap();
        assert_eq!(p.request_id, "req-1");
        assert_eq!(p.tool_name, "Bash");
        assert_eq!(p.input["command"], "echo hi > out.txt");
        assert_eq!(p.description.as_deref(), Some("Write \"hi\" into out.txt"));
        assert!(p.suggestions.is_array());
        apply(&mut c, &json!({"type": "control_cancel_request", "request_id": "other"}));
        assert!(c.permission.is_some());
        apply(&mut c, &json!({"type": "control_cancel_request", "request_id": "req-1"}));
        assert!(c.permission.is_none());
    }

    #[test]
    fn permission_denied_notice() {
        let mut c = conv();
        apply(&mut c, &json!({"type": "system", "subtype": "permission_denied", "tool_name": "Bash", "message": "不允许"}));
        assert_eq!(c.blocks, vec![Block::Notice("Bash 被拒绝：不允许".into())]);
    }

    fn result(cost_total: f64, cost_turn: f64, is_error: bool) -> Value {
        json!({
            "type": "result", "subtype": if is_error { "error_during_execution" } else { "success" },
            "is_error": is_error, "stop_reason": "end_turn", "session_id": "s1",
            "total_cost_usd": cost_total, "result": if is_error { "API Error: 500" } else { "ok" },
            "usage": {"input_tokens": 6},
            "modelUsage": {"claude-haiku-5-5": {"costUSD": cost_total, "contextWindow": 1000000, "maxOutputTokens": 128000}},
            "model_usage_since_last_result": {"claude-haiku-5-5": {"costUSD": cost_turn, "contextWindow": 1000000}}
        })
    }

    #[test]
    fn result_finishes_turn() {
        let mut c = conv();
        c.model = Some("claude-haiku-5-5".into());
        apply(&mut c, &delta("没收到完整消息的文字"));
        apply(&mut c, &result(0.004, 0.004, false));
        assert!(!c.busy);
        assert!(c.turn_started.is_none());
        assert!(c.streaming.is_empty());
        assert_eq!(c.blocks, vec![Block::Text("没收到完整消息的文字".into())]);
        assert_eq!(c.usage.context_window, Some(1_000_000));
        // 第二轮：total 是累计值，按本轮增量累加
        apply(&mut c, &result(0.005, 0.001, false));
        assert!((c.usage.cost_usd - 0.005).abs() < 1e-9);
        assert!(c.error.is_none());
    }

    #[test]
    fn result_without_per_turn_usage_uses_total() {
        let mut c = conv();
        let mut r = result(0.003, 0.0, false);
        r.as_object_mut().unwrap().remove("model_usage_since_last_result");
        apply(&mut c, &r);
        assert!((c.usage.cost_usd - 0.003).abs() < 1e-9);
    }

    #[test]
    fn result_error() {
        let mut c = conv();
        apply(&mut c, &result(0.0, 0.0, true));
        assert_eq!(c.error.as_deref(), Some("API Error: 500"));
        assert_eq!(c.blocks, vec![Block::Notice("出错了：API Error: 500".into())]);
        assert!(!c.busy);
    }

    #[test]
    fn result_interrupted() {
        let mut c = conv();
        c.usage.cost_usd = 0.002;
        apply(&mut c, &delta("写到一半"));
        // 实测的中断 result：没有 model_usage_since_last_result
        apply(&mut c, &json!({
            "type": "result", "subtype": "error_during_execution", "is_error": true, "stop_reason": null,
            "terminal_reason": "aborted_streaming", "total_cost_usd": 0.002, "session_id": "s1",
            "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"]
        }));
        assert!(!c.busy);
        assert!(c.error.is_none());
        assert_eq!(c.blocks, vec![Block::Text("写到一半".into()), Block::Notice("已中断".into())]);
        assert!((c.usage.cost_usd - 0.002).abs() < 1e-9);
    }

    #[test]
    fn title_from_first_user_message() {
        let mut c = conv();
        c.blocks.push(Block::User("\n帮我把这个项目里所有的 TODO 注释都整理成一个清单，按文件分组，再估一下工作量".into()));
        apply(&mut c, &delta("x"));
        assert_eq!(c.title.chars().count(), TITLE_CHARS + 1);
        assert!(c.title.starts_with("帮我把这个项目"));
        assert!(c.title.ends_with('…'));
        let before = c.title.clone();
        c.blocks.push(Block::User("第二条".into()));
        apply(&mut c, &delta("y"));
        assert_eq!(c.title, before);
    }
}
