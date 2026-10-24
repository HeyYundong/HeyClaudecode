//! 一个 claude 子进程：stdin 写 stream-json，stdout 每行转成 AppEvent::Claude 发回主循环。

use std::path::Path;
use std::process::Stdio;

use anyhow::anyhow;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

use crate::model::{AppEvent, ConvId, PermissionDecision, PermissionRequest};

/// stderr 只保留最后这么多字节
const STDERR_KEEP: usize = 8 * 1024;

#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// 继续已有会话：--resume <session_id>
    pub resume: Option<String>,
    /// 原样追加到命令行末尾的参数（如 --model opus）
    pub extra_args: Vec<String>,
}

pub struct ClaudeProcess {
    /// 发给写任务的一行行 JSON；丢弃后 stdin 关闭
    stdin: UnboundedSender<String>,
    /// 丢弃时通知等待任务杀掉子进程
    _kill: oneshot::Sender<()>,
}

impl ClaudeProcess {
    pub fn spawn(conv: ConvId, cwd: &Path, opts: &SpawnOptions, events: UnboundedSender<AppEvent>) -> anyhow::Result<Self> {
        let mut cmd = Command::new("claude");
        cmd.args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
        ]);
        if let Some(id) = &opts.resume {
            cmd.arg("--resume").arg(id);
        }
        cmd.args(&opts.extra_args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                anyhow!("找不到 claude 命令：请先安装 Claude Code CLI，并确认 claude 在 PATH 里")
            }
            _ => anyhow!("启动 claude 失败（目录 {}）：{e}", cwd.display()),
        })?;

        let mut stdin = child.stdin.take().expect("stdin 已设为 piped");
        let stdout = child.stdout.take().expect("stdout 已设为 piped");
        let stderr = child.stderr.take().expect("stderr 已设为 piped");

        // 写任务：通道关闭后 stdin 跟着关闭，claude 会自行退出
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(mut line) = rx.recv().await {
                line.push('\n');
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // 读 stdout：每行一个 JSON，解析失败的跳过
        let out_events = events.clone();
        let out_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_str::<Value>(line)
                    && out_events.send(AppEvent::Claude { conv, msg }).is_err()
                {
                    break;
                }
            }
        });

        let err_task = tokio::spawn(collect_tail(stderr));

        // 等待任务：进程退出（或 ClaudeProcess 被丢弃时杀掉）后，等输出读完再发 ClaudeExit
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = kill_rx => {
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
            };
            let _ = out_task.await;
            let stderr = err_task.await.unwrap_or_default();
            let code = status.and_then(|s| s.code());
            let _ = events.send(AppEvent::ClaudeExit { conv, code, stderr });
        });

        let proc = Self { stdin: tx, _kill: kill_tx };
        proc.control_request(json!({ "subtype": "initialize", "hooks": null }));
        Ok(proc)
    }

    /// 发一条用户消息
    pub fn send_user(&self, text: &str) {
        self.send(json!({
            "type": "user",
            "message": { "role": "user", "content": text },
            "parent_tool_use_id": null,
            "session_id": "",
        }));
    }

    pub fn answer_permission(&self, req: &PermissionRequest, decision: PermissionDecision) {
        let response = match decision {
            PermissionDecision::AllowOnce => json!({ "behavior": "allow", "updatedInput": req.input }),
            PermissionDecision::AllowAlways => json!({
                "behavior": "allow",
                "updatedInput": req.input,
                "updatedPermissions": req.suggestions,
            }),
            PermissionDecision::Deny => json!({ "behavior": "deny", "message": "用户拒绝了这次操作" }),
        };
        self.send(json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": req.request_id, "response": response },
        }));
    }

    /// 中断当前这一轮（control_request interrupt）
    pub fn interrupt(&self) {
        self.control_request(json!({ "subtype": "interrupt" }));
    }

    fn control_request(&self, request: Value) {
        self.send(json!({
            "type": "control_request",
            "request_id": uuid::Uuid::new_v4().to_string(),
            "request": request,
        }));
    }

    /// 进程已退出时写不进去，静默丢弃，退出由 ClaudeExit 报告
    fn send(&self, msg: Value) {
        let _ = self.stdin.send(msg.to_string());
    }
}

/// 读完整个流，只保留最后 STDERR_KEEP 字节
async fn collect_tail(mut r: impl AsyncRead + Unpin) -> String {
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > STDERR_KEEP * 2 {
            tail.drain(..tail.len() - STDERR_KEEP);
        }
    }
    if tail.len() > STDERR_KEEP {
        tail.drain(..tail.len() - STDERR_KEEP);
    }
    String::from_utf8_lossy(&tail).trim().to_string()
}

