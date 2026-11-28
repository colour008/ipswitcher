<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, shallowReactive, shallowRef, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { currentAdapter } from '@/composables/useStore';
import { listScanSubnets, onScanDone, onScanHostResults, onScanRoundStarted, onScanWaiting, runHostScan, stopHostScan } from '@/api/ipc';
import type { ScanHostResult } from '@/types';
import { useScopedListeners } from '@/composables/useScopedListeners';

type HostRow = Omit<ScanHostResult, 'status'> & {
	status: ScanHostResult['status'] | 'scanning' | 'unchecked';
};
type Filter = 'all' | ScanHostResult['status'];

const props = defineProps<{ active: boolean }>();
const adapter = computed(currentAdapter);
const subnets = ref<string[]>([]);
const selectedSubnet = ref('');
const repeat = ref(false);
const running = ref(false);
const waiting = ref(false);
const stopping = ref(false);
const scanned = ref(0);
const ready = ref(false);
const error = ref('');
const rows = shallowRef<HostRow[]>([]);
const byIp = new Map<string, HostRow>();
const counts = reactive({ all: 0, online: 0, offline: 0, undiscovered: 0, scanning: 0, unchecked: 0 });
const filter = ref<Filter>('all');
const filterItems = ['all', 'online', 'offline', 'undiscovered'] as const;
const labels = { all: '全部', online: '在线', offline: '离线', undiscovered: '从未发现', scanning: '扫描中', unchecked: '未检查' };
const search = ref('');
const nowSec = ref(Math.floor(Date.now() / 1000));
const registerListener = useScopedListeners();
let taskId = '';
let discard = false;
const ROW_HEIGHT = 34;
const OVERSCAN = 6;
const hostList = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const listHeight = ref(0);

function clearRows() {
	rows.value = [];
	byIp.clear();
	Object.assign(counts, { all: 0, online: 0, offline: 0, undiscovered: 0, scanning: 0, unchecked: 0 });
	scanned.value = 0;
	resetScroll();
}

function beginRound(ips: string[]) {
	scanned.value = 0;
	waiting.value = false;
	// Keep the previous observations visible during automatic refreshes.
	if (ips.length === rows.value.length && ips.every(ip => byIp.has(ip))) return;
	clearRows();
	rows.value = ips.map(ip => {
		const row = shallowReactive<HostRow>({ ip, status: 'scanning', hostname: null, mac: null, last_seen: null });
		byIp.set(ip, row);
		return row;
	});
	counts.all = counts.scanning = ips.length;
}

onMounted(async () => {
	window.addEventListener('pagehide', stopScan);
	try {
		await Promise.all([
			registerListener(onScanRoundStarted(p => {
				if (p.task_id === taskId && !discard) beginRound(p.ips);
			})),
			registerListener(onScanHostResults(p => {
				if (p.task_id !== taskId || discard) return;
				scanned.value = p.scanned;
				for (const result of p.rows) {
					const row = byIp.get(result.ip);
					if (!row) continue;
					if (row.status !== result.status) {
						counts[row.status]--;
						counts[result.status]++;
					}
					Object.assign(row, result);
				}
			})),
			registerListener(onScanWaiting(id => {
				if (id === taskId && !discard) waiting.value = true;
			})),
			registerListener(onScanDone(p => {
				if (p.task_id !== taskId) return;
				taskId = '';
				running.value = waiting.value = stopping.value = false;
				if (discard) return;
				for (const row of rows.value) {
					if (row.status === 'scanning') { row.status = 'unchecked'; counts.unchecked++; }
				}
				counts.scanning = 0;
				if (p.error) error.value = p.error;
				else if (!p.cancelled) ElMessage.success(`扫描完成，共检查 ${p.scanned} 个地址`);
			})),
		]);
		ready.value = true;
	} catch (cause) {
		error.value = String(cause);
	}
});

async function stopScan() {
	if (!taskId || stopping.value) return;
	stopping.value = true;
	try { await stopHostScan(taskId); }
	catch (cause) { error.value = String(cause); stopping.value = false; }
}

onUnmounted(() => {
	window.removeEventListener('pagehide', stopScan);
	void stopScan();
});

