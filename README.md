# HeyClaude

按项目管理 Claude Code 后台任务的终端 TUI。不修改 Claude Code 本身，只调用它公开的命令。

流程是“项目 → 状态 → 会话”。任务有四种状态，其中「待确认」是 HeyClaude 加的：Claude 说做完了，但你还没确认。你确认过才算「已完成」，这样完成的任务不会和没看过的混在一起。

| 状态 | 含义 |
| --- | --- |
| ◌ 运行中 | Claude 正在干活 |
| ● 等你回复 | Claude 在等你输入或授权（Claude Code 里的 Needs input） |
| ◆ 待确认 | Claude 说做完了，你还没确认。完成、失败、已停止都进这里，后面标出是哪种 |
| ✓ 已完成 | 你确认过的任务，一直留在列表里，变灰排在最后 |

用到的 Claude Code 命令只有三个：`claude agents --json --all` 读取会话和状态，`claude attach <id>` 打开会话，`claude --bg <内容>` 启动新任务。

## 安装

需要 Node.js 22+，并且 `claude` 在 PATH 里。

```bash
git clone <repo> HeyClaudeCode && cd HeyClaudeCode
npm install
npm link        # 全局注册 heyclaude 和 hcc 两个命令
```

## 两种布局

```bash
hcc             # 默认：项目列表 → 项目里按状态分组的任务列表
hcc --tabs      # 一个界面里用标签切换项目和状态
```

**默认布局**：先是项目列表，每种状态一列数量。进入项目后，任务按状态分组。

```
╭──────────────────────────────────────────────────────────────────────╮
│ HeyClaude  4 个项目 · 2 等你回复 · 4 待确认 · 3 运行中                 │
╰──────────────────────────────────────────────────────────────────────╯
      项目           等你回复  待确认    运行中    已完成    目录
 ❯ 1. HeyAgent       1         1         1         9         /home/me/Projects/HeyAgent
   2. HeyClaudecode  1         3         1         3         /home/me/Projects/HeyClaudecode

 等你回复 (1)
 ❯ ●  deploy-config-fix          需要输入  3 分钟前开始
 待确认 (3)
   ◆  readme-refresh             完成      12 分钟前开始
   ◆  migrate-db-schema          失败      1 小时前开始
   ◆  flaky-test-hunt            已停止    昨天 18:20 开始
 运行中 (1)
   ◌  add-review-page            运行中    25 分钟前开始
 已完成 (3)
   ✓  sort-projects              完成      昨天 14:02 开始   昨天 15:10 确认
```

**`--tabs` 布局**：启动就是任务视图。上面一排是项目，用 `←` `→` 切换；第二排是状态标签，用 `Tab` 切换；下面是任务，每行还会显示它在哪个 worktree 里。

```
 项目    1 HeyAgent    2 HeyClaudecode    3 my-api
 状态    全部 8   ● 等你回复 1   ◆ 待确认 3   ◌ 运行中 1   ✓ 已完成 3
 ❯ ◆  readme-refresh       完成     12 分钟前开始     worktree: readme
   ◆  migrate-db-schema    失败     1 小时前开始      项目根目录
```

## 按键

`Ctrl+Z` 统一表示“返回上一级”：在打开的会话里按，回到任务列表；在任务列表里按，回到项目列表；在项目列表里按，退出 HeyClaude。连按时每 0.4 秒只退一级，从会话回来后的 0.6 秒内多按的 `Ctrl+Z` 会被忽略，不会一路退出。

项目列表：

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` / `k` `j` | 选择项目 |
| `Enter` | 进入项目 |
| `1`–`9`… | 按序号直接进入。超过 9 个项目时可以连按多位（如 `1` `2` 进入第 12 个）；按完一位后如果还可能有下一位，会等 0.6 秒，也可以按 `Enter` 立刻进入 |
| `a` | 添加项目 |
| `d` | 从列表移除项目（只改 HeyClaude 的列表，目录和会话都不动） |
| `r` | 刷新（每 10 秒也会自动刷新） |
| `Ctrl+Z` / `q` | 退出 |

任务列表（两种布局相同）：

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` / `k` `j` | 选择任务 |
| `Enter` / `o` | 用 `claude attach` 打开会话，看结果或回复它。按 `Ctrl+Z` 回来，光标停在刚才的任务上 |
| `c` | 确认选中的待确认任务，它变成已完成；在已完成的任务上按则取消确认 |
| `C` | 确认这个项目下全部待确认任务（会先问 y/N） |
| `u` | 撤销最近一次确认，可以连续撤销 |
| `n` | 新任务：输入要做的事，在项目目录里用 `claude --bg` 启动 |
| `←` `→` | 切换项目（仅 `--tabs`） |
| `Tab` / `Shift+Tab` | 切换状态标签（仅 `--tabs`） |
| `r` | 刷新 |
| `Ctrl+Z` | 回到项目列表 |

**会话里的 /bg 和 ←**：这两个会进入 Claude Code 自己的 agents 列表，这是 `claude attach` 的固定行为。从那里快速按两次 `Ctrl+C` 退出，也会回到 HeyClaude。交互会话运行在它自己的终端里，不能从这里打开。

**添加项目**：输入路径，`Tab` 补全目录，留空就是当前目录。下面会列出在 `claude agents` 里有会话记录、但还没添加的目录，按 `↑↓` 选中后 `Enter` 一键导入；输入的文字也会用来过滤这份列表。

**参数透传**：除 `--tabs`、`-h`、`-v` 以外的参数会在按 `n` 新建任务时传给 `claude --bg`，例如：

```bash
hcc --model opus --effort high
```

## 状态规则

- 只管后台任务。done、failed、stopped 都算结束，都要你确认。Claude Code 以后新增的结束状态也一律进待确认。
- 运行中和等你回复的任务不能确认。
- 已完成的任务如果你又回复了它、它重新开始运行，会自动取消确认，等它再结束时重新进入待确认。
- 项目目录下的子目录（包括 `.claude/worktrees/*`）都算这个项目的任务。
- `claude agents --json` 只给出开始时间，所以列表里显示的是“多久之前开始”。
- 确认记录只存在 HeyClaude 自己的 `reviews.json` 里，Claude Code 的数据一点不动。

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

- `bin/heyclaude.js`：入口。解析 `--tabs`；主循环是“显示界面 → 把终端交给 `claude attach` → 回到任务列表”
- `src/app.js`：Ink 界面（项目列表、添加、删除确认、新任务）
- `src/sessionsScreen.js`：任务列表，两种布局共用；状态定义和时间格式
- `src/sessions.js`：读取 `claude agents --json`，给会话归类并按项目统计
- `src/reviews.js`：reviews.json 的读写
- `src/store.js`：projects.json 的读写、校验和排序
- `src/complete.js`：路径 Tab 补全
