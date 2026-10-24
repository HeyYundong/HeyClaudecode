//! 用 TestBackend 渲染整个界面，检查关键文字。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde_json::json;

use crate::app::{App, Focus, InputState, Overlay, PanelTab, SwitchEntry, SwitchTarget};
use crate::model::*;

fn empty_app() -> App {
    App {
        projects: Vec::new(),
        chats: Vec::new(),
        expanded: None,
        sessions: Vec::new(),
        switch_index: Vec::new(),
        sidebar_selected: 0,
        focus: Focus::Sidebar,
        sidebar_visible: true,
        panel_visible: true,
        panel_tab: PanelTab::Changes,
        changes: Vec::new(),
        convs: HashMap::new(),
        procs: HashMap::new(),
        current: None,
        input: InputState::default(),
        chat_scroll: 0,
        overlay: None,
        permission_choice: 0,
        flash: None,
        claude_args: Vec::new(),
        next_conv: 1,
        should_quit: false,
    }
}

fn ago(secs: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(secs)
}

fn session(title: &str, status: SessionStatus, secs: u64, conv: Option<ConvId>) -> SessionInfo {
    SessionInfo {
        session_id: format!("id-{title}"),
        title: title.into(),
        cwd: PathBuf::from("/home/u/Projects/HeyClaudecode"),
        transcript: None,
        updated: ago(secs),
        status,
        conv,
    }
}

fn demo_conv() -> Conversation {
    let cwd = PathBuf::from("/home/u/Projects/HeyClaudecode");
    let mut c = Conversation::new(1, cwd.clone(), Some(cwd));
    c.title = "会话列表折叠逻辑".into();
    c.model = Some("claude-opus-5-5".into());
    c.blocks = vec![
        Block::User("每个状态分组只显示最新 5 条，剩下的折叠成一行，可以用方向键选中再展开。这一句话故意写得很长很长，用来测试中文按两列宽度折行是否正确，不应该超出对话区的右边界。".into()),
        Block::Text("先看一下现在的**分组逻辑**，主要在 `sessionsScreen.js` 里。".into()),
        Block::Tool {
            id: "t1".into(),
            name: "Read".into(),
            input: json!({"file_path": "/home/u/Projects/HeyClaudecode/src/sessionsScreen.js"}),
            result: Some(ToolResult { text: "line\n".repeat(312), is_error: false }),
        },
        Block::Tool {
            id: "t2".into(),
            name: "Edit".into(),
            input: json!({
                "file_path": "/home/u/Projects/HeyClaudecode/src/sessionsScreen.js",
                "old_string": "  const groups = groupBy(sessions, s => s.status);\n  return groups.map(g => g.items);\n}",
                "new_string": "  const groups = groupBy(sessions, s => s.status);\n  return groups.map(g => fold(g.items, 5));\n  // 折叠行本身也是一个可选中的条目\n}"
            }),
            result: Some(ToolResult { text: "ok".into(), is_error: false }),
        },
        Block::Text("跑一下测试，确认没有回归：\n\n- 单元测试\n- 集成测试\n\n```sh\nnode --test\n```".into()),
        Block::Tool {
            id: "t3".into(),
            name: "Bash".into(),
            input: json!({"command": "node --test"}),
            result: Some(ToolResult {
                text: "✔ fold keeps newest 5\n✔ fold row selectable\nℹ tests 23\nℹ pass 23\nℹ fail 0\nℹ duration_ms 1400\nℹ todo 0".into(),
                is_error: false,
            }),
        },
        Block::Tool {
            id: "t4".into(),
            name: "Grep".into(),
            input: json!({"pattern": "fold\\("}),
            result: Some(ToolResult { text: "No such file".into(), is_error: true }),
        },
        Block::Notice("已中断".into()),
    ];
    c.streaming = "折叠行已经可以被选中，按 Enter 展开剩余会话。下一步处理折叠行在窄终端里的截断".into();
    c.busy = true;
    c.turn_started = Some(Instant::now() - Duration::from_secs(12));
    c.usage = Usage { context_tokens: 38_200, context_window: Some(90_000), cost_usd: 0.031 };
    c.todos = vec![
        Todo { content: "读取分组逻辑".into(), status: TodoStatus::Completed },
        Todo { content: "实现 fold()".into(), status: TodoStatus::Completed },
        Todo { content: "折叠行支持选中".into(), status: TodoStatus::InProgress },
        Todo { content: "更新 README".into(), status: TodoStatus::Pending },
    ];
    c
}

