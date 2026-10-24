import React, {useEffect, useMemo, useRef, useState} from 'react';
import {Box, Text, useInput} from 'ink';
import {classify, sessionsFor} from './sessions.js';
import {withConfirmed, withoutConfirmed} from './reviews.js';
import {STATUS, STATUS_ORDER, nameColumn} from './ui.js';

const h = React.createElement;

// 完成的任务默认只列最近几个，按 h 展开
const RECENT_DONE = 3;

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

// 会话在项目里的位置：worktree 名字或子目录；就在项目目录里时不显示
export function whereIn(session, projectPath) {
	const cwd = session.cwd || '';
	if (cwd === projectPath) return '';
	const m = cwd.match(/\/\.claude\/worktrees\/([^/]+)/);
	if (m) return `worktree: ${m[1]}`;
	return cwd.startsWith(projectPath + '/') ? cwd.slice(projectPath.length + 1) : cwd;
}

// 失败的任务具体是哪种
const failTag = session => (session.state === 'stopped' ? '已停止' : '出错');

// 按状态分组；完成的按确认时间倒序，其余按开始时间倒序
export function groupSessions(sessions, projectPath, confirmed) {
	const groups = Object.fromEntries(STATUS_ORDER.map(k => [k, []]));
	for (const s of sessionsFor(sessions ?? [], projectPath)) {
		const g = classify(s, confirmed);
		if (groups[g]) groups[g].push(s);
	}
	const newest = (a, b) => (b.startedAt || 0) - (a.startedAt || 0);
	for (const key of STATUS_ORDER) if (key !== 'confirmed') groups[key].sort(newest);
	// 一起确认的（确认时间相同）再按开始时间排
	groups.confirmed.sort(
		(a, b) => String(confirmed[b.sessionId]).localeCompare(String(confirmed[a.sessionId])) || newest(a, b),
	);
	return groups;
}

const countsOf = groups => Object.fromEntries(STATUS_ORDER.map(k => [k, groups[k].length]));
const nameOf = session => (session.name || session.id || session.sessionId) + (session.kind !== 'background' ? '（交互）' : '');

