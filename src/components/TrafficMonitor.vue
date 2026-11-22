<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue';
import { currentName, adapters } from '@/composables/useStore';
import { getAdapterTraffic } from '@/api/ipc';
import { RefreshRight, Top, Bottom } from '@element-plus/icons-vue';

interface TrafficSession {
	lastSent: number | null;
	lastReceived: number | null;
	lastSampleAt: number | null;
	uploaded: number;
	downloaded: number;
	startedAt: number;
}

const sessions = reactive(new Map<string, TrafficSession>());
const uploadRate = ref(0);
const downloadRate = ref(0);
const HISTORY_WINDOW_SECONDS = 60;
const uploadHistory = ref<number[]>(Array(HISTORY_WINDOW_SECONDS).fill(0));
const downloadHistory = ref<number[]>(Array(HISTORY_WINDOW_SECONDS).fill(0));
const uploadPath = computed(() => sparkline(uploadHistory.value));
const downloadPath = computed(() => sparkline(downloadHistory.value));
const uploadArea = computed(() => `${uploadPath.value} L120 27 L0 27 Z`);
const downloadArea = computed(() => `${downloadPath.value} L120 27 L0 27 Z`);
const reading = ref(false);
const unavailable = ref(false);
const clock = ref(Date.now());

const adapter = computed(
	() => adapters.value.find((item) => item.name === currentName.value) ?? null,
);
const session = computed(() =>
	currentName.value ? sessions.get(currentName.value) ?? null : null,
);
const monitoredFor = computed(() =>
	session.value?.startedAt
		? formatDuration(Math.floor((clock.value - session.value.startedAt) / 1000))
		: '00:00',
);

let timer: number | undefined;
let clockTimer: number | undefined;
let sampling = false;
let mounted = false;

function sparkline(values: number[]) {
	// 固定 60 秒窗口；缩放基于当前窗口峰值并留出顶部余量，零流量贴近基线。
	const peak = Math.max(0, ...values);
	const scale = Math.max(1024, peak * 1.18);
	const points = values.map((value, index) => ({
		x: (index / Math.max(1, values.length - 1)) * 120,
		y: 26 - (Math.max(0, value) / scale) * 21,
	}));
	if (!points.length) return 'M0 26 L120 26';
	// 二次曲线只柔化折点，采样点本身仍来自真实速率，不做均值滤波。
	let path = `M${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`;
	for (let i = 1; i < points.length - 1; i++) {
		const midX = (points[i].x + points[i + 1].x) / 2;
		const midY = (points[i].y + points[i + 1].y) / 2;
		path += ` Q${points[i].x.toFixed(2)} ${points[i].y.toFixed(2)} ${midX.toFixed(2)} ${midY.toFixed(2)}`;
	}
	const last = points[points.length - 1];
	return path + ` L${last.x.toFixed(2)} ${last.y.toFixed(2)}`;
}

function pushHistory() {
	uploadHistory.value = [...uploadHistory.value.slice(-(HISTORY_WINDOW_SECONDS - 1)), uploadRate.value];
	downloadHistory.value = [...downloadHistory.value.slice(-(HISTORY_WINDOW_SECONDS - 1)), downloadRate.value];
}

function newSession(): TrafficSession {
	return {
		lastSent: null,
		lastReceived: null,
		lastSampleAt: null,
		uploaded: 0,
		downloaded: 0,
		startedAt: Date.now(),
	};
}

async function sample() {
	const name = currentName.value;
	if (!name || sampling || !mounted) return;
	sampling = true;
	reading.value = true;
	try {
		const counters = await getAdapterTraffic(name);
		const now = Date.now();
		const current = sessions.get(name) ?? newSession();
		if (current.lastSent !== null && current.lastReceived !== null) {
			const sentDelta = counters.sent_bytes >= current.lastSent
				? counters.sent_bytes - current.lastSent
				: 0;
			const receivedDelta = counters.received_bytes >= current.lastReceived
				? counters.received_bytes - current.lastReceived
				: 0;
			current.uploaded += sentDelta;
			current.downloaded += receivedDelta;
			const elapsed = current.lastSampleAt
				? (now - current.lastSampleAt) / 1000
				: 0;
			if (name === currentName.value && elapsed > 0 && elapsed <= 3) {
				uploadRate.value = sentDelta / elapsed;
				downloadRate.value = receivedDelta / elapsed;
			} else if (name === currentName.value) {
				uploadRate.value = 0;
				downloadRate.value = 0;
			}
		} else if (name === currentName.value) {
			uploadRate.value = 0;
			downloadRate.value = 0;
		}
		current.lastSent = counters.sent_bytes;
		current.lastReceived = counters.received_bytes;
		current.lastSampleAt = now;
		sessions.set(name, current);
		if (name === currentName.value) unavailable.value = false;
		if (name === currentName.value) pushHistory();
	} catch {
		if (name === currentName.value) {
			unavailable.value = true;
			uploadRate.value = 0;
			downloadRate.value = 0;
			pushHistory();
		}
	} finally {
		reading.value = false;
		sampling = false;
	}
}

