<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import {
	adapters,
	currentName,
	appConfig,
	refreshing,
	scheduleDelayedRefresh,
} from '@/composables/useStore';
import {
	saveConfig,
	setAdapterEnabled,
	elevateAction,
	emitConfigChanged,
	openNetworkConnections,
	renameAdapter,
} from '@/api/ipc';
import type { PendingAction, AdapterInfo } from '@/types';
import { Refresh, Search, Star, Connection } from '@element-plus/icons-vue';

const emit = defineEmits<{ (e: 'refresh'): void }>();

const keyword = ref('');
const toggling = ref<string | null>(null);

/// 行右键业务菜单（设为默认网卡）
const ctxMenuVisible = ref(false);
const ctxMenuPos = ref({ x: 0, y: 0 });
const ctxRow = ref<{ name: string } | null>(null);

function closeCtxMenu() {
	ctxMenuVisible.value = false;
	ctxRow.value = null;
}

function isDefault(name: string): boolean {
	return appConfig.value.default_adapter === name;
}

function onRowContextMenu(row: { name: string }, e: MouseEvent) {
	e.preventDefault();
	e.stopPropagation();
	// 右键时同样选中该行，右侧详情同步切换到当前网卡
	currentName.value = row.name;
	ctxRow.value = row;
	const menuW = 132;
	const menuH = 34;
	ctxMenuPos.value = {
		x: Math.min(e.clientX, window.innerWidth - menuW - 8),
		y: Math.min(e.clientY, window.innerHeight - menuH - 8),
	};
	ctxMenuVisible.value = true;
}

async function setAsDefault() {
	const row = ctxRow.value;
	closeCtxMenu();
	if (!row || isDefault(row.name)) return;
	appConfig.value.default_adapter = row.name;
	await saveConfig(appConfig.value);
	emitConfigChanged();
	ElMessage.success(`已将「${row.name}」设为默认网卡`);
}

