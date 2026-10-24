//! 解析 stream-json 消息并更新对话状态。

use serde_json::Value;

use crate::model::Conversation;

/// 把 claude 输出的一行消息应用到对话上
pub fn apply(_conv: &mut Conversation, _msg: &Value) {
    todo!()
}
