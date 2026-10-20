import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {Box, Text, useApp, useInput, useStdout} from 'ink';
import {addProject, dirExists, removeProject, saveProjects} from './store.js';
import {fetchSessions, statsFor, suggestPaths} from './sessions.js';
import {completePath} from './complete.js';

const h = React.createElement;
const REFRESH_MS = 10_000;

function useTerminalSize() {
	const {stdout} = useStdout();
	const [size, setSize] = useState({columns: stdout.columns || 80, rows: stdout.rows || 24});
	useEffect(() => {
		const onResize = () => setSize({columns: stdout.columns || 80, rows: stdout.rows || 24});
		stdout.on('resize', onResize);
		return () => stdout.off('resize', onResize);
	}, [stdout]);
	return size;
}

function Header({subtitle}) {
	return h(
		Box,
		{borderStyle: 'round', borderColor: 'red', paddingX: 1},
		h(Text, {bold: true, color: 'red'}, 'HeyClaude'),
		h(Text, {dimColor: true}, `  ${subtitle}`),
	);
}

function Footer({keys, message}) {
	return h(
		Box,
		{flexDirection: 'column', marginTop: 1},
		message ? h(Text, {color: message.color}, message.text) : null,
		h(
			Text,
			{dimColor: true},
			keys.map(([key, label]) => `${key} ${label}`).join('  ·  '),
		),
	);
}

function Stats({stats, loading}) {
	if (!stats) return h(Text, {dimColor: true}, loading ? '统计中…' : '');
	return h(
		Text,
		null,
		h(Text, {color: stats.working ? 'yellow' : undefined, dimColor: !stats.working}, `${stats.working} 运行`),
		h(Text, {dimColor: true}, ' · '),
		h(Text, {color: stats.idle ? 'cyan' : undefined, dimColor: !stats.idle}, `${stats.idle} 空闲`),
		h(Text, {dimColor: true}, ` · ${stats.done} 完成`),
	);
}

