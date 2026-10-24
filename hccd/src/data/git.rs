//! 变更面板的数据：git status 加上增删行数。

use std::path::Path;

use crate::model::FileChange;

/// 不是 git 仓库或 git 出错时返回空列表
pub fn changed_files(_cwd: &Path) -> Vec<FileChange> {
    todo!()
}
