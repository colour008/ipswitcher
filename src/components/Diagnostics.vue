<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue';
import { ElMessage } from 'element-plus';
import {
	runDiagnostic,
	stopDiagnostic,
	onDiagLine,
	onDiagDone,
} from '@/api/ipc';
import { useScopedListeners } from '@/composables/useScopedListeners';

interface Preset {
	key: string;
	label: string;
	needTarget?: boolean;
	placeholder?: string;
	defaultTarget?: string;
}

interface Token {
	text: string;
	cls: string;
}

interface OutputLine {
	id: number;
	text: string;
	cls: string;
	tokens: Token[];
}

const MAX_OUTPUT_LINES = 1500;
const OUTPUT_BATCH_MS = 80;
const OUTPUT_BATCH_LIMIT = 128;
const registerListener = useScopedListeners();
const presets: Preset[] = [
	{ key: 'ipconfig_all', label: 'ipconfig /all' },
	{
		key: 'ping',
		label: 'ping',
		needTarget: true,
		placeholder: '目标地址或域名，如 baidu.com',
		defaultTarget: 'baidu.com',
	},
	{
		key: 'tracert',
		label: 'tracert',
		needTarget: true,
		placeholder: '目标地址或域名，如 baidu.com',
		defaultTarget: 'baidu.com',
	},
	{ key: 'arp', label: 'arp -a' },
	{
		key: 'nslookup',
		label: 'nslookup',
		needTarget: true,
		placeholder: '待解析域名，如 baidu.com；可在附加参数中指定 DNS 服务器',
		defaultTarget: 'baidu.com',
	},
	{ key: 'netstat', label: 'netstat' },
	{ key: 'route', label: 'route print' },
	{ key: 'flushdns', label: '清空 DNS 缓存' },
];

const activePreset = ref('ipconfig_all');
const target = ref('');
const extraArgs = ref('');
const running = ref(false);
const lines = shallowRef<OutputLine[]>([]);
const ready = ref(false);
const truncated = ref(false);
const outputBox = ref<HTMLElement | null>(null);
let pendingLines: string[] = [];
let outputTimer: number | undefined;
let lineId = 0;
let currentTask = '';

const activePresetObj = computed(
	() => presets.find((p) => p.key === activePreset.value)!
);

watch(activePreset, (key) => {
	const p = presets.find((x) => x.key === key);
	target.value = p?.defaultTarget ?? '';
});

