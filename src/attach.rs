use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

// 在 HeyClaude 自己开的伪终端里运行 claude（`claude attach`，或者第一次用「对话」时的信任确认），
// 把按键和输出原样转发。
//
// 直接把终端交给 claude 时，claude 离开它的界面后还要半秒左右进程才退出，
// 这期间按的键（比如连按的第二下 Ctrl+Z）会被它读走丢掉。转发时能看到 claude 的输出：
// 它一离开备用屏幕，HeyClaude 就停止读键盘、拿回终端，claude 在后台慢慢退出。
// 之后按的键留在终端里，由 HeyClaude 自己处理。

pub enum Outcome {
	// claude 离开了它的界面，HeyClaude 已经拿回终端（claude 可能还在后台退出）
	Detached,
	// claude 自己退出了，没有走到离开界面那一步（比如会话打不开）
	Exited(ExitStatus),
	Failed(io::Error),
}

const LEAVE_ALT: &[u8] = b"\x1b[?1049l";
const ENTER_ALT: &[u8] = b"\x1b[?1049h";
// 离开备用屏幕后等这么久：没有马上又进入（比如 claude 打开外部编辑器）才算真的离开
const DETACH_CONFIRM: Duration = Duration::from_millis(100);

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack.windows(needle.len()).rposition(|w| w == needle)
}

// 最后一次切换屏幕是不是离开备用屏幕
fn left_alt(data: &[u8]) -> Option<bool> {
	match (rfind(data, LEAVE_ALT), rfind(data, ENTER_ALT)) {
		(None, None) => None,
		(Some(_), None) => Some(true),
		(None, Some(_)) => Some(false),
		(Some(l), Some(e)) => Some(l > e),
	}
}

fn winsize() -> libc::winsize {
	let (cols, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
	libc::winsize {
		ws_row: rows,
		ws_col: cols,
		ws_xpixel: 0,
		ws_ypixel: 0,
	}
}

fn open_pty() -> io::Result<(File, File)> {
	let (mut master, mut slave) = (0, 0);
	let ws = winsize();
	if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &ws) } != 0 {
		return Err(io::Error::last_os_error());
	}
	// master 不能传给 claude，否则 claude 退出后读 master 等不到结束
	unsafe { libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC) };
	Ok(unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) })
}

fn spawn(cwd: &str, args: &[&str], slave: &File) -> io::Result<Child> {
	let mut cmd = Command::new("claude");
	cmd.args(args)
		.current_dir(cwd)
		.stdin(Stdio::from(slave.try_clone()?))
		.stdout(Stdio::from(slave.try_clone()?))
		.stderr(Stdio::from(slave.try_clone()?));
	// 新会话，伪终端作为控制终端：claude 收不到我们终端上的信号，Ctrl+C / Ctrl+Z 都只是按键
	unsafe {
		cmd.pre_exec(|| {
			if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
				return Err(io::Error::last_os_error());
			}
			Ok(())
		});
	}
	cmd.spawn()
}

// 拿回终端后 claude 还在退出：继续读它的输出（不读它会卡在写上）直到它结束
fn drain_in_background(mut master: File, mut child: Child) {
	std::thread::spawn(move || {
		let mut buf = [0u8; 8192];
		while matches!(master.read(&mut buf), Ok(n) if n > 0) {}
		let _ = child.wait();
	});
}

pub fn run(cwd: &str, args: &[&str]) -> Outcome {
	let (master, slave) = match open_pty() {
		Ok(p) => p,
		Err(e) => return Outcome::Failed(e),
	};
	let mut child = match spawn(cwd, args, &slave) {
		Ok(c) => c,
		Err(e) => return Outcome::Failed(e),
	};
	// 父进程关掉 slave，claude 退出后读 master 才会结束
	drop(slave);

	let mut master = master;
	let mut stdout = io::stdout();
	let mut size = (0, 0);
	let mut buf = [0u8; 16384];
	// 离开备用屏幕的时间；确认之前先不读键盘，claude 的输出也先攒着
	let mut leaving: Option<Instant> = None;
	let mut held: Vec<u8> = Vec::new();
	// 上一块输出的末尾，防止切屏的控制序列被拆在两块里
	let mut tail: Vec<u8> = Vec::new();

	loop {
		let ws = winsize();
		if (ws.ws_col, ws.ws_row) != size {
			size = (ws.ws_col, ws.ws_row);
			unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
		}

		let mut fds = [
			libc::pollfd {
				fd: master.as_raw_fd(),
				events: libc::POLLIN,
				revents: 0,
			},
			libc::pollfd {
				fd: 0,
				events: libc::POLLIN,
				revents: 0,
			},
		];
		let count = if leaving.is_some() { 1 } else { 2 };
		let ready = unsafe { libc::poll(fds.as_mut_ptr(), count, 20) };
		if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
			break;
		}

		if fds[0].revents != 0 {
			match master.read(&mut buf) {
				Ok(n) if n > 0 => {
					let chunk = &buf[..n];
					let mut scan = std::mem::take(&mut tail);
					scan.extend_from_slice(chunk);
					if leaving.is_some() {
						held.extend_from_slice(chunk);
						if left_alt(&scan) == Some(false) {
							// 又进入了备用屏幕：不是离开，把攒着的输出补上，继续转发
							let _ = stdout.write_all(&held);
							let _ = stdout.flush();
							held.clear();
							leaving = None;
						}
					} else {
						let _ = stdout.write_all(chunk);
						let _ = stdout.flush();
						if left_alt(&scan) == Some(true) {
							leaving = Some(Instant::now());
						}
					}
					let keep = scan.len().saturating_sub(LEAVE_ALT.len() - 1);
					tail = scan.split_off(keep);
				}
				// claude 关掉了终端（读到 EIO），等它退出
				_ => break,
			}
		}

		if fds[1].revents != 0 && leaving.is_none() {
			let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
			if n > 0 && master.write_all(&buf[..n as usize]).is_err() {
				break;
			}
		}

		if leaving.is_some_and(|t| t.elapsed() >= DETACH_CONFIRM) {
			drain_in_background(master, child);
			return Outcome::Detached;
		}
		if let Ok(Some(status)) = child.try_wait() {
			// 退出前最后的输出（比如打不开的原因）照样显示
			if leaving.is_none() {
				let mut fds = [libc::pollfd {
					fd: master.as_raw_fd(),
					events: libc::POLLIN,
					revents: 0,
				}];
				while unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) } > 0 {
					match master.read(&mut buf) {
						Ok(n) if n > 0 => {
							let _ = stdout.write_all(&buf[..n]);
						}
						_ => break,
					}
				}
				let _ = stdout.flush();
			}
			return if leaving.is_some() {
				Outcome::Detached
			} else {
				Outcome::Exited(status)
			};
		}
	}
	let status = child.wait();
	if leaving.is_some() {
		return Outcome::Detached;
	}
	match status {
		Ok(status) => Outcome::Exited(status),
		Err(e) => Outcome::Failed(e),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn detects_last_screen_switch() {
		assert_eq!(left_alt(b"abc"), None);
		assert_eq!(left_alt(b"x\x1b[?1049ly"), Some(true));
		assert_eq!(left_alt(b"\x1b[?1049l..\x1b[?1049h"), Some(false));
		assert_eq!(left_alt(b"\x1b[?1049h..\x1b[?1049l"), Some(true));
	}
}
