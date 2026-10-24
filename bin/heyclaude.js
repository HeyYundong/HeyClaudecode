#!/usr/bin/env node
import {spawnSync} from 'node:child_process';
import {createRequire} from 'node:module';
import React from 'react';
import {render} from 'ink';
import {App} from '../src/app.js';
import {configDir, dirExists, loadProjects} from '../src/store.js';

const {version} = createRequire(import.meta.url)('../package.json');
const forwardedArgs = process.argv.slice(2);

if (forwardedArgs.includes('-h') || forwardedArgs.includes('--help')) {
	console.log(`HeyClaude ${version} — 按项目管理 Claude Code agents

用法: heyclaude (或 hcc) [claude agents 的参数…]

  启动项目列表 TUI。进入项目看到按状态分组的后台任务，选中任务按 Enter
  用 \`claude attach\` 直接打开那个会话，按 Ctrl+Z 回到 HeyClaude。
  除 -h/--help、-v/--version 外的参数会原样传给 claude agents（审阅页
  按 A 打开），例如:

    heyclaude --model opus --effort high

项目列表保存在 ${configDir()}/projects.json`);
	process.exit(0);
}

if (forwardedArgs.includes('-v') || forwardedArgs.includes('--version')) {
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

async function showList(initialPath, initialMessage, initialScreen, initialSessionId) {
	// 自己切换备用屏幕：Ink 的 alternateScreen 不把光标移到左上角，
	// Windows Terminal 等会沿用原来的光标行，界面就画到了屏幕底部
	process.stdout.write('\x1b[?1049h\x1b[2J\x1b[H');
	try {
		const instance = render(
			React.createElement(App, {initialProjects: projects, initialPath, initialMessage, initialScreen, initialSessionId, forwardedArgs}),
		);
		const result = await instance.waitUntilExit();
		return result ?? {type: 'quit'};
	} finally {
		process.stdout.write('\x1b[?1049l');
	}
}

function runClaude(args, cwd) {
	// 子进程和我们在同一个进程组，Ctrl+C / Ctrl+Z 只应该交给 claude 处理
	const ignore = () => {};
	process.on('SIGINT', ignore);
	process.on('SIGTSTP', ignore);
	const result = spawnSync('claude', args, {cwd, stdio: 'inherit'});
	process.off('SIGINT', ignore);
	process.off('SIGTSTP', ignore);
	if (result.error) {
		return {text: `无法启动 claude：${result.error.message}`, color: 'red'};
	}
	if (result.status) {
		return {text: `claude ${args[0]} 退出码 ${result.status}`, color: 'yellow'};
	}
	return null;
}

function runAction(action) {
	const {project, session} = action;
	if (action.type === 'attach') {
		// claude attach 用短 ID 打开后台会话；在会话自己的目录里运行（可能是 worktree）
		return runClaude(['attach', session.id], dirExists(session.cwd) ? session.cwd : project.path);
	}
	// cwd 也设成项目目录，这样在 agents 视图里新派发的会话会落在这个项目下
	return runClaude(['agents', '--cwd', project.path, ...forwardedArgs], project.path);
}

let lastPath;
let message;
let returnTo;
let lastSessionId;
for (;;) {
	const action = await showList(lastPath, message, returnTo, lastSessionId);
	if (action.type !== 'attach' && action.type !== 'agents') break;
	// 打开会话或 agents 列表之后，都回到这个项目的审阅页，光标停在刚才的会话上
	lastPath = action.project.path;
	returnTo = 'review';
	lastSessionId = action.session?.sessionId;
	message = runAction(action);
	// 重新读取，以防在别处改过 projects.json
	try {
		projects = loadProjects();
	} catch (error) {
		message = {text: error.message, color: 'red'};
	}
}