// Refresh the offered prefixes when the selected adapter or its address configuration changes.
watch(() => [adapter.value?.if_index, adapter.value?.name, adapter.value?.status,
	adapter.value?.ipv4.join(','), adapter.value?.subnet_mask.join(',')].join('|'),
	async (_key, _old, onCleanup) => {
		let stale = false;
		onCleanup(() => { stale = true; });
		discard = true;
		void stopScan();
		clearRows();
		subnets.value = [];
		selectedSubnet.value = '';
		error.value = '';
		const nic = adapter.value;
		if (!nic?.if_index || nic.status !== 'connected') return;
		try {
			const list = await listScanSubnets(nic.name, nic.if_index);
			if (!stale) {
				subnets.value = list;
				if (list.length) selectedSubnet.value = 'all';
			}
		} catch (cause) { if (!stale) error.value = String(cause); }
	}, { immediate: true });

watch(selectedSubnet, () => { clearRows(); error.value = ''; });
watch(() => props.active, active => { if (!active && repeat.value) void stopScan(); });
watch(() => props.active && rows.value.length > 0, (visible, _, onCleanup) => {
	if (!visible) return;
	const tick = () => { nowSec.value = Math.floor(Date.now() / 1000); };
	tick();
	const timer = window.setInterval(tick, 1000);
	onCleanup(() => window.clearInterval(timer));
}, { immediate: true });

async function startScan() {
	const nic = adapter.value;
	if (!ready.value || running.value || !nic?.if_index || !subnets.value.length) return;
	clearRows();
	filter.value = 'all';
	error.value = '';
	discard = false;
	running.value = true;
	try {
		taskId = crypto.randomUUID();
		await runHostScan(taskId, nic.name, nic.if_index, selectedSubnet.value === 'all' ? null : selectedSubnet.value, repeat.value);
	} catch (cause) {
		taskId = '';
		running.value = stopping.value = false;
		if (!discard) error.value = String(cause);
	}
}

const visibleRows = computed(() => {
	const kw = search.value.trim();
	if (filter.value === 'all' && !kw) return rows.value;
	return rows.value.filter(
		(row) =>
			(filter.value === 'all' || row.status === filter.value) &&
			(!kw || row.ip.includes(kw)),
	);
});

const firstVisibleRow = computed(() => Math.max(
	0,
	Math.min(Math.floor(scrollTop.value / ROW_HEIGHT), visibleRows.value.length - 1) - OVERSCAN,
));
const lastVisibleRow = computed(() => Math.min(
	visibleRows.value.length,
	firstVisibleRow.value + Math.ceil(listHeight.value / ROW_HEIGHT) + OVERSCAN * 2 + 1,
));
const renderedRows = computed(() => visibleRows.value.slice(firstVisibleRow.value, lastVisibleRow.value));

function resetScroll() {
	scrollTop.value = 0;
	if (hostList.value) hostList.value.scrollTop = 0;
}

watch([filter, search], resetScroll);
watch(hostList, (list, _previous, onCleanup) => {
	if (!list) return;
	const observer = new ResizeObserver(() => { listHeight.value = list.clientHeight; });
	listHeight.value = list.clientHeight;
	observer.observe(list);
	onCleanup(() => observer.disconnect());
}, { flush: 'post' });

function formatSeen(value: number | null): string {
	if (!value) return '—';
	const elapsed = Math.max(0, nowSec.value - value);
	if (elapsed < 5) return '刚刚';
	if (elapsed < 60) return `${elapsed} 秒前`;
	if (elapsed < 3600) return `${Math.floor(elapsed / 60)} 分钟前`;
	if (elapsed < 86400) return `${Math.floor(elapsed / 3600)} 小时前`;
	return `${Math.floor(elapsed / 86400)} 天前`;
}
</script>

