<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { ElMessage, ElConfigProvider } from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import {
	getConfig,
	saveConfig,
	checkAdmin,
	showWidget,
	hideWidget,
	setWindowTheme,
	onAdaptersChanged,
	getAutostart,
	setAutostart,
	createDesktopShortcut,
	emitConfigChanged,
	openDevTools,
	checkUpdate,
} from '@/api/ipc';
import type { UpdateInfo } from '@/types';
import {
	appConfig,
	adapters,
	currentName,
	refreshAdapters,
	scheduleDelayedRefresh,
	normalizeConfig,
} from '@/composables/useStore';
import type { UnlistenFn } from '@tauri-apps/api/event';
import AdapterList from '@/components/AdapterList.vue';
import TrafficMonitor from '@/components/TrafficMonitor.vue';
import AdapterDetail from '@/components/AdapterDetail.vue';
import Diagnostics from '@/components/Diagnostics.vue';
import NetworkScan from '@/components/NetworkScan.vue';
import UpdateDialog from '@/components/UpdateDialog.vue';
import { Setting, Connection, Sunny, Moon, Link, Refresh } from '@element-plus/icons-vue';

const admin = ref(false);
const appVersion = __APP_VERSION__;
const settingsVisible = ref(false);
const activeTab = ref('ip');
const defaultAdapter = ref('');
const closeAction = ref<'exit' | 'tray'>('tray');
const widgetVisible = ref(true);
const autostart = ref(true);
const shortcutBusy = ref(false);
const updateVisible = ref(false);
const updateInfo = ref<UpdateInfo | null>(null);
const updateAvailable = ref(false);
const checkingUpdate = ref(false);
const sidebarWidth = ref(350);
const theme = ref<'light' | 'dark'>('dark');
const bodyRef = ref<HTMLElement | null>(null);

/// 调试模式下自绘的极简右键菜单（仅保留「检查元素」）；生产模式不显示任何菜单
const devMenuVisible = ref(false);
const devMenuPos = ref({ x: 0, y: 0 });

function closeDevMenu() {
	devMenuVisible.value = false;
}

function onContextMenu(e: MouseEvent) {
	// 一律屏蔽 WebView2 原生整组右键菜单（复制 / 粘贴 / 后退 / 检查等）
	e.preventDefault();
	if (!import.meta.env.DEV) {
		closeDevMenu();
		return;
	}
	// 调试模式：在光标位置弹出仅含「检查元素」的自绘菜单，并防止超出视口
	const menuW = 116;
	const menuH = 34;
	devMenuPos.value = {
		x: Math.min(e.clientX, window.innerWidth - menuW - 8),
		y: Math.min(e.clientY, window.innerHeight - menuH - 8),
	};
	devMenuVisible.value = true;
}

function handleOpenDevTools() {
	closeDevMenu();
	void openDevTools();
}

function onDocPointerDown(e: MouseEvent) {
	if (!devMenuVisible.value) return;
	if ((e.target as HTMLElement).closest('.dev-context-menu')) return;
	closeDevMenu();
}

function onDocKeydown(e: KeyboardEvent) {
	if (e.key === 'Escape') closeDevMenu();
}

const MIN_WIDTH = 220;
const MAX_WIDTH = 520;

function applyTheme(t: 'light' | 'dark') {
	theme.value = t;
	document.documentElement.classList.toggle('dark', t === 'dark');
	void setWindowTheme(t === 'dark');
}

function toggleTheme() {
	applyTheme(theme.value === 'dark' ? 'light' : 'dark');
	appConfig.value.theme = theme.value;
	void saveConfig(appConfig.value);
}

async function init() {
	appConfig.value = normalizeConfig(await getConfig());
	void saveConfig(appConfig.value);
	sidebarWidth.value = appConfig.value.sidebar_width ?? 350;
	widgetVisible.value = appConfig.value.widget_visible ?? true;
	try {
		autostart.value = await getAutostart();
	} catch {
		autostart.value = appConfig.value.autostart ?? true;
	}
	applyTheme(appConfig.value.theme === 'light' ? 'light' : 'dark');
	admin.value = await checkAdmin();
	await refreshAdapters();
	if (!currentName.value) {
		currentName.value =
			appConfig.value.default_adapter ?? adapters.value[0]?.name ?? null;
	}
}

