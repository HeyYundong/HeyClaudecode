import React, {useMemo, useRef, useState} from 'react';
import {Box, Text, useInput} from 'ink';
import {classify, sessionsFor} from './sessions.js';
import {withConfirmed, withoutConfirmed} from './reviews.js';

const h = React.createElement;

const ENDED = {
	done: {icon: '✓', label: '完成', color: 'green'},
	failed: {icon: '✗', label: '失败', color: 'red'},
	stopped: {icon: '■', label: '已停止', color: 'yellow'},
};
const BLOCKED = {icon: '●', label: '需要输入', color: 'magenta'};
const WORKING = {icon: '◌', label: '运行中', color: 'yellow'};

function look(session, group) {
	if (group === 'blocked') return BLOCKED;
	if (group === 'working') return WORKING;
	// Claude Code 以后新增的结束状态：原样显示
	return ENDED[session.state] ?? {icon: '?', label: session.state || '已结束', color: 'white'};
}

const pad = n => String(n).padStart(2, '0');
// “昨天 18:20 开始”要空格，“12 分钟前开始”不要
const withSuffix = (text, suffix) => (/\d$/.test(text) ? `${text} ${suffix}` : `${text}${suffix}`);

// 10 分钟前 / 3 小时前 / 昨天 18:20 / 10月3日 09:15
export function relativeTime(time, now = Date.now()) {
	if (!Number.isFinite(time)) return '';
	const diff = Math.max(0, now - time);
	const minutes = Math.floor(diff / 60_000);
	if (minutes < 1) return '刚刚';
	if (minutes < 60) return `${minutes} 分钟前`;
	if (minutes < 24 * 60) {
		const today = new Date(now).toDateString() === new Date(time).toDateString();
		if (today) return `${Math.floor(minutes / 60)} 小时前`;
	}
	const date = new Date(time);
	const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
	const yesterday = new Date(now);
	yesterday.setDate(yesterday.getDate() - 1);
	if (yesterday.toDateString() === date.toDateString()) return `昨天 ${clock}`;
	return `${date.getMonth() + 1}月${date.getDate()}日 ${clock}`;
}

const GROUPS = [
	{key: 'blocked', title: '等你回复'},
	{key: 'pending', title: '待确认'},
	{key: 'working', title: '运行中'},
	{key: 'confirmed', title: '已确认'},
];

export function buildGroups(sessions, projectPath, confirmed, showConfirmed) {
	const buckets = {blocked: [], pending: [], working: [], confirmed: []};
	for (const s of sessionsFor(sessions, projectPath)) {
		const group = classify(s, confirmed);
		if (buckets[group]) buckets[group].push(s);
	}
	const newest = (a, b) => (b.startedAt || 0) - (a.startedAt || 0);
	for (const key of ['blocked', 'pending', 'working']) buckets[key].sort(newest);
	buckets.confirmed.sort((a, b) => String(confirmed[b.sessionId]).localeCompare(String(confirmed[a.sessionId])));
	return {
		groups: GROUPS.filter(g => buckets[g.key].length && (g.key !== 'confirmed' || showConfirmed)).map(g => ({
			...g,
			items: buckets[g.key],
		})),
		hiddenConfirmed: showConfirmed ? 0 : buckets.confirmed.length,
		counts: Object.fromEntries(Object.entries(buckets).map(([k, v]) => [k, v.length])),
	};
}

