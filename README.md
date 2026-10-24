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
╭──────────────────────────────────────────────────────────────────────╮
│ HeyClaude  按项目管理 claude agents                                   │
╰──────────────────────────────────────────────────────────────────────╯
 ❯ 1. HeyAgent     1 运行 · 1 等你回复 · 3 待确认   /home/me/Projects/HeyAgent
   2. my-api       0 运行 · 0 等你回复 · 0 待确认   /home/me/Projects/my-api
   3. website      2 运行 · 0 等你回复 · 1 待确认   /home/me/Projects/website

↑↓ 选择  ·  Enter 审阅  ·  1-3 直接进入  ·  a 添加  ·  d 删除  ·  r 刷新  ·  q 退出
```

项目按名字排序（不区分大小写），前面的序号可以直接按数字键进入。目录被删掉或移走的项目会在路径后标注「目录不存在」，不能进入，按 `d` 移除即可。

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` / `k` `j` | 选择项目 |
| `Enter` | 打开项目的审阅页（见下） |
| `1`–`9`… | 按序号直接进入项目的 `claude agents --cwd <项目目录>`，跳过审阅页。超过 9 个项目时可以连按多位（如 `1` `2` 进入第 12 个）；按完一位后如果还可能有下一位，会等 0.6 秒，也可以按 `Enter` 立刻进入 |
| `a` | 添加项目（见下） |
| `d` | 从列表移除项目（只改 HeyClaude 的列表，目录和会话都不动） |
| `r` | 刷新会话统计（每 10 秒也会自动刷新一次） |
| `q` / `Esc` | 退出 |

**返回上一级**：在 agents 视图里按它自己的退出键（快速按两次 `Ctrl+C`），就会回到进入之前的页面（审阅页或项目列表），光标停在刚才的项目上。

### 审阅页：由你确认才算完成

Claude Code 认为任务完成了，不代表你已经看过。HeyClaude 给后台任务加了一层“由你确认”的状态：任务结束后先进入「待确认」，你确认之后才算真正完成，并从列表里隐藏。

```
 等你回复 (1)
 ❯ ●  deploy-config-fix        需要输入   3 分钟前开始
 待确认 (3)
   ✓  readme-refresh           完成       12 分钟前开始
   ✗  migrate-db-schema        失败       1 小时前开始
   ■  flaky-test-hunt          已停止     昨天 18:20 开始
 运行中 (1)
   ◌  add-review-page          运行中     25 分钟前开始

 已隐藏 14 个已确认的任务（h 显示）
```

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` / `k` `j` | 选择任务 |
| `c` | 确认选中的任务；在已确认的任务上按则取消确认 |
| `C` | 确认这个项目下全部待确认任务（会先问 y/N） |
| `u` | 撤销最近一次确认，可以连续撤销 |
| `o` / `Enter` | 打开 `claude agents --cwd <项目目录>` 看详情或回复，退出后回到审阅页 |
| `h` | 显示或隐藏已确认的任务 |
| `r` | 刷新 |
| `Esc` / `q` / `←` | 回到项目列表 |

规则：
- 只管后台任务。done、failed、stopped 都算结束，都需要你确认。Claude Code 以后新增的结束状态也一律进待确认。
- 运行中和等你回复的任务不能确认。
- 已确认的任务如果你又回复了它、它重新开始运行，会自动取消确认，等它再结束时重新进入待确认。
- `claude agents --json` 只给出开始时间，所以列表里显示的是“多久之前开始”。
- 确认记录只存在 HeyClaude 自己的 `reviews.json` 里，`claude agents` 自己的界面照常显示所有任务。

**添加项目**：输入路径，`Tab` 补全目录，留空就是当前目录。下面会列出在 `claude agents` 里有会话记录、但还没添加的目录，按 `↑↓` 选中后 `Enter` 一键导入；输入的文字也会用来过滤这份列表。

**会话统计**：来自 `claude agents --json --all`。项目目录下的子目录（包括 `.claude/worktrees/*`）都算这个项目的，和 `--cwd` 的过滤规则一致。
- 运行：正在工作的会话，包括你回复后重新运行的后台任务
- 等你回复：在等你输入的会话（Claude Code 里的 Needs input）
- 待确认：已结束、你还没确认的后台任务
- 空闲的交互会话和已确认的任务不计数

**参数透传**：除 `-h`、`-v` 以外的参数原样传给每次 `claude agents`，例如：

```bash
hcc --model opus --effort high
```

## 数据

项目列表保存在 `~/.config/heyclaude/projects.json`（如果设置了 `$XDG_CONFIG_HOME` 就用它），可以直接手动编辑，比如改项目名（显示顺序跟着名字走，文件里的顺序无所谓）：

```json
{
  "projects": [
    {"name": "my-api", "path": "/home/me/Projects/my-api", "addedAt": "2026-10-17T15:49:50.870Z"}
  ]
}
```

确认记录保存在同一目录的 `reviews.json`，按会话 ID 记录确认时间。删掉某一行，那个任务就会回到待确认：

```json
{
  "confirmed": {
    "8c51dde5-8354-4d37-99e3-991c1e2da9ba": "2026-10-24T10:34:41.623Z"
  }
}
```

## 代码结构

- `bin/heyclaude.js`：入口。主循环是"显示列表或审阅页 → 把终端交给 `claude agents` → 回到原来的页面"
- `src/app.js`：Ink 界面（列表、添加、删除确认）
- `src/review.js`：审阅页（分组、确认、撤销）
- `src/sessions.js`：读取 `claude agents --json`，给会话归类并按项目统计
- `src/reviews.js`：reviews.json 的读写
- `src/store.js`：projects.json 的读写、校验和排序
- `src/complete.js`：路径 Tab 补全