let adaptersUnlisten: UnlistenFn | null = null;

onMounted(async () => {
	await init();
	void silentCheckUpdate();
	// 挂件切换后通知主窗：本地延时刷新等待收敛，不再回播挂件，避免互相触发
	adaptersUnlisten = await onAdaptersChanged(() =>
		scheduleDelayedRefresh(false),
	);
	document.addEventListener('pointerdown', onDocPointerDown);
	document.addEventListener('keydown', onDocKeydown);
});

let dragging = false;

function onDragMove(e: PointerEvent) {
	if (!dragging || !bodyRef.value) return;
	const left = bodyRef.value.getBoundingClientRect().left;
	const w = Math.round(
		Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, e.clientX - left - 5)),
	);
	sidebarWidth.value = w;
}

function stopDrag() {
	if (!dragging) return;
	dragging = false;
	window.removeEventListener('pointermove', onDragMove);
	window.removeEventListener('pointerup', stopDrag);
	appConfig.value.sidebar_width = sidebarWidth.value;
	void saveConfig(appConfig.value);
}

function startDrag(e: PointerEvent) {
	dragging = true;
	e.preventDefault();
	window.addEventListener('pointermove', onDragMove);
	window.addEventListener('pointerup', stopDrag);
}

onUnmounted(() => {
	window.removeEventListener('pointermove', onDragMove);
	window.removeEventListener('pointerup', stopDrag);
	document.removeEventListener('pointerdown', onDocPointerDown);
	document.removeEventListener('keydown', onDocKeydown);
	if (adaptersUnlisten) void adaptersUnlisten();
});

async function openSettings() {
	defaultAdapter.value = appConfig.value.default_adapter ?? '';
	closeAction.value =
		appConfig.value.close_action === 'exit' ? 'exit' : 'tray';
	widgetVisible.value = appConfig.value.widget_visible ?? true;
	try {
		autostart.value = await getAutostart();
	} catch {
		autostart.value = appConfig.value.autostart ?? true;
	}
	settingsVisible.value = true;
}

async function saveDefaultAdapter() {
	appConfig.value.default_adapter = defaultAdapter.value || null;
	appConfig.value.close_action = closeAction.value;
	appConfig.value.widget_visible = widgetVisible.value;
	appConfig.value.autostart = autostart.value;
	await saveConfig(appConfig.value);
	await setAutostart(autostart.value);
	emitConfigChanged();
	if (widgetVisible.value) {
		await showWidget();
	} else {
		await hideWidget();
	}
	ElMessage.success('设置已保存');
	settingsVisible.value = false;
}

async function onCreateShortcut() {
	if (shortcutBusy.value) return;
	shortcutBusy.value = true;
	try {
		await createDesktopShortcut();
		ElMessage.success('已在桌面创建快捷方式');
	} catch (e) {
		ElMessage.error(typeof e === 'string' ? e : '创建桌面快捷方式失败');
	} finally {
		shortcutBusy.value = false;
	}
}

/// 启动时静默检查更新：有新版才弹窗，失败（断网 / 限流 / 异常）静默忽略不打扰
async function silentCheckUpdate() {
	try {
		const info = await checkUpdate();
		updateAvailable.value = info.hasUpdate && !!info.assetUrl;
		if (updateAvailable.value) {
			updateInfo.value = info;
			updateVisible.value = true;
		}
	} catch {
		// 静默忽略
	}
}

/// 设置页手动检查更新：失败 / 已是最新 / 无匹配资产均给出明确提示
async function manualCheckUpdate() {
	if (checkingUpdate.value) return;
	checkingUpdate.value = true;
	try {
		const info = await checkUpdate();
		if (!info.hasUpdate) {
			updateAvailable.value = false;
			ElMessage.success('已是最新版本');
			return;
		}
		if (!info.assetUrl) {
			updateAvailable.value = false;
			ElMessage.warning(info.message ?? '暂未匹配到更新包');
			return;
		}
		updateAvailable.value = true;
		updateInfo.value = info;
		updateVisible.value = true;
	} catch (e) {
		ElMessage.error(typeof e === 'string' ? e : '检查更新失败，请稍后重试');
	} finally {
		checkingUpdate.value = false;
	}
}
</script>

