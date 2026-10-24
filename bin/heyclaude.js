#!/usr/bin/env node
import {spawnSync} from 'node:child_process';
import {createRequire} from 'node:module';
import React from 'react';
import {render} from 'ink';
import {App} from '../src/app.js';
import {configDir, dirExists, loadProjects} from '../src/store.js';

const {version} = createRequire(import.meta.url)('../package.json');
const argv = process.argv.slice(2);
// --tabs：方案 D（项目和状态标签）；默认方案 A（项目列表 → 按状态分组的会话列表）
const layout = argv.includes('--tabs') ? 'tabs' : 'grouped';
// 其余参数在按 n 新建任务时传给 claude --bg
const forwardedArgs = argv.filter(a => a !== '--tabs');

if (argv.includes('-h') || argv.includes('--help')) {
	console.log(`HeyClaude ${version} — 按项目管理 Claude Code 后台任务

用法: heyclaude (或 hcc) [--tabs] [claude 的参数…]

  默认：项目列表 → 项目里按状态分组的任务列表。
  --tabs：一个界面里用标签切换项目和状态。

  任务有四种状态：运行中、等你回复、待确认、已完成。Claude 说做完了的任务
  先进「待确认」，你按 c 确认后才算「已完成」。

  选中任务按 Enter 用 \`claude attach\` 打开会话，按 Ctrl+Z 回到 HeyClaude。
  Ctrl+Z 在 HeyClaude 里也是“返回上一级”，在项目列表里按就退出。

  除 --tabs、-h、-v 外的参数会在按 n 新建任务时传给 \`claude --bg\`，例如:

    hcc --model opus --effort high

项目列表保存在 ${configDir()}/projects.json，确认记录保存在 reviews.json`);
	process.exit(0);
}

if (argv.includes('-v') || argv.includes('--version')) {
	console.log(version);
	process.exit(0);
}

if (!process.stdin.isTTY || !process.stdout.isTTY) {
	console.error('heyclaude 需要在交互式终端中运行。');
	process.exit(1);
}

let projects;
try {
	projects = loadProjects();
} catch (error) {
	console.error(error.message);
	process.exit(1);
}

async function showApp(state) {
	// 自己切换备用屏幕：Ink 的 alternateScreen 不把光标移到左上角，
	// Windows Terminal 等会沿用原来的光标行，界面就画到了屏幕底部
	process.stdout.write('\x1b[?1049h\x1b[2J\x1b[H');
	try {
		const instance = render(React.createElement(App, {layout, initialProjects: projects, forwardedArgs, ...state}));
		const result = await instance.waitUntilExit();
		return result ?? {type: 'quit'};
	} finally {
		process.stdout.write('\x1b[?1049l');
	}
}

function attach(project, session) {
	// 子进程和我们在同一个进程组，Ctrl+C / Ctrl+Z 只应该交给 claude 处理
	const ignore = () => {};
	process.on('SIGINT', ignore);
	process.on('SIGTSTP', ignore);
	// claude attach 用短 ID 打开后台会话；在会话自己的目录里运行（可能是 worktree）
	const cwd = dirExists(session.cwd) ? session.cwd : project.path;
	const result = spawnSync('claude', ['attach', session.id], {cwd, stdio: 'inherit'});
	process.off('SIGINT', ignore);
	process.off('SIGTSTP', ignore);
	if (result.error) return {text: `无法启动 claude：${result.error.message}`, color: 'red'};
	if (result.status) return {text: `claude attach 退出码 ${result.status}`, color: 'yellow'};
	return null;
}

let state = {};
for (;;) {
	const action = await showApp(state);
	if (action.type !== 'attach') break;
	const message = attach(action.project, action.session);
	// 回到这个项目的任务列表，光标和状态标签停在刚才的位置
	state = {
		initialPath: action.project.path,
		initialScreen: 'sessions',
		initialSessionId: action.session.sessionId,
		initialTab: action.tab,
		initialMessage: message,
	};
	// 重新读取，以防在别处改过 projects.json
	try {
		projects = loadProjects();
	} catch (error) {
		state.initialMessage = {text: error.message, color: 'red'};
	}
}