function resetSession() {
	const name = currentName.value;
	if (!name) return;
	const existing = sessions.get(name);
	sessions.set(name, {
		lastSent: existing?.lastSent ?? null,
		lastReceived: existing?.lastReceived ?? null,
		lastSampleAt: existing?.lastSampleAt ?? null,
		uploaded: 0,
		downloaded: 0,
		startedAt: Date.now(),
	});
	clock.value = Date.now();
}

function formatRate(value: number): string {
	if (!Number.isFinite(value) || value < 1024) return `${Math.max(0, value).toFixed(0)} B/s`;
	const units = ['KB/s', 'MB/s', 'GB/s', 'TB/s'];
	let amount = value / 1024;
	let unit = 0;
	while (amount >= 1024 && unit < units.length - 1) {
		amount /= 1024;
		unit++;
	}
	return `${amount.toFixed(amount >= 100 ? 0 : 1)} ${units[unit]}`;
}

function formatBytes(value: number): string {
	if (!Number.isFinite(value) || value < 1024) return `${Math.max(0, value).toFixed(0)} B`;
	const units = ['KB', 'MB', 'GB', 'TB', 'PB'];
	let amount = value / 1024;
	let unit = 0;
	while (amount >= 1024 && unit < units.length - 1) {
		amount /= 1024;
		unit++;
	}
	return `${amount.toFixed(amount >= 100 ? 0 : 1)} ${units[unit]}`;
}

