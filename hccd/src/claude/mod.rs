//! 和 claude CLI 通信：启动 `claude -p --input-format stream-json --output-format stream-json`，
//! 读它的输出并应用到 Conversation 上。

pub mod process;
pub mod protocol;
