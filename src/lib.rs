// hcc（Claude Code）和 hagy（Antigravity 的 agy）共用同一套界面，只是读会话、打开会话、
// 启动任务用的命令不同，见 backend.rs
mod agy;
mod app;
mod attach;
pub mod backend;
mod cli;
mod input;
mod model;
mod sessions;
mod store;
mod text;
mod transcripts;
mod worker;

pub use cli::main;