onMounted(async () => {
	try {
		await Promise.all([
			registerListener(onDiagLine((p) => {
				if (p.task_id !== currentTask || /^\s*\[命令结束，退出码/.test(p.line)) return;
				appendLine(p.line);
			})),
			registerListener(onDiagDone((p) => {
				if (p.task_id !== currentTask) return;
				flushOutput();
				running.value = false;
			})),
		]);
		ready.value = true;
	} catch (error) {
		ElMessage.error(String(error));
	}
});

onUnmounted(() => {
	window.clearTimeout(outputTimer);
	if (running.value) void stopDiagnostic(currentTask).catch(() => undefined);
});

function appendLine(text: string) {
	pendingLines.push(text);
	if (pendingLines.length >= OUTPUT_BATCH_LIMIT) flushOutput();
	else outputTimer ??= window.setTimeout(flushOutput, OUTPUT_BATCH_MS);
}

function flushOutput() {
	window.clearTimeout(outputTimer);
	outputTimer = undefined;
	if (!pendingLines.length) return;
	const batch = pendingLines.splice(0).map((text) => ({
		id: ++lineId,
		text,
		cls: lineClass(text),
		tokens: tokenize(text),
	}));
	const output = [...lines.value, ...batch];
	if (output.length > MAX_OUTPUT_LINES) truncated.value = true;
	lines.value = output.slice(-MAX_OUTPUT_LINES);
	void scrollBottom();
}

async function scrollBottom() {
	await nextTick();
	if (outputBox.value)
		outputBox.value.scrollTop = outputBox.value.scrollHeight;
}

async function run() {
	if (!ready.value || running.value) return;
	running.value = true;
	clearOutput();
	currentTask = `task-${Date.now()}`;
	const args = extraArgs.value.trim()
		? extraArgs.value.trim().split(/\s+/)
		: [];
	try {
		await runDiagnostic(
			currentTask,
			activePreset.value,
			target.value || null,
			args
		);
	} catch (e) {
		appendLine(`命令启动失败：${e}`);
		flushOutput();
		running.value = false;
	}
}

async function stop() {
	if (!running.value || !currentTask) return;
	try {
		await stopDiagnostic(currentTask);
		appendLine('— 已手动停止命令 —');
		flushOutput();
	} catch (e) {
		ElMessage.warning(`停止失败：${e}`);
	}
}

function clearOutput() {
	window.clearTimeout(outputTimer);
	outputTimer = undefined;
	pendingLines = [];
	lines.value = [];
	truncated.value = false;
}

async function copyOutput() {
	const text = lines.value.map((line) => line.text).join('\n');
	if (!text) {
		ElMessage.warning('暂无输出内容');
		return;
	}
	await navigator.clipboard.writeText(text);
	ElMessage.success('已复制到剪贴板');
}

const MASTER =
	/(\b\d{1,3}(?:\.\d{1,3}){3}\b)|(失败|错误|超时|无法到达|unreachable|could not|fail\w*)|(Reply from|来自|TTL|成功|已完成|bytes=\d+)/gi;

function tokenize(line: string): Token[] {
	const tokens: Token[] = [];
	let last = 0;
	for (const m of line.matchAll(MASTER)) {
		const idx = m.index ?? 0;
		if (idx > last) tokens.push({ text: line.slice(last, idx), cls: '' });
		const cls = m[1] ? 'tok-ip' : m[2] ? 'tok-err' : 'tok-ok';
		tokens.push({ text: m[0], cls });
		last = idx + m[0].length;
	}
	if (last < line.length) tokens.push({ text: line.slice(last), cls: '' });
	return tokens;
}

function lineClass(line: string): string {
	const text = line.trim();
	if (!text) return 'is-blank';
	if (/(失败|错误|超时|无法到达|unreachable|could not|fail\w*)/i.test(text))
		return 'is-error';
	if (/(已完成|成功|statistics|统计信息)/i.test(text)) return 'is-summary';
	if (
		/^(Windows IP 配置|以太网适配器|无线局域网适配器|接口列表|接口:|Interface:|Internet 地址|网络目标|Active Connections|IPv4 Route Table|IPv4 路由表|路由表)/i.test(
			text
		)
	)
		return 'is-section';
	return '';
}
</script>

<template>
	<div class="diag-wrap">
		<div class="panel diag-toolbar">
			<div class="preset-row">
				<el-radio-group
					v-model="activePreset"
					size="default"
					:disabled="running">
					<el-radio-button
						v-for="p in presets"
						:key="p.key"
						:value="p.key">
						{{ p.label }}
					</el-radio-button>
				</el-radio-group>
			</div>

			<div class="arg-row" v-if="activePresetObj.needTarget">
				<el-input
					v-model="target"
					:placeholder="activePresetObj.placeholder"
					class="target-input"
					clearable
					:disabled="running" />
			</div>
			<div class="arg-row">
				<el-input
					v-model="extraArgs"
					placeholder="附加参数（可选，空格分隔），如 -n 10 -l 1000"
					class="extra-args-input"
					clearable
					:disabled="running" />
			</div>

			<div class="btn-row">
				<el-button type="primary" :disabled="!ready" :loading="running" @click="run">
					{{ running ? '执行中…' : '执行命令' }}
				</el-button>
				<el-button type="danger" :disabled="!running" @click="stop"
					>停止执行</el-button
				>
				<el-button @click="clearOutput" :disabled="running"
					>清空输出</el-button
				>
				<el-button
					@click="copyOutput"
					:disabled="running || !lines.length"
					>全选复制</el-button
				>
				<span v-if="running" class="run-hint"
					>命令执行中，可点击「停止执行」终止，请勿重复点击其他命令</span
				>
			</div>
		</div>

		<div class="panel diag-output">
			<div class="diag-output-header">
				<div class="output-title-group">
					<span class="output-status-dot" :class="{ running }"></span>
					<span class="output-title">命令输出</span>
					<span class="output-command">
						{{ activePresetObj.label }}
						{{ activePresetObj.needTarget && target ? target : '' }}
					</span>
				</div>
				<span class="output-state" :class="{ running }">
					{{ running ? '执行中' : lines.length ? '已完成' : '就绪' }}
				</span>
			</div>
			<div v-if="running" class="diag-progress">
				<div class="progress-bar"></div>
			</div>
			<div ref="outputBox" class="diag-lines">
				<div v-if="truncated" class="diag-line is-summary">仅保留最近 {{ MAX_OUTPUT_LINES }} 行</div>
				<div v-if="!lines.length && !running" class="diag-empty muted">
					选择命令后点击「执行命令」，结果将在此显示
				</div>
				<div
					v-for="line in lines"
					:key="line.id"
					v-memo="[line]"
					class="diag-line"
					:class="line.cls">
					<template v-for="(t, j) in line.tokens" :key="j">
						<span :class="t.cls">{{ t.text }}</span>
					</template>
				</div>
			</div>
		</div>
	</div>
</template>

<style scoped>
.diag-wrap {
	display: flex;
	flex-direction: column;
	gap: 10px;
	height: 100%;
	min-height: 0;
	font-family: inherit;
}
.diag-toolbar {
	padding: 12px;
	display: flex;
	flex-direction: column;
	gap: 12px;
}
.preset-row {
	flex-wrap: wrap;
}
.preset-row :deep(.el-radio-group) {
	display: flex;
	flex-wrap: wrap;
	gap: 6px;
}
.preset-row :deep(.el-radio-button__inner) {
	border: 1px solid var(--panel-border-glass);
	border-left: 1px solid var(--panel-border-glass);
	border-radius: 7px;
	background: var(--app-bg);
	box-shadow: none;
	color: var(--text-secondary);
	font-family: inherit;
	font-size: 12px;
	transition:
		background 0.15s,
		border-color 0.15s,
	color 0.15s;
}
.preset-row :deep(.el-radio-button__inner:hover) {
	border-color: var(--accent);
	color: var(--accent);
}
.preset-row
	:deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) {
	border-color: var(--accent);
	background: var(--accent-soft);
	box-shadow: none;
	color: var(--accent);
}
.arg-row {
	display: flex;
	gap: 8px;
}
.diag-toolbar :deep(.el-input__wrapper) {
	min-height: 36px;
	font-family: inherit;
	font-size: 12px;
}
.diag-toolbar :deep(.el-input__inner) {
	font-family: inherit;
	font-size: 12px;
}
.target-input {
	flex: 1;
}
html:not(.dark) .extra-args-input {
	--el-input-bg-color: transparent;
}
.btn-row {
	display: flex;
	align-items: center;
	gap: 8px;
}
.diag-toolbar :deep(.el-button) {
	min-height: 34px;
	border-radius: 7px;
	font-family: inherit;
	font-size: 12px;
}
.run-hint {
	font-size: 12px;
	color: var(--text-secondary);
}
.diag-output {
	flex: 1;
	min-height: 0;
	position: relative;
	overflow: hidden;
	display: flex;
	flex-direction: column;
}
.diag-output-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 12px;
	flex: none;
	min-height: 36px;
	padding: 8px 12px;
	border-bottom: 1px solid var(--panel-border);
	background: linear-gradient(90deg, var(--accent-soft), var(--panel-bg) 65%);
}
.output-title-group {
	display: flex;
	align-items: center;
	gap: 8px;
	min-width: 0;
}
.output-status-dot {
	width: 7px;
	height: 7px;
	flex: none;
	border-radius: 50%;
	background: var(--text-secondary);
}
.output-status-dot.running {
	background: var(--accent);
	box-shadow: 0 0 8px rgba(76, 141, 255, 0.55);
}
.output-title {
	color: var(--text-primary);
	font-size: 12px;
	font-weight: 600;
	letter-spacing: 0.02em;
}
.output-command {
	max-width: min(48vw, 420px);
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	padding: 2px 7px;
	border: 1px solid var(--panel-border);
	border-radius: 5px;
	background: var(--app-bg);
	color: var(--text-secondary);
	font-size: 11px;
}
.output-state {
	flex: none;
	color: var(--text-secondary);
	font-size: 11px;
	font-variant-numeric: tabular-nums;
}
.output-state.running {
	color: var(--accent);
}
.diag-progress {
	height: 3px;
	overflow: hidden;
	background: var(--panel-border);
}
.progress-bar {
	height: 100%;
	width: 40%;
	border-radius: 3px;
	background: var(--accent);
	animation: diag-slide 1.1s ease-in-out infinite;
}
@keyframes diag-slide {
	0% {
		transform: translateX(-100%);
	}
	100% {
		transform: translateX(250%);
	}
}
.diag-lines {
	flex: 1;
	overflow-y: auto;
	padding: 10px 12px;
	font-family: inherit;
	font-size: 12px;
	line-height: 1.7;
	font-variant-numeric: tabular-nums;
	white-space: pre-wrap;
	word-break: break-all;
}
.diag-line {
	min-height: 1.7em;
	padding: 1px 7px;
	border-left: 2px solid transparent;
	border-radius: 3px;
	transition: background 0.12s;
}
.diag-line:hover {
	background: var(--accent-soft);
}
.diag-line.is-blank {
	padding: 0;
}
.diag-line.is-section {
	margin-top: 8px;
	padding-top: 4px;
	padding-bottom: 4px;
	border-bottom: 1px solid var(--panel-border);
	color: var(--text-primary);
	font-weight: 600;
}
.diag-line.is-error {
	border-left-color: var(--danger);
	background: rgba(245, 108, 108, 0.07);
}
.diag-line.is-summary {
	color: var(--success);
	font-weight: 600;
}
.diag-empty {
	font-size: 12px;
	padding: 12px 4px;
}
.tok-ip {
	color: var(--accent);
	font-weight: 600;
}
.tok-err {
	color: #f56c6c;
	font-weight: 600;
}
.tok-ok {
	color: var(--success);
}
</style>
