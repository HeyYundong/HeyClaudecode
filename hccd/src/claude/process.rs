//! 一个 claude 子进程：stdin 写 stream-json，stdout 每行转成 AppEvent::Claude 发回主循环。

use std::path::Path;

use tokio::sync::mpsc::UnboundedSender;

use crate::model::{AppEvent, ConvId, PermissionDecision, PermissionRequest};

#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// 继续已有会话：--resume <session_id>
    pub resume: Option<String>,
    /// 原样追加到命令行末尾的参数（如 --model opus）
    pub extra_args: Vec<String>,
}

pub struct ClaudeProcess {}

impl ClaudeProcess {
    pub fn spawn(_conv: ConvId, _cwd: &Path, _opts: &SpawnOptions, _events: UnboundedSender<AppEvent>) -> anyhow::Result<Self> {
        todo!()
    }

    /// 发一条用户消息
    pub fn send_user(&self, _text: &str) {
        todo!()
    }

    pub fn answer_permission(&self, _req: &PermissionRequest, _decision: PermissionDecision) {
        todo!()
    }

    /// 中断当前这一轮（control_request interrupt）
    pub fn interrupt(&self) {
        todo!()
    }
}
