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

// HeyClaude 自己不响应 Ctrl+Z 的暂停信号：界面里 Ctrl+Z 是“返回上一级”，
// 交接终端的空档里多按的 Ctrl+Z 也不能把我们挂起
process.on('SIGTSTP', () => {});

function attach(project, session) {
	// 子进程和我们在同一个进程组，Ctrl+C 只应该交给 claude 处理
	const ignore = () => {};
	process.on('SIGINT', ignore);
	// 先把终端设成原始模式：claude 接管终端之前按的 Ctrl+Z / Ctrl+C 只是普通按键，
	// 不会变成信号把 claude 暂停（暂停的 claude 和等它退出的我们会互相卡住）
	process.stdin.setRawMode(true);
	// claude attach 用短 ID 打开后台会话；在会话自己的目录里运行（可能是 worktree）
	const cwd = dirExists(session.cwd) ? session.cwd : project.path;
	const result = spawnSync('claude', ['attach', session.id], {cwd, stdio: 'inherit'});
	process.stdin.setRawMode(false);
	if (result.status && !result.error) {
		// 会话打不开时 claude 会打印原因然后退出，停一下让人看清
		process.stdout.write(`\n\x1b[33m会话 ${session.name || session.id} 打不开（退出码 ${result.status}），按回车返回 HeyClaude\x1b[0m`);
		spawnSync('sh', ['-c', 'read _'], {stdio: 'inherit'});
	}
	process.off('SIGINT', ignore);
	if (result.error) return {text: `无法启动 claude：${result.error.message}`, color: 'red'};
	if (result.status) return {text: `会话 ${session.name || session.id} 打不开（claude attach 退出码 ${result.status}）`, color: 'yellow'};
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
		initialView: action.view,
		initialMessage: message,
	};
	// 重新读取，以防在别处改过 projects.json
	try {
		projects = loadProjects();
	} catch (error) {
		state.initialMessage = {text: error.message, color: 'red'};
	}
}
