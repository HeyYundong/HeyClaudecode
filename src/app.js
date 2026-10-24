import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {execFile} from 'node:child_process';
import {Box, Text, useApp, useInput, useStdout} from 'ink';
import {addProject, dirExists, removeProject, saveProjects} from './store.js';
import {fetchSessions, reactivated, statsFor, suggestPaths} from './sessions.js';
import {repliedAfterConfirm} from './transcripts.js';
import {completePath} from './complete.js';
import {loadReviews, saveReviews, withoutConfirmed} from './reviews.js';
import {NewTask, SessionsScreen, sessionHelp, sessionKeys} from './sessionsScreen.js';
import {HelpScreen, KeyHints, STATUS_ORDER, StatusCounts, TopBar, nameColumn} from './ui.js';

const h = React.createElement;
const REFRESH_MS = 10_000;
// 多位序号：两次数字键间隔超过这个时间就按已输入的序号进入
const DIGIT_WAIT_MS = 600;

const isBack = (input, key) => key.ctrl && input === 'z';
// 连按 Ctrl+Z 时，每隔这么久才退一级，免得一路退出 HeyClaude
const BACK_GAP_MS = 400;
// 从会话回来后，这段时间内的 Ctrl+Z 是离开会话时多按的，忽略
const RETURN_QUIET_MS = 600;

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

// 所有项目的合计
function totalCounts(projects, sessions, confirmed) {
	const total = Object.fromEntries(STATUS_ORDER.map(k => [k, 0]));
	if (!sessions) return total;
	for (const p of projects) {
		const s = statsFor(sessions, p.path, confirmed);
		for (const k of STATUS_ORDER) total[k] += s[k];
	}
	return total;
}

const LIST_HELP = [
	{
		title: '项目列表',
		keys: [
			['↑ ↓ / k j', '选择项目'],
			['Enter', '进入项目'],
			['1–9…', '按序号直接进入（超过 9 个可以连按多位）'],
			['a', '添加项目'],
			['d', '从列表移除项目（目录和会话不动）'],
			['r', '刷新（每 10 秒也会自动刷新）'],
			['Ctrl+Z / q', '退出'],
		],
	},
];

