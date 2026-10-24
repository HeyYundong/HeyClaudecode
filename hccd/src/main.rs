mod actions;
mod app;
mod claude;
mod data;
mod model;
mod ui;

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, EventStream};
use crossterm::execute;
use futures::StreamExt;
use tokio::sync::mpsc;

use app::App;

const HELP: &str = "hccd — 在终端里和 Claude Code 对话，按项目管理会话

用法: hccd [claude 的参数…]

  多余的参数会原样传给每个 claude 进程，例如：
    hccd --model opus
    hccd --permission-mode acceptEdits

  -h, --help     显示帮助
  -V, --version  显示版本

项目列表和 hcc 共用 ~/.config/heyclaude/projects.json。
进入界面后按 ? 查看快捷键。";

/// 正在输出时按这个间隔重画，让计时和光标动起来
const TICK: Duration = Duration::from_millis(250);
/// 定期重读会话列表和 git 变更
const REFRESH: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("hccd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut app = App::new(args);
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste, EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app).await;
    let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    // 退出时 procs 被丢弃，子进程随之结束；会话记录已由 claude 落盘，下次可以继续
    drop(app);
    result
}

async fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut term_events = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    let mut refresh = tokio::time::interval(REFRESH);
    refresh.tick().await;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        tokio::select! {
            ev = term_events.next() => match ev {
                Some(Ok(ev)) => app.on_term(ev, &tx),
                Some(Err(e)) => return Err(e.into()),
                None => break,
            },
            Some(ev) = rx.recv() => {
                app.on_app_event(ev);
                // 流式输出时一次处理完积压的消息再重画
                while let Ok(ev) = rx.try_recv() {
                    app.on_app_event(ev);
                }
            }
            _ = tick.tick(), if app.convs.values().any(|c| c.busy) => {}
            _ = refresh.tick() => app.refresh(),
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}
