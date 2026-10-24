//! 读 Claude Code 存在 ~/.claude/projects 下的会话记录（只读）。

use std::path::{Path, PathBuf};

use crate::model::{Block, SessionInfo, Todo};

/// cwd 对应的记录目录：$CLAUDE_CONFIG_DIR 或 ~/.claude，再加 projects/<编码后的 cwd>
pub fn transcript_dir(_cwd: &Path) -> PathBuf {
    todo!()
}

/// 某个目录下的所有会话，新的在前。include_worktrees 为 true 时也包括 <cwd>/.claude/worktrees/* 下的会话
pub fn list_sessions(_cwd: &Path, _include_worktrees: bool) -> Vec<SessionInfo> {
    todo!()
}

/// 所有不关联项目的对话（chats_root 下每个子目录的会话），新的在前
pub fn list_chats() -> Vec<SessionInfo> {
    todo!()
}

pub struct Transcript {
    pub blocks: Vec<Block>,
    pub todos: Vec<Todo>,
    pub title: String,
}

pub fn load_transcript(_path: &Path) -> anyhow::Result<Transcript> {
    todo!()
}
