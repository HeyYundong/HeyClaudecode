import {execFile} from 'node:child_process';

const FINISHED_STATES = new Set(['done', 'stopped', 'failed']);

// 读取 `claude agents --json --all` 的会话列表
export function fetchSessions() {
	return new Promise(resolve => {
		execFile(
			'claude',
			['agents', '--json', '--all'],
			{timeout: 20_000, maxBuffer: 16 * 1024 * 1024},
			(error, stdout) => {
				if (error) {
					resolve({sessions: [], error: error.code === 'ENOENT' ? '找不到 claude 命令' : error.message});
					return;
				}
				try {
					const data = JSON.parse(stdout);
					resolve({sessions: Array.isArray(data) ? data : [], error: null});
				} catch (parseError) {
					resolve({sessions: [], error: `无法解析会话列表: ${parseError.message}`});
				}
			},
		);
	});
}

export function classify(session) {
	if (session.kind === 'background' && FINISHED_STATES.has(session.state)) return 'done';
	if (session.status === 'busy') return 'working';
	if (session.kind === 'background') return 'working';
	return 'idle';
}

// 与 `claude agents --cwd` 一致：目录本身及其子目录（含 .claude/worktrees）都算
export function isUnder(cwd, projectPath) {
	if (!cwd) return false;
	if (cwd === projectPath) return true;
	const prefix = projectPath.endsWith('/') ? projectPath : projectPath + '/';
	return cwd.startsWith(prefix);
}

export function statsFor(sessions, projectPath) {
	const stats = {working: 0, idle: 0, done: 0};
	for (const s of sessions) {
		if (isUnder(s.cwd, projectPath)) stats[classify(s)]++;
	}
	return stats;
}

// worktree 会话归到它所属的项目目录
function projectRoot(cwd) {
	const i = cwd.indexOf('/.claude/worktrees/');
	return i === -1 ? cwd : cwd.slice(0, i);
}

// 有会话记录但还没添加的目录，按会话数倒序
export function suggestPaths(sessions, projects) {
	const added = new Set(projects.map(p => p.path));
	const counts = new Map();
	for (const s of sessions) {
		if (!s.cwd) continue;
		const root = projectRoot(s.cwd);
		if (added.has(root)) continue;
		counts.set(root, (counts.get(root) || 0) + 1);
	}
	return [...counts.entries()]
		.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
		.map(([path, count]) => ({path, count}));
}
