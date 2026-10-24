import {execFile} from 'node:child_process';

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

// 会话归类：
//   working   进行中（包括已结束后又被你唤醒的后台任务）
//   blocked   需要输入
//   pending   待确认：Claude 说做完了，你还没确认
//   confirmed 完成：你确认过
//   failed    失败：出错（failed）或被停止（stopped），不需要确认
//   idle      空闲的交互会话（不需要处理，不统计）
// 其余结束状态（包括 Claude Code 以后新增的）都按“做完了”处理，进待确认
const FAILED_STATES = new Set(['failed', 'stopped']);

export function classify(session, confirmed = {}) {
	if (session.status === 'busy' || session.state === 'working') return 'working';
	if (session.state === 'blocked') return 'blocked';
	if (session.kind !== 'background') return 'idle';
	if (FAILED_STATES.has(session.state)) return 'failed';
	return confirmed[session.sessionId] ? 'confirmed' : 'pending';
}

// 与 `claude agents --cwd` 一致：目录本身及其子目录（含 .claude/worktrees）都算
export function isUnder(cwd, projectPath) {
	if (!cwd) return false;
	if (cwd === projectPath) return true;
	const prefix = projectPath.endsWith('/') ? projectPath : projectPath + '/';
	return cwd.startsWith(prefix);
}

export function sessionsFor(sessions, projectPath) {
	return sessions.filter(s => isUnder(s.cwd, projectPath));
}

export function statsFor(sessions, projectPath, confirmed) {
	const stats = {working: 0, blocked: 0, pending: 0, confirmed: 0, failed: 0, idle: 0};
	for (const s of sessionsFor(sessions, projectPath)) stats[classify(s, confirmed)]++;
	return stats;
}

// 已确认但又开始运行或在等你回复的会话：它有了新进展，需要重新审阅
export function reactivated(sessions, confirmed) {
	return sessions
		.filter(s => confirmed[s.sessionId] && ['working', 'blocked'].includes(classify(s)))
		.map(s => s.sessionId);
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
