import fs from 'node:fs';
import path from 'node:path';
import {expandPath} from './store.js';

function commonPrefix(names) {
	let prefix = names[0] ?? '';
	for (const name of names) {
		while (!name.startsWith(prefix)) prefix = prefix.slice(0, -1);
	}
	return prefix;
}

// Tab 补全目录：返回补全后的输入和候选列表
export function completePath(input) {
	const raw = input === '' ? './' : input;
	const endsWithSlash = raw.endsWith('/');
	const expanded = expandPath(raw);
	const dir = endsWithSlash ? expanded : path.dirname(expanded);
	const partial = endsWithSlash ? '' : path.basename(expanded);
	// 保留用户原来写的前半段（例如 ~/），只替换最后一段
	const head = endsWithSlash ? raw : raw.slice(0, raw.length - partial.length);

	let entries;
	try {
		entries = fs.readdirSync(dir, {withFileTypes: true});
	} catch {
		return {value: input, candidates: []};
	}

	const showHidden = partial.startsWith('.');
	const matches = entries
		.filter(e => e.isDirectory() || (e.isSymbolicLink() && isDir(path.join(dir, e.name))))
		.map(e => e.name)
		.filter(name => name.startsWith(partial) && (showHidden || !name.startsWith('.')))
		.sort();

	if (matches.length === 0) return {value: input, candidates: []};
	if (matches.length === 1) return {value: head + matches[0] + '/', candidates: []};
	return {value: head + commonPrefix(matches), candidates: matches};
}

function isDir(p) {
	try {
		return fs.statSync(p).isDirectory();
	} catch {
		return false;
	}
}