function ProjectList({projects, cursor, sessions, loading, columns, rows}) {
	if (projects.length === 0) {
		return h(
			Box,
			{paddingX: 2, paddingY: 1, flexDirection: 'column'},
			h(Text, null, '还没有项目。'),
			h(Text, {dimColor: true}, '按 a 添加项目目录，之后可以在这里按项目进入 claude agents。'),
		);
	}

	const nameWidth = Math.min(28, Math.max(...projects.map(p => p.name.length)) + 2);
	const statsWidth = 28;
	const pathWidth = Math.max(10, columns - nameWidth - statsWidth - 4);

	// 列表过长时让光标保持在可见窗口内
	const visible = Math.max(3, rows - 9);
	const start = Math.min(Math.max(0, cursor - visible + 1), Math.max(0, projects.length - visible));
	const slice = projects.slice(start, start + visible);

	return h(
		Box,
		{flexDirection: 'column', paddingX: 1},
		start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 个`) : null,
		...slice.map((project, i) => {
			const index = start + i;
			const selected = index === cursor;
			const missing = !dirExists(project.path);
			return h(
				Box,
				{key: project.path},
				h(Box, {width: 2}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
				h(
					Box,
					{width: nameWidth},
					h(Text, {bold: selected, color: selected ? 'red' : undefined, wrap: 'truncate-end'}, project.name),
				),
				h(
					Box,
					{width: statsWidth, flexShrink: 0},
					h(Stats, {stats: sessions ? statsFor(sessions, project.path) : null, loading}),
				),
				h(
					Box,
					{width: pathWidth},
					h(Text, {dimColor: true, wrap: 'truncate-middle'}, missing ? `${project.path}（目录不存在）` : project.path),
				),
			);
		}),
		start + visible < projects.length
			? h(Text, {dimColor: true}, `  ↓ 还有 ${projects.length - start - visible} 个`)
			: null,
	);
}

function AddProject({projects, sessions, onDone, onCancel}) {
	const [value, setValue] = useState('');
	const [candidates, setCandidates] = useState([]);
	const [selected, setSelectedState] = useState(-1); // -1 = 输入框
	// 同一批按键（如快速 ↓↓Enter）会在重渲染前连续触发，用 ref 读最新值
	const selectedRef = useRef(-1);
	const setSelected = update => {
		selectedRef.current = typeof update === 'function' ? update(selectedRef.current) : update;
		setSelectedState(selectedRef.current);
	};
	const valueRef = useRef('');
	const [error, setError] = useState(null);

	const suggestions = useMemo(() => {
		const all = sessions ? suggestPaths(sessions, projects) : [];
		const q = value.trim().toLowerCase();
		return (q ? all.filter(s => s.path.toLowerCase().includes(q)) : all).slice(0, 8);
	}, [sessions, projects, value]);

	const submit = rawPath => {
		try {
			onDone(addProject(projects, rawPath));
		} catch (e) {
			setError(e.message);
		}
	};

	const edit = next => {
		valueRef.current = next;
		setValue(next);
		setSelected(-1);
		setCandidates([]);
		setError(null);
	};

	useInput((input, key) => {
		if (key.escape) return onCancel();
		const value = valueRef.current;
		const selected = selectedRef.current;
		if (key.return) return submit(selected >= 0 ? suggestions[selected].path : value);
		if (key.upArrow) return setSelected(s => Math.max(-1, s - 1));
		if (key.downArrow) return setSelected(s => Math.min(suggestions.length - 1, s + 1));
		if (key.tab) {
			if (selected >= 0) return edit(suggestions[selected].path);
			const result = completePath(value);
			valueRef.current = result.value;
			setValue(result.value);
			setCandidates(result.candidates);
			setError(null);
			return;
		}
		if (key.backspace || key.delete) return edit(value.slice(0, -1));
		if (key.ctrl && input === 'u') return edit('');
		if (key.ctrl || key.meta) return;
		// 过滤掉粘贴进来的换行等控制字符
		const text = input.replace(/[\u0000-\u001f\u007f]/g, '');
		if (text) edit(value + text);
	});

	return h(
		Box,
		{flexDirection: 'column', paddingX: 1},
		h(Text, {bold: true}, '添加项目'),
		h(
			Box,
			{marginTop: 1},
			h(Text, {color: selected === -1 ? 'red' : undefined}, selected === -1 ? '❯ ' : '  '),
			h(Text, null, '路径: '),
			value
				? h(Text, null, value)
				: h(Text, {dimColor: true}, `留空 = 当前目录 (${process.cwd()})`),
			selected === -1 ? h(Text, {inverse: true}, ' ') : null,
		),
		candidates.length
			? h(
					Box,
					{paddingLeft: 10},
					h(Text, {dimColor: true, wrap: 'truncate-end'}, candidates.slice(0, 20).join('  ') + (candidates.length > 20 ? '  …' : '')),
			  )
			: null,
		error ? h(Box, {paddingLeft: 2}, h(Text, {color: 'red'}, error)) : null,
		suggestions.length
			? h(
					Box,
					{flexDirection: 'column', marginTop: 1},
					h(Text, {dimColor: true}, '  有会话记录、还没添加的目录：'),
					...suggestions.map((s, i) =>
						h(
							Box,
							{key: s.path},
							h(Text, {color: 'red'}, i === selected ? '❯ ' : '  '),
							h(Text, {bold: i === selected}, s.path),
							h(Text, {dimColor: true}, `  (${s.count} 个会话)`),
						),
					),
			  )
			: null,
	);
}

export function App({initialProjects, initialPath, initialMessage, forwardedArgs}) {
	const {exit} = useApp();
	// 根节点比终端窄一列：有的终端（如 Windows Terminal）写满最后一列会被截断
	const {columns, rows} = useTerminalSize();
	const [projects, setProjects] = useState(initialProjects);
	const [cursor, setCursorState] = useState(() => Math.max(0, projects.findIndex(p => p.path === initialPath)));
	const cursorRef = useRef(cursor);
	const setCursor = update => {
		cursorRef.current = typeof update === 'function' ? update(cursorRef.current) : update;
		setCursorState(cursorRef.current);
	};
	const [screen, setScreen] = useState('list'); // list | add | delete
	const [sessions, setSessions] = useState(null);
	const [loading, setLoading] = useState(false);
	const [message, setMessage] = useState(initialMessage ?? null);

	const refresh = useCallback(async () => {
		setLoading(true);
		const result = await fetchSessions();
		setLoading(false);
		if (result.error) {
			setMessage({text: `会话统计失败：${result.error}`, color: 'red'});
			return;
		}
		setSessions(result.sessions);
	}, []);

	useEffect(() => {
		refresh();
		const timer = setInterval(refresh, REFRESH_MS);
		return () => clearInterval(timer);
	}, [refresh]);

	const persist = next => {
		try {
			saveProjects(next);
			setProjects(next);
			return true;
		} catch (e) {
			setMessage({text: `保存失败：${e.message}`, color: 'red'});
			return false;
		}
	};

	const current = projects[cursor];

	useInput(
		(input, key) => {
			const current = projects[cursorRef.current];
			setMessage(null);
			if (input === 'q' || key.escape) return exit({type: 'quit'});
			if (key.upArrow || input === 'k') return setCursor(c => Math.max(0, c - 1));
			if (key.downArrow || input === 'j') return setCursor(c => Math.min(projects.length - 1, c + 1));
			if (input === 'a') return setScreen('add');
			if (input === 'r') return refresh();
			if (!current) return;
			if (input === 'd') return setScreen('delete');
			if (key.return) {
				if (!dirExists(current.path)) {
					return setMessage({text: `目录不存在：${current.path}`, color: 'red'});
				}
				return exit({type: 'enter', project: current});
			}
		},
		{isActive: screen === 'list'},
	);

	useInput(
		input => {
			if (input === 'y' || input === 'Y') {
				if (persist(removeProject(projects, current.path))) {
					setMessage({text: `已移除 ${current.name}（目录和会话都没有动）`, color: 'green'});
					setCursor(c => Math.max(0, Math.min(c, projects.length - 2)));
				}
			}
			setScreen('list');
		},
		{isActive: screen === 'delete'},
	);

	const extra = forwardedArgs.length ? `claude agents ${forwardedArgs.join(' ')}` : '按项目管理 claude agents';

	if (screen === 'add') {
		return h(
			Box,
			{flexDirection: 'column', width: columns - 1},
			h(Header, {subtitle: extra}),
			h(AddProject, {
				projects,
				sessions,
				onCancel: () => setScreen('list'),
				onDone: ({projects: next, project}) => {
					if (persist(next)) {
						setCursor(next.findIndex(p => p.path === project.path));
						setMessage({text: `已添加 ${project.name}`, color: 'green'});
					}
					setScreen('list');
				},
			}),
			h(Footer, {
				keys: [
					['Tab', '补全/选用'],
					['↑↓', '选择建议'],
					['Enter', '添加'],
					['Ctrl+U', '清空'],
					['Esc', '取消'],
				],
			}),
		);
	}

	return h(
		Box,
		{flexDirection: 'column', width: columns - 1},
		h(Header, {subtitle: extra}),
		h(ProjectList, {projects, cursor, sessions, loading, columns, rows}),
		screen === 'delete' && current
			? h(
					Box,
					{marginTop: 1, paddingX: 1},
					h(Text, {color: 'yellow'}, `从列表移除 ${current.name}？目录和会话不受影响。(y/N)`),
			  )
			: null,
		h(Footer, {
			message,
			keys: [
				['↑↓', '选择'],
				['Enter', '进入'],
				['a', '添加'],
				['d', '删除'],
				['r', '刷新'],
				['q', '退出'],
			],
		}),
	);
}
