mod app;
mod sessions;
mod store;
mod text;
mod transcripts;
mod worker;

use std::io::{self, IsTerminal, Write};
use std::process::{Command, ExitCode};
use std::sync::mpsc::channel;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, MoveTo, Show};
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::style::Color;

use app::{Action, App, Layout};
use sessions::Session;
use store::Project;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() -> String {
	format!(
		"HeyClaude {VERSION} — 按项目管理 Claude Code 后台任务

用法: heyclaude (或 hcc) [--tabs] [claude 的参数…]

  默认：项目列表 → 项目里按状态分组的任务列表。
  --tabs：一个界面里用标签切换项目和状态。

  任务有五种状态：进行中、需要输入、待确认、完成、失败。Claude 说做完了的任务
  先进「待确认」，你按 c 确认后才算「完成」。

  选中任务按 Enter 用 `claude attach` 打开会话，按 Ctrl+Z 回到 HeyClaude。
  Ctrl+Z 在 HeyClaude 里也是“返回上一级”，在项目列表里按就退出。

  除 --tabs、-h、-v 外的参数会在按 n 新建任务时传给 `claude --bg`，例如:

    hcc --model opus --effort high

项目列表保存在 {}/projects.json，确认记录保存在 reviews.json",
		store::config_dir().display()
	)
}

extern "C" fn ignore_signal(_: libc::c_int) {}

// 装一个什么也不做的处理函数，而不是 SIG_IGN：处理函数在 exec 时会恢复成默认，
// 这样 claude 子进程收到的信号行为不受影响
fn set_signal(signal: libc::c_int, ignore: bool) {
	unsafe {
		let mut action: libc::sigaction = std::mem::zeroed();
		action.sa_sigaction = if ignore {
			ignore_signal as *const () as usize
		} else {
			libc::SIG_DFL
		};
		libc::sigemptyset(&mut action.sa_mask);
		libc::sigaction(signal, &action, std::ptr::null_mut());
	}
}

fn enter_screen() -> io::Result<()> {
	enable_raw_mode()?;
	execute!(
		io::stdout(),
		EnterAlternateScreen,
		Clear(ClearType::All),
		MoveTo(0, 0),
		EnableBracketedPaste,
		Hide
	)
}

fn leave_screen() {
	let _ = execute!(io::stdout(), DisableBracketedPaste, Show, LeaveAlternateScreen);
}

fn attach(project: &Project, session: &Session) -> Option<(String, Color)> {
	// 子进程和我们在同一个进程组，Ctrl+C 只应该交给 claude 处理。
	// 终端保持原始模式：claude 接管终端之前按的 Ctrl+Z / Ctrl+C 只是普通按键，
	// 不会变成信号把 claude 暂停（暂停的 claude 和等它退出的我们会互相卡住）
	set_signal(libc::SIGINT, true);
	let _ = enable_raw_mode();
	// claude attach 用短 ID 打开后台会话；在会话自己的目录里运行（可能是 worktree）
	let cwd = if store::dir_exists(session.cwd()) {
		session.cwd()
	} else {
		&project.path
	};
	let result = Command::new("claude").args(["attach", &session.id]).current_dir(cwd).status();
	let _ = enable_raw_mode();
	let name = session.short_name();
	let message = match result {
		Err(e) => Some((format!("无法启动 claude：{e}"), Color::Red)),
		Ok(status) if !status.success() => {
			let code = status.code().unwrap_or(-1);
			// 会话打不开时 claude 会打印原因然后退出，停一下让人看清
			print!("\r\n\x1b[33m会话 {name} 打不开（退出码 {code}），按回车返回 HeyClaude\x1b[0m");
			let _ = io::stdout().flush();
			while let Ok(ev) = event::read() {
				if matches!(ev, Event::Key(k) if k.code == KeyCode::Enter) {
					break;
				}
			}
			Some((format!("会话 {name} 打不开（claude attach 退出码 {code}）"), Color::Yellow))
		}
		Ok(_) => None,
	};
	set_signal(libc::SIGINT, false);
	message
}

fn run(layout: Layout, projects: Vec<Project>, forwarded: Vec<String>) -> io::Result<()> {
	let (msg_tx, msg_rx) = channel();
	let fetch_tx = worker::spawn_fetcher(msg_tx.clone());
	worker::spawn_watcher(msg_tx.clone());
	let mut app = App::new(layout, projects, forwarded, fetch_tx, msg_tx);

	enter_screen()?;
	let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
	terminal.clear()?;
	let mut dirty = true;
	// 交接给 claude attach 时在主屏幕上画过界面，退出时要擦掉
	let mut painted_main = false;
	loop {
		if dirty {
			terminal.draw(|f| app.draw(f))?;
			dirty = false;
		}
		// 一次把排队的按键都处理完再画，按住方向键时不会一帧一帧地追
		if event::poll(Duration::from_millis(50))? {
			loop {
				app.handle_event(event::read()?);
				if app.has_action() || !event::poll(Duration::ZERO)? {
					break;
				}
			}
			dirty = true;
		}
		while let Ok(msg) = msg_rx.try_recv() {
			app.handle_msg(msg);
			dirty = true;
		}
		dirty |= app.tick();
		match app.take_action() {
			None => {}
			Some(Action::Quit) => break,
			Some(Action::Attach(project, session)) => {
				// claude 退出时先离开它自己的备用屏幕，进程要再过半秒左右才结束。
				// 这段空档露出的是主屏幕，所以先把当前界面也画到主屏幕上，免得闪回 shell
				leave_screen();
				terminal.clear()?;
				terminal.draw(|f| app.draw(f))?;
				execute!(io::stdout(), Show)?;
				painted_main = true;
				let message = attach(&project, &session);
				enter_screen()?;
				terminal.clear()?;
				app.returned(&project.path, &session.session_id, message);
				dirty = true;
			}
		}
	}
	leave_screen();
	if painted_main {
		// 主屏幕上留着交接时画的界面，退出时擦掉
		execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0))?;
	}
	disable_raw_mode()
}

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().skip(1).collect();
	if args.iter().any(|a| a == "-h" || a == "--help") {
		println!("{}", usage());
		return ExitCode::SUCCESS;
	}
	if args.iter().any(|a| a == "-v" || a == "--version") {
		println!("{VERSION}");
		return ExitCode::SUCCESS;
	}
	// --tabs：项目和状态标签；默认：项目列表 → 按状态分组的会话列表
	let layout = if args.iter().any(|a| a == "--tabs") {
		Layout::Tabs
	} else {
		Layout::Grouped
	};
	// 其余参数在按 n 新建任务时传给 claude --bg
	let forwarded: Vec<String> = args.into_iter().filter(|a| a != "--tabs").collect();

	if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
		eprintln!("heyclaude 需要在交互式终端中运行。");
		return ExitCode::FAILURE;
	}
	let projects = match store::load_projects() {
		Ok(p) => p,
		Err(e) => {
			eprintln!("{e}");
			return ExitCode::FAILURE;
		}
	};

	// HeyClaude 自己不响应 Ctrl+Z 的暂停信号：界面里 Ctrl+Z 是“返回上一级”，
	// 交接终端的空档里多按的 Ctrl+Z 也不能把我们挂起
	set_signal(libc::SIGTSTP, true);
	let default_hook = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |info| {
		leave_screen();
		let _ = disable_raw_mode();
		default_hook(info);
	}));

	match run(layout, projects, forwarded) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			leave_screen();
			let _ = disable_raw_mode();
			eprintln!("{e}");
			ExitCode::FAILURE
		}
	}
}