<template>
	<el-config-provider :locale="zhCn">
	<div class="app-shell" @contextmenu="onContextMenu">
		<teleport to="body">
			<div
				v-if="devMenuVisible"
				class="dev-context-menu"
				:style="{ left: devMenuPos.x + 'px', top: devMenuPos.y + 'px' }"
				@contextmenu.stop.prevent>
				<div class="dev-menu-item" @click="handleOpenDevTools">
					检查元素
				</div>
			</div>
		</teleport>
		<header class="app-header">
			<div class="brand">
				<div class="brand-logo">
					<Connection :size="20" />
				</div>
				<div class="brand-text">
					<div class="brand-title">IPSwitcher</div>
					<div class="brand-sub-row">
						<span class="muted">网卡 IP 快速切换工具</span>
						<el-badge
							:is-dot="true"
							:hidden="!updateAvailable"
							class="version-badge">
							<span class="brand-version">v{{ appVersion }}</span>
						</el-badge>
					</div>
				</div>
			</div>
			<div class="header-right">
				<el-tag :type="admin ? 'success' : 'warning'" size="small">
					{{ admin ? '管理员权限' : '标准权限' }}
				</el-tag>
				<el-tooltip
					:content="
						theme === 'dark' ? '切换到浅色主题' : '切换到深色主题'
					"
					placement="bottom"
					popper-class="app-tip">
					<el-button
						:icon="theme === 'dark' ? Sunny : Moon"
						circle
						@click="toggleTheme" />
				</el-tooltip>
				<el-tooltip
					content="软件设置"
					placement="bottom"
					popper-class="app-tip">
					<el-badge :is-dot="true" :hidden="!updateAvailable">
						<el-button :icon="Setting" circle @click="openSettings" />
					</el-badge>
				</el-tooltip>
			</div>
		</header>

		<div class="app-body" ref="bodyRef">
			<aside class="sidebar" :style="{ width: sidebarWidth + 'px' }">
				<AdapterList class="sidebar-adapter-list" @refresh="refreshAdapters" />
				<TrafficMonitor />
			</aside>
			<div
				class="splitter"
				role="separator"
				aria-orientation="vertical"
				@pointerdown="startDrag"></div>
			<section class="main">
				<el-tabs v-model="activeTab" class="main-tabs">
					<el-tab-pane label="IP 配置" name="ip">
						<AdapterDetail />
					</el-tab-pane>
					<el-tab-pane label="主机扫描" name="scan">
						<NetworkScan />
					</el-tab-pane>
					<el-tab-pane label="网络诊断" name="diag">
						<Diagnostics />
					</el-tab-pane>
				</el-tabs>
			</section>
		</div>

		<el-dialog
			v-model="settingsVisible"
			title="软件设置"
			width="480px"
			class="settings-dialog"
			destroy-on-close>
			<el-form label-position="top">
				<el-form-item label="默认网卡（启动时自动选中）">
					<el-select
						v-model="defaultAdapter"
						placeholder="请选择默认网卡"
						clearable
						:fit-input-width="false"
						popper-class="settings-select-popper"
						style="width: 100%">
						<el-option
							v-for="a in adapters"
							:key="a.name"
							:label="`${a.name}（${a.description}）`"
							:value="a.name" />
					</el-select>
				</el-form-item>
				<el-form-item>
					<div class="settings-inline-row">
						<span class="settings-inline-label">点击关闭按钮时</span>
						<el-radio-group v-model="closeAction">
							<el-radio value="exit">直接退出软件</el-radio>
							<el-radio value="tray">最小化到系统托盘</el-radio>
						</el-radio-group>
					</div>
				</el-form-item>
				<el-form-item>
					<div class="settings-switch-row">
						<div class="settings-switch-cell">
							<span class="settings-switch-label">桌面挂件</span>
							<el-switch v-model="widgetVisible" size="small" />
						</div>
						<div class="settings-switch-cell">
							<span class="settings-switch-label">开机自启</span>
							<el-switch v-model="autostart" size="small" />
						</div>
						<el-button
							class="shortcut-btn"
							:icon="Link"
							:loading="shortcutBusy"
							@click="onCreateShortcut"
							>在桌面创建快捷方式</el-button
						>
					</div>
				</el-form-item>
				</el-form>
				<template #footer>
					<div class="settings-footer-row">
						<el-badge :is-dot="true" :hidden="!updateAvailable">
							<el-button
								class="shortcut-btn"
								:icon="Refresh"
								:loading="checkingUpdate"
								@click="manualCheckUpdate"
								>检查更新</el-button
							>
						</el-badge>
						<div class="settings-footer-actions">
							<el-button @click="settingsVisible = false">取消</el-button>
							<el-button type="primary" @click="saveDefaultAdapter"
								>保存</el-button
							>
						</div>
					</div>
				</template>
		</el-dialog>

		<UpdateDialog v-model:visible="updateVisible" :info="updateInfo" />
	</div>
	</el-config-provider>
