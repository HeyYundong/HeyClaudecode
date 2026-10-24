import React, {useEffect, useMemo, useRef, useState} from 'react';
import {Box, Text, useInput} from 'ink';
import {classify, sessionsFor} from './sessions.js';
import {withConfirmed, withoutConfirmed} from './reviews.js';

const h = React.createElement;

// 四种状态。「待确认」是 HeyClaude 加的：Claude 说完成了，你还没确认
export const STATUS = {
	blocked: {icon: '●', label: '等你回复', color: 'magenta'},
	pending: {icon: '◆', label: '待确认', color: 'cyan'},
	working: {icon: '◌', label: '运行中', color: 'yellow'},
	confirmed: {icon: '✓', label: '已完成', color: undefined},
};
export const GROUP_ORDER = ['blocked', 'pending', 'working', 'confirmed'];

// 已结束的任务具体是怎么结束的
const ENDED = {
	done: {label: '完成', color: 'green'},
	failed: {label: '失败', color: 'red'},
	stopped: {label: '已停止', color: 'yellow'},
};

function detail(session, group) {
	if (group === 'blocked') return {label: '需要输入', color: 'magenta'};
	if (group === 'working') return {label: '运行中', color: 'yellow'};
	// Claude Code 以后新增的结束状态：原样显示
	return ENDED[session.state] ?? {label: session.state || '已结束', color: undefined};
}

const pad = n => String(n).padStart(2, '0');
// “昨天 18:20 开始”要空格，“12 分钟前开始”不要
export const withSuffix = (text, suffix) => (/\d$/.test(text) ? `${text} ${suffix}` : `${text}${suffix}`);

// 刚刚 / 10 分钟前 / 3 小时前 / 昨天 18:20 / 10月3日
export function relativeTime(time, now = Date.now()) {
	if (!Number.isFinite(time)) return '';
	const minutes = Math.floor(Math.max(0, now - time) / 60_000);
	if (minutes < 1) return '刚刚';
	if (minutes < 60) return `${minutes} 分钟前`;
	if (minutes < 24 * 60 && new Date(now).toDateString() === new Date(time).toDateString()) {
		return `${Math.floor(minutes / 60)} 小时前`;
	}
	const date = new Date(time);
	const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
	const yesterday = new Date(now);
	yesterday.setDate(yesterday.getDate() - 1);
	if (yesterday.toDateString() === date.toDateString()) return `昨天 ${clock}`;
	// 更早的只显示日期，跨年时带上年份
	const sameYear = date.getFullYear() === new Date(now).getFullYear();
	return `${sameYear ? '' : date.getFullYear() + '年'}${date.getMonth() + 1}月${date.getDate()}日`;
}

// 会话在项目里的位置：项目根目录 / worktree 名字 / 子目录
export function whereIn(session, projectPath) {
	const cwd = session.cwd || '';
	if (cwd === projectPath) return '项目根目录';
	const m = cwd.match(/\/\.claude\/worktrees\/([^/]+)/);
	if (m) return `worktree: ${m[1]}`;
	return cwd.startsWith(projectPath + '/') ? cwd.slice(projectPath.length + 1) : cwd;
}

// 按状态分组；已完成的按确认时间倒序，其余按开始时间倒序
export function groupSessions(sessions, projectPath, confirmed) {
	const groups = {blocked: [], pending: [], working: [], confirmed: []};
	for (const s of sessionsFor(sessions ?? [], projectPath)) {
		const g = classify(s, confirmed);
		if (groups[g]) groups[g].push(s);
	}
	const newest = (a, b) => (b.startedAt || 0) - (a.startedAt || 0);
	for (const key of ['blocked', 'pending', 'working']) groups[key].sort(newest);
	groups.confirmed.sort((a, b) => String(confirmed[b.sessionId]).localeCompare(String(confirmed[a.sessionId])));
	return groups;
}

const TABS = [{key: 'all', label: '全部'}, ...GROUP_ORDER.map(key => ({key, label: STATUS[key].label}))];

