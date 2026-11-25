// 管理的是哪一种 agent：hcc 管 Claude Code，hagy 管 Antigravity CLI（agy）。
// 程序启动时设一次，之后到处只读，所以用全局变量，不必一层层传下去
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
	Claude,
	Agy,
}

static CURRENT: OnceLock<Backend> = OnceLock::new();

pub fn set(backend: Backend) {
	let _ = CURRENT.set(backend);
}

pub fn current() -> Backend {
	CURRENT.get().copied().unwrap_or(Backend::Claude)
}

pub fn is_agy() -> bool {
	current() == Backend::Agy
}

impl Backend {
	// 要调用的命令
	pub fn program(self) -> &'static str {
		match self {
			Backend::Claude => "claude",
			Backend::Agy => "agy",
		}
	}

	// 左上角的名字
	pub fn brand(self) -> &'static str {
		match self {
			Backend::Claude => "HeyClaude",
			Backend::Agy => "HeyAgy",
		}
	}

	// 界面里怎么称呼干活的 agent
	pub fn agent(self) -> &'static str {
		match self {
			Backend::Claude => "Claude",
			Backend::Agy => "Agy",
		}
	}

	pub fn product(self) -> &'static str {
		match self {
			Backend::Claude => "Claude Code",
			Backend::Agy => "Antigravity CLI",
		}
	}

	pub fn bin(self) -> &'static str {
		match self {
			Backend::Claude => "hcc",
			Backend::Agy => "hagy",
		}
	}

	// 启动后台任务用的命令（显示用）
	pub fn start_cmd(self) -> &'static str {
		match self {
			Backend::Claude => "claude --bg",
			Backend::Agy => "agy -p",
		}
	}

	// 打开会话用的命令（显示用）
	pub fn open_cmd(self) -> &'static str {
		match self {
			Backend::Claude => "claude attach",
			Backend::Agy => "agy --conversation",
		}
	}
}