fn demo_app() -> App {
    let mut app = empty_app();
    app.projects = vec![
        Project { name: "HeyClaudecode".into(), path: "/home/u/Projects/HeyClaudecode".into() },
        Project { name: "blog-site".into(), path: "/home/u/Projects/blog-site".into() },
        Project { name: "infra-scripts".into(), path: "/home/u/Projects/infra-scripts".into() },
    ];
    app.chats = vec![
        session("Rust TUI 框架选型", SessionStatus::Working, 300, None),
        session("解释 ratatui 布局", SessionStatus::Idle, 90_000, None),
    ];
    app.expanded = Some(0);
    app.sessions = vec![
        session("会话列表折叠逻辑", SessionStatus::Working, 180, Some(1)),
        session("修复 Ctrl+Z 连按退出", SessionStatus::Working, 720, None),
        session("增加 --tabs 布局", SessionStatus::Blocked, 3600, None),
        session("状态计数列对齐", SessionStatus::Idle, 90_000, None),
        session("隐藏项目路径", SessionStatus::Idle, 100_000, None),
        session("早期的一次尝试", SessionStatus::Failed, 400_000, None),
    ];
    for (g, l, st) in [
        ("blog-site", "文章目录 foldable 侧栏", SessionStatus::Blocked),
        ("blog-site", "RSS 输出", SessionStatus::Idle),
        ("HeyClaudecode", "会话列表fold折叠逻辑", SessionStatus::Working),
        ("对话", "Rust TUI 框架选型", SessionStatus::Working),
    ] {
        app.switch_index.push(SwitchEntry {
            label: l.into(),
            group: g.into(),
            status: st,
            updated: ago(200),
            target: SwitchTarget::Chat(0),
        });
    }
    // 选中第一个会话：NewChat, 2 个 Chat, Project(0), Session(0)
    app.sidebar_selected = 4;
    app.changes = vec![
        FileChange { status: 'M', path: "src/sessionsScreen.js".into(), added: 18, removed: 4 },
        FileChange { status: 'M', path: "src/ui.js".into(), added: 6, removed: 2 },
        FileChange { status: 'A', path: "src/fold.js".into(), added: 31, removed: 0 },
    ];
    app.convs.insert(1, demo_conv());
    app.current = Some(1);
    app.focus = Focus::Input;
    app
}

fn render(app: &App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| super::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

/// buffer 转成文本，宽字符后面的占位格跳过
fn dump(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut x = 0;
        while x < buf.area.width {
            let sym = buf[(x, y)].symbol();
            out.push_str(sym);
            x += unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
        }
        out.push('\n');
    }
    out
}

fn has(buf: &Buffer, s: &str) -> bool {
    dump(buf).contains(s)
}

#[test]
fn empty_state() {
    let app = empty_app();
    let buf = render(&app, 120, 30);
    let t = dump(&buf);
    assert!(t.contains("✻ HeyClaude"));
    assert!(t.contains("新对话"));
    assert!(t.contains("Ctrl+N"));
    assert!(t.contains("Ctrl+K"));
    assert!(t.contains("回复 Claude…"));
    assert!(t.contains("暂无对话"));
}