function formatDuration(totalSeconds: number): string {
	const safe = Math.max(0, totalSeconds);
	const hours = Math.floor(safe / 3600);
	const minutes = Math.floor((safe % 3600) / 60);
	const seconds = safe % 60;
	return hours > 0
		? `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`
		: `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
}

watch(currentName, () => {
	uploadRate.value = 0;
	downloadRate.value = 0;
	uploadHistory.value = Array(HISTORY_WINDOW_SECONDS).fill(0);
	downloadHistory.value = Array(HISTORY_WINDOW_SECONDS).fill(0);
	unavailable.value = false;
	void sample();
});

onMounted(() => {
	mounted = true;
	void sample();
	timer = window.setInterval(() => void sample(), 1000);
	clockTimer = window.setInterval(() => {
		clock.value = Date.now();
	}, 1000);
});

onUnmounted(() => {
	mounted = false;
	if (timer !== undefined) window.clearInterval(timer);
	if (clockTimer !== undefined) window.clearInterval(clockTimer);
});
</script>

<template>
	<section class="traffic-card panel">
		<div class="traffic-header">
			<div class="traffic-heading">
				<span class="traffic-title">流量监控</span>
			</div>
			<div class="traffic-controls">
				<span class="monitor-state" :class="{ unavailable, 'is-live': session && !unavailable }">
					<i></i>{{ unavailable ? '暂不可用' : session ? '实时监测' : reading ? '连接中' : '等待数据' }}
				</span>
				<el-tooltip content="重置本次累计" placement="top" popper-class="app-tip">
					<button class="reset-button" :disabled="!adapter" @click="resetSession">
						<el-icon><RefreshRight /></el-icon>
					</button>
				</el-tooltip>
			</div>
		</div>
		<div v-if="adapter" class="traffic-adapter" :title="adapter.description">
			{{ adapter.name }}<span v-if="adapter.description"> · {{ adapter.description }}</span>
		</div>
		<div v-else class="traffic-adapter muted">请选择网卡以查看流量</div>

		<div class="traffic-metrics">
			<div class="metric-item">
				<div class="metric-current">
					<span class="metric-label upload-rate"><el-icon><Top /></el-icon>上传</span>
					<strong>{{ unavailable ? '—' : formatRate(uploadRate) }}</strong>
				</div>
				<svg class="metric-wave upload-wave" viewBox="0 0 120 30" preserveAspectRatio="none" aria-hidden="true"><path class="grid-line" d="M0 3 H120 M0 13 H120 M0 23 H120" /><path class="wave-area" :d="uploadArea" /><path class="wave-line" :d="uploadPath" /></svg>
				<div class="metric-total"><span>累计</span><strong>{{ formatBytes(session?.uploaded ?? 0) }}</strong></div>
			</div>
			<div class="metric-item">
				<div class="metric-current">
					<span class="metric-label download-rate"><el-icon><Bottom /></el-icon>下载</span>
					<strong>{{ unavailable ? '—' : formatRate(downloadRate) }}</strong>
				</div>
				<svg class="metric-wave download-wave" viewBox="0 0 120 30" preserveAspectRatio="none" aria-hidden="true"><path class="grid-line" d="M0 3 H120 M0 13 H120 M0 23 H120" /><path class="wave-area" :d="downloadArea" /><path class="wave-line" :d="downloadPath" /></svg>
				<div class="metric-total"><span>累计</span><strong>{{ formatBytes(session?.downloaded ?? 0) }}</strong></div>
			</div>
		</div>
		<div class="traffic-footnote"><span>监控时长 {{ monitoredFor }}</span><span>重启软件后重新统计</span></div>
	</section>
</template>

<style scoped>
.traffic-card {
	container-type: inline-size;
	flex: none;
	padding: 12px;
	border-radius: var(--radius);
	background: linear-gradient(145deg, var(--panel-glass), var(--panel-bg));
}
.traffic-header,
.traffic-heading,
.traffic-controls,
.metric-current,
.metric-total {
	display: flex;
	align-items: center;
}
.traffic-header {
	justify-content: space-between;
	gap: 8px;
}
.traffic-heading {
	gap: 8px;
}

.traffic-title {
	font-size: 13px;
	font-weight: 650;
	letter-spacing: 0.02em;
}
.traffic-controls {
	gap: 8px;
}
.monitor-state {
	display: inline-flex;
	flex: none;
	align-items: center;
	gap: 5px;
	color: var(--success);
	font-size: 10px;
}
.monitor-state i {
	width: 6px;
	height: 6px;
	border-radius: 50%;
	background: currentColor;
}
.monitor-state.is-live i {
	animation: monitor-pulse 1.8s ease-in-out infinite;
}
@keyframes monitor-pulse {
	0%,
	100% {
		opacity: 0.65;
		transform: scale(0.85);
		box-shadow: 0 0 0 0 color-mix(in srgb, currentColor 24%, transparent);
	}
	50% {
		opacity: 1;
		transform: scale(1.15);
		box-shadow: 0 0 0 4px color-mix(in srgb, currentColor 0%, transparent);
	}
}
.monitor-state.unavailable {
	color: var(--danger);
}
.reset-button {
	display: grid;
	place-items: center;
	width: 25px;
	height: 25px;
	border: 0;
	border-radius: 6px;
	background: transparent;
	color: var(--text-secondary);
	cursor: pointer;
	transition: color 0.15s, background 0.15s;
}
.reset-button:hover:not(:disabled) {
	color: var(--accent);
}
.reset-button:focus-visible {
	outline: 2px solid var(--accent);
	outline-offset: 2px;
}
.reset-button:disabled {
	opacity: 0.45;
	cursor: not-allowed;
}
@media (prefers-reduced-motion: reduce) {
	.monitor-state.is-live i {
		animation: none;
	}
}
.traffic-adapter {
	margin-top: 5px;
	overflow: hidden;
	color: var(--text-secondary);
	font-size: 10px;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.traffic-metrics {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	margin-top: 9px;
	border: 1px solid var(--panel-border);
	border-radius: 9px;
	background: linear-gradient(110deg, var(--accent-soft), var(--panel-bg) 48%, var(--panel-bg) 52%, rgba(74, 222, 128, 0.045));
}

.metric-item {
	min-width: 0;
	padding: 7px 9px 6px;
}
.metric-wave { display:block; width:100%; height:30px; margin:4px 0 2px; pointer-events:none; overflow:visible; }
.metric-wave .grid-line { fill:none; stroke:var(--panel-border); stroke-width:.65; stroke-dasharray:2 2; opacity:.75; }
.metric-wave .wave-area { fill:var(--accent); fill-opacity:.16; stroke:none; transition:d .35s ease; }
.metric-wave .wave-line { fill:none; stroke:var(--accent); stroke-width:.8; stroke-linecap:round; stroke-linejoin:round; transition:d .35s ease; }
.download-wave .wave-area { fill:var(--success); }
.download-wave .wave-line { stroke:var(--success); }
.metric-item + .metric-item {
	border-left: 1px solid var(--panel-border);
}
.metric-current,
.metric-total {
	justify-content: space-between;
	gap: 8px;
}
.metric-label {
	display: inline-flex;
	align-items: center;
	gap: 4px;
	color: var(--text-secondary);
	font-size: 10px;
}
.metric-label :deep(.el-icon) {
	font-size: 11px;
}
.metric-label.upload-rate :deep(.el-icon) {
	color: var(--accent);
}
.metric-label.download-rate :deep(.el-icon) {
	color: var(--success);
}
.metric-current > strong {
	color: var(--text-primary);
	font-size: 12px;
	font-weight: 650;
	font-variant-numeric: tabular-nums;
	white-space: nowrap;
}
.metric-total {
	margin-top: 4px;
	color: var(--text-secondary);
	font-size: 9px;
}
.metric-total strong {
	color: var(--text-secondary);
	font-size: 10px;
	font-weight: 550;
	font-variant-numeric: tabular-nums;
}
.traffic-footnote {
	display: flex;
	justify-content: space-between;
	gap: 8px;
	margin-top: 7px;
	color: var(--text-secondary);
	font-size: 9px;
}
@container (max-width: 280px) {
	.traffic-header {
		flex-wrap: wrap;
	}
	.traffic-controls {
		margin-left: auto;
	}
	.metric-item {
		padding-right: 6px;
		padding-left: 6px;
	}
	.metric-current {
		gap: 4px;
	}
	.metric-current > strong {
		font-size: 11px;
	}
}
</style>