</template>

<style scoped>
.app-shell {
	height: 100%;
	display: flex;
	flex-direction: column;
}
.app-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	padding: 10px 14px;
	border-bottom: 1px solid var(--panel-border);
}
.brand {
	display: flex;
	align-items: center;
	gap: 8px;
}
.brand-logo {
	width: 32px;
	height: 32px;
	border-radius: var(--radius-sm);
	background: var(--accent-soft);
	color: var(--accent);
	display: flex;
	align-items: center;
	justify-content: center;
}
.brand-title {
	font-weight: 700;
	font-size: 16px;
}
.brand-sub-row {
	display: flex;
	align-items: center;
	gap: 6px;
	font-size: 12px;
}
.brand-version {
	font-size: 11px;
	line-height: 1.4;
	color: var(--text-secondary);
	background: var(--accent-soft);
	border-radius: var(--radius-sm);
	padding: 1px 6px;
}
.version-badge {
	display: inline-flex;
	align-items: center;
}
.version-badge :deep(.el-badge__content.is-dot) {
	top: 2px;
	right: -2px;
}
.header-right {
	display: flex;
	align-items: center;
	gap: 10px;
}
.app-body {
	flex: 1;
	display: flex;
	gap: 0;
	padding: 10px;
	min-height: 0;
}
.sidebar {
	flex-shrink: 0;
	min-width: 0;
	height: 100%;
	display: flex;
	flex-direction: column;
	gap: 10px;
	overflow: hidden;
	width: 100%;
}
.sidebar-adapter-list {
	flex: 1;
	min-height: 0;
	height: auto;
	width: 100%;
}
.splitter {
	width: 6px;
	flex-shrink: 0;
	cursor: col-resize;
	transition: background 0.15s;
	position: relative;
}
.splitter::after {
	content: '';
	position: absolute;
	top: 1px;
	bottom: 1px;
	left: 50%;
	width: 3px;
	transform: translateX(-50%);
}
.splitter:hover,
.splitter:active {
	background: transparent;
}
.splitter:hover::after,
.splitter:active::after {
	background: var(--accent);
}
.main {
	flex: 1;
	min-width: 0;
	margin-left: 4px;
	display: flex;
	flex-direction: column;
}
.main-tabs {
	flex: 1;
	display: flex;
	flex-direction: column;
	min-height: 0;
}
.main-tabs :deep(.el-tabs__header) {
	margin: 0 0 10px;
	border-bottom: 1px solid var(--panel-border);
}
.main-tabs :deep(.el-tabs__nav-wrap)::after {
	background: transparent;
}
.main-tabs :deep(.el-tabs__item) {
	color: var(--text-secondary);
	font-size: 14px;
	height: 38px;
	line-height: 38px;
}
.main-tabs :deep(.el-tabs__item:hover) {
	color: var(--accent);
}
.main-tabs :deep(.el-tabs__item.is-active) {
	color: var(--accent);
	font-weight: 600;
}
.main-tabs :deep(.el-tabs__active-bar) {
	background: var(--accent);
	height: 2px;
	border-radius: 2px;
}
.main-tabs :deep(.el-tabs__content),
.main-tabs :deep(.el-tab-pane) {
	flex: 1;
	height: 100%;
	min-height: 0;
}

.el-button+.el-button {
    margin-left: 0px;
}
</style>
