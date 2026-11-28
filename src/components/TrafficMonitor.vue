<script setup lang="ts">
import { computed, onUnmounted, ref, shallowReactive, shallowRef, watch } from 'vue';
import { currentName, adapters } from '@/composables/useStore';
import { getAdapterTraffic } from '@/api/ipc';
import type { AdapterInfo, AdapterTrafficCounters } from '@/types';
import { RefreshRight, Top, Bottom } from '@element-plus/icons-vue';

const props = defineProps<{ active: boolean }>();

interface CounterBaseline {
	initial: number;
	retained: number;
}

interface TrafficSession {
	sent: CounterBaseline;
	received: CounterBaseline;
	latest: AdapterTrafficCounters;
	startedAt: number;
}

const sessions = shallowReactive(new Map<string, TrafficSession>());
const uploadRate = ref(0);
const downloadRate = ref(0);
const HISTORY_WINDOW_SECONDS = 60;
const uploadHistory = shallowRef<number[]>(Array(HISTORY_WINDOW_SECONDS).fill(0));
const downloadHistory = shallowRef<number[]>(Array(HISTORY_WINDOW_SECONDS).fill(0));
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
const adapterKey = computed(() => adapter.value ? trafficKey(adapter.value) : null);
const session = computed(() =>
	adapterKey.value ? sessions.get(adapterKey.value) ?? null : null,
);
const uploadedText = computed(() => {
	const current = session.value;
	return formatBytes(current ? counterTotal(current.sent, current.latest.sent_bytes) : 0);
});
const downloadedText = computed(() => {
	const current = session.value;
	return formatBytes(current ? counterTotal(current.received, current.latest.received_bytes) : 0);
});
const monitoredFor = computed(() =>
	session.value?.startedAt
		? formatDuration(Math.floor((clock.value - session.value.startedAt) / 1000))
		: '00:00',
);

let timer: number | undefined;
let sampling = false;
let sampleGeneration = 0;
let lastRateSample: AdapterTrafficCounters | null = null;
let disposed = false;

function trafficKey(selected: AdapterInfo): string {
	return selected.interface_guid
		? 'guid:' + selected.interface_guid.toLowerCase()
		: 'name:' + selected.name + ':' + (selected.mac ?? selected.if_index ?? '');
}

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
	if (uploadHistory.value.some((rate) => rate !== uploadRate.value)) {
		uploadHistory.value = [...uploadHistory.value.slice(1), uploadRate.value];
	}
	if (downloadHistory.value.some((rate) => rate !== downloadRate.value)) {
		downloadHistory.value = [...downloadHistory.value.slice(1), downloadRate.value];
	}
}

function resetHistory() {
	uploadRate.value = 0;
	downloadRate.value = 0;
	uploadHistory.value = Array(HISTORY_WINDOW_SECONDS).fill(0);
	downloadHistory.value = Array(HISTORY_WINDOW_SECONDS).fill(0);
}

function newSession(counters: AdapterTrafficCounters, startedAt = counters.sampled_at): TrafficSession {
	return {
		sent: { initial: counters.sent_bytes, retained: 0 },
		received: { initial: counters.received_bytes, retained: 0 },
		latest: counters,
		startedAt,
	};
}

function counterTotal(baseline: CounterBaseline, latest: number): number {
	return baseline.retained + (latest - baseline.initial);
}

function updateBaseline(baseline: CounterBaseline, previous: number, latest: number): CounterBaseline {
	// 网卡或驱动重置计数时，保留已观察到的累计，从新计数继续。
	return latest < previous ? { initial: latest, retained: counterTotal(baseline, previous) } : baseline;
}

async function sample() {
	const selected = adapter.value;
	const key = adapterKey.value;
	if (!selected || !key || sampling || !props.active || disposed) return;
	const { name, if_index } = selected;
	const generation = sampleGeneration;
	clock.value = Date.now();
	sampling = true;
	if (!session.value) reading.value = true;
	try {
		const counters = await getAdapterTraffic(name, if_index);
		if (generation !== sampleGeneration || !props.active || adapterKey.value !== key) return;

		const current = sessions.get(key);
		const previous = current?.latest;
		if (!current || !previous) {
			sessions.set(key, newSession(counters));
		} else if (previous.sampled_at !== counters.sampled_at) {
			sessions.set(key, {
				sent: updateBaseline(current.sent, previous.sent_bytes, counters.sent_bytes),
				received: updateBaseline(current.received, previous.received_bytes, counters.received_bytes),
				latest: counters,
				startedAt: current.startedAt,
			});
		}
		unavailable.value = false;

		const rateBaseline = lastRateSample;
		if (rateBaseline?.sampled_at === counters.sampled_at) return;
		lastRateSample = counters;
		if (!rateBaseline) return;

		const elapsed = (counters.sampled_at - rateBaseline.sampled_at) / 1000;
		if (elapsed <= 0 || elapsed > 3) {
			resetHistory();
			return;
		}
		uploadRate.value = Math.max(0, counters.sent_bytes - rateBaseline.sent_bytes) / elapsed;
		downloadRate.value = Math.max(0, counters.received_bytes - rateBaseline.received_bytes) / elapsed;
		pushHistory();
	} catch {
		if (generation === sampleGeneration && props.active && adapterKey.value === key) {
			unavailable.value = true;
			lastRateSample = null;
			uploadRate.value = 0;
			downloadRate.value = 0;
			pushHistory();
		}
	} finally {
		reading.value = false;
		sampling = false;
		// 恢复窗口或切换网卡时，等旧请求结束就读取当前网卡。
		if (generation !== sampleGeneration) void sample();
	}
}