<template>
	<div class="scan-page">
		<section class="panel scan-toolbar">
			<div class="scan-heading">
				<div>
					<div class="scan-title">主机扫描</div>
					<div class="current-adapter-info" :title="adapter?.description">
						<span class="adapter-info-label">当前网卡</span>
						<strong class="adapter-info-name">{{ adapter?.name || '未选择' }}</strong>
						<span v-if="adapter?.description" class="adapter-info-description">({{ adapter.description }})</span>
					</div>
				</div>
				<div class="scan-actions">
					<el-button
						type="primary"
						:disabled="!subnets.length || !ready"
						:loading="running"
						@click="startScan">
						{{
							running
								? (waiting ? '等待复扫' : '扫描中…')
								: rows.length
									? '重新扫描'
									: '开始扫描'
						}}
					</el-button>
					<el-button v-if="running" :disabled="stopping" @click="stopScan">{{ stopping ? '停止中…' : '停止' }}</el-button>
				</div>
			</div>
			<div class="scan-options">
				<el-select v-model="selectedSubnet" aria-label="扫描网段" size="small"
					:disabled="running || !subnets.length" :placeholder="subnets.length ? '全部网段' : '无可用 IPv4 网段'">
					<el-option label="全部网段" value="all" />
					<el-option v-for="subnet in subnets" :key="subnet" :label="subnet" :value="subnet" />
				</el-select>
				<label><input type="checkbox" v-model="repeat" :disabled="running" /> 自动复扫（每轮结束后 30 秒）</label>
			</div>
			<div v-if="rows.length" class="scan-summary">
				<button
					v-for="item in filterItems"
					:key="item"
					class="summary-chip"
					:class="[
						`state-${item}`,
						{ active: filter === item },
					]"
					@click="filter = item">
					<span>{{ labels[item] }}</span
					><b>{{ counts[item] }}</b>
				</button>
				<span v-if="running" class="muted scan-progress"
					>已检查 {{ scanned }} /
					{{ rows.length }} 个地址</span
				>
			</div>
			<div v-if="error" class="scan-error">{{ error }}</div>
		</section>

		<section class="panel scan-results">
			<div class="results-head">
				<div class="results-title-group">
					<span class="results-title-mark"></span>
					<span class="results-title">扫描结果</span>
					<span class="results-count">{{ visibleRows.length }} 个地址</span>
				</div>
				<el-input
					v-model="search"
					size="small"
					placeholder="筛选 IP 地址"
					clearable
					class="scan-search" />
			</div>
			<div v-if="!rows.length" class="scan-empty muted">
				选择网段后点击“开始扫描”，查看该范围内发现的设备。
			</div>
			<div v-else-if="!visibleRows.length" class="scan-empty muted">
				没有符合条件的地址。
			</div>
			<div
				v-else
				ref="hostList"
				class="host-list"
				@scroll.passive="scrollTop = ($event.currentTarget as HTMLElement).scrollTop">
				<div class="host-row host-header">
					<span></span>
					<span>IP 地址</span>
					<span>主机名</span>
					<span>MAC 地址</span>
					<span>状态</span>
					<span class="host-seen">最近在线</span>
				</div>
				<div aria-hidden="true" :style="{ height: firstVisibleRow * ROW_HEIGHT + 'px' }"></div>
				<div v-for="row in renderedRows" :key="row.ip" class="host-row">
					<span class="host-dot" :class="`dot-${row.status}`"></span>
					<code class="host-ip">{{ row.ip }}</code>
					<span class="host-value" :title="row.hostname || ''">{{
						row.hostname || '—'
					}}</span>
					<code class="host-value" :title="row.mac || ''">{{
						row.mac || '—'
					}}</code>
					<span class="host-status" :class="`text-${row.status}`">{{
						labels[row.status]
					}}</span>
					<span class="host-seen muted">{{ formatSeen(row.last_seen) }}</span>
				</div>
				<div aria-hidden="true" :style="{ height: (visibleRows.length - lastVisibleRow) * ROW_HEIGHT + 'px' }"></div>
			</div>
		</section>
	</div>
</template>