// 每个项目一行：序号、名字、不为零的状态数量。样式和任务列表的行一致
function ProjectList({projects, cursor, sessions, confirmed, loading, columns, rows}) {
	if (projects.length === 0) {
		return h(
			Box,
			{flexDirection: 'column'},
			h(Text, null, '还没有项目。'),
			h(Text, {dimColor: true}, '按 a 添加项目目录。'),
		);
	}

	const numberWidth = Math.max(3, String(projects.length).length + 2);
	const nameWidth = nameColumn(projects.map(p => p.name), columns - 4);

	// 列表过长时让光标保持在可见窗口内
	const visible = Math.max(1, rows - 7);
	const start = Math.min(Math.max(0, cursor - visible + 1), Math.max(0, projects.length - visible));
	const slice = projects.slice(start, start + visible);

	return h(
		Box,
		{flexDirection: 'column'},
		start > 0 ? h(Text, {dimColor: true}, `  ↑ 还有 ${start} 个`) : null,
		...slice.map((project, i) => {
			const index = start + i;
			const selected = index === cursor;
			const missing = !dirExists(project.path);
			const stats = sessions ? statsFor(sessions, project.path, confirmed) : null;
			const empty = stats && !STATUS_ORDER.some(k => stats[k]);
			return h(
				Box,
				{key: project.path},
				h(
					Box,
					null,
					h(Box, {width: 2, flexShrink: 0}, h(Text, {color: 'red'}, selected ? '❯' : ' ')),
					h(Box, {width: numberWidth, flexShrink: 0}, h(Text, {dimColor: true}, `${index + 1}`)),
					h(
						Box,
						{width: nameWidth, flexShrink: 0},
						h(Text, {bold: selected, color: selected ? 'red' : undefined, wrap: 'truncate-end'}, project.name),
					),
					missing
						? h(Text, {color: 'yellow'}, '目录不存在（按 d 移除）')
						: stats
							? empty
								? h(Text, {dimColor: true}, '没有任务')
								: h(StatusCounts, {counts: stats, sep: '   '})
							: h(Text, {dimColor: true}, loading ? '…' : ''),
				),
			);
		}),
		start + visible < projects.length ? h(Text, {dimColor: true}, `  ↓ 还有 ${projects.length - start - visible} 个`) : null,
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
		if (key.escape || isBack(input, key)) return onCancel();
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
			value ? h(Text, null, value) : h(Text, {dimColor: true}, `留空 = 当前目录 (${process.cwd()})`),
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

// 用 claude --bg 在项目目录里启动后台任务，返回它打印的 ID
function dispatchTask(project, prompt, forwardedArgs) {
	return new Promise(resolve => {
		execFile(
			'claude',
			[...forwardedArgs, '--bg', prompt],
			{cwd: project.path, timeout: 60_000},
			(error, stdout, stderr) => {
				if (error) return resolve({error: (stderr || error.message).trim().split('\n').pop()});
				resolve({output: stdout.trim().split('\n').pop() || ''});
			},
		);
	});
}

// layout: grouped（默认，方案 A）或 tabs（hcc --tabs，方案 D）
export function App({layout, initialProjects, initialPath, initialMessage, initialScreen, initialSessionId, initialView, forwardedArgs}) {
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
	// list | add | delete | sessions | newtask。sessions 时 cursor 指向当前项目
	// 方案 D 启动时直接进会话视图；从会话回来时回到会话视图
	const [screen, setScreen] = useState(() => {
		if (!initialProjects.length) return 'list';
		if (initialScreen === 'sessions' || layout === 'tabs') return 'sessions';
		return 'list';
	});
	const [sessions, setSessions] = useState(null);
	const [loading, setLoading] = useState(false);
	const [message, setMessage] = useState(initialMessage ?? null);
	// ? 打开的按键说明（内容随所在界面变化），null 表示没打开
	const [help, setHelp] = useState(null);

	// 返回上一级（Ctrl+Z / Esc）都经过这里：间隔太短的重复按键忽略
	const backAtRef = useRef(initialScreen === 'sessions' ? Date.now() + RETURN_QUIET_MS - BACK_GAP_MS : 0);
	const goBack = fn => {
		const now = Date.now();
		if (now - backAtRef.current < BACK_GAP_MS) return;
		backAtRef.current = now;
		fn();
	};

	// 你确认过的会话。用 ref 保存最新值，异步刷新时也能读到
	const [confirmed, setConfirmedState] = useState(() => {
		try {
			return loadReviews();
		} catch (e) {
			setTimeout(() => setMessage({text: e.message, color: 'red'}));
			return {};
		}
	});
	const confirmedRef = useRef(confirmed);
	const saveConfirmed = useCallback(next => {
		try {
			saveReviews(next);
		} catch (e) {
			setMessage({text: `保存确认记录失败：${e.message}`, color: 'red'});
			return false;
		}
		confirmedRef.current = next;
		setConfirmedState(next);
		return true;
	}, []);

	const refresh = useCallback(async () => {
		setLoading(true);
		const result = await fetchSessions();
		setLoading(false);
		if (result.error) {
			setMessage({text: `读取会话失败：${result.error}`, color: 'red'});
			return;
		}
		// 完成的任务又有了新进展，取消确认，让它重新走一遍待确认：
		// 一是此刻又在进行中或需要输入；二是确认之后又有过新对话（即使已经答完了，
		// 比如你在打开的会话里回复、Claude 在你回到 HeyClaude 之前就答完）
		const revived = new Set([
			...reactivated(result.sessions, confirmedRef.current),
			...repliedAfterConfirm(confirmedRef.current),
		]);
		if (revived.size) saveConfirmed(withoutConfirmed(confirmedRef.current, [...revived]));
		setSessions(result.sessions);
	}, [saveConfirmed]);

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

	const openSessions = index => {
		setCursor(index);
		setScreen('sessions');
	};

	const submitTask = async prompt => {
		const project = current;
		setScreen('sessions');
		setMessage({text: `正在 ${project.name} 里启动新任务…`, color: 'yellow'});
		const result = await dispatchTask(project, prompt, forwardedArgs);
		if (result.error) return setMessage({text: `启动失败：${result.error}`, color: 'red'});
		setMessage({text: `已启动新任务 ${result.output}`, color: 'green'});
		refresh();
	};

	// 数字键按序号进入；序号还可能更长时（比如有 12 个项目时按了 1）稍等下一位
	const digitsRef = useRef('');
	const digitTimerRef = useRef(null);
	const clearDigits = () => {
		clearTimeout(digitTimerRef.current);
		digitsRef.current = '';
	};
	useEffect(() => clearDigits, []);

	const typeDigit = digit => {
		clearTimeout(digitTimerRef.current);
		const digits = digitsRef.current + digit;
		const n = Number(digits);
		if (n < 1 || n > projects.length) {
			clearDigits();
			return setMessage({text: `没有序号 ${digits} 的项目`, color: 'yellow'});
		}
		setCursor(n - 1);
		if (n * 10 > projects.length) {
			clearDigits();
			return openSessions(n - 1);
		}
		digitsRef.current = digits;
		digitTimerRef.current = setTimeout(() => {
			digitsRef.current = '';
			openSessions(n - 1);
		}, DIGIT_WAIT_MS);
	};

	useInput(
		(input, key) => {
			const current = projects[cursorRef.current];
			setMessage(null);
			if (/^[0-9]$/.test(input)) return typeDigit(input);
			clearDigits();
			if (input === 'q') return exit({type: 'quit'});
			if (key.escape || isBack(input, key)) return goBack(() => exit({type: 'quit'}));
			if (key.upArrow || input === 'k') return setCursor(c => Math.max(0, c - 1));
			if (key.downArrow || input === 'j') return setCursor(c => Math.min(projects.length - 1, c + 1));
			if (input === 'a') return setScreen('add');
			if (input === 'r') return refresh();
			if (input === '?') return setHelp(LIST_HELP);
			if (!current) return;
			if (input === 'd') return setScreen('delete');
			if (key.return) return openSessions(cursorRef.current);
		},
		{isActive: screen === 'list' && !help},
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

	// 四周留白：左右两格，上面一行
	const frame = (...children) =>
		h(Box, {flexDirection: 'column', width: columns - 1, paddingX: 2, paddingTop: 1}, ...children);
	const brand = h(Text, {bold: true, color: 'red'}, 'HeyClaude');
	// 所有页面的左上角都是红色的 HeyClaude，进入下一级时在后面接“› 名字”
	const crumb = name => h(Text, null, brand, h(Text, {dimColor: true}, ' › '), h(Text, {bold: true}, name));

	const helpScreen = help ? h(HelpScreen, {title: '全部按键', sections: help, onClose: () => setHelp(null)}) : null;

	if (screen === 'sessions' && current) {
		// 帮助页打开时任务列表只是藏起来，光标和状态标签都保留
		return frame(
			helpScreen,
			h(
				Box,
				{flexDirection: 'column', display: help ? 'none' : 'flex'},
				layout === 'tabs'
					? null
					: h(TopBar, {
							left: crumb(current.name),
							right: sessions ? h(StatusCounts, {counts: statsFor(sessions, current.path, confirmed)}) : null,
					  }),
				h(SessionsScreen, {
				active: !help,
				key: layout === 'tabs' ? 'tabs' : current.path,
				layout,
				projects,
				projectIndex: cursor,
				onSwitchProject: step => setCursor(c => (c + step + projects.length) % projects.length),
				sessions,
				loading,
				confirmed,
				saveConfirmed,
				initialSessionId,
				initialView,
				setMessage,
				columns,
				rows,
				onOpenSession: (session, view) => exit({type: 'attach', project: current, session, view}),
				onNewTask: () => setScreen('newtask'),
				onHelp: () => setHelp(sessionHelp(layout)),
				onBack: () => goBack(() => setScreen('list')),
				onRefresh: refresh,
			}),
				h(KeyHints, {message, keys: sessionKeys(layout)}),
			),
		);
	}

	if (help) return frame(helpScreen);

	if (screen === 'newtask' && current) {
		return frame(
			h(TopBar, {left: crumb(`${current.name} › 新任务`)}),
			h(NewTask, {project: current, onSubmit: submitTask, onCancel: () => goBack(() => setScreen('sessions'))}),
			h(KeyHints, {keys: [['Enter', '启动'], ['Ctrl+U', '清空'], ['Ctrl+Z', '取消']]}),
		);
	}

	if (screen === 'add') {
		return frame(
			h(TopBar, {left: crumb('添加项目')}),
			h(AddProject, {
				projects,
				sessions,
				onCancel: () => goBack(() => setScreen('list')),
				onDone: ({projects: next, project}) => {
					if (persist(next)) {
						setCursor(next.findIndex(p => p.path === project.path));
						setMessage({text: `已添加 ${project.name}`, color: 'green'});
					}
					setScreen('list');
				},
			}),
			h(KeyHints, {
				keys: [
					['Tab', '补全/选用'],
					['↑↓', '选择建议'],
					['Enter', '添加'],
					['Ctrl+Z', '取消'],
				],
			}),
		);
	}

	const totals = totalCounts(projects, sessions, confirmed);
	return frame(
		h(TopBar, {
			left: brand,
			right: h(Text, null, h(Text, {dimColor: true}, `${projects.length} 个项目`), h(Text, {dimColor: true}, STATUS_ORDER.some(k => totals[k]) ? ' · ' : ''), h(StatusCounts, {counts: totals})),
		}),
		h(ProjectList, {projects, cursor, sessions, confirmed, loading, columns, rows}),
		screen === 'delete' && current
			? h(Text, {color: 'yellow'}, `从列表移除 ${current.name}？目录和会话不受影响。(y/N)`)
			: null,
		h(KeyHints, {
			message,
			keys: [
				['Enter', '进入'],
				['a', '添加'],
				['d', '删除'],
				['Ctrl+Z', '退出'],
				['?', '全部按键'],
			],
		}),
	);
}