#[test]
fn sidebar_projects_and_sessions() {
    let app = demo_app();
    let buf = render(&app, 160, 45);
    let t = dump(&buf);
    for s in ["对话 · 不关联项目", "Rust TUI 框架选型", "项目 3", "HeyClaudecode", "blog-site", "运行中 2", "等待确认 1", "已完成 2", "失败 1", "修复 Ctrl+Z", "5m", "昨天", "n 新建"] {
        assert!(t.contains(s), "缺少 {s}\n{t}");
    }
    // 选中行有高亮背景
    let row = t.lines().position(|l| l.contains("● 会话列表折叠逻辑") && !l.contains("HeyClaude ")).unwrap();
    assert_eq!(buf[(1, row as u16)].bg, super::theme::SEL_BG);
    // 未展开项目的状态计数来自 switch_index
    assert!(t.contains("◐1"));
}

#[test]
fn sidebar_scrolls_to_selection() {
    let mut app = demo_app();
    for i in 0..40 {
        app.sessions.push(session(&format!("旧会话 {i}"), SessionStatus::Idle, 500_000 + i, None));
    }
    app.sidebar_selected = app.sidebar_items().len() - 1;
    let buf = render(&app, 160, 30);
    assert!(has(&buf, "旧会话 39"));
}

#[test]
fn conversation_blocks() {
    let app = demo_app();
    let buf = render(&app, 160, 60);
    let t = dump(&buf);
    for s in [
        "› 每个状态分组",
        "● 先看一下现在的分组逻辑",
        "⎿ Read  src/sessionsScreen.js  (312 行)",
        "⎿ Edit  src/sessionsScreen.js  +2 −1",
        "+   return groups.map(g => fold(g.items, 5));",
        "-   return groups.map(g => g.items);",
        "⎿ Bash  node --test",
        "✔ fold keeps newest 5",
        "… 另外 2 行",
        "No such file",
        "已中断",
        "• 单元测试",
        "node --test",
        "截断▌",
        "思考中… 12s · Esc 中断",
        "会话列表折叠逻辑",
        "运行中",
        "Opus 5.5",
        "上下文 42%",
        "38.2k tokens",
        "$0.03",
        "变更 3",
        "待办 2/4",
        "M src/sessionsScreen.js",
    ] {
        assert!(t.contains(s), "缺少 {s}\n{t}");
    }
    assert!(!t.contains("**"));
    // diff 行有红/绿底色
    let row = t.lines().position(|l| l.contains("+   return groups.map(g => fold")).unwrap();
    let x = t.lines().nth(row).unwrap();
    let col = unicode_width::UnicodeWidthStr::width(&x[..x.find("+   return").unwrap()]) as u16;
    assert_eq!(buf[(col, row as u16)].bg, super::theme::ADD_BG);
}

#[test]
fn long_cjk_text_wraps_within_chat() {
    let app = demo_app();
    // 120 列：没有面板，侧栏 32
    let buf = render(&app, 120, 60);
    let t = dump(&buf);
    assert!(t.contains("右边界"));
    assert!(!t.contains("变更 3"));
    // 每行显示宽度都不超过终端宽度
    for l in t.lines() {
        assert!(unicode_width::UnicodeWidthStr::width(l) <= 120);
    }
}

#[test]
fn todos_tab() {
    let mut app = demo_app();
    app.panel_tab = PanelTab::Todos;
    let buf = render(&app, 160, 45);
    let t = dump(&buf);
    assert!(t.contains("✔ 读取分组逻辑"));
    assert!(t.contains("▸ 折叠行支持选中"));
    assert!(t.contains("☐ 更新 README"));
}

#[test]
fn chat_without_project_has_no_panel() {
    let mut app = demo_app();
    let c = app.convs.get_mut(&1).unwrap();
    c.project = None;
    let buf = render(&app, 160, 45);
    let t = dump(&buf);
    assert!(t.contains("对话 · 不关联项目"));
    assert!(!t.contains("变更 3"));
}

#[test]
fn permission_box() {
    let mut app = demo_app();
    let c = app.convs.get_mut(&1).unwrap();
    c.permission = Some(PermissionRequest {
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        input: json!({"command": "rm -rf dist/ && npm run build"}),
        description: Some("会删除文件".into()),
        suggestions: json!([]),
    });
    app.permission_choice = 1;
    let buf = render(&app, 160, 45);
    let t = dump(&buf);
    for s in ["Claude 想运行这条命令", "rm -rf dist/ && npm run build", "工作目录", "会删除文件", "1. 允许一次", "› 2. 允许，本会话不再询问", "3. 拒绝", "◐ 等待确认"] {
        assert!(t.contains(s), "缺少 {s}\n{t}");
    }
}

