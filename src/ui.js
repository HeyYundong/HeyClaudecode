import React from 'react';
import {Box, Text, useInput} from 'ink';

const h = React.createElement;

// 五种状态，按这个顺序显示。「待确认」是 HeyClaude 加的：Claude 说做完了，你还没确认
export const STATUS = {
	working: {icon: '◌', label: '进行中', color: 'yellow'},
	blocked: {icon: '●', label: '需要输入', color: 'magenta'},
	pending: {icon: '◆', label: '待确认', color: 'cyan'},
	confirmed: {icon: '✓', label: '完成', color: undefined, dim: true},
	failed: {icon: '✗', label: '失败', color: 'red'},
};
export const STATUS_ORDER = ['working', 'blocked', 'pending', 'confirmed', 'failed'];

// 终端里的显示宽度：中日韩文字和全角符号占两格
export function textWidth(text) {
	let width = 0;
	for (const ch of text) {
		const code = ch.codePointAt(0);
		width += code >= 0x1100 && (code <= 0x115f || (code >= 0x2e80 && code <= 0xa4cf) || (code >= 0xac00 && code <= 0xd7a3) || (code >= 0xf900 && code <= 0xfaff) || (code >= 0xfe30 && code <= 0xfe4f) || (code >= 0xff00 && code <= 0xff60) || (code >= 0xffe0 && code <= 0xffe6)) ? 2 : 1;
	}
	return width;
}

// 列表里名字一栏的宽度：项目列表和任务列表用同一个算法，右侧信息紧跟在名字后面
export function nameColumn(names, columns) {
	const longest = Math.max(0, ...names.map(textWidth));
	return Math.max(12, Math.min(40, columns - 30, longest + 3));
}

// 一个状态的数量文字：不为零时用状态色，为零时变灰
function countText(k, n, numWidth = 0) {
	const s = STATUS[k];
	return h(Text, {key: k, color: n ? s.color : undefined, dimColor: !n || s.dim}, `${String(n).padStart(numWidth)} ${s.label}`);
}

// “3 进行中 · 1 需要输入 · 0 待确认 · 41 完成 · 0 失败”：数字在前，五种状态都列，为零的变灰
export function StatusCounts({counts, sep = ' · ', keys = STATUS_ORDER}) {
	return h(
		Text,
		null,
		...keys.flatMap((k, i) => [i ? h(Text, {key: `s${k}`, dimColor: true}, sep) : null, countText(k, counts[k] || 0)]),
	);
}

// 列表行里的五列数量：每种状态固定一列，数字右对齐，这样各行上下对齐
// numWidth：这一屏里最大的数有几位
export function StatusColumns({counts, numWidth = 1, keys = STATUS_ORDER}) {
	return h(
		Box,
		null,
		...keys.map(k =>
			h(Box, {key: k, width: numWidth + 1 + textWidth(STATUS[k].label) + 3, flexShrink: 0}, countText(k, counts[k] || 0, numWidth)),
		),
	);
}

// 顶部一行：左边是位置，右边是概况
export function TopBar({left, right}) {
	return h(
		Box,
		{justifyContent: 'space-between', marginBottom: 1},
		h(Box, {flexShrink: 0, marginRight: 2}, left),
		right ? h(Box, {flexShrink: 1}, right) : null,
	);
}

// 底部：一行提示消息，再一行常用按键
export function KeyHints({keys, message}) {
	return h(
		Box,
		{flexDirection: 'column', marginTop: 1},
		message ? h(Text, {color: message.color}, message.text) : null,
		h(Text, {dimColor: true}, keys.map(([key, label]) => `${key} ${label}`).join('    ')),
	);
}

// ? 打开的全部按键说明，按任意键关闭
export function HelpScreen({title, sections, onClose}) {
	useInput(() => onClose());
	const keyWidth = Math.max(...sections.flatMap(s => s.keys.map(([k]) => textWidth(k)))) + 4;
	return h(
		Box,
		{flexDirection: 'column'},
		h(TopBar, {left: h(Text, {bold: true}, title), right: h(Text, {dimColor: true}, '按任意键返回')}),
		...sections.map(section =>
			h(
				Box,
				{key: section.title, flexDirection: 'column', marginBottom: 1},
				h(Text, {bold: true}, section.title),
				...section.keys.map(([k, desc]) =>
					h(Box, {key: k}, h(Box, {width: keyWidth, flexShrink: 0}, h(Text, {color: 'cyan'}, `  ${k}`)), h(Text, null, desc)),
				),
			),
		),
	);
}
