<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import appIcon from '../src-tauri/icons/128x128.png';
import { ElMessage, ElMessageBox, ElConfigProvider } from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import {
	getConfig,
	saveConfig,
	checkAdmin,
	restartAsAdmin,
	shouldPromptAdmin,
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
import {
	Setting,
	Sunny,
	Moon,
	Link,
	Refresh,
	Key,
	Monitor,
	Timer,
	Close,
	Minus,
	FullScreen,
	CopyDocument,
	Download,
} from '@element-plus/icons-vue';

const admin = ref(false);
const appVersion = __APP_VERSION__;
const settingsVisible = ref(false);
const settingsPosition = ref<Record<string, string>>({
	top: '48px',
	right: '12px',
	maxHeight: 'calc(100vh - 60px)',
});
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
const mainWindow = getCurrentWindow();
const windowMaximized = ref(false);
const bodyRef = ref<HTMLElement | null>(null);

/// 调试模式下自绘的极简右键菜单（仅保留「检查元素」）；生产模式不显示任何菜单
const devMenuVisible = ref(false);
const devMenuPos = ref({ x: 0, y: 0 });

/// 版本号展示：后端 tag_name 可能自带 v 前缀，统一为“至多一个 v”
function displayVersion(v?: string | null): string {
	const s = (v ?? '').trim();
	if (!s) return '';
	return /^[vV]/.test(s) ? s : `v${s}`;
}

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

function applyTheme(
	t: 'light' | 'dark',
	revealFrom?: { x: number; y: number },
) {
	const update = () => {
		theme.value = t;
		document.documentElement.classList.toggle('dark', t === 'dark');
		void setWindowTheme(t === 'dark');
	};
	if (!revealFrom || window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
		update();
		return;
	}
	const transitionDocument = document as Document & {
		startViewTransition?: (callback: () => void) => { finished: Promise<void> };
	};
	if (!transitionDocument.startViewTransition) {
		update();
		return;
	}
	const root = document.documentElement;
	const farthestX = Math.max(revealFrom.x, window.innerWidth - revealFrom.x);
	const farthestY = Math.max(revealFrom.y, window.innerHeight - revealFrom.y);
	root.style.setProperty('--theme-origin-x', `${revealFrom.x}px`);
	root.style.setProperty('--theme-origin-y', `${revealFrom.y}px`);
	root.style.setProperty(
		'--theme-reveal-radius',
		`${Math.hypot(farthestX, farthestY) + 24}px`,
	);
	const transition = transitionDocument.startViewTransition(update);
	void transition.finished
		.catch(() => undefined)
		.finally(() => {
			root.style.removeProperty('--theme-origin-x');
			root.style.removeProperty('--theme-origin-y');
			root.style.removeProperty('--theme-reveal-radius');
		});
}

function toggleTheme(event: MouseEvent) {
	const nextTheme = theme.value === 'dark' ? 'light' : 'dark';
	applyTheme(nextTheme, {
		x: event.clientX,
		y: event.clientY,
	});
	appConfig.value.theme = nextTheme;
	void saveConfig(appConfig.value);
}

async function toggleMaximize() {
	windowMaximized.value = await mainWindow.isMaximized();
	if (windowMaximized.value) {
		await mainWindow.unmaximize();
	} else {
		await mainWindow.maximize();
	}
	windowMaximized.value = await mainWindow.isMaximized();
}

async function syncWindowMaximized() {
	windowMaximized.value = await mainWindow.isMaximized();
}

function onTitlebarDoubleClick(event: MouseEvent) {
	if ((event.target as HTMLElement).closest('button, .header-right')) return;
	void toggleMaximize();
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
let windowStateUnlisten: UnlistenFn | null = null;
let windowStateSyncTimer: ReturnType<typeof setTimeout> | null = null;

onMounted(async () => {
	await init();
	await syncWindowMaximized();
	windowStateUnlisten = await mainWindow.onResized(() => {
		if (windowStateSyncTimer) clearTimeout(windowStateSyncTimer);
		windowStateSyncTimer = setTimeout(() => {
			void syncWindowMaximized();
			windowStateSyncTimer = null;
		}, 100);
	});
	// 先处理权限询问（内部已吞掉取消/异常），结束后再静默检查更新，避免弹窗叠加
	await promptAdminIfNeeded();
	void silentCheckUpdate();
	// 挂件切换后通知主窗：本地延时刷新等待收敛，不再回播挂件，避免互相触发
	adaptersUnlisten = await onAdaptersChanged(() =>
		scheduleDelayedRefresh(false),
	);
	document.addEventListener('pointerdown', onDocPointerDown);
	document.addEventListener('keydown', onDocKeydown);
	window.addEventListener('resize', updateSettingsPosition);
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
	window.removeEventListener('resize', updateSettingsPosition);
	if (adaptersUnlisten) void adaptersUnlisten();
	if (windowStateUnlisten) void windowStateUnlisten();
	if (windowStateSyncTimer) clearTimeout(windowStateSyncTimer);
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
	await nextTick();
	updateSettingsPosition();
}

function updateSettingsPosition() {
	if (!settingsVisible.value) return;
	const anchor = document.querySelector<HTMLElement>('.settings-trigger');
	if (!anchor) return;
	const rect = anchor.getBoundingClientRect();
	const dialogWidth = Math.min(440, window.innerWidth - 24);
	const maxRight = Math.max(12, window.innerWidth - dialogWidth - 12);
	const anchorRight = Math.max(12, window.innerWidth - rect.right);
	const right = Math.min(anchorRight, maxRight);
	const top = Math.max(
		8,
		Math.min(rect.bottom + 6, window.innerHeight - 320),
	);
	settingsPosition.value = {
		top: `${top}px`,
		right: `${right}px`,
		width: `${dialogWidth}px`,
		maxHeight: `${window.innerHeight - top - 12}px`,
	};
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

/// 启动时若非管理员，询问是否以管理员身份重启；提权拉起的实例不再重复弹
async function promptAdminIfNeeded() {
	let need = false;
	try {
		need = await shouldPromptAdmin();
	} catch {
		return;
	}
	if (!need) return;
	try {
		await ElMessageBox.confirm(
			'当前为标准权限，修改网卡 IP/DNS、启用或禁用网卡等操作需要管理员权限。是否以管理员身份重启？',
			'权限提示',
			{
				confirmButtonText: '以管理员重启',
				cancelButtonText: '暂不',
				type: 'warning',
				distinguishCancelAndClose: true,
			}
		);
		await restartAsAdmin();
	} catch {
		// 用户选择“暂不”或关闭弹窗：记录偏好，后续启动不再自动询问，可在设置页手动启用
		appConfig.value.admin_prompt_dismissed = true;
		void saveConfig(appConfig.value);
	}
}

/// 设置页：以管理员身份重启（成功后当前进程由后端退出；UAC 取消则留在标准权限）
async function onRestartAsAdmin() {
	try {
		await restartAsAdmin();
	} catch (e) {
		ElMessage.error(typeof e === 'string' ? e : '提权重启失败');
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

/// 已有更新时，从设置页打开更新弹窗进入下载流程
function openUpdateDialog() {
	updateVisible.value = true;
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
		<header
			class="app-header"
			data-tauri-drag-region
			@dblclick="onTitlebarDoubleClick">
			<div class="brand" data-tauri-drag-region>
				<img class="brand-logo" :src="appIcon" alt="IPSwitcher" />
				<div class="brand-text">
					<div class="brand-title">IPSwitcher</div>
					<div class="brand-sub-row">
						<span class="brand-separator" aria-hidden="true">·</span>
						<span class="muted">网卡 IP 快速切换工具</span>
						<span class="brand-version">v{{ appVersion }}</span>
					</div>
				</div>
			</div>
			<div class="header-right">
				<el-tag
					class="header-admin-tag"
					:type="admin ? 'success' : 'warning'"
					size="small">
					{{ admin ? '管理员权限' : '标准权限' }}
				</el-tag>
				<el-tooltip
					:content="theme === 'dark' ? '切换到浅色主题' : '切换到深色主题'"
					placement="bottom"
					popper-class="app-tip">
					<el-button
						class="titlebar-action-button theme-toggle-button"
						:icon="theme === 'dark' ? Sunny : Moon"
						@click="toggleTheme" />
				</el-tooltip>
        <el-tooltip
            content="软件设置"
            placement="bottom"
            popper-class="app-tip">
          <el-badge :is-dot="true" :hidden="!updateAvailable" class="settings-badge">
            <el-button
                class="titlebar-action-button settings-trigger"
                :icon="Setting"
                @click="openSettings" />
          </el-badge>
        </el-tooltip>
				<div class="window-controls">
					<el-tooltip content="最小化" placement="bottom" popper-class="app-tip">
						<button
							class="window-control"
							aria-label="最小化"
							@click="mainWindow.minimize()">
							<el-icon><Minus /></el-icon>
						</button>
					</el-tooltip>
					<el-tooltip
						:content="windowMaximized ? '还原' : '最大化'"
						placement="bottom"
						popper-class="app-tip">
						<button
							class="window-control"
							:aria-label="windowMaximized ? '还原' : '最大化'"
							@click="toggleMaximize">
							<el-icon>
								<component :is="windowMaximized ? CopyDocument : FullScreen" />
							</el-icon>
						</button>
					</el-tooltip>
					<el-tooltip content="关闭" placement="bottom" popper-class="app-tip">
						<button
							class="window-control close-control"
							aria-label="关闭"
							@click="mainWindow.close()">
							<el-icon><Close /></el-icon>
						</button>
					</el-tooltip>
				</div>
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
			width="440px"
			:style="settingsPosition"
			class="settings-dialog"
			modal-class="settings-overlay"
			:show-close="false"
			destroy-on-close>
			<template #header>
				<div class="settings-header">
					<div class="settings-header-mark"><el-icon><Setting /></el-icon></div>
					<div class="settings-header-copy">
						<div class="settings-title">软件设置</div>
					</div>
					<el-button
						class="settings-close"
						:icon="Close"
						circle
						aria-label="关闭设置"
						@click="settingsVisible = false" />
				</div>
			</template>
			<div class="settings-content">
				<section class="settings-section">
					<div class="settings-section-heading">
						<span class="settings-section-title">默认网卡</span>
					</div>
					<el-select
						v-model="defaultAdapter"
						placeholder="请选择网卡"
						clearable
						:fit-input-width="true"
						popper-class="settings-select-popper"
						class="settings-default-select">
						<el-option
							v-for="a in adapters"
							:key="a.name"
							:label="a.name"
							:value="a.name">
							<div class="settings-adapter-option">
								<span class="settings-adapter-option-name">{{ a.name }}</span>
								<span class="settings-adapter-option-description">{{ a.description }}</span>
							</div>
						</el-option>
					</el-select>
				</section>

				<section class="settings-section">
					<div class="settings-section-heading">
						<span class="settings-section-title">常规</span>
					</div>
					<div class="settings-close-card">
						<span class="settings-close-label">点击关闭按钮时</span>
						<el-radio-group v-model="closeAction" class="settings-close-radios">
							<el-radio value="tray">最小化到托盘</el-radio>
							<el-radio value="exit">退出软件</el-radio>
						</el-radio-group>
					</div>
					<div class="settings-toggle-grid">
						<div class="settings-surface settings-toggle-card">
							<div class="settings-item-icon"><el-icon><Monitor /></el-icon></div>
							<div class="settings-item-copy"><strong>桌面挂件</strong></div>
							<el-switch v-model="widgetVisible" size="small" />
						</div>
						<div class="settings-surface settings-toggle-card">
							<div class="settings-item-icon"><el-icon><Timer /></el-icon></div>
							<div class="settings-item-copy"><strong>开机自启</strong></div>
							<el-switch v-model="autostart" size="small" />
						</div>
					</div>
				</section>

				<section class="settings-section">
					<div class="settings-section-heading">
						<span class="settings-section-title">权限</span>
					</div>
					<div class="settings-permission-grid">
						<div class="settings-surface settings-permission-card">
							<div class="settings-permission-mark" :class="{ enabled: admin }">
								<el-icon><Key /></el-icon>
							</div>
							<div class="settings-permission-copy">
								<div class="settings-permission-heading">
									<strong>管理员权限</strong>
								</div>
								<span>修改 IP、DNS 和网卡状态</span>
							</div>
							<el-tag v-if="admin" type="success" effect="dark" round>已启用</el-tag>
							<el-button
								v-else
								class="settings-secondary-button"
								:icon="Key"
								@click="onRestartAsAdmin">启用</el-button>
						</div>
						<button
							type="button"
							class="settings-surface settings-shortcut-card"
							:disabled="shortcutBusy"
							@click="onCreateShortcut">
							<span class="settings-item-icon"><el-icon><Link /></el-icon></span>
							<span class="settings-shortcut-label">{{ shortcutBusy ? '创建中…' : '创建快捷方式' }}</span>
						</button>
					</div>
				</section>
			</div>
			<template #footer>
				<div class="settings-footer-row">
					<div class="settings-update-group">
						<el-button
							class="settings-update-button"
							:icon="Refresh"
							:loading="checkingUpdate"
							@click="manualCheckUpdate">检查更新</el-button>
						<el-button
							v-if="updateAvailable"
							class="settings-update-button has-update"
							:icon="Download"
							@click="openUpdateDialog">
							下载 {{ displayVersion(updateInfo?.latestVersion) }}
						</el-button>
					</div>
					<div class="settings-footer-actions">
						<el-button class="settings-cancel-button" @click="settingsVisible = false">取消</el-button>
						<el-button type="primary" class="settings-save-button" @click="saveDefaultAdapter">保存设置</el-button>
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
	background: var(--app-bg);
	border: 1px solid var(--panel-border);
}
.app-header {
	height: 40px;
	flex: none;
	display: flex;
	align-items: center;
	justify-content: space-between;
	padding: 0 0 0 12px;
	border-bottom: 1px solid var(--panel-border);
}
.brand {
	display: flex;
	align-items: center;
	gap: 8px;
}
.brand-logo {
	width: 18px;
	height: 18px;
	flex: none;
	object-fit: contain;
}
.brand-text {
	min-width: 0;
	display: flex;
	align-items: center;
	gap: 6px;
}
.brand-title {
	font-weight: 600;
	font-size: 13px;
	line-height: 1.2;
	white-space: nowrap;
}
.brand-sub-row {
	display: flex;
	align-items: center;
	gap: 6px;
	font-size: 10px;
	white-space: nowrap;
}
.brand-separator {
	color: var(--text-secondary);
	opacity: 0.72;
}
.brand-version {
	font-size: 10px;
	line-height: 1.4;
	color: var(--text-secondary);
	background: var(--accent-soft);
	border-radius: var(--radius-sm);
	padding: 2px 8px;
}
/* 齿轮更新红点：缩小并贴合图标右上角齿尖，橙红色 */
.settings-badge :deep(.el-badge__content.is-fixed.is-dot) {
	width: 6px;
	height: 6px;
	border: none;
	background: var(--update-accent);
	box-shadow: 0 0 0 1px var(--panel-border);
	top: 15px;
	right: 15px;
	transform: translate(50%, -50%);
}
.header-right {
	height: 100%;
	display: flex;
	align-items: center;
	gap: 1px;
	-webkit-app-region: no-drag;
}
.header-right :deep(*) {
	-webkit-app-region: no-drag;
}
.header-right :deep(.header-admin-tag.el-tag) {
	height: 22px;
	padding: 0 6px;
	border-radius: 6px;
	font-size: 11px;
	line-height: 20px;
}
.header-right :deep(.titlebar-action-button.el-button) {
	width: 40px;
	height: 40px;
	flex: none;
	margin: 0;
	padding: 0;
	border: 0 !important;
	border-radius: 0;
	background: transparent !important;
	box-shadow: none !important;
	color: var(--text-secondary);
}
.header-right :deep(.titlebar-action-button.el-button:hover) {
	border: 0 !important;
	background: color-mix(in srgb, var(--text-primary) 9%, transparent) !important;
	color: var(--text-primary);
}
.window-controls {
	height: 100%;
	display: flex;
	align-items: stretch;
	margin-left: -1px;
}
.window-control {
	width: 44px;
	height: 40px;
	display: grid;
	place-items: center;
	padding: 0;
	border: 0;
	background: transparent;
	color: var(--text-secondary);
	cursor: pointer;
}
.window-control .el-icon {
	font-size: 14px;
}
.window-control:hover {
	background: color-mix(in srgb, var(--text-primary) 9%, transparent);
	color: var(--text-primary);
}
.window-control.close-control {
	width: 48px;
}
.window-control.close-control:hover {
	background: #c42b1c;
	color: #fff;
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
