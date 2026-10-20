import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export function configDir() {
	const base = process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config');
	return path.join(base, 'heyclaude');
}

function projectsFile() {
	return path.join(configDir(), 'projects.json');
}

export function expandPath(input) {
	let p = input.trim();
	if (p === '~') p = os.homedir();
	else if (p.startsWith('~/')) p = path.join(os.homedir(), p.slice(2));
	return path.resolve(p);
}

// 按名字排序，不区分大小写，数字按数值比较（project2 在 project10 前面）
export function sortProjects(projects) {
	return [...projects].sort((a, b) => a.name.localeCompare(b.name, undefined, {sensitivity: 'base', numeric: true}));
}

export function loadProjects() {
	try {
		const data = JSON.parse(fs.readFileSync(projectsFile(), 'utf8'));
		return Array.isArray(data.projects) ? sortProjects(data.projects) : [];
	} catch (error) {
		if (error.code === 'ENOENT') return [];
		throw new Error(`无法读取 ${projectsFile()}: ${error.message}`);
	}
}

export function saveProjects(projects) {
	fs.mkdirSync(configDir(), {recursive: true});
	const file = projectsFile();
	const tmp = `${file}.tmp`;
	fs.writeFileSync(tmp, JSON.stringify({projects}, null, 2) + '\n');
	fs.renameSync(tmp, file);
}

// 校验并返回新列表；出错时抛出带中文说明的 Error
export function addProject(projects, rawPath) {
	const resolved = expandPath(rawPath || process.cwd());
	let stat;
	try {
		stat = fs.statSync(resolved);
	} catch {
		throw new Error(`目录不存在: ${resolved}`);
	}
	if (!stat.isDirectory()) throw new Error(`不是目录: ${resolved}`);
	if (projects.some(p => p.path === resolved)) throw new Error(`已经添加过: ${resolved}`);

	const project = {
		name: path.basename(resolved) || resolved,
		path: resolved,
		addedAt: new Date().toISOString(),
	};
	return {projects: sortProjects([...projects, project]), project};
}

export function removeProject(projects, projectPath) {
	return projects.filter(p => p.path !== projectPath);
}

export function dirExists(p) {
	try {
		return fs.statSync(p).isDirectory();
	} catch {
		return false;
	}
}
