import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// 读 Claude Code 存在本机的对话记录，判断一个会话在某个时间之后有没有新的对话。
// 这是 Claude Code 的内部存储，不是公开接口：只读不写；找不到文件或格式变了就返回 null，
// 调用方当作“不知道”处理，退回到只看 claude agents --json 的逻辑。

function projectsDir() {
	const base = process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), '.claude');
	return path.join(base, 'projects');
}

// 会话 ID → 记录文件路径。文件位置不会变，找到一次就记住
const pathCache = new Map();

function findTranscript(sessionId) {
	const cached = pathCache.get(sessionId);
	if (cached && fs.existsSync(cached)) return cached;
	let dirs;
	try {
		dirs = fs.readdirSync(projectsDir(), {withFileTypes: true}).filter(d => d.isDirectory());
	} catch {
		return null;
	}
	for (const d of dirs) {
		const file = path.join(projectsDir(), d.name, `${sessionId}.jsonl`);
		if (fs.existsSync(file)) {
			pathCache.set(sessionId, file);
			return file;
		}
	}
	return null;
}

// 只有你发的消息和 Claude 的回复算对话；斜杠命令、命令输出和附属记录不算
function isConversation(entry) {
	if (!entry || !entry.timestamp || entry.isMeta) return false;
	if (entry.type === 'assistant') return true;
	if (entry.type !== 'user') return false;
	const content = entry.message?.content;
	const text = typeof content === 'string' ? content : Array.isArray(content) && content[0]?.type === 'text' ? content[0].text : '';
	return !(typeof text === 'string' && /^\s*<(command-name|local-command)/.test(text));
}

const TAIL_BYTES = 256 * 1024;

// 记录文件里最后一条对话的时间（毫秒），读不到返回 null。只读文件末尾一段
function lastConversationTime(file) {
	let fd;
	try {
		fd = fs.openSync(file, 'r');
		const {size} = fs.fstatSync(fd);
		const length = Math.min(size, TAIL_BYTES);
		const buffer = Buffer.alloc(length);
		fs.readSync(fd, buffer, 0, length, size - length);
		const lines = buffer.toString('utf8').split('\n');
		// 从头截断的第一行可能不完整，倒着找，解析失败的跳过
		for (let i = lines.length - 1; i >= 0; i--) {
			if (!lines[i]) continue;
			let entry;
			try {
				entry = JSON.parse(lines[i]);
			} catch {
				continue;
			}
			if (isConversation(entry)) {
				const t = Date.parse(entry.timestamp);
				return Number.isFinite(t) ? t : null;
			}
		}
		return null;
	} catch {
		return null;
	} finally {
		if (fd !== undefined) fs.closeSync(fd);
	}
}

// 确认之后又有新对话的会话 ID。confirmed: {sessionId: 确认时间 ISO}
// 先看文件修改时间，没晚于确认时间的直接跳过，不读内容
export function repliedAfterConfirm(confirmed) {
	const ids = [];
	for (const [sessionId, at] of Object.entries(confirmed)) {
		const confirmedAt = Date.parse(at);
		if (!Number.isFinite(confirmedAt)) continue;
		const file = findTranscript(sessionId);
		if (!file) continue;
		let mtime;
		try {
			mtime = fs.statSync(file).mtimeMs;
		} catch {
			continue;
		}
		if (mtime <= confirmedAt) continue;
		const last = lastConversationTime(file);
		if (last !== null && last > confirmedAt) ids.push(sessionId);
	}
	return ids;
}
