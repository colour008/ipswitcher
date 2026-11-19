<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import {
	runDiagnostic,
	stopDiagnostic,
	onDiagLine,
	onDiagDone,
	type UnlistenFn,
} from '@/api/ipc';

interface Preset {
	key: string;
	label: string;
	needTarget: boolean;
	placeholder?: string;
	defaultTarget?: string;
}

interface Token {
	text: string;
	cls: string;
}

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
const lines = ref<string[]>([]);
const outputBox = ref<HTMLElement | null>(null);
let unlistenLine: UnlistenFn | null = null;
let unlistenDone: UnlistenFn | null = null;
let currentTask = '';

const activePresetObj = computed(
	() => presets.find((p) => p.key === activePreset.value)!
);

watch(activePreset, (key) => {
	const p = presets.find((x) => x.key === key);
	target.value = p?.defaultTarget ?? '';
});

onMounted(async () => {
	unlistenLine = await onDiagLine((p) => {
		if (p.task_id !== currentTask) return;
		if (/^\s*\[命令结束，退出码/.test(p.line)) return;
		lines.value.push(p.line);
		scrollBottom();
	});
	unlistenDone = await onDiagDone((p) => {
		if (p.task_id !== currentTask) return;
		running.value = false;
	});
});

onUnmounted(() => {
	unlistenLine?.();
	unlistenDone?.();
});

async function scrollBottom() {
	await nextTick();
	if (outputBox.value)
		outputBox.value.scrollTop = outputBox.value.scrollHeight;
}

async function run() {
	if (running.value) return;
	running.value = true;
	lines.value = [];
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
		lines.value.push(`命令启动失败：${e}`);
		running.value = false;
	}
}

async function stop() {
	if (!running.value || !currentTask) return;
	try {
		await stopDiagnostic(currentTask);
		lines.value.push('— 已手动停止命令 —');
		scrollBottom();
	} catch (e) {
		ElMessage.warning(`停止失败：${e}`);
	}
}

function clearOutput() {
	lines.value = [];
}

async function copyOutput() {
	const text = lines.value.join('\n');
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
					clearable
					:disabled="running" />
			</div>

			<div class="btn-row">
				<el-button type="primary" :loading="running" @click="run">
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
			<div v-if="running" class="diag-progress">
				<div class="progress-bar"></div>
			</div>
			<div ref="outputBox" class="diag-lines">
				<div v-if="!lines.length && !running" class="diag-empty muted">
					选择命令后点击「执行命令」，结果将在此显示
				</div>
				<div v-for="(line, i) in lines" :key="i" class="diag-line">
					<template v-for="(t, j) in tokenize(line)" :key="j">
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
}
.diag-toolbar {
	padding: 10px 12px;
	display: flex;
	flex-direction: column;
	gap: 10px;
}
.preset-row {
	flex-wrap: wrap;
}
.arg-row {
	display: flex;
	gap: 8px;
}
.target-input {
	flex: 1;
}
.btn-row {
	display: flex;
	align-items: center;
	gap: 8px;
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
	font-family: 'Cascadia Code', Consolas, 'Courier New', monospace;
	font-size: 13px;
	line-height: 1.6;
	white-space: pre-wrap;
	word-break: break-all;
}
.diag-line {
	min-height: 1.6em;
}
.diag-empty {
	font-size: 13px;
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