#[test]
fn switcher_overlay() {
    let mut app = demo_app();
    app.overlay = Some(Overlay::Switcher { query: "fold".into(), selected: 0 });
    let buf = render(&app, 160, 45);
    let t = dump(&buf);
    assert!(t.contains("› fold▌"));
    assert!(t.contains("2 / 4"));
    assert!(t.contains("文章目录 foldable 侧栏"));
    assert!(t.contains("等待确认 · 3m"));
    assert!(t.contains("↑↓ 选择"));
}

#[test]
fn help_and_add_project() {
    let mut app = demo_app();
    app.overlay = Some(Overlay::Help);
    let t = dump(&render(&app, 120, 40));
    for s in ["快捷键", "Ctrl+N", "Ctrl+T", "PgUp / PgDn", "Ctrl+C", "退出"] {
        assert!(t.contains(s), "缺少 {s}");
    }
    app.overlay = Some(Overlay::AddProject { input: "~/Projects/new".into() });
    let t = dump(&render(&app, 120, 40));
    assert!(t.contains("添加项目"));
    assert!(t.contains("› ~/Projects/new▌"));
}

#[test]
fn input_multiline_and_cursor() {
    let mut app = demo_app();
    app.input = InputState { text: "第一行\nsecond line".into(), cursor: 4 };
    let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
    term.draw(|f| super::draw(f, &app)).unwrap();
    let t = dump(term.backend().buffer());
    assert!(t.contains("› 第一行"));
    assert!(t.contains("  second line"));
    let pos = term.get_cursor_position().unwrap();
    let row = t.lines().position(|l| l.contains("second line")).unwrap();
    assert_eq!(pos.y as usize, row);
}

#[test]
fn tiny_terminals_do_not_panic() {
    let mut app = demo_app();
    for (w, h) in [(20, 5), (1, 1), (10, 3), (89, 10), (139, 10), (200, 8), (40, 2)] {
        render(&app, w, h);
    }
    app.overlay = Some(Overlay::Switcher { query: "x".into(), selected: 3 });
    render(&app, 20, 5);
    app.overlay = Some(Overlay::Help);
    render(&app, 20, 5);
    app.overlay = Some(Overlay::AddProject { input: String::new() });
    render(&app, 20, 5);
    render(&empty_app(), 20, 5);
}

#[test]
fn scrolled_view() {
    let mut app = demo_app();
    app.chat_scroll = 5;
    let t = dump(&render(&app, 160, 30));
    assert!(t.contains("↓ 5 行"));
    app.chat_scroll = 9999;
    let t = dump(&render(&app, 160, 30));
    assert!(t.contains("› 每个状态分组"));
}

/// 打印主界面方便人看：cargo test ui:: -- --ignored --nocapture
#[test]
#[ignore]
fn print_main() {
    let app = demo_app();
    println!("{}", dump(&render(&app, 160, 45)));
    let mut app2 = demo_app();
    let c = app2.convs.get_mut(&1).unwrap();
    c.permission = Some(PermissionRequest {
        request_id: "r1".into(),
        tool_name: "Edit".into(),
        input: json!({"file_path": "/home/u/Projects/HeyClaudecode/src/ui.js", "old_string": "a = 1", "new_string": "a = 2"}),
        description: None,
        suggestions: json!([]),
    });
    c.busy = false;
    c.streaming.clear();
    println!("{}", dump(&render(&app2, 160, 45)));
    app2.overlay = Some(Overlay::Switcher { query: "fold".into(), selected: 1 });
    println!("{}", dump(&render(&app2, 120, 30)));
    println!("{}", dump(&render(&empty_app(), 100, 24)));
}
