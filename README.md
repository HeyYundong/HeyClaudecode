# HeyClaude

按项目分组管理 Claude Code agents 的终端 TUI。不修改 Claude Code 本身，只是在 `claude agents` 外面加一层项目列表。

`claude agents` 默认显示所有目录的会话，只能用 `claude agents --cwd <path>` 手动过滤。同时开多个项目时，HeyClaude 让你在一个列表里挑项目进入，退出后再回到列表。

## 安装

需要 Node.js 22+，并且 `claude` 在 PATH 里。

```bash
git clone <repo> HeyClaudeCode && cd HeyClaudeCode
npm install
npm link        # 全局注册 heyclaude 和 hcc 两个命令
```

## 使用

```bash
hcc             # 或 heyclaude
```

```
╭────────────────────────────────────────────────────────────────────╮
│ HeyClaude  按项目管理 claude agents                                 │
╰────────────────────────────────────────────────────────────────────╯
   HeyAgent        /mnt/toshiba/Projects/HeyAgent        0 运行 · 5 空闲 · 1 完成
 ❯ EdgePredictfun  /mnt/toshiba/Projects/EdgePredictfun  1 运行 · 1 空闲 · 0 完成
↑↓ 选择  ·  Enter 进入  ·  a 添加  ·  d 删除  ·  r 刷新  ·  q 退出
```

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` / `k` `j` | 选择项目 |
| `Enter` | 进入项目：运行 `claude agents --cwd <项目目录>` |
| `a` | 添加项目（见下） |
| `d` | 从列表移除项目（只改 HeyClaude 的列表，目录和会话都不动） |
| `r` | 刷新会话统计（每 10 秒也会自动刷新一次） |
| `q` / `Esc` | 退出 |

**返回上一级**：在 agents 视图里按它自己的退出键（快速按两次 `Ctrl+C`），就会回到项目列表，光标停在刚才的项目上。

**添加项目**：输入路径，`Tab` 补全目录，留空就是当前目录。下面会列出在 `claude agents` 里有会话记录、但还没添加的目录，按 `↑↓` 选中后 `Enter` 一键导入；输入的文字也会用来过滤这份列表。

**会话统计**：来自 `claude agents --json --all`。项目目录下的子目录（包括 `.claude/worktrees/*`）都算这个项目的，和 `--cwd` 的过滤规则一致。
- 运行：正在工作的会话
- 空闲：开着但空闲的交互会话
- 完成：已结束的后台会话（done / stopped / failed）

**参数透传**：除 `-h`、`-v` 以外的参数原样传给每次 `claude agents`，例如：

```bash
hcc --model opus --effort high
```

## 数据

项目列表保存在 `~/.config/heyclaude/projects.json`（如果设置了 `$XDG_CONFIG_HOME` 就用它），可以直接手动编辑，比如改项目名：

```json
{
  "projects": [
    {"name": "EdgePredictfun", "path": "/mnt/toshiba/Projects/EdgePredictfun", "addedAt": "2026-10-17T15:49:50.870Z"}
  ]
}
```

## 代码结构

- `bin/heyclaude.js`：入口。主循环是"显示列表 → 把终端交给 `claude agents` → 回到列表"
- `src/app.js`：Ink 界面（列表、添加、删除确认）
- `src/sessions.js`：读取 `claude agents --json` 并按项目统计
- `src/store.js`：projects.json 的读写
- `src/complete.js`：路径 Tab 补全