function resetSession() {
	const key = adapterKey.value;
	const current = session.value;
	if (!key || !current) return;
	const now = Date.now();
	sessions.set(key, newSession(current.latest, now));
	clock.value = now;
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

watch(adapters, (available) => {
	const keys = new Set(available.map(trafficKey));
	for (const key of sessions.keys()) {
		if (!keys.has(key)) sessions.delete(key);
	}
});

watch([() => props.active, adapterKey, () => adapter.value?.name, () => adapter.value?.if_index], ([active, key]) => {
	sampleGeneration++;
	lastRateSample = null;
	window.clearInterval(timer);
	timer = undefined;
	if (active) {
		resetHistory();
		unavailable.value = false;
	}
	if (active && key) {
		void sample();
		timer = window.setInterval(() => void sample(), 1000);
	}
}, { immediate: true });

onUnmounted(() => {
	disposed = true;
	sampleGeneration++;
	window.clearInterval(timer);
});
</script>

<template>
	<section class="traffic-card panel">
		<div class="traffic-header">
			<div class="traffic-heading">
				<span class="traffic-title">流量监控</span>
			</div>
			<div class="traffic-controls">
				<span class="monitor-state" :class="{ unavailable, 'is-live': active && session && !unavailable }">
					<i></i>{{ unavailable ? '暂不可用' : session ? '实时监测' : reading ? '连接中' : '等待数据' }}
				</span>
				<el-tooltip content="重置本次累计" placement="top" popper-class="app-tip">
					<button class="reset-button" :disabled="!session" @click="resetSession">
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
					<span class="metric-label"><el-icon><Top /></el-icon>上传</span>
					<strong>{{ unavailable ? '—' : formatRate(uploadRate) }}</strong>
				</div>
				<svg class="metric-wave upload-wave" viewBox="0 0 120 30" preserveAspectRatio="none" aria-hidden="true"><path class="grid-line" d="M0 3 H120 M0 13 H120 M0 23 H120" /><path class="wave-area" :d="uploadArea" /><path class="wave-line" :d="uploadPath" /></svg>
				<div class="metric-total"><span>累计</span><strong>{{ uploadedText }}</strong></div>
			</div>
			<div class="metric-item is-download">
				<div class="metric-current">
					<span class="metric-label"><el-icon><Bottom /></el-icon>下载</span>
					<strong>{{ unavailable ? '—' : formatRate(downloadRate) }}</strong>
				</div>
				<svg class="metric-wave download-wave" viewBox="0 0 120 30" preserveAspectRatio="none" aria-hidden="true"><path class="grid-line" d="M0 3 H120 M0 13 H120 M0 23 H120" /><path class="wave-area" :d="downloadArea" /><path class="wave-line" :d="downloadPath" /></svg>
				<div class="metric-total"><span>累计</span><strong>{{ downloadedText }}</strong></div>
			</div>
		</div>
		<div class="traffic-footnote"><span>监控时长 {{ monitoredFor }}</span><span>关闭监控或退出软件后清零</span></div>
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
	color: var(--state-live);
	font-size: 10px;
}
.monitor-state i {
	position: relative;
	width: 6px;
	height: 6px;
	border-radius: 50%;
	background: currentColor;
}
.monitor-state.is-live i {
	animation: monitor-pulse 1.8s ease-in-out infinite;
}
.monitor-state.is-live i::before {
	content: '';
	position: absolute;
	inset: -2px;
	border-radius: inherit;
	background: currentColor;
	animation: monitor-halo 1.8s ease-in-out infinite;
}
@keyframes monitor-pulse {
	0%,
	100% {
		opacity: 0.65;
		transform: scale(0.85);
	}
	50% {
		opacity: 1;
		transform: scale(1.15);
	}
}
@keyframes monitor-halo {
	0%, 100% { opacity: 0.22; transform: scale(0.7); }
	50% { opacity: 0; transform: scale(1.45); }
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
	.monitor-state.is-live i,
	.monitor-state.is-live i::before {
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
	--metric-color: var(--accent);
	min-width: 0;
	padding: 7px 9px 6px;
}
.metric-item.is-download {
	--metric-color: var(--success);
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
	color: var(--metric-color);
	font-size: 10px;
}
.metric-label :deep(.el-icon) {
	color: var(--metric-color);
	font-size: 11px;
}
.metric-current > strong {
	color: var(--metric-color);
	font-size: 12px;
	font-weight: 400;
	font-variant-numeric: tabular-nums;
	white-space: nowrap;
}
.metric-total {
	margin-top: 4px;
	color: var(--text-secondary);
	font-size: 9px;
}
.metric-total strong {
	color: var(--metric-color);
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
