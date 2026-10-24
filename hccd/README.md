# hccd

在终端里和 Claude Code 对话的 TUI，界面参照 Claude 桌面版：左侧是对话和项目，中间是对话，右侧是变更和待办。用 Rust + ratatui 写成，和仓库里的 Node 版 `hcc` 相互独立。

- 不选项目也能直接对话（Ctrl+N），对话在 `~/.local/share/heyclaude/chats/<id>` 这样的临时目录里进行
- 在项目里新建会话（选中项目按 n），或打开 Claude Code 已有的会话接着聊（含 worktree 里的会话）
- 流式显示回复、工具调用、Edit 的 diff 和 Bash 输出
- 工具权限在界面里确认：允许一次 / 本会话不再询问 / 拒绝
- Ctrl+K 跨项目搜索会话；Esc 中断当前这一轮
- 项目列表和 hcc 共用 `~/.config/heyclaude/projects.json`

## 安装

需要 Rust 工具链和已登录的 `claude` 命令。

```sh
cargo install --path hccd
```

## 用法

```sh
hccd                              # 启动
hccd --model opus                 # 多余的参数原样传给每个 claude 进程
hccd --permission-mode default    # 每次需要权限都在界面里询问
```

进入后按 `?` 看全部快捷键。

## 工作方式

每个对话对应一个 `claude -p --input-format stream-json --output-format stream-json --permission-prompt-tool stdio` 子进程。hccd 读它的流式输出更新界面，权限请求通过 stdio 的 `can_use_tool` 控制消息交给界面确认。打开历史会话时先读 `~/.claude/projects` 下的记录显示出来，发消息时再用 `--resume` 接上。退出 hccd 会结束这些子进程，会话记录由 Claude Code 保存，下次可以继续。

## 代码结构

- `src/model.rs`：各模块共用的数据结构
- `src/claude/`：启动 claude 进程（`process.rs`）和解析 stream-json（`protocol.rs`）
- `src/data/`：项目列表（`store.rs`）、Claude Code 会话记录（`sessions.rs`）、git 变更（`git.rs`）
- `src/app.rs`、`src/actions.rs`：应用状态、按键和事件处理
- `src/ui/`：界面渲染

```sh
cargo test                                          # 单元测试
cargo test claude::process -- --ignored --nocapture # 用 haiku 跑一次真实的端到端测试
cargo test ui:: -- --ignored --nocapture            # 把主界面渲染成文本看效果
```