function Row({session, group, selected, showLabel, extra, columns}) {
	const status = STATUS[group];
	const dim = group === 'confirmed';
	const d = detail(session, group);
	const labelWidth = showLabel ? 10 : 0;
	const detailWidth = 10;
	const timeWidth = 18;
	const extraWidth = 22;
	const nameWidth = Math.max(12, columns - 5 - labelWidth - detailWidth - timeWidth - extraWidth - 2);
	const interactive = session.kind !== 'background';
	return h(
		Box,
		null,
		h(Box, {width: 2, flexShrink: 0}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
		h(Box, {width: 3, flexShrink: 0}, h(Text, {color: status.color, dimColor: dim}, status.icon)),
		showLabel
			? h(Box, {width: labelWidth, flexShrink: 0}, h(Text, {color: status.color, dimColor: dim}, status.label))
			: null,
		h(
			Box,
			{width: nameWidth},
			h(
				Text,
				{bold: selected, dimColor: dim && !selected, wrap: 'truncate-end'},
				(session.name || session.id || session.sessionId) + (interactive ? '（交互）' : ''),
			),
		),
		h(Box, {width: detailWidth, flexShrink: 0}, h(Text, {color: dim ? undefined : d.color, dimColor: dim}, d.label)),
		h(
			Box,
			{width: timeWidth, flexShrink: 0},
			h(Text, {dimColor: true, wrap: 'truncate-end'}, withSuffix(relativeTime(session.startedAt), '开始')),
		),
		h(Box, {width: extraWidth, flexShrink: 0}, h(Text, {dimColor: true, wrap: 'truncate-end'}, extra)),
	);
}

function Chip({label, selected, color, dim}) {
	return h(
		Text,
		{inverse: selected, bold: selected, color: selected ? undefined : color, dimColor: dim && !selected},
		` ${label} `,
	);
}

export function sessionKeys(layout) {
	return [
		['Enter', '打开会话'],
		['c', '确认'],
		['C', '全部确认'],
		['u', '撤销'],
		['n', '新任务'],
		...(layout === 'tabs' ? [['←→', '项目'], ['Tab', '状态']] : []),
		['Ctrl+Z', layout === 'tabs' ? '项目列表' : '返回'],
	];
}

// 项目里的会话列表。layout = grouped（方案 A，按状态分组）或 tabs（方案 D，项目和状态标签）
export function SessionsScreen({
	layout,
	projects,
	projectIndex,
	onSwitchProject,
	sessions,
	loading,
	confirmed,
	saveConfirmed,
	initialSessionId,
	initialTab,
	onOpenSession,
	onNewTask,
	onBack,
	onRefresh,
	setMessage,
	columns,
	rows,
}) {
	const project = projects[projectIndex];
	const tabs = layout === 'tabs';
	const [tab, setTab] = useState(() => (TABS.some(t => t.key === initialTab) ? initialTab : 'all'));
	const [asking, setAsking] = useState(false);
	const [cursor, setCursorState] = useState(0);
	const cursorRef = useRef(0);
	const setCursor = update => {
		cursorRef.current = typeof update === 'function' ? update(cursorRef.current) : update;
		setCursorState(cursorRef.current);
	};
	// 撤销栈：每次确认的一批会话 ID
	const undoRef = useRef([]);

	const groups = useMemo(() => groupSessions(sessions, project.path, confirmed), [sessions, project.path, confirmed]);
	const visibleGroups = tabs && tab !== 'all' ? [tab] : GROUP_ORDER;
	const items = visibleGroups.flatMap(g => groups[g].map(session => ({session, group: g})));
	const clamp = i => Math.max(0, Math.min(i, items.length - 1));

	// 从会话返回时，数据到了以后把光标放回刚才打开的任务上
	const restoredRef = useRef(!initialSessionId);
	useEffect(() => {
		if (restoredRef.current || !sessions) return;
		restoredRef.current = true;
		const index = items.findIndex(i => i.session.sessionId === initialSessionId);
		if (index >= 0) setCursor(index);
	}, [sessions]);

	const confirm = (ids, text) => {
		if (!saveConfirmed(withConfirmed(confirmed, ids))) return;
		undoRef.current.push(ids);
		setMessage({text: `${text}（u 撤销）`, color: 'green'});
	};

	const switchTab = step => {
		const i = TABS.findIndex(t => t.key === tab);
		setTab(TABS[(i + step + TABS.length) % TABS.length].key);
		setCursor(0);
	};

	useInput(
		(input, key) => {
			setMessage(null);
			if ((key.ctrl && input === 'z') || key.escape) return onBack();
			if (key.upArrow || input === 'k') return setCursor(c => clamp(c - 1));
			if (key.downArrow || input === 'j') return setCursor(c => clamp(c + 1));
			if (tabs && (key.leftArrow || key.rightArrow)) {
				setCursor(0);
				return onSwitchProject(key.leftArrow ? -1 : 1);
			}
			if (tabs && key.tab) return switchTab(key.shift ? -1 : 1);
			if (input === 'r') return onRefresh();
			if (input === 'n') return onNewTask();
			if (input === 'u') {
				const ids = undoRef.current.pop();
				if (!ids) return setMessage({text: '没有可以撤销的确认', color: 'yellow'});
				if (saveConfirmed(withoutConfirmed(confirmed, ids))) {
					setMessage({text: `已撤销 ${ids.length} 个任务的确认`, color: 'green'});
				}
				return;
			}
			if (input === 'C') {
				if (!groups.pending.length) return setMessage({text: '没有待确认的任务', color: 'yellow'});
				return setAsking(true);
			}
			const item = items[clamp(cursorRef.current)];
			if (!item) return;
			const {session, group} = item;
			const name = session.name || session.id;
			if (input === 'o' || key.return) {
				if (session.kind !== 'background') {
					return setMessage({text: '这是交互会话，请在它自己的终端里操作', color: 'yellow'});
				}
				return onOpenSession(session, tab);
			}
			if (input === 'c') {
				if (group === 'pending') return confirm([session.sessionId], `已确认 ${name}`);
				if (group === 'confirmed') {
					if (saveConfirmed(withoutConfirmed(confirmed, [session.sessionId]))) {
						setMessage({text: `已取消确认 ${name}，它回到了待确认`, color: 'green'});
					}
					return;
				}
				return setMessage({
					text: group === 'blocked' ? '这个任务在等你回复，按 Enter 打开它' : '任务还在运行，结束后才能确认',
					color: 'yellow',
				});
			}
		},
		{isActive: !asking},
	);

	useInput(
		input => {
			if (input === 'y' || input === 'Y') {
				const ids = groups.pending.map(s => s.sessionId);
				confirm(ids, `已确认 ${project.name} 下全部 ${ids.length} 个待确认任务`);
			}
			setAsking(false);
		},
		{isActive: asking},
	);

	if (!sessions) {
		return h(Box, {paddingX: 1}, h(Text, {dimColor: true}, loading ? '读取会话中…' : '没有会话数据，按 r 重试'));
	}

	// 列表行：方案 A 带分组标题，方案 D 只有任务行
	const lines = [];
	let index = 0;
	for (const g of visibleGroups) {
		if (!groups[g].length) continue;
		if (!tabs) lines.push({type: 'header', group: g});
		for (const session of groups[g]) lines.push({type: 'item', session, group: g, index: index++});
	}
	const reserved = tabs ? 13 : 10;
	const visible = Math.max(3, rows - reserved);
	const cursorLine = Math.max(0, lines.findIndex(l => l.type === 'item' && l.index === clamp(cursor)));
	const start = Math.min(Math.max(0, cursorLine - visible + 1), Math.max(0, lines.length - visible));
	const slice = lines.slice(start, start + visible);

	const extraFor = line => {
		if (tabs) return whereIn(line.session, project.path);
		const at = confirmed[line.session.sessionId];
		return line.group === 'confirmed' && at ? withSuffix(relativeTime(Date.parse(at)), '确认') : '';
	};

	const empty = tabs && tab !== 'all' ? `没有${STATUS[tab].label}的任务` : '这个项目还没有后台任务，按 n 派发一个';

	return h(
		Box,
		{flexDirection: 'column', paddingX: 1},
		tabs
			? h(
					Box,
					{flexDirection: 'column', marginBottom: 1},
					h(
						Box,
						{flexWrap: 'wrap'},
						h(Box, {width: 6}, h(Text, {dimColor: true}, '项目')),
						...projects.map((p, i) =>
							h(Box, {key: p.path, marginRight: 1}, h(Chip, {label: `${i + 1} ${p.name}`, selected: i === projectIndex})),
						),
					),
					h(
						Box,
						{flexWrap: 'wrap'},
						h(Box, {width: 6}, h(Text, {dimColor: true}, '状态')),
						...TABS.map(t => {
							const count = t.key === 'all' ? GROUP_ORDER.reduce((n, g) => n + groups[g].length, 0) : groups[t.key].length;
							const s = STATUS[t.key];
							return h(
								Box,
								{key: t.key, marginRight: 1},
								h(Chip, {
									label: `${s ? s.icon + ' ' : ''}${t.label} ${count}`,
									selected: t.key === tab,
									color: s?.color,
									dim: t.key === 'confirmed' || (s && !count),
								}),
							);
						}),
					),
			  )
			: null,
		lines.length ? null : h(Text, {dimColor: true}, empty),
		start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 行`) : null,
		...slice.map(line =>
			line.type === 'header'
				? h(
						Box,
						{key: `h-${line.group}`, marginTop: line === lines[0] ? 0 : 1},
						h(
							Text,
							{bold: line.group !== 'confirmed', color: STATUS[line.group].color, dimColor: line.group === 'confirmed'},
							`${STATUS[line.group].label} (${groups[line.group].length})`,
						),
				  )
				: h(Row, {
						key: line.session.sessionId,
						session: line.session,
						group: line.group,
						selected: line.index === clamp(cursor),
						showLabel: tabs && tab === 'all',
						extra: extraFor(line),
						columns: columns - 3,
				  }),
		),
		start + visible < lines.length ? h(Text, {dimColor: true}, `  ↓ 还有 ${lines.length - start - visible} 行`) : null,
		items.length ? h(Box, {marginTop: 1}, h(Text, {dimColor: true}, '打开会话后按 Ctrl+Z 回到这里')) : null,
		asking
			? h(
					Box,
					{marginTop: 1},
					h(Text, {color: 'yellow'}, `确认 ${project.name} 下全部 ${groups.pending.length} 个待确认任务？(y/N)`),
			  )
			: null,
	);
}

// 新任务：输入要做的事，用 claude --bg 在项目目录里启动
export function NewTask({project, onSubmit, onCancel}) {
	const [value, setValue] = useState('');
	const valueRef = useRef('');
	const edit = next => {
		valueRef.current = next;
		setValue(next);
	};
	useInput((input, key) => {
		if (key.escape || (key.ctrl && input === 'z')) return onCancel();
		if (key.return) {
			const text = valueRef.current.trim();
			return text ? onSubmit(text) : onCancel();
		}
		if (key.backspace || key.delete) return edit(valueRef.current.slice(0, -1));
		if (key.ctrl && input === 'u') return edit('');
		if (key.ctrl || key.meta) return;
		const text = input.replace(/[\u0000-\u001f\u007f]/g, '');
		if (text) edit(valueRef.current + text);
	});
	return h(
		Box,
		{flexDirection: 'column', paddingX: 1},
		h(Text, {bold: true}, `新任务 · ${project.name}`),
		h(Text, {dimColor: true}, `在 ${project.path} 里用 claude --bg 启动一个后台任务`),
		h(
			Box,
			{marginTop: 1},
			h(Text, {color: 'red'}, '❯ '),
			value ? h(Text, null, value) : h(Text, {dimColor: true}, '要 Claude 做什么？'),
			h(Text, {inverse: true}, ' '),
		),
	);
}