<style scoped>
.scan-page {
	display: flex;
	flex-direction: column;
	gap: 10px;
	height: 100%;
	min-height: 0;
}
.scan-toolbar {
	padding: 14px;
	display: flex;
	flex-direction: column;
	gap: 12px;
}
.scan-heading,
.scan-actions,
.results-head {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 12px;
}
.results-title-group {
	display: flex;
	align-items: center;
	gap: 9px;
}
.results-title-mark {
	width: 3px;
	height: 17px;
	border-radius: 3px;
	background: linear-gradient(180deg, var(--accent), var(--accent-soft));
	box-shadow: 0 0 8px rgba(76, 141, 255, 0.22);
}
.results-title {
	color: var(--text-primary);
	font-size: 15px;
	font-weight: 650;
	letter-spacing: 0.02em;
}
.results-count {
	padding: 3px 8px;
	border: 1px solid var(--panel-border);
	border-radius: 999px;
	background: var(--app-bg);
	color: var(--text-secondary);
	font-size: 11px;
	font-variant-numeric: tabular-nums;
}
.scan-title {
	font-size: 16px;
	font-weight: 700;
}
.scan-options, .scan-options label {
	display: flex;
	align-items: center;
	gap: 8px;
}
.scan-options {
	flex-wrap: wrap;
	color: var(--text-secondary);
	font-size: 12px;
}
.scan-options .el-select {
	width: 180px;
}
.current-adapter-info {
	display: flex;
	align-items: center;
	flex-wrap: wrap;
	gap: 4px 8px;
	max-width: 100%;
	margin-top: 8px;
	padding: 6px 10px;
	border-left: 3px solid var(--accent);
	border-radius: 4px 8px 8px 4px;
	background: var(--accent-soft);
	color: var(--text-secondary);
	font-size: 12px;
	line-height: 1.4;
}
.adapter-info-label {
	color: var(--accent);
	font-weight: 600;
}
.adapter-info-name {
	color: var(--text-primary);
	font-weight: 600;
}
.adapter-info-description {
	color: var(--text-secondary);
}
.scan-summary {
	display: flex;
	align-items: center;
	flex-wrap: wrap;
	gap: 8px;
}
.summary-chip {
	display: inline-flex;
	align-items: center;
	gap: 8px;
	padding: 6px 10px;
	border: 1px solid var(--panel-border);
	border-radius: 8px;
	color: var(--text-secondary);
	background: transparent;
	cursor: pointer;
}
.summary-chip b {
	color: var(--text-primary);
}
.summary-chip.active {
	border-color: var(--accent);
	background: var(--accent-soft);
	color: var(--accent);
}
.scan-progress {
	margin-left: auto;
	font-size: 12px;
}
.scan-error {
	color: var(--danger);
	font-size: 12px;
}
.scan-results {
	padding: 12px;
	display: flex;
	flex-direction: column;
	flex: 1;
	min-height: 0;
}
.scan-search {
	width: 180px;
}
html:not(.dark) .scan-search {
	--el-input-bg-color: transparent;
}
.scan-search :deep(.el-input__inner) {
	font-size: 12px;
}
.scan-search :deep(.el-input__wrapper) {
	min-height: 30px;
	padding: 1px 8px;
}
.host-list {
	overflow: auto;
	margin-top: 10px;
	min-width: 0;
	width: 100%;
}
.host-row {
	box-sizing: border-box;
	height: 34px;
	display: grid;
	grid-template-columns:
		14px minmax(105px, 1fr) minmax(110px, 1.1fr) minmax(145px, 1.25fr)
		minmax(90px, 0.75fr) minmax(110px, 0.9fr);
	align-items: center;
	gap: 6px;
	border-bottom: 1px solid var(--panel-border);
	padding: 3px 6px;
	min-width: 0;
	font-size: 12px;
}
.host-row > * {
	min-width: 0;
}
.host-row,
.host-row code {
	font-family: inherit;
	font-size: 12px;
}
.host-header {
	position: sticky;
	top: 0;
	z-index: 1;
	min-height: 34px;
	background: linear-gradient(180deg, var(--panel-bg), var(--app-bg));
	color: var(--text-primary);
	font-weight: 600;
	font-size: 11px;
	letter-spacing: 0.035em;
	border-bottom-color: var(--panel-border);
	box-shadow: 0 2px 6px rgba(0, 0, 0, 0.12);
}
.host-value {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.host-dot {
	width: 9px;
	height: 9px;
	border-radius: 50%;
	background: #9aa0a8;
}
.dot-online {
	background: #4ade80;
	box-shadow: 0 0 8px rgba(74, 222, 128, 0.35);
}
.dot-offline {
	background: #f56c6c;
}
.dot-undiscovered {
	background: #9aa0a8;
}
.host-ip {
	font-size: inherit;
}
.host-status {
	font-size: inherit;
	font-weight: 400;
}
.text-online {
	color: var(--success);
}
.text-offline {
	color: var(--danger);
}
.text-undiscovered,
.text-scanning {
	color: var(--text-secondary);
}
.host-seen {
	font-size: inherit;
	text-align: center;
}
.scan-empty {
	display: grid;
	flex: 1;
	place-items: center;
	text-align: center;
	font-size: 13px;
}
@media (max-width: 760px) {
	.host-row {
		grid-template-columns:
			14px minmax(95px, 1fr) minmax(100px, 1fr) minmax(125px, 1.1fr)
			minmax(78px, 0.7fr) minmax(96px, 0.8fr);
		gap: 5px;
	}
	.scan-search {
		width: 145px;
	}
}
</style>
