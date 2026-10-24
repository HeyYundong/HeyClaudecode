//! 项目列表与 hcc 共用 ~/.config/heyclaude/projects.json；不关联项目的对话放在数据目录下。

use std::path::{Path, PathBuf};

use crate::model::Project;

pub fn load_projects() -> anyhow::Result<Vec<Project>> {
    todo!()
}

/// 校验目录、去重、按名字排序后写回文件，返回新列表
pub fn add_project(_projects: &[Project], _raw_path: &str) -> anyhow::Result<Vec<Project>> {
    todo!()
}

pub fn remove_project(_projects: &[Project], _path: &Path) -> anyhow::Result<Vec<Project>> {
    todo!()
}

/// 不关联项目的对话的根目录：$XDG_DATA_HOME/heyclaude/chats（默认 ~/.local/share/heyclaude/chats）
pub fn chats_root() -> PathBuf {
    todo!()
}

/// 为一个新对话建一个空目录并返回
pub fn new_chat_dir() -> anyhow::Result<PathBuf> {
    todo!()
}
