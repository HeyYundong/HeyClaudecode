import fs from 'node:fs';
import path from 'node:path';
import {configDir} from './store.js';

// 你确认过的会话：{sessionId: 确认时间 ISO 字符串}
// 只存在 HeyClaude 自己的配置目录里，不写 Claude Code 的任何数据
function reviewsFile() {
	return path.join(configDir(), 'reviews.json');
}

export function loadReviews() {
	try {
		const data = JSON.parse(fs.readFileSync(reviewsFile(), 'utf8'));
		return data && typeof data.confirmed === 'object' && data.confirmed ? data.confirmed : {};
	} catch (error) {
		if (error.code === 'ENOENT') return {};
		throw new Error(`无法读取 ${reviewsFile()}: ${error.message}`);
	}
}

export function saveReviews(confirmed) {
	fs.mkdirSync(configDir(), {recursive: true});
	const file = reviewsFile();
	const tmp = `${file}.tmp`;
	fs.writeFileSync(tmp, JSON.stringify({confirmed}, null, 2) + '\n');
	fs.renameSync(tmp, file);
}

export function withConfirmed(confirmed, ids, at = new Date().toISOString()) {
	const next = {...confirmed};
	for (const id of ids) next[id] = at;
	return next;
}

export function withoutConfirmed(confirmed, ids) {
	const next = {...confirmed};
	for (const id of ids) delete next[id];
	return next;
}
