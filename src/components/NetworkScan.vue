<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { currentAdapter } from '@/composables/useStore';
import {
	onScanDone,
	onScanHostResult,
	runHostScan,
	stopHostScan,
} from '@/api/ipc';
import type { ScanDoneResult, ScanHostResult } from '@/types';
import type { UnlistenFn } from '@tauri-apps/api/event';

type HostRow = {
	ip: string;
	status: 'scanning' | ScanHostResult['status'];
	last_seen: number | null;
	hostname: string | null;
	mac: string | null;
};
type Filter = 'all' | HostRow['status'];

const adapter = computed(() => currentAdapter());
const rows = ref<HostRow[]>([]);
const running = ref(false);
const taskId = ref('');
const scannedAdapter = ref('');
const scannedAdapterDescription = ref('');
const scannedIp = ref('');
const scannedMask = ref('');
const filter = ref<Filter>('all');
const filterItems = [
	['all', '全部', 'all'],
	['online', '在线', 'online'],
	['offline', '离线', 'offline'],
	['undiscovered', '从未发现', 'undiscovered'],
] as const;
const search = ref('');
const error = ref('');
let unlistenHost: UnlistenFn | null = null;
let unlistenDone: UnlistenFn | null = null;

function ipv4Number(ip: string): number | null {
	const parts = ip.split('.').map(Number);
	if (
		parts.length !== 4 ||
		parts.some((n) => !Number.isInteger(n) || n < 0 || n > 255)
	)
		return null;
	return (
		((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0
	);
}

function numberIp(value: number): string {
	return [
		value >>> 24,
		(value >>> 16) & 255,
		(value >>> 8) & 255,
		value & 255,
	].join('.');
}

function subnetHosts(ip: string, mask: string): string[] {
	const addr = ipv4Number(ip);
	const maskNum = ipv4Number(mask);
	if (addr === null || maskNum === null) return [];
	const inverse = ~maskNum >>> 0;
	if ((inverse & ((inverse + 1) >>> 0)) >>> 0 !== 0) return [];
	const prefix = maskNum.toString(2).replace(/0/g, '').length;
	const network = (addr & maskNum) >>> 0;
	const broadcast = (network | inverse) >>> 0;
	const start = prefix <= 30 ? network + 1 : network;
	const end = prefix <= 30 ? broadcast - 1 : broadcast;
	const result: string[] = [];
	for (let n = start; n <= end && result.length <= 4096; n++) {
		if (n !== addr) result.push(numberIp(n >>> 0));
	}
	return result;
}

onMounted(async () => {
	unlistenHost = await onScanHostResult((result) => {
		if (result.task_id !== taskId.value) return;
		const row = rows.value.find((x) => x.ip === result.ip);
		if (row) {
			row.status = result.status;
			row.last_seen = result.last_seen;
			row.hostname = result.hostname;
			row.mac = result.mac;
		}
	});
	unlistenDone = await onScanDone((result: ScanDoneResult) => {
		if (result.task_id !== taskId.value) return;
		running.value = false;
		if (result.error)
			error.value = `扫描完成，但历史记录保存失败：${result.error}`;
		else if (result.cancelled)
			ElMessage.info(`扫描已停止，已检查 ${result.scanned} 个地址`);
		else ElMessage.success(`扫描完成，共检查 ${result.scanned} 个地址`);
	});
});

onUnmounted(() => {
	unlistenHost?.();
	unlistenDone?.();
});

async function startScan() {
	const a = adapter.value;
	if (!a) return;
	const ip = a.ipv4.find((x) => ipv4Number(x) !== null);
	const mask = a.subnet_mask.find((x) => ipv4Number(x) !== null);
	if (!ip || !mask) {
		ElMessage.warning('当前网卡没有可用的 IPv4 地址和子网掩码');
		return;
	}
	const hosts = subnetHosts(ip, mask);
	if (!hosts.length || hosts.length > 4096) {
		ElMessage.warning('无法计算扫描范围，或网段超过 4096 个地址');
		return;
	}
	taskId.value = `scan-${Date.now()}`;
	scannedAdapter.value = a.name;
	scannedAdapterDescription.value = a.description;
	scannedIp.value = ip;
	scannedMask.value = mask;
	rows.value = hosts.map((host) => ({
		ip: host,
		status: 'scanning',
		last_seen: null,
		hostname: null,
		mac: null,
	}));
	error.value = '';
	filter.value = 'all';
	running.value = true;
	try {
		await runHostScan(taskId.value, a.name, ip, mask);
	} catch (e) {
		running.value = false;
		error.value = typeof e === 'string' ? e : String(e);
	}
}

async function stopScan() {
	if (!running.value) return;
	await stopHostScan(taskId.value);
}

const counts = computed(() => ({
	all: rows.value.length,
	online: rows.value.filter((r) => r.status === 'online').length,
	offline: rows.value.filter((r) => r.status === 'offline').length,
	undiscovered: rows.value.filter((r) => r.status === 'undiscovered').length,
	scanning: rows.value.filter((r) => r.status === 'scanning').length,
}));

const visibleRows = computed(() => {
	const kw = search.value.trim();
	return rows.value.filter(
		(row) =>
			(filter.value === 'all' || row.status === filter.value) &&
			(!kw || row.ip.includes(kw)),
	);
});

function statusLabel(status: HostRow['status']): string {
	return status === 'online'
		? '在线'
		: status === 'offline'
			? '离线（曾在线）'
			: status === 'undiscovered'
				? '从未发现'
				: '扫描中';
}

function formatSeen(value: number | null): string {
	return value ? new Date(value * 1000).toLocaleString() : '—';
}
</script>

<template>
	<div class="scan-page">
		<section class="panel scan-toolbar">
			<div class="scan-heading">
				<div>
					<div class="scan-title">主机扫描</div>
					<div
						class="current-adapter-info"
						:title="
							rows.length
								? scannedAdapterDescription
								: adapter?.description || ''
						">
						<span class="adapter-info-label">当前网卡</span>
						<strong class="adapter-info-name">{{
							rows.length
								? scannedAdapter
								: adapter?.name || '未选择'
						}}</strong>
						<span
							v-if="
								rows.length
									? scannedAdapterDescription
									: adapter?.description
							"
							class="adapter-info-description"
							>({{
								rows.length
									? scannedAdapterDescription
									: adapter?.description
							}})</span>
						<span v-if="rows.length || adapter?.ipv4?.[0]" class="adapter-info-network">
							{{ rows.length ? scannedIp : adapter?.ipv4[0] }} /
							{{
								rows.length
									? scannedMask
									: adapter?.subnet_mask[0] || '未知掩码'
							}}
						</span>
					</div>
				</div>
				<div class="scan-actions">
					<el-button
						type="primary"
						:disabled="!adapter"
						:loading="running"
						@click="startScan">
						{{
							running
								? '扫描中…'
								: rows.length
									? '重新扫描'
									: '开始扫描'
						}}
					</el-button>
					<el-button v-if="running" @click="stopScan">停止</el-button>
				</div>
			</div>
			<div class="scan-note muted">
				通过当前网卡发送 IPv4
				探测。无响应不代表设备确定不存在；“离线”表示此前扫描中曾响应，“从未发现”表示本工具尚未记录到它在线。
			</div>
			<div v-if="rows.length" class="scan-summary">
				<button
					v-for="item in filterItems"
					:key="item[0]"
					class="summary-chip"
					:class="[
						`state-${item[0]}`,
						{ active: filter === item[0] },
					]"
					@click="filter = item[0]">
					<span>{{ item[1] }}</span
					><b>{{ counts[item[2]] }}</b>
				</button>
				<span v-if="running" class="muted scan-progress"
					>已返回 {{ rows.length - counts.scanning }} /
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
				点击“开始扫描”，探测当前选中网卡所在子网的可用 IPv4 地址。
			</div>
			<div v-else-if="!visibleRows.length" class="scan-empty muted">
				没有符合条件的地址。
			</div>
			<div v-else class="host-list">
				<div class="host-row host-header">
					<span></span>
					<span>IP 地址</span>
					<span>主机名</span>
					<span>MAC 地址</span>
					<span>状态</span>
					<span class="host-seen">最近在线</span>
				</div>
				<div v-for="row in visibleRows" :key="row.ip" class="host-row">
					<span class="host-dot" :class="`dot-${row.status}`"></span>
					<code class="host-ip">{{ row.ip }}</code>
					<span class="host-value" :title="row.hostname || ''">{{
						row.hostname || '—'
					}}</span>
					<code class="host-value" :title="row.mac || ''">{{
						row.mac || '—'
					}}</code>
					<span class="host-status" :class="`text-${row.status}`">{{
						statusLabel(row.status)
					}}</span>
					<span class="host-seen muted">{{
						row.status === 'scanning'
							? '正在探测…'
							: row.status === 'online'
								? '刚刚'
								: row.status === 'offline'
									? formatSeen(row.last_seen)
									: '—'
					}}</span>
				</div>
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
.scan-subtitle,
.scan-note {
	font-size: 12px;
	margin-top: 4px;
}
.scan-note {
	line-height: 1.5;
	margin-top: 0;
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
.adapter-info-network {
	padding-left: 8px;
	border-left: 1px solid var(--panel-border);
	color: var(--accent);
	font-variant-numeric: tabular-nums;
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
.card-title {
	font-weight: 600;
}
.card-title .muted {
	font-size: 12px;
	font-weight: 400;
	margin-left: 6px;
}
.scan-search {
	width: 180px;
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
	min-height: 34px;
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