/// 端到端实测，会真的调用 claude（haiku）并花一点钱：
/// `cargo test claude::process -- --ignored --nocapture`
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::time::timeout;

    use super::*;
    use crate::claude::protocol::apply;
    use crate::model::{Block, Conversation};

    fn opts(resume: Option<String>) -> SpawnOptions {
        let extra = ["--model", "haiku", "--permission-mode", "default"];
        SpawnOptions { resume, extra_args: extra.iter().map(|s| s.to_string()).collect() }
    }

    /// 处理事件直到 result；遇到新的权限请求时调用 on_perm
    async fn run_turn(
        proc: &ClaudeProcess,
        conv: &mut Conversation,
        rx: &mut UnboundedReceiver<AppEvent>,
        mut on_perm: impl FnMut(&ClaudeProcess, &PermissionRequest),
    ) -> Value {
        loop {
            let ev = timeout(Duration::from_secs(180), rx.recv()).await.expect("等待超时").expect("通道关闭");
            match ev {
                AppEvent::Claude { msg, .. } => {
                    if msg["type"] != "stream_event" {
                        eprintln!("<< {} {}", msg["type"], msg["subtype"]);
                    }
                    let had = conv.permission.as_ref().map(|p| p.request_id.clone());
                    apply(conv, &msg);
                    if let Some(p) = conv.permission.clone()
                        && had.as_deref() != Some(p.request_id.as_str())
                    {
                        on_perm(proc, &p);
                    }
                    if msg["type"] == "result" {
                        return msg;
                    }
                }
                AppEvent::ClaudeExit { code, stderr, .. } => panic!("进程提前退出 {code:?}: {stderr}"),
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn end_to_end() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/probe-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();

        // 第一轮：需要权限的 Bash
        let proc = ClaudeProcess::spawn(1, &dir, &opts(None), tx.clone()).unwrap();
        let mut conv = Conversation::new(1, dir.clone(), None);
        conv.blocks.push(Block::User("run echo".into()));
        conv.busy = true;
        proc.send_user("Use the Bash tool to run exactly this command: echo hi > out.txt  Then reply with just: done");
        let mut asked = false;
        let res = run_turn(&proc, &mut conv, &mut rx, |p, req| {
            asked = true;
            assert_eq!(req.tool_name, "Bash");
            p.answer_permission(req, PermissionDecision::AllowOnce);
        })
        .await;
        assert!(asked, "没有收到 can_use_tool");
        assert_eq!(res["is_error"], false);
        assert_eq!(std::fs::read_to_string(dir.join("out.txt")).unwrap().trim(), "hi");
        assert!(conv.blocks.iter().any(|b| matches!(b, Block::Tool { result: Some(_), .. })));
        assert!(conv.usage.context_tokens > 0);
        assert!(conv.usage.context_window.is_some());
        let session = conv.session_id.clone().expect("没有 session_id");
        eprintln!("第一轮 cost={} window={:?}", conv.usage.cost_usd, conv.usage.context_window);

        // 第二轮：开始流式输出文字后中断
        conv.busy = true;
        proc.send_user("Write a 1500-word essay about rivers. Do not use any tools.");
        let res = loop {
            let ev = timeout(Duration::from_secs(180), rx.recv()).await.unwrap().unwrap();
            let AppEvent::Claude { msg, .. } = ev else { panic!("进程提前退出") };
            let before = conv.streaming.is_empty();
            apply(&mut conv, &msg);
            if before && !conv.streaming.is_empty() {
                eprintln!(">> interrupt");
                proc.interrupt();
            }
            if msg["type"] != "stream_event" {
                eprintln!("<< {} {}", msg["type"], msg["subtype"]);
            }
            if msg["type"] == "result" {
                break msg;
            }
        };
        let mut brief = res.clone();
        for k in ["modelUsage", "model_usage_since_last_result", "usage", "subagent_stats"] {
            brief.as_object_mut().unwrap().remove(k);
        }
        eprintln!("中断后的 result：{brief}");
        eprintln!("中断后最后一块：{:?}", conv.blocks.last());
        assert!(!conv.busy);
        assert_eq!(conv.blocks.last(), Some(&Block::Notice("已中断".into())));
        assert!(conv.error.is_none());

        // 丢弃进程，应收到 ClaudeExit
        drop(proc);
        loop {
            match timeout(Duration::from_secs(30), rx.recv()).await.unwrap().unwrap() {
                AppEvent::ClaudeExit { code, stderr, .. } => {
                    eprintln!("退出码 {code:?} stderr={stderr:?}");
                    break;
                }
                AppEvent::Claude { .. } => {}
            }
        }

        // 第三轮：--resume 继续
        let proc = ClaudeProcess::spawn(1, &dir, &opts(Some(session.clone())), tx.clone()).unwrap();
        conv.busy = true;
        proc.send_user("What file did you create with echo earlier? Reply with only the file name.");
        let res = run_turn(&proc, &mut conv, &mut rx, |p, req| p.answer_permission(req, PermissionDecision::Deny)).await;
        eprintln!("resume result：{}", res["result"]);
        assert_eq!(conv.session_id.as_deref(), Some(session.as_str()));
        let last = conv.blocks.iter().rev().find_map(|b| match b {
            Block::Text(t) => Some(t.clone()),
            _ => None,
        });
        assert!(last.unwrap_or_default().contains("out.txt"));
        eprintln!("最终 cost={}", conv.usage.cost_usd);

        drop(proc);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
