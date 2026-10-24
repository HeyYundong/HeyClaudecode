//! 各模块共用的数据结构。claude/、data/、ui/ 和 app 都只通过这里的类型交流。

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use serde_json::Value;

/// hccd 内部给每个对话分配的编号。session_id 要等 claude 启动后才知道，所以不用它当键
pub type ConvId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionStatus {
    /// 正在跑一轮
    Working,
    /// 等你确认工具权限
    Blocked,
    /// 这一轮结束了，或者是历史会话
    Idle,
    /// claude 进程异常退出
    Failed,
}

/// 侧栏里的一条会话：可能是历史记录，也可能是 hccd 正在运行的
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    /// 第一条用户消息（截断），没有就是 "(空会话)"
    pub title: String,
    pub cwd: PathBuf,
    /// 记录文件路径（~/.claude/projects/<编码后的 cwd>/<id>.jsonl），新建还没落盘的会话为 None
    pub transcript: Option<PathBuf>,
    pub updated: SystemTime,
    pub status: SessionStatus,
    /// hccd 正在托管它时的对话编号
    pub conv: Option<ConvId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

/// 对话里的一段内容，按出现顺序排列
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    User(String),
    /// Claude 的文字回复（markdown 原文）
    Text(String),
    Tool {
        id: String,
        name: String,
        /// 工具的原始参数，UI 据此生成一行摘要、Edit 的 diff 等
        input: Value,
        result: Option<ToolResult>,
    },
    /// 系统提示，比如“已中断”“进程退出：…”
    Notice(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    /// claude 给的一句话说明，可能没有
    pub description: Option<String>,
    /// can_use_tool 里的 permission_suggestions 原样保存，“总是允许”时回传
    pub suggestions: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    /// 允许，并把 suggestions 作为 updatedPermissions 回传，本会话不再询问
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// 最近一次请求的上下文大小：input + cache_creation + cache_read
    pub context_tokens: u64,
    pub context_window: Option<u64>,
    pub cost_usd: f64,
}

/// 一个打开的对话的全部状态
#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: ConvId,
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    /// 所属项目；None 表示不关联项目的对话
    pub project: Option<PathBuf>,
    pub title: String,
    pub blocks: Vec<Block>,
    /// 正在流式输出、还没收到完整 assistant 消息的文字
    pub streaming: String,
    pub busy: bool,
    pub turn_started: Option<Instant>,
    pub permission: Option<PermissionRequest>,
    pub usage: Usage,
    pub todos: Vec<Todo>,
    pub model: Option<String>,
    pub error: Option<String>,
}

impl Conversation {
    pub fn new(id: ConvId, cwd: PathBuf, project: Option<PathBuf>) -> Self {
        Self {
            id,
            session_id: None,
            cwd,
            project,
            title: String::from("新对话"),
            blocks: Vec::new(),
            streaming: String::new(),
            busy: false,
            turn_started: None,
            permission: None,
            usage: Usage::default(),
            todos: Vec::new(),
            model: None,
            error: None,
        }
    }

    pub fn status(&self) -> SessionStatus {
        if self.permission.is_some() {
            SessionStatus::Blocked
        } else if self.busy {
            SessionStatus::Working
        } else if self.error.is_some() {
            SessionStatus::Failed
        } else {
            SessionStatus::Idle
        }
    }
}

/// 变更面板的一行：git status 里的一个文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// 'M' 'A' 'D' 'R' '?'
    pub status: char,
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// 后台任务发给主循环的事件
#[derive(Debug)]
pub enum AppEvent {
    /// claude 输出的一行 stream-json
    Claude { conv: ConvId, msg: Value },
    /// claude 进程退出
    ClaudeExit { conv: ConvId, code: Option<i32>, stderr: String },
}