async function renameRow() {
	const row = ctxRow.value;
	closeCtxMenu();
	if (!row) return;
	let newName: string | undefined;
	try {
		const ret = await ElMessageBox.prompt(
			'请输入新的网卡名称',
			'重命名网卡',
			{
				inputPlaceholder: row.name,
				inputValue: row.name,
				inputValidator: (v) => {
					const t = (v || '').trim();
					if (!t) return '名称不能为空';
					if (t.length > 32) return '名称不能超过 32 个字符';
					if (/[\\/:*?"<>|]/.test(t)) return '名称不能包含 \\ / : * ? " < > | 字符';
					return true;
				},
			},
		);
		newName = ret.value?.trim();
	} catch {
		return;
	}
	if (!newName || newName === row.name) return;

	const conflict = adapters.value.find((a) => a.name === newName);
	if (conflict && conflict.name !== row.name) {
		ElMessage.warning(`名称「${newName}」已被网卡「${conflict.name}」占用，请更换`);
		return;
	}

	const migrateConfig = () => {
		if (appConfig.value.default_adapter === row.name)
			appConfig.value.default_adapter = newName;
		if (appConfig.value.widget_adapter === row.name)
			appConfig.value.widget_adapter = newName;
		if (Array.isArray(appConfig.value.pinned)) {
			appConfig.value.pinned = [...new Set(
				appConfig.value.pinned.map((n) =>
					n === row.name ? (newName as string) : n,
				),
			)];
		}
	};

	try {
		await renameAdapter(row.name, newName);
		migrateConfig();
		if (currentName.value === row.name) currentName.value = newName;
		await saveConfig(appConfig.value);
		emitConfigChanged();
		ElMessage.success(`已重命名为「${newName}」`);
		emit('refresh');
		scheduleDelayedRefresh();
	} catch (e) {
		const msg = typeof e === 'string' ? e : String(e);
		if (msg === 'ELEVATION_REQUIRED') {
			const action: PendingAction = {
				kind: 'Rename',
				name: row.name,
				new_name: newName,
			};
			await elevateAction(action);
			ElMessage.info('已提交提权执行，请查看弹出的提示框');
			scheduleDelayedRefresh();
		} else if (msg === 'NAME_IN_USE_RESERVED') {
			showRenameInUseGuide();
		} else {
			ElMessage.error(msg || '重命名失败');
		}
	}
}

async function showRenameInUseGuide() {
	const html = `
		<div style="font-size:12px;line-height:1.7">
			<p>该名称已被系统中一条旧的网络连接占用（已隐藏或卸载残留，当前网卡列表里看不到）。可按以下步骤手动清理后再重命名：</p>
			<ol style="margin:6px 0 0 18px;padding:0">
				<li>按 <b>Win + R</b>，输入 <b>devmgmt.msc</b> 回车，打开设备管理器；</li>
				<li>顶部菜单「查看」→ 勾选「显示隐藏的设备」；</li>
				<li>展开「网络适配器」，找到灰色半透明的同名旧适配器；</li>
				<li>右键 →「卸载设备」，完成后回到本工具重新重命名。</li>
			</ol>
			<p style="margin-top:8px;opacity:.7">也可以直接换一个不冲突的名称完成重命名。</p>
		</div>`;
	try {
		await ElMessageBox.alert(html, '名称被残留网络连接占用', {
			dangerouslyUseHTMLString: true,
			confirmButtonText: '知道了',
		});
	} catch {
		/* 用户关闭即可 */
	}
}

function onDocPointerDown(e: MouseEvent) {
	if (!ctxMenuVisible.value) return;
	if ((e.target as HTMLElement).closest('.row-context-menu')) return;
	closeCtxMenu();
}

function onDocKeydown(e: KeyboardEvent) {
	if (e.key === 'Escape') closeCtxMenu();
}

onMounted(() => {
	document.addEventListener('pointerdown', onDocPointerDown);
	document.addEventListener('keydown', onDocKeydown);
});

onUnmounted(() => {
	document.removeEventListener('pointerdown', onDocPointerDown);
	document.removeEventListener('keydown', onDocKeydown);
});

function pinnedSet(): string[] {
	return [...new Set(appConfig.value.pinned ?? [])];
}

function isPinned(name: string): boolean {
	return pinnedSet().includes(name);
}

function isDisabled(row: { status: string }): boolean {
	return row.status === 'disabled';
}

/// 置顶网卡排在前面（保持置顶先后顺序），其余按原始顺序排列
const ordered = computed(() => {
	const pinned = pinnedSet().filter((n) =>
		adapters.value.some((a) => a.name === n),
	);
	const rest = adapters.value.filter((a) => !pinned.includes(a.name));
	const byName = new Map(adapters.value.map((a) => [a.name, a]));
	const pinnedRows = pinned
		.map((n) => byName.get(n))
		.filter((x): x is AdapterInfo => !!x);
	const seen = new Set<string>();
	return [...pinnedRows, ...rest].filter((a) => {
		const k = a.if_index != null ? `i${a.if_index}` : `m${a.mac ?? a.name}`;
		if (seen.has(k)) return false;
		seen.add(k);
		return true;
	});
});

const filtered = computed(() => {
	const kw = keyword.value.trim().toLowerCase();
	if (!kw) return ordered.value;
	return ordered.value.filter(
		(a) =>
			a.name.toLowerCase().includes(kw) ||
			a.description.toLowerCase().includes(kw),
	);
});

function togglePin(row: { name: string }) {
	const cur = pinnedSet();
	appConfig.value.pinned = isPinned(row.name)
		? cur.filter((n) => n !== row.name)
		: [...new Set([...cur, row.name])];
	void saveConfig(appConfig.value);
}

function onRow(row: { name: string }) {
	currentName.value = row.name;
}

async function toggleEnabled(row: { name: string; status: string }) {
	const enable = isDisabled(row);
	toggling.value = row.name;
	try {
		await setAdapterEnabled(row.name, enable);
		ElMessage.success(
			enable ? `已启用「${row.name}」` : `已禁用「${row.name}」`,
		);
		emit('refresh');
		scheduleDelayedRefresh();
	} catch (e) {
		const msg = typeof e === 'string' ? e : String(e);
		if (msg === 'ELEVATION_REQUIRED') {
			const action: PendingAction = {
				kind: 'Toggle',
				names: [row.name],
				enabled: enable,
			};
			await elevateAction(action);
			ElMessage.info('已提交提权执行，请查看弹出的提示框');
			scheduleDelayedRefresh();
		} else {
			ElMessage.error(msg || '操作失败');
		}
	} finally {
		toggling.value = null;
	}
}

function openNetworkPanel() {
	openNetworkConnections().catch((e) => {
		ElMessage.error(typeof e === 'string' ? e : '打开网络连接失败');
	});
}
</script>

<template>
	<div class="list-panel panel">
		<div class="list-title">网卡列表</div>
		<div class="list-header">
			<el-input
				v-model="keyword"
				placeholder="搜索网卡名称 / 描述"
				:prefix-icon="Search"
				clearable
				size="small" />
			<el-tooltip
				content="刷新网卡列表"
				placement="top"
				popper-class="app-tip">
				<el-button
					class="list-refresh"
					:icon="Refresh"
					:loading="refreshing"
					circle
					@click="emit('refresh')" />
			</el-tooltip>
		</div>

		<div class="list-body">
			<div class="list-colhead">
				<span class="col-main">网卡</span>
				<span class="col-status">状态</span>
				<span class="col-toggle">开关</span>
				<span class="col-action">置顶</span>
			</div>
			<div
				v-for="row in filtered"
				:key="row.name"
				class="adapter-row"
				:class="{
					'is-current': currentName === row.name,
					'is-pinned': isPinned(row.name),
				}"
				@click="onRow(row)"
				@contextmenu="onRowContextMenu(row, $event)">
				<el-tooltip
					placement="right"
					:show-after="200"
					popper-class="adapter-tip">
					<template #content>
						<div class="tip-info">
							<div class="tip-heading">
								<div class="tip-name" :title="row.name">{{ row.name }}</div>
								<span class="tip-status" :class="`is-${row.status}`">{{ row.status === 'connected' ? '已连接' : row.status === 'disabled' ? '已禁用' : '未连接' }}</span>
							</div>
							<div class="tip-device" :title="row.description">{{ row.description || '网络适配器' }}</div>
							<div class="tip-grid">
								<div class="tip-cell"><span>IP地址</span><strong>{{ row.ipv4.join('、') || '-' }}</strong></div>
								<div class="tip-cell"><span>速率</span><strong>{{ row.link_speed || '-' }}</strong></div>
								<div class="tip-cell"><span>掩码</span><strong>{{ row.subnet_mask.join('、') || '-' }}</strong></div>
								<div class="tip-cell"><span>MAC</span><strong>{{ row.mac || '-' }}</strong></div>
								<div class="tip-cell"><span>网关</span><strong>{{ row.gateway || '-' }}</strong></div>
								<div class="tip-cell"><span>DNS</span><strong>{{ row.dns.join('、') || '-' }}</strong></div>
							</div>
						</div>
					</template>
					<div class="adapter-main">
						<div class="adapter-name">
							<el-icon
								v-if="appConfig.default_adapter === row.name"
								color="#e6a23c"
								class="default-star">
								<Star />
							</el-icon>
							<span class="name-text">{{ row.name }}</span>
						</div>
						<div class="adapter-desc muted">
							{{ row.description }}
						</div>
					</div>
				</el-tooltip>
				<div class="col-status">
					<el-tag
						v-if="row.status === 'connected'"
						type="success"
						size="small"
						class="status-tag">
						已连接
					</el-tag>
					<el-tag
						v-else-if="row.status === 'disabled'"
						type="info"
						size="small"
						class="status-tag">
						已禁用
					</el-tag>
					<el-tag v-else type="danger" size="small" class="status-tag"
						>未连接</el-tag
					>
				</div>
				<div class="col-toggle">
					<el-tooltip
						:content="isDisabled(row) ? '启用网卡' : '禁用网卡'"
						placement="top"
						popper-class="app-tip">
						<button
							class="switch"
							:class="{ 'is-on': !isDisabled(row) }"
							:disabled="toggling === row.name"
							@click.stop="toggleEnabled(row)">
							<span class="switch-thumb"></span>
						</button>
					</el-tooltip>
				</div>
				<div class="col-action">
					<el-tooltip
						:content="isPinned(row.name) ? '取消置顶' : '置顶'"
						placement="top"
						popper-class="app-tip">
						<button
							class="pin-btn"
							:class="{ 'is-on': isPinned(row.name) }"
							@click.stop="togglePin(row)">
							<svg viewBox="0 0 24 24" aria-hidden="true">
								<path
									d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z" />
							</svg>
						</button>
					</el-tooltip>
				</div>
			</div>
		</div>

		<div class="list-footer">
			<span class="muted">共 {{ adapters.length }} 张网卡</span>
			<button class="net-link" @click="openNetworkPanel">
				<el-icon :size="12"><Connection /></el-icon>
				<span>网络连接</span>
			</button>
		</div>

		<teleport to="body">
			<div
				v-if="ctxMenuVisible"
				class="row-context-menu"
				:style="{ left: ctxMenuPos.x + 'px', top: ctxMenuPos.y + 'px' }"
				@contextmenu.stop.prevent>
				<div class="row-menu-item" @click="renameRow">重命名网卡</div>
				<div class="row-menu-sep"></div>
				<div
					class="row-menu-item"
					:class="{ 'is-disabled': ctxRow && isDefault(ctxRow.name) }"
					@click="setAsDefault">
					{{
						ctxRow && isDefault(ctxRow.name)
							? '当前为默认网卡'
							: '设为默认网卡'
					}}
				</div>
			</div>
		</teleport>
	</div>
</template>

<style scoped>
.list-panel {
	display: flex;
	flex-direction: column;
	flex: 1;
	min-width: 0;
	min-height: 0;
	width: 100%;
	height: 100%;
	padding: 10px;
	gap: 10px;
	border-radius: var(--radius);
	box-shadow: none;
}
.list-header {
	display: flex;
	gap: 8px;
}
.list-header .el-input {
	flex: 1;
}
.list-header :deep(.el-input__wrapper) {
	background-color: var(--panel-bg) !important;
}
.list-title {
	flex: none;
	padding: 0 2px;
	font-size: 13px;
	font-weight: 650;
	letter-spacing: 0.02em;
}
.list-header :deep(.list-refresh.el-button) {
	background-color: transparent;
}
.list-body {
	flex: 1;
	min-height: 0;
	overflow-y: auto;
	display: flex;
	flex-direction: column;
	gap: 8px;
}
.list-colhead {
	display: flex;
	align-items: center;
	gap: 4px;
	padding: 0 10px;
	font-size: 11px;
	color: var(--text-secondary);
}
.col-main {
	flex: 1;
	min-width: 0;
}
.col-status {
	width: 72px;
	flex: none;
	display: flex;
	justify-content: center;
}
.col-toggle {
	width: 30px;
	flex: none;
	display: flex;
	justify-content: center;
}
.col-action {
	width: 40px;
	flex: none;
	display: flex;
	justify-content: flex-end;
}
.adapter-row {
	display: flex;
	align-items: center;
	gap: 4px;
	padding: 10px;
	border-radius: 10px;
	cursor: pointer;
	background: transparent;
	transition: background 0.15s;
}
.adapter-row:hover {
	background: rgba(255, 255, 255, 0.04);
}
.adapter-row.is-pinned:not(.is-current) {
	background: color-mix(in srgb, var(--text-secondary) 12%, transparent);
}
.adapter-row.is-pinned:not(.is-current):hover {
	background: color-mix(in srgb, var(--text-secondary) 18%, transparent);
}
.adapter-row.is-current {
	background: var(--accent-soft);
}

/* 信息区：名称一行、描述一行，纵向紧凑 */
.adapter-main {
	display: flex;
	flex-direction: column;
	gap: 2px;
	min-width: 0;
}
.adapter-name {
	font-weight: 600;
	font-size: 12px;
	display: flex;
	align-items: center;
	gap: 5px;
	min-width: 0;
}
.default-star {
	flex-shrink: 0;
}
.name-text {
	flex: 1;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.adapter-desc {
	font-size: 11px;
	line-height: 1.3;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

.status-tag {
	white-space: nowrap;
	max-width: none;
	flex: none;
	font-size: 11px;
	padding: 1px 6px;
	height: auto;
	line-height: 1.4;
}

/* 启用 / 禁用滑动开关：开启绿色圆点在右，关闭灰色圆点在左 */
.switch {
	position: relative;
	width: 24px;
	height: 14px;
	padding: 0;
	border: none;
	border-radius: 7px;
	background: var(--text-secondary);
	cursor: pointer;
	transition: background 0.15s;
}
.switch-thumb {
	position: absolute;
	top: 2px;
	left: 2px;
	width: 10px;
	height: 10px;
	border-radius: 50%;
	background: #fff;
	box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
	transition: left 0.15s;
}
.switch.is-on {
	background: var(--success);
}
.switch.is-on .switch-thumb {
	left: 12px;
}
.switch:hover:not(:disabled) {
	filter: brightness(1.1);
}
.switch:disabled {
	opacity: 0.5;
	cursor: not-allowed;
}

/* 操作列：图标按钮垂直居中，独立命中区域 */
.pin-btn {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	width: 26px;
	height: 26px;
	padding: 0;
	border: none;
	background: transparent;
	border-radius: 6px;
	color: var(--text-secondary);
	cursor: pointer;
	transition:
		color 0.15s,
		background 0.15s;
}
.pin-btn svg {
	width: 15px;
	height: 15px;
	fill: currentColor;
}
.pin-btn:hover {
	color: var(--accent);
	background: var(--accent-soft);
}
.pin-btn.is-on {
	color: var(--accent);
}

.list-footer {
	display: flex;
	align-items: center;
	justify-content: space-between;
	font-size: 11px;
	padding-top: 6px;
}
.net-link {
	display: inline-flex;
	align-items: center;
	gap: 4px;
	padding: 2px 6px;
	border: none;
	background: transparent;
	border-radius: 6px;
	font-size: 11px;
	color: var(--text-secondary);
	cursor: pointer;
	transition:
		color 0.15s,
		background 0.15s;
}
.net-link:hover {
	color: var(--accent);
	background: var(--accent-soft);
}
</style>

<style>
.adapter-tip.el-popper {
	background-color: var(--panel-bg) !important;
	color: var(--text-primary) !important;
	border: 1px solid var(--panel-border) !important;
	border-radius: 10px !important;
	box-shadow: 0 6px 18px rgba(0, 0, 0, 0.24) !important;
	padding: 6px 8px !important;
	max-width: min(280px, calc(100vw - 32px)) !important;
}
.adapter-tip .tip-info {
	width: min(224px, calc(100vw - 64px));
	min-width: 176px;
	color: var(--text-primary);
}
.adapter-tip .tip-heading {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 4px;
}
.adapter-tip .tip-name {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	font-size: 11px;
	font-weight: 650;
	color: var(--text-primary);
}
.adapter-tip .tip-status {
	flex: none;
	padding: 2px 6px;
	border-radius: 999px;
	font-size: 9px;
	line-height: 1;
	color: var(--success);
	background: color-mix(in srgb, var(--success) 12%, transparent);
}
.adapter-tip .tip-status.is-disabled {
	color: var(--text-secondary);
	background: color-mix(in srgb, var(--text-secondary) 12%, transparent);
}
.adapter-tip .tip-status.is-disconnected {
	color: var(--danger);
	background: color-mix(in srgb, var(--danger) 12%, transparent);
}
.adapter-tip .tip-device {
	margin-top: 2px;
	padding-bottom: 5px;
	border-bottom: 1px solid var(--panel-border);
	color: var(--text-secondary);
	font-size: 9px;
	line-height: 1.4;
	overflow-wrap: anywhere;
}
.adapter-tip .tip-grid {
	display: grid;
	grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
	column-gap: 3px;
	row-gap: 3px;
	padding-top: 1px;
}
.adapter-tip .tip-cell {
	display: flex;
	flex-direction: column;
	gap: 3px;
	min-width: 0;
	font-size: 9px;
	line-height: 1.35;
}
.adapter-tip .tip-cell span {
	color: var(--text-secondary);
	font-size: 8.5px;
	line-height: 1.35;
}
.adapter-tip .tip-cell strong {
	min-width: 0;
	color: var(--text-primary);
	font-size: 9.5px;
	font-weight: 550;
	line-height: 1.35;
	font-variant-numeric: tabular-nums;
	overflow-wrap: anywhere;
}
</style>