function Row({session, group, selected, confirmedAt, columns}) {
	const {icon, label, color} = look(session, group);
	const dim = group === 'confirmed';
	const statusWidth = 10;
	const timeWidth = 18;
	const extraWidth = confirmedAt ? 14 : 0;
	const nameWidth = Math.max(10, columns - 6 - statusWidth - timeWidth - extraWidth - 4);
	const interactive = session.kind !== 'background';
	return h(
		Box,
		null,
		h(Box, {width: 2}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
		h(Box, {width: 3}, h(Text, {color: dim ? undefined : color, dimColor: dim}, icon)),
		h(
			Box,
			{width: nameWidth},
			h(
				Text,
				{bold: selected, dimColor: dim && !selected, wrap: 'truncate-end'},
				(session.name || session.id || session.sessionId) + (interactive ? '（交互）' : ''),
			),
		),
		h(Box, {width: statusWidth, flexShrink: 0}, h(Text, {color: dim ? undefined : color, dimColor: dim}, label)),
		h(
			Box,
			{width: timeWidth, flexShrink: 0},
			h(Text, {dimColor: true, wrap: 'truncate-end'}, withSuffix(relativeTime(session.startedAt), '开始')),
		),
		confirmedAt
			? h(
					Box,
					{width: extraWidth, flexShrink: 0},
					h(Text, {dimColor: true, wrap: 'truncate-end'}, withSuffix(relativeTime(Date.parse(confirmedAt)), '确认')),
			  )
			: null,
	);
}

export const REVIEW_KEYS = [
	['↑↓', '选择'],
	['c', '确认'],
	['C', '全部确认'],
	['u', '撤销'],
	['o', '打开 agents'],
	['h', '显示已确认'],
	['r', '刷新'],
	['Esc', '返回'],
];

export function ReviewPage({project, sessions, loading, confirmed, saveConfirmed, onOpen, onBack, onRefresh, setMessage, columns, rows}) {
	const [showConfirmed, setShowConfirmed] = useState(false);
	const [asking, setAsking] = useState(false);
	const [cursor, setCursorState] = useState(0);
	const cursorRef = useRef(0);
	const setCursor = update => {
		cursorRef.current = typeof update === 'function' ? update(cursorRef.current) : update;
		setCursorState(cursorRef.current);
	};
	// 撤销栈：每次确认的一批会话 ID
	const undoRef = useRef([]);

	const {groups, hiddenConfirmed, counts} = useMemo(
		() => buildGroups(sessions ?? [], project.path, confirmed, showConfirmed),
		[sessions, project.path, confirmed, showConfirmed],
	);
	const items = groups.flatMap(g => g.items.map(session => ({session, group: g.key})));
	const clamp = i => Math.max(0, Math.min(i, items.length - 1));

	const confirm = (ids, text) => {
		if (!saveConfirmed(withConfirmed(confirmed, ids))) return;
		undoRef.current.push(ids);
		setMessage({text: `${text}（u 撤销）`, color: 'green'});
	};

	useInput(
		(input, key) => {
			setMessage(null);
			if (key.escape || input === 'q' || key.leftArrow) return onBack();
			if (key.upArrow || input === 'k') return setCursor(c => clamp(c - 1));
			if (key.downArrow || input === 'j') return setCursor(c => clamp(c + 1));
			if (input === 'o' || key.return) return onOpen();
			if (input === 'r') return onRefresh();
			if (input === 'h') {
				setShowConfirmed(v => !v);
				return setCursor(0);
			}
			if (input === 'u') {
				const ids = undoRef.current.pop();
				if (!ids) return setMessage({text: '没有可以撤销的确认', color: 'yellow'});
				if (saveConfirmed(withoutConfirmed(confirmed, ids))) {
					setMessage({text: `已撤销 ${ids.length} 个任务的确认`, color: 'green'});
				}
				return;
			}
			if (input === 'C') {
				if (!counts.pending) return setMessage({text: '没有待确认的任务', color: 'yellow'});
				return setAsking(true);
			}
			if (input === 'c') {
				const item = items[clamp(cursorRef.current)];
				if (!item) return;
				const {session, group} = item;
				const name = session.name || session.id;
				if (group === 'pending') return confirm([session.sessionId], `已确认 ${name}`);
				if (group === 'confirmed') {
					if (saveConfirmed(withoutConfirmed(confirmed, [session.sessionId]))) {
						setMessage({text: `已取消确认 ${name}，它回到了待确认`, color: 'green'});
					}
					return;
				}
				return setMessage({
					text: group === 'blocked' ? '这个任务在等你回复，按 o 进入处理' : '任务还在运行，结束后才能确认',
					color: 'yellow',
				});
			}
		},
		{isActive: !asking},
	);

	useInput(
		input => {
			if (input === 'y' || input === 'Y') {
				const ids = items.filter(i => i.group === 'pending').map(i => i.session.sessionId);
				confirm(ids, `已确认 ${project.name} 下全部 ${ids.length} 个待确认任务`);
			}
			setAsking(false);
		},
		{isActive: asking},
	);

	if (!sessions) {
		return h(Box, {paddingX: 1}, h(Text, {dimColor: true}, loading ? '读取会话中…' : '没有会话数据，按 r 重试'));
	}

	// 分组标题 + 任务行，过长时让光标保持在可见窗口内
	const lines = [];
	let index = 0;
	for (const g of groups) {
		lines.push({type: 'header', group: g});
		for (const session of g.items) lines.push({type: 'item', session, group: g.key, index: index++});
	}
	const cursorLine = Math.max(0, lines.findIndex(l => l.type === 'item' && l.index === clamp(cursor)));
	const visible = Math.max(4, rows - 11);
	const start = Math.min(Math.max(0, cursorLine - visible + 1), Math.max(0, lines.length - visible));
	const slice = lines.slice(start, start + visible);

	const nothingToDo = !counts.blocked && !counts.pending;

	return h(
		Box,
		{flexDirection: 'column', paddingX: 1},
		nothingToDo ? h(Text, null, '没有需要你处理的任务。') : null,
		start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 行`) : null,
		...slice.map(line =>
			line.type === 'header'
				? h(
						Text,
						{key: `h-${line.group.key}`, bold: line.group.key !== 'confirmed', dimColor: line.group.key === 'confirmed'},
						`${line.group.title} (${line.group.items.length})`,
				  )
				: h(Row, {
						key: line.session.sessionId,
						session: line.session,
						group: line.group,
						selected: line.index === clamp(cursor),
						confirmedAt: line.group === 'confirmed' ? confirmed[line.session.sessionId] : null,
						columns: columns - 3,
				  }),
		),
		start + visible < lines.length ? h(Text, {dimColor: true}, `  ↓ 还有 ${lines.length - start - visible} 行`) : null,
		hiddenConfirmed
			? h(Box, {marginTop: 1}, h(Text, {dimColor: true}, `已隐藏 ${hiddenConfirmed} 个已确认的任务（h 显示）`))
			: null,
		asking
			? h(
					Box,
					{marginTop: 1},
					h(Text, {color: 'yellow'}, `确认 ${project.name} 下全部 ${counts.pending} 个待确认任务？(y/N)`),
			  )
			: null,
	);
}