// 默认布局的一行，和项目列表同一套样式：箭头、图标、名字、失败原因、开始时间
function Row({session, group, selected, nameWidth}) {
	const s = STATUS[group];
	const dim = group === 'confirmed';
	const tag = group === 'failed' ? failTag(session) : '';
	return h(
		Box,
		null,
		h(Box, {width: 2, flexShrink: 0}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
		h(Box, {width: 3, flexShrink: 0}, h(Text, {color: s.color, dimColor: s.dim}, s.icon)),
		h(
			Box,
			{width: nameWidth, flexShrink: 0},
			h(Text, {bold: selected, color: selected ? 'red' : undefined, dimColor: dim && !selected, wrap: 'truncate-end'}, nameOf(session)),
		),
		h(Text, {dimColor: true}, [tag, relativeTime(session.startedAt)].filter(Boolean).join('  ')),
	);
}

// --tabs 布局的一项：名字一行，灰色的详情一行
function TwoLineRow({session, group, selected, projectPath, confirmedAt}) {
	const s = STATUS[group];
	const dim = group === 'confirmed';
	const meta = [
		group === 'failed' ? failTag(session) : null,
		group === 'confirmed' && confirmedAt
			? withSuffix(relativeTime(Date.parse(confirmedAt)), '确认')
			: withSuffix(relativeTime(session.startedAt), '开始'),
		whereIn(session, projectPath),
	].filter(Boolean);
	return h(
		Box,
		{flexDirection: 'column', marginBottom: 1},
		h(
			Box,
			null,
			h(Box, {width: 2, flexShrink: 0}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
			h(Box, {width: 3, flexShrink: 0}, h(Text, {color: s.color, dimColor: s.dim}, s.icon)),
			h(Text, {bold: !dim || selected, dimColor: dim && !selected, wrap: 'truncate-end'}, nameOf(session)),
		),
		h(Box, {paddingLeft: 5}, h(Text, {dimColor: true, wrap: 'truncate-end'}, meta.join(' · '))),
	);
}

// 分组标题，写法和状态数量一样：“3 待确认”
function GroupHeader({group, count, note}) {
	const s = STATUS[group];
	return h(
		Box,
		null,
		h(Text, {bold: true, color: s.color, dimColor: s.dim}, `${count} ${s.label}`),
		note ? h(Text, {dimColor: true}, ` · ${note}`) : null,
	);
}

export function sessionKeys(layout) {
	return [
		['Enter', '打开'],
		['c', '确认'],
		['n', '新任务'],
		['Ctrl+Z', layout === 'tabs' ? '项目列表' : '返回'],
		['?', '全部按键'],
	];
}

export function sessionHelp(layout) {
	return [
		{
			title: '任务',
			keys: [
				['↑ ↓ / k j', '选择任务'],
				['Enter / o', '打开会话（claude attach），在会话里按 Ctrl+Z 回来'],
				['c', '确认待确认的任务，它变成完成；在完成的任务上按则取消确认'],
				['C', '确认这个项目下全部待确认任务'],
				['u', '撤销最近一次确认'],
				['n', '新任务：在项目目录里用 claude --bg 启动'],
				...(layout === 'tabs' ? [] : [['h', '展开或收起全部完成的任务']]),
				['r', '刷新'],
			],
		},
		...(layout === 'tabs'
			? [{title: '切换', keys: [['← →', '切换项目'], ['Tab / Shift+Tab', '切换状态']]}]
			: []),
		{title: '返回', keys: [['Ctrl+Z / Esc', layout === 'tabs' ? '回到项目列表' : '回到项目列表（连按时每 0.4 秒只退一级）']]},
	];
}

// 项目里的会话列表。layout = grouped（默认，按状态分组）或 tabs（项目和状态标签）
export function SessionsScreen({
	active = true,
	layout,
	projects,
	projectIndex,
	onSwitchProject,
	sessions,
	loading,
	confirmed,
	saveConfirmed,
	initialSessionId,
	initialView,
	onOpenSession,
	onNewTask,
	onHelp,
	onBack,
	onRefresh,
	setMessage,
	columns,
	rows,
}) {
	const project = projects[projectIndex];
	const tabs = layout === 'tabs';
	// tabs 布局当前的状态标签；null 表示还没手动选过，用默认的
	// 从会话回来时 initialView 带着离开前的标签和展开状态
	const [tabChoice, setTabChoice] = useState(() => (STATUS_ORDER.includes(initialView?.tab) ? initialView.tab : null));
	const [showAllDone, setShowAllDone] = useState(() => Boolean(initialView?.showAllDone));
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
	const counts = countsOf(groups);
	// 默认标签：先看需要你处理的
	const tab = tabChoice ?? ['blocked', 'pending', 'failed', 'working', 'confirmed'].find(k => counts[k]) ?? 'working';

	// 可以选中的任务，按显示顺序
	const shownIn = g => (!tabs && g === 'confirmed' && !showAllDone ? groups[g].slice(0, RECENT_DONE) : groups[g]);
	const visibleGroups = tabs ? [tab] : STATUS_ORDER;
	const items = visibleGroups.flatMap(g => shownIn(g).map(session => ({session, group: g})));
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
		const i = STATUS_ORDER.indexOf(tab);
		setTabChoice(STATUS_ORDER[(i + step + STATUS_ORDER.length) % STATUS_ORDER.length]);
		setCursor(0);
	};

	useInput(
		(input, key) => {
			setMessage(null);
			if ((key.ctrl && input === 'z') || key.escape) return onBack();
			if (input === '?') return onHelp();
			if (key.upArrow || input === 'k') return setCursor(c => clamp(c - 1));
			if (key.downArrow || input === 'j') return setCursor(c => clamp(c + 1));
			if (tabs && (key.leftArrow || key.rightArrow)) {
				setCursor(0);
				return onSwitchProject(key.leftArrow ? -1 : 1);
			}
			if (tabs && key.tab) return switchTab(key.shift ? -1 : 1);
			if (!tabs && input === 'h') return setShowAllDone(v => !v);
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
				return onOpenSession(session, {tab, showAllDone});
			}
			if (input === 'c') {
				if (group === 'pending') return confirm([session.sessionId], `已确认 ${name}`);
				if (group === 'confirmed') {
					if (saveConfirmed(withoutConfirmed(confirmed, [session.sessionId]))) {
						setMessage({text: `已取消确认 ${name}，它回到了待确认`, color: 'green'});
					}
					return;
				}
				const why = {
					working: '任务还在进行，结束后才能确认',
					blocked: '这个任务在等你输入，按 Enter 打开它',
					failed: '失败的任务不需要确认',
				};
				return setMessage({text: why[group], color: 'yellow'});
			}
		},
		{isActive: active && !asking},
	);

	useInput(
		input => {
			if (input === 'y' || input === 'Y') {
				const ids = groups.pending.map(s => s.sessionId);
				confirm(ids, `已确认 ${project.name} 下全部 ${ids.length} 个待确认任务`);
			}
			setAsking(false);
		},
		{isActive: active && asking},
	);

	const askLine = asking
		? h(Box, {marginTop: 1}, h(Text, {color: 'yellow'}, `确认 ${project.name} 下全部 ${groups.pending.length} 个待确认任务？(y/N)`))
		: null;

	if (!sessions) {
		return h(Text, {dimColor: true}, loading ? '读取会话中…' : '没有会话数据，按 r 重试');
	}

	if (tabs) {
		// 第一行：项目；第二行：状态标签；下面是这个状态的任务，每个两行
		const projectLine = h(
			Box,
			{flexWrap: 'wrap'},
			h(Text, {bold: true, color: 'red'}, 'HeyClaude'),
			...projects.map((p, i) =>
				h(
					Box,
					{key: p.path, marginLeft: 3},
					h(Text, {bold: i === projectIndex, underline: i === projectIndex, dimColor: i !== projectIndex}, p.name),
				),
			),
			h(Box, {marginLeft: 3}, h(Text, {dimColor: true}, '← →')),
		);
		const tabLine = h(
			Box,
			{flexWrap: 'wrap', marginTop: 1},
			...STATUS_ORDER.map((k, i) => {
				const s = STATUS[k];
				const selected = k === tab;
				return h(
					Box,
					{key: k, marginLeft: i ? 3 : 0},
					h(
						Text,
						{color: s.color, dimColor: (s.dim || !counts[k]) && !selected, bold: selected, underline: selected},
						`${counts[k]} ${s.label}`,
					),
				);
			}),
			h(Box, {marginLeft: 3}, h(Text, {dimColor: true}, 'Tab')),
		);
		const perItem = 3;
		const visible = Math.max(1, Math.floor((rows - 10) / perItem));
		const c = clamp(cursor);
		const start = Math.min(Math.max(0, c - visible + 1), Math.max(0, items.length - visible));
		return h(
			Box,
			{flexDirection: 'column'},
			projectLine,
			tabLine,
			h(Text, {dimColor: true}, '─'.repeat(Math.max(10, columns - 5))),
			h(Box, {marginTop: 1, flexDirection: 'column'},
				items.length ? null : h(Text, {dimColor: true}, `没有${STATUS[tab].label}的任务`),
				start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 个`) : null,
				...items.slice(start, start + visible).map((item, i) =>
					h(TwoLineRow, {
						key: item.session.sessionId,
						session: item.session,
						group: item.group,
						selected: start + i === c,
						projectPath: project.path,
						confirmedAt: confirmed[item.session.sessionId],
					}),
				),
				start + visible < items.length ? h(Text, {dimColor: true}, `  ↓ 还有 ${items.length - start - visible} 个`) : null,
			),
			askLine,
		);
	}

	// 默认布局：按状态分组，组之间空一行
	const nameWidth = nameColumn(items.map(i => nameOf(i.session)), columns - 4);
	const lines = [];
	let index = 0;
	for (const g of STATUS_ORDER) {
		if (!groups[g].length) continue;
		if (lines.length) lines.push({type: 'gap', key: `gap-${g}`});
		const collapsed = g === 'confirmed' && !showAllDone && groups[g].length > RECENT_DONE;
		const note = collapsed ? `最近 ${RECENT_DONE} 个` : null;
		lines.push({type: 'header', key: `h-${g}`, group: g, count: groups[g].length, note});
		for (const session of shownIn(g)) lines.push({type: 'item', key: session.sessionId, session, group: g, index: index++});
		if (collapsed) lines.push({type: 'more', key: 'more', text: `还有 ${groups[g].length - RECENT_DONE} 个，按 h 展开`});
	}
	const visible = Math.max(4, rows - 9);
	const cursorLine = Math.max(0, lines.findIndex(l => l.type === 'item' && l.index === clamp(cursor)));
	const start = Math.min(Math.max(0, cursorLine - visible + 1), Math.max(0, lines.length - visible));

	return h(
		Box,
		{flexDirection: 'column'},
		lines.length ? null : h(Text, {dimColor: true}, '这个项目还没有后台任务，按 n 启动一个'),
		start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 行`) : null,
		...lines.slice(start, start + visible).map(line => {
			if (line.type === 'gap') return h(Text, {key: line.key}, ' ');
			if (line.type === 'header') return h(GroupHeader, {key: line.key, group: line.group, count: line.count, note: line.note});
			if (line.type === 'more') return h(Box, {key: line.key, paddingLeft: 5}, h(Text, {dimColor: true}, line.text));
			return h(Row, {key: line.key, session: line.session, group: line.group, selected: line.index === clamp(cursor), nameWidth});
		}),
		start + visible < lines.length ? h(Text, {dimColor: true}, `  ↓ 还有 ${lines.length - start - visible} 行`) : null,
		askLine,
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
		{flexDirection: 'column'},
		h(Text, {dimColor: true}, `在 ${project.name} 里用 claude --bg 启动一个后台任务`),
		h(
			Box,
			{marginTop: 1},
			h(Text, {color: 'red'}, '❯ '),
			value ? h(Text, null, value) : h(Text, {dimColor: true}, '要 Claude 做什么？'),
			h(Text, {inverse: true}, ' '),
		),
	);
}

