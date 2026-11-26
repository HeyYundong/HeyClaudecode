use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, MoveTo, Show};
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
	BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
	enable_raw_mode,
};
use ratatui::crossterm::{QueueableCommand, queue};
use ratatui::style::Color;

use crate::app::{Action, App, Layout};
use crate::backend::{self, Backend};
use crate::sessions::Session;
use crate::store::{self, Project};
use crate::{attach, worker};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() -> String {
	if backend::is_agy() {
		return agy_usage();
	}
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

  按 n 新建任务时也可以单独给这个任务选：Tab 换模型，Shift+Tab 换思考强度。
  没选时显示实际会用的值（来自上面的参数或 Claude Code 的设置），选过的标红。

项目列表保存在 {}/projects.json，确认记录保存在 reviews.json",
		store::config_dir().display()
	)
}

fn agy_usage() -> String {
	format!(
		"HeyAgy {VERSION} — 按项目管理 Antigravity CLI（agy）的任务

用法: hagy [--tabs] [agy 的参数…]

  和 hcc 一样的界面，管的是 agy 的会话。项目列表、确认记录、标题和 hcc 分开存。

  按 n 新建任务：在项目目录里后台运行 `agy -p <内容>`，关掉 HeyAgy 也会继续跑完。
  任务记录（输出、退出码）放在 {}。
  agy 里直接开的会话也会列出来（读 agy 的会话缓存）。

  状态：会话正被 agy 打开着（后台任务在跑，或者你在别处开着它）算进行中；
  跑完的进「待确认」，按 c 确认后才算「完成」；agy 报错的算失败。
  agy -p 不会停下来等你授权，所以没有「需要输入」。

  选中任务按 Enter 用 `agy --conversation` 打开会话，按 Ctrl+Z 或输入 /exit 回到 HeyAgy。
  agy 还在干活时按 Ctrl+Z 会打断它（对话本身已经存好，可以再打开）。

  除 --tabs、-h、-v 外的参数会在按 n 新建任务时传给 agy，例如:

    hagy --model gemini-4-neon --effort high

  按 n 新建任务时也可以单独给这个任务选：Tab 换模型，Shift+Tab 换思考强度。

项目列表保存在 {}/projects.json，确认记录保存在 reviews.json",
		crate::agy::jobs_dir().display(),
		store::config_dir().display()
	)
}

extern "C" fn ignore_signal(_: libc::c_int) {}

// 装一个什么也不做的处理函数，而不是 SIG_IGN：处理函数在 exec 时会恢复成默认，
// 这样 claude 子进程收到的信号行为不受影响
fn ignore(signal: libc::c_int) {
	unsafe {
		let mut action: libc::sigaction = std::mem::zeroed();
		action.sa_sigaction = ignore_signal as *const () as usize;
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

// 从 claude 那里拿回终端：切回备用屏幕并立刻画出来
fn take_back(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &App) -> io::Result<()> {
	enable_raw_mode()?;
	queue!(
		io::stdout(),
		BeginSynchronizedUpdate,
		EnterAlternateScreen,
		EnableBracketedPaste,
		Hide
	)?;
	terminal.clear()?;
	terminal.draw(|f| app.draw(f))?;
	io::stdout().queue(EndSynchronizedUpdate)?.flush()
}

fn attach(project: &Project, session: &Session) -> Option<(String, Color)> {
	// 终端保持原始模式，按键原样转给 claude
	let _ = enable_raw_mode();
	// claude attach 用短 ID 打开后台会话；在会话自己的目录里运行（可能是 worktree）
	let cwd = if store::dir_exists(session.cwd()) {
		session.cwd()
	} else {
		&project.path
	};
	let name = session.short_name();
	let b = backend::current();
	let outcome = match b {
		Backend::Claude => attach::run(b.program(), cwd, &["attach", &session.id], false),
		Backend::Agy => {
			// 后台任务刚启动、agy 还没建好会话时没有会话 ID
			if session.session_id.starts_with("job:") {
				// agy 一开始就报错（比如模型不支持这个强度）时根本没建会话，显示它的报错
				return Some(match &session.error {
					Some(e) => (format!("任务失败：{e}"), Color::Red),
					None => ("这个任务还没有会话，稍后再打开".into(), Color::Yellow),
				});
			}
			attach::run(b.program(), cwd, &["--conversation", &session.session_id], true)
		}
	};
	match outcome {
		attach::Outcome::Detached => None,
		attach::Outcome::Exited(status) if status.success() => None,
		attach::Outcome::Failed(e) => Some((format!("无法启动 {}：{e}", b.program()), Color::Red)),
		attach::Outcome::Exited(status) => {
			let code = status.code().unwrap_or(-1);
			let brand = b.brand();
			// 会话打不开时 claude 会打印原因然后退出，停一下让人看清
			print!("\r\n\x1b[33m会话 {name} 打不开（退出码 {code}），按回车返回 {brand}\x1b[0m");
			let _ = io::stdout().flush();
			while let Ok(ev) = event::read() {
				if matches!(ev, Event::Key(k) if k.code == KeyCode::Enter) {
					break;
				}
			}
			Some((format!("会话 {name} 打不开（{} 退出码 {code}）", b.open_cmd()), Color::Yellow))
		}
	}
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
			io::stdout().queue(BeginSynchronizedUpdate)?;
			terminal.draw(|f| app.draw(f))?;
			io::stdout().queue(EndSynchronizedUpdate)?.flush()?;
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
				// 切屏、清屏和重画包在一次同步更新里，终端一次显示完，不会闪出空白或 shell
				queue!(io::stdout(), BeginSynchronizedUpdate, DisableBracketedPaste, LeaveAlternateScreen)?;
				terminal.clear()?;
				terminal.draw(|f| app.draw(f))?;
				execute!(io::stdout(), Show, EndSynchronizedUpdate)?;
				painted_main = true;
				let message = attach(&project, &session);
				// 先把状态准备好（会读 projects.json），再切回备用屏幕并立刻画出来
				app.returned(&project.path, &session.session_id, message);
				take_back(&mut terminal, &app)?;
				dirty = false;
			}
			Some(Action::Trust(prompt, args)) => {
				// 信任提示画在主屏幕上，先清掉，留一行说明
				execute!(
					io::stdout(),
					DisableBracketedPaste,
					LeaveAlternateScreen,
					Clear(ClearType::All),
					MoveTo(0, 0),
					Show
				)?;
				painted_main = true;
				print!("第一次用「对话」：Claude Code 要先确认信任 HeyClaude 准备的对话目录（空目录）。\r\n");
				print!("选 Yes 后会自动回到 HeyClaude 并发出你的问题；选 No 就取消。\r\n\r\n");
				io::stdout().flush()?;
				let _ = enable_raw_mode();
				// 带上 /exit：确认信任之后 claude 马上退出，不用自己再退
				attach::run("claude", app.chat_dir(), &["/exit"], false);
				app.trusted(prompt, args);
				take_back(&mut terminal, &app)?;
				dirty = false;
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

pub fn main(which: Backend) -> ExitCode {
	backend::set(which);
	let args: Vec<String> = std::env::args().skip(1).collect();
	if which == Backend::Agy && !args.iter().any(|a| a == "-h" || a == "--help" || a == "-v" || a == "--version") {
		crate::agy::refresh_models();
	}
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
		eprintln!("{} 需要在交互式终端中运行。", which.bin());
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
	ignore(libc::SIGTSTP);
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
