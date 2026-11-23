<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { ElMessage, ElConfigProvider } from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import {
	DocumentCopy,
	Lock,
	Unlock,
	WarningFilled,
	Connection,
	Link,
	Odometer,
	Top,
	Bottom,
	ArrowDown,
	Check,
} from '@element-plus/icons-vue';
import {
	getCurrentWindow,
	LogicalSize,
	LogicalPosition,
} from '@tauri-apps/api/window';
import {
	applyDhcp,
	applyStatic,
	getAdapterTraffic,
	elevateAction,
	getConfig,
	getWidgetPinned,
	onAdaptersChanged,
	onConfigChanged,
	onTrafficMonitorChanged,
	restoreMainWindow,
	setWidgetAdapter,
	setWidgetOpacity,
	saveWidgetSize,
	setWidgetPinned,
	onWidgetPinChanged,
	openDevTools,
} from '@/api/ipc';
import type { AppConfig, IpProfile, PendingAction } from '@/types';
import { adapters, applyAdapterSnapshot, normalizeConfig, refreshAdapters, scheduleDelayedRefresh } from '@/composables/useStore';
import { useScopedListeners } from '@/composables/useScopedListeners';

const win = getCurrentWindow();
const registerListener = useScopedListeners();
const currentName = ref<string | null>(null);
const appConfig = ref<AppConfig | null>(null);
const trafficMonitorPreference = ref<boolean>();
const trafficMonitorEnabled = computed(() => appConfig.value !== null
	&& (trafficMonitorPreference.value ?? appConfig.value.traffic_monitor_enabled ?? true));
let configRequest = 0;
const applying = ref<null | 'dhcp' | 'static'>(null);
/// 背景不透明度（0.0 ~ 1.0），仅作用于卡片背景，字体始终保持不透明
const opacity = ref<number>(0.92);
/// 挂件固定基线高度，纵向不可调整
const BASE_HEIGHT = 246;
/// 横向拉宽区间（逻辑像素）
const WIDTH_MIN = 240;
const WIDTH_MAX = 640;
/// 网卡列表在底部向下展开时临时扩展窗口底部
const selectOpen = ref<boolean>(false);
const SELECT_MENU_MAX_HEIGHT = 180;
/// 固定（鼠标穿透）状态：固定后位置锁定、停用拖拽缩放与悬停交互
const pinned = ref<boolean>(false);
/// 「暂无固定 IP 方案」自绘提示框是否显示
const showNoProfile = ref<boolean>(false);
const uploadRate = ref(0);
const downloadRate = ref(0);
let trafficTimer: number | undefined;
let trafficSampling = false;
let disposed = false;
let trafficGeneration = 0;
let trafficBaseline: {
	name: string;
	sent: number;
	received: number;
	at: number;
} | null = null;

/// 由背景透明度派生各处 alpha，以数值直接写入 CSS 变量（rgba alpha 内 calc 乘法在 WebView 中会失效）
const styleVars = computed(() => ({
	'--widget-bg-alpha': opacity.value,
	'--root-border-alpha': 0.12 * opacity.value,
	'--select-bg-alpha': 0.035 * opacity.value,
}));

/// 网卡菜单展开期间临时扩展窗口底部，不将临时尺寸写入配置
let selectGeom: { x: number; y: number; w: number; h: number } | null = null;

const adapter = computed(
	() => adapters.value.find((a) => a.name === currentName.value) ?? null,
);

/// 当前网卡可套用的静态方案（挂件仅取第一个静态方案）
const staticProfile = computed<IpProfile | null>(() => {
	if (!adapter.value) return null;
	return (
		appConfig.value?.profiles.find(
			(p) => p.mode === 'static' && p.ip && p.mask,
		) ?? null
	);
});

function selectAvailableAdapter(...names: (string | null | undefined)[]) {
	currentName.value = names.find((name) => adapters.value.some((a) => a.name === name))
		?? adapters.value[0]?.name ?? null;
}

/// 每秒读取所选网卡的系统累计计数，并用相邻采样差计算实时速率
async function sampleTraffic() {
	const selected = adapter.value;
	if (!trafficMonitorEnabled.value || !selected || trafficSampling || disposed || document.hidden) return;
	const { name, if_index } = selected;
	const generation = trafficGeneration;
	trafficSampling = true;
	try {
		const counters = await getAdapterTraffic(name, if_index);
		const now = counters.sampled_at;
		if (generation !== trafficGeneration) return;
		if (trafficBaseline?.name === name) {
			if (now === trafficBaseline.at) return;
			const elapsed = (now - trafficBaseline.at) / 1000;
			const sent = Math.max(
				0,
				counters.sent_bytes - trafficBaseline.sent,
			);
			const received = Math.max(
				0,
				counters.received_bytes - trafficBaseline.received,
			);
			uploadRate.value = elapsed > 0 && elapsed <= 3 ? sent / elapsed : 0;
			downloadRate.value =
				elapsed > 0 && elapsed <= 3 ? received / elapsed : 0;
		} else {
			uploadRate.value = 0;
			downloadRate.value = 0;
		}
		trafficBaseline = {
			name,
			sent: counters.sent_bytes,
			received: counters.received_bytes,
			at: now,
		};
	} catch {
		if (generation === trafficGeneration) {
			uploadRate.value = 0;
			downloadRate.value = 0;
			trafficBaseline = null;
		}
	} finally {
		trafficSampling = false;
	}
}

function formatTrafficRate(value: number): string {
	if (!Number.isFinite(value) || value < 1024) {
		return `${Math.max(0, value).toFixed(0)} B/s`;
	}
	const units = ['KB/s', 'MB/s', 'GB/s', 'TB/s'];
	let amount = value / 1024;
	let unit = 0;
	while (amount >= 1024 && unit < units.length - 1) {
		amount /= 1024;
		unit++;
	}
	return `${amount.toFixed(amount >= 100 ? 0 : 1)} ${units[unit]}`;
}

function scheduleWidgetRefresh() {
	void refreshAdapters().catch(() => undefined);
	scheduleDelayedRefresh();
}

function syncTrafficTimer() {
	trafficGeneration++;
	trafficBaseline = null;
	uploadRate.value = 0;
	downloadRate.value = 0;
	window.clearInterval(trafficTimer);
	trafficTimer = undefined;
	if (!trafficMonitorEnabled.value || !currentName.value || document.hidden || disposed) return;
	void sampleTraffic();
	trafficTimer = window.setInterval(() => void sampleTraffic(), 1000);
}

async function reloadConfig() {
	const request = ++configRequest;
	const config = normalizeConfig(await getConfig());
	if (disposed || request !== configRequest) return;
	const firstLoad = appConfig.value === null;
	appConfig.value = config;
	opacity.value = config.widget_opacity ?? 0.92;
	if (!firstLoad) selectAvailableAdapter(config.default_adapter, currentName.value, config.widget_adapter);
}

/// 调试模式自绘右键菜单（仅「检查元素」）；生产模式不显示菜单
const devMenuVisible = ref(false);
const devMenuPos = ref({ x: 0, y: 0 });

function closeDevMenu() {
	devMenuVisible.value = false;
}

function onContextMenu(e: MouseEvent) {
	e.preventDefault();
	if (!import.meta.env.DEV) {
		closeDevMenu();
		return;
	}
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
	const target = e.target as HTMLElement;
	if (devMenuVisible.value && !target.closest('.dev-context-menu')) {
		closeDevMenu();
	}
	if (selectOpen.value && !target.closest('.adapter-picker')) {
		selectOpen.value = false;
	}
}

function onDocKeydown(e: KeyboardEvent) {
	if (e.key !== 'Escape') return;
	if (selectOpen.value) {
		selectOpen.value = false;
		return;
	}
	closeDevMenu();
}

async function chooseAdapter(name: string) {
	selectOpen.value = false;
	if (name === currentName.value) return;
	try {
		await setWidgetAdapter(name);
		currentName.value = name;
	} catch (error) {
		ElMessage.error(String(error));
	}
}

onMounted(async () => {
	document.addEventListener('pointerdown', onDocPointerDown);
	document.addEventListener('keydown', onDocKeydown);
	document.addEventListener('visibilitychange', syncTrafficTimer);
	try {
		await Promise.all([
			registerListener(onAdaptersChanged(applyAdapterSnapshot)),
			registerListener(onConfigChanged(() => void reloadConfig().catch(() => undefined))),
			registerListener(onTrafficMonitorChanged((enabled) => { trafficMonitorPreference.value = enabled; })),
			registerListener(onWidgetPinChanged((p) => { pinned.value = p; })),
		]);
		const [, currentPin] = await Promise.all([reloadConfig(), getWidgetPinned()]);
		if (disposed) return;
		pinned.value = currentPin;
		await refreshAdapters(false);
		if (disposed) return;
		selectAvailableAdapter(appConfig.value?.widget_adapter, appConfig.value?.default_adapter, currentName.value);
	} catch (error) {
		if (!disposed) ElMessage.error(String(error));
	}
});

onUnmounted(() => {
	disposed = true;
	syncTrafficTimer();
	cancelPointerGesture?.();
	document.removeEventListener('pointerdown', onDocPointerDown);
	document.removeEventListener('keydown', onDocKeydown);
	document.removeEventListener('visibilitychange', syncTrafficTimer);
});

watch(adapters, () => {
	selectAvailableAdapter(currentName.value, appConfig.value?.widget_adapter, appConfig.value?.default_adapter);
});

watch([trafficMonitorEnabled, currentName], syncTrafficTimer);

async function captureGeom() {
	const [scale, size, pos] = await Promise.all([
		win.scaleFactor(),
		win.innerSize(),
		win.outerPosition(),
	]);
	return {
		x: pos.x / scale,
		y: pos.y / scale,
		w: size.width / scale,
		h: size.height / scale,
	};
}

function adapterMenuHeight() {
	return (
		Math.min(
			SELECT_MENU_MAX_HEIGHT,
			Math.max(38, adapters.value.length * 38),
		) + 10
	);
}

/// 网卡菜单在底部向下展开，临时扩展窗口底边；该过程不落盘
watch(selectOpen, async (open) => {
	if (open) {
		if (!selectGeom) selectGeom = await captureGeom();
		if (!selectOpen.value || !selectGeom) {
			selectGeom = null;
			return;
		}
		const g = selectGeom;
		await nextTick();
		if (!selectOpen.value) return;
		const rootRect = document
			.querySelector('.widget-root')
			?.getBoundingClientRect();
		const menuRect = document
			.querySelector('.adapter-menu')
			?.getBoundingClientRect();
		const menuSpace =
			rootRect && menuRect
				? Math.max(0, Math.ceil(menuRect.bottom - rootRect.bottom))
				: adapterMenuHeight() + 6;
		await win.setSize(new LogicalSize(g.w, g.h + menuSpace));
		await win.setPosition(new LogicalPosition(g.x, g.y));
	} else if (selectGeom) {
		const g = selectGeom;
		await win.setSize(new LogicalSize(g.w, g.h));
		await win.setPosition(new LogicalPosition(g.x, g.y));
		selectGeom = null;
	}
});

async function onOpacityChange(v: number) {
	try {
		await setWidgetOpacity(v);
	} catch (error) {
		ElMessage.error(String(error));
	}
}

async function handleApplyError(e: unknown) {
	const msg = typeof e === 'string' ? e : String(e);
	if (msg === 'ELEVATION_REQUIRED') {
		let action: PendingAction;
		if (applying.value === 'dhcp') {
			action = {
				kind: 'Dhcp',
				names: [adapter.value!.name],
				reset_dns: true,
			};
		} else {
			const p = staticProfile.value!;
			action = {
				kind: 'Static',
				names: [adapter.value!.name],
				ip: p.ip!,
				mask: p.mask!,
				gateway: p.gateway ?? null,
				dns1: p.dns1 ?? null,
				dns2: p.dns2 ?? null,
			};
		}
		await elevateAction(action);
		ElMessage.info('已提交提权执行，请查看弹出的提示框');
		scheduleWidgetRefresh();
	} else {
		ElMessage.error(msg || '操作失败');
	}
}

async function applyDhcpNow() {
	if (!adapter.value) {
		ElMessage.warning('请先选择网卡');
		return;
	}
	applying.value = 'dhcp';
	try {
		await applyDhcp(adapter.value.name, true);
		ElMessage.success('已切换为 DHCP 自动获取');
		scheduleWidgetRefresh();
	} catch (e) {
		await handleApplyError(e);
	} finally {
		applying.value = null;
	}
}

async function applyStaticNow() {
	if (!adapter.value) {
		ElMessage.warning('请先选择网卡');
		return;
	}
	const p = staticProfile.value;
	if (!p) {
		showNoProfile.value = true;
		return;
	}
	applying.value = 'static';
	try {
		await applyStatic({
			name: adapter.value.name,
			ip: p.ip!,
			mask: p.mask!,
			gateway: p.gateway ?? null,
			dns1: p.dns1 ?? null,
			dns2: p.dns2 ?? null,
		});
		ElMessage.success(`已套用方案：${p.name}`);
		scheduleWidgetRefresh();
	} catch (e) {
		await handleApplyError(e);
	} finally {
		applying.value = null;
	}
}

let cancelPointerGesture: (() => void) | undefined;

function trackPointer(pointerId: number, move: (event: PointerEvent) => void, finish: () => void, cancel?: () => void) {
	cancelPointerGesture?.();
	const onMove = (event: PointerEvent) => {
		if (event.pointerId === pointerId) move(event);
	};
	const onEnd = (event: PointerEvent) => {
		if (event.pointerId !== pointerId) return;
		cleanup();
		finish();
	};
	const cleanup = () => {
		window.removeEventListener('pointermove', onMove);
		window.removeEventListener('pointerup', onEnd);
		window.removeEventListener('pointercancel', onEnd);
		window.removeEventListener('blur', onCancel);
		cancelPointerGesture = undefined;
	};
	const onCancel = () => {
		cleanup();
		cancel?.();
	};
	cancelPointerGesture = onCancel;
	window.addEventListener('pointermove', onMove);
	window.addEventListener('pointerup', onEnd);
	window.addEventListener('pointercancel', onEnd);
	window.addEventListener('blur', onCancel);
}

/// 阈值拖拽：超过阈值后交给系统移动窗口。
function onDragStart(e: PointerEvent) {
	if (pinned.value) return;
	if (e.button !== 0) return;
	const el = e.target as HTMLElement;
	// 控件自身保留点击/拖动交互，不参与窗口拖动
	if (el.closest('button, .adapter-picker, .el-slider, .el-button')) return;
	trackPointer(e.pointerId, (event) => {
		if (Math.hypot(event.clientX - e.clientX, event.clientY - e.clientY) > 5) {
			cancelPointerGesture?.();
			void win.startDragging().catch(() => undefined);
		}
	}, () => undefined);
}

/// 右侧热区手动横向拖拽：仅改宽度、高度固定基线（系统原生缩放已关闭，不调用 startResizeDragging）
function onResizeStart(e: PointerEvent) {
	if (pinned.value || selectOpen.value || showNoProfile.value || e.button !== 0) return;
	e.preventDefault();
	e.stopPropagation();
	(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
	const startX = e.screenX;
	const startWidth = window.innerWidth;
	let width = startWidth;
	let frame = 0;
	let moved = false;
	const resize = () => {
		frame = 0;
		return win.setSize(new LogicalSize(width, BASE_HEIGHT));
	};
	trackPointer(e.pointerId, (event) => {
		width = Math.min(WIDTH_MAX, Math.max(WIDTH_MIN, startWidth + event.screenX - startX));
		moved = true;
		if (!frame) frame = requestAnimationFrame(() => void resize().catch(() => undefined));
	}, () => {
		cancelAnimationFrame(frame);
		if (!moved) return;
		void resize().then(() => saveWidgetSize(width, BASE_HEIGHT))
			.catch((error) => ElMessage.error(String(error)));
	}, () => cancelAnimationFrame(frame));
}

function closeNoProfile() {
	showNoProfile.value = false;
}

async function copyInfo() {
	const a = adapter.value;
	if (!a) return;
	const mode = a.dhcp_enabled ? 'DHCP' : '静态';
	const lines = [
		`网卡：${a.name}`,
		`获取方式：${mode}`,
		`IPv4 地址：${a.ipv4.join('、') || '-'}`,
		`子网掩码：${a.subnet_mask.join('、') || '-'}`,
		`默认网关：${a.gateway || '-'}`,
		`首选DNS：${a.dns[0] || '-'}`,
		`备用DNS：${a.dns[1] || '-'}`,
	];
	await navigator.clipboard.writeText(lines.join('\n'));
	ElMessage.success('已复制网络信息');
}

async function togglePin() {
	await setWidgetPinned(!pinned.value);
}

async function openMain() {
	await restoreMainWindow();
}
</script>

<template>
	<el-config-provider :locale="zhCn">
		<div
			class="widget-root dark"
			:class="{ pinned }"
			:style="styleVars"
			@pointerdown="onDragStart"
			@contextmenu="onContextMenu">
			<teleport to="body">
				<div
					v-if="devMenuVisible"
					class="dev-context-menu"
					:style="{
						left: devMenuPos.x + 'px',
						top: devMenuPos.y + 'px',
					}"
					@contextmenu.stop.prevent>
					<div class="dev-menu-item" @click="handleOpenDevTools">
						检查元素
					</div>
				</div>
			</teleport>
			<div class="widget-body">
				<div class="widget-toolbar">
					<div class="opacity-control">
						<el-tooltip
							content="调整背景透明度"
							placement="top"
							popper-class="app-tip widget-opacity-tip">
							<el-slider
								v-model="opacity"
								class="opacity-slider"
								tooltip-class="app-tip widget-opacity-tip"
								:min="0"
								:max="1"
								:step="0.05"
								placement="top"
								size="small"
								@change="onOpacityChange" />
						</el-tooltip>
					</div>
					<div class="info-actions">
						<el-tooltip
							:content="
								pinned
									? '已固定（在托盘菜单取消固定）'
									: '固定挂件（鼠标穿透）'
							"
							placement="top"
							popper-class="app-tip widget-control-tip">
							<button
								class="icon-btn"
								:class="{ 'is-active': pinned }"
								@click="togglePin">
								<el-icon
									><Lock v-if="pinned" /><Unlock v-else
								/></el-icon>
							</button>
						</el-tooltip>
						<el-tooltip
							content="复制网络信息"
							placement="top"
							popper-class="app-tip widget-control-tip">
							<button class="icon-btn" @click="copyInfo">
								<el-icon><DocumentCopy /></el-icon>
							</button>
						</el-tooltip>
						<el-tooltip
							content="打开主窗口"
							placement="top"
							popper-class="app-tip widget-control-tip">
							<button class="icon-btn" @click="openMain">
								⤢
							</button>
						</el-tooltip>
					</div>
				</div>

				<div v-if="adapter" class="status-summary">
					<span class="summary-label">状态</span>
					<el-tag
						v-if="adapter.dhcp_enabled"
						type="success"
						size="small"
						class="tag-dhcp"
						>DHCP</el-tag
					>
					<el-tag
						v-else
						type="warning"
						size="small"
						class="tag-static"
						>静态</el-tag
					>
					<div class="summary-metrics">
						<el-tag
							class="status-tag"
							:class="
								adapter.status === 'connected'
									? 'status-on'
									: 'status-off'
							"
							size="small">
							<el-icon
								><Connection
									v-if="
										adapter.status === 'connected'
									" /><Link v-else
							/></el-icon>
							<span>{{
								adapter.status === 'connected'
									? '已连接'
									: '未连接'
							}}</span>
						</el-tag>
						<div class="speed-row">
							<el-icon><Odometer /></el-icon>
							<span>{{ adapter.link_speed || '-' }}</span>
						</div>
					</div>
				</div>

				<div v-if="adapter" class="detail-grid" :class="{ 'without-traffic': !trafficMonitorEnabled }">
					<div class="info-list">
						<div class="info-row">
							<span class="info-label">IPv4</span
							><span>{{ adapter.ipv4.join('、') || '-' }}</span>
						</div>
						<div class="info-row">
							<span class="info-label">掩码</span
							><span>{{
								adapter.subnet_mask.join('、') || '-'
							}}</span>
						</div>
						<div class="info-row">
							<span class="info-label">网关</span
							><span>{{ adapter.gateway || '-' }}</span>
						</div>
						<div class="info-row">
							<span class="info-label">主DNS</span
							><span>{{ adapter.dns[0] || '-' }}</span>
						</div>
						<div class="info-row">
							<span class="info-label">备DNS</span
							><span>{{ adapter.dns[1] || '-' }}</span>
						</div>
					</div>
					<div v-if="trafficMonitorEnabled" class="traffic-panel">
						<div
							class="traffic-metric traffic-up"
							:title="`上行 ${formatTrafficRate(uploadRate)}`">
							<span class="traffic-label"
								><el-icon><Top /></el-icon>上行</span
							>
							<strong>{{ formatTrafficRate(uploadRate) }}</strong>
						</div>
						<div
							class="traffic-metric traffic-down"
							:title="`下行 ${formatTrafficRate(downloadRate)}`">
							<span class="traffic-label"
								><el-icon><Bottom /></el-icon>下行</span
							>
							<strong>{{
								formatTrafficRate(downloadRate)
							}}</strong>
						</div>
					</div>
				</div>
				<div class="info-row info-empty" v-if="!adapter">
					<span class="muted">未检测到可用网卡</span>
					<el-tooltip
						content="打开主窗口"
						placement="top"
						popper-class="app-tip">
						<button class="icon-btn" @click="openMain">⤢</button>
					</el-tooltip>
				</div>
				<div class="actions" role="group" aria-label="IP 获取方式">
					<el-button
						size="small"
						:class="{ 'is-current': adapter?.dhcp_enabled }"
						:loading="applying === 'dhcp'"
						:disabled="!adapter"
						@click="applyDhcpNow">
						DHCP
					</el-button>
					<el-button
						size="small"
						:class="{
							'is-current': adapter && !adapter.dhcp_enabled,
						}"
						:loading="applying === 'static'"
						:disabled="!adapter"
						@click="applyStaticNow">
						固定IP
					</el-button>
				</div>

				<div class="adapter-picker">
					<button
						type="button"
						class="adapter-trigger"
						:class="{ 'is-open': selectOpen }"
						:aria-expanded="selectOpen"
						aria-haspopup="listbox"
						:title="
							adapter
								? `${adapter.name}（${adapter.description}）`
								: '选择网卡'
						"
						@click="selectOpen = !selectOpen">
						<span class="adapter-current">{{
							adapter
								? `${adapter.name}（${adapter.description}）`
								: '选择网卡'
						}}</span>
						<el-icon
							class="adapter-arrow"
							:class="{ 'is-open': selectOpen }"
							><ArrowDown
						/></el-icon>
					</button>
					<div
						v-if="selectOpen"
						class="adapter-menu"
						role="listbox"
						aria-label="选择网卡">
						<button
							v-for="a in adapters"
							:key="a.name"
							type="button"
							class="adapter-option"
							:class="{ 'is-selected': a.name === currentName }"
							role="option"
							:aria-selected="a.name === currentName"
							:title="`${a.name}（${a.description}）`"
							@click="chooseAdapter(a.name)">
							<span class="adapter-option-copy"
								><strong>{{ a.name }}</strong
								><small>{{ a.description }}</small></span
							>
							<el-icon
								v-if="a.name === currentName"
								class="adapter-option-check"
								><Check
							/></el-icon>
						</button>
						<div v-if="!adapters.length" class="adapter-empty">
							未检测到可用网卡
						</div>
					</div>
				</div>
			</div>

			<!-- 边缘缩放热区：仅东侧横向拉宽，高度固定默认值 -->
			<div
				class="resize-edge resize-e"
				@pointerdown="onResizeStart($event)" />

			<!-- 暂无固定 IP 方案：组件内定位浮层，不进文档流、不撑大窗口 -->
			<div
				v-if="showNoProfile"
				class="np-mask"
				@click.self="closeNoProfile">
				<div class="np-card">
					<div class="np-body">
						<el-icon class="np-icon"><WarningFilled /></el-icon>
						<span>暂无固定IP方案，请在主窗口配置后保存</span>
					</div>
					<div class="np-actions">
						<el-button
							size="small"
							type="primary"
							@click="closeNoProfile"
							>确定</el-button
						>
					</div>
				</div>
			</div>
		</div>
	</el-config-provider>
</template>

<style scoped>
.widget-root {
	width: 100%;
	height: 100%;
	display: flex;
	flex-direction: column;
	/* 背景与边框透明度跟随全局背景 alpha，字体保持不透明 */
	background:
		linear-gradient(
			135deg,
			rgba(76, 141, 255, var(--select-bg-alpha, 0.06)),
			transparent 48%
		),
		rgba(18, 19, 23, var(--widget-bg-alpha, 0.92));
	border: 1px solid rgba(255, 255, 255, var(--root-border-alpha, 0.12));
	border-radius: 10px;
	overflow: hidden;
	font-size: 10px;
	font-family:
		'Segoe UI', 'Microsoft YaHei UI', 'Microsoft YaHei', sans-serif;
	font-synthesis: none;
	position: relative;
	color: #e6e8eb;
}

.widget-root :deep(.el-button),
.widget-root :deep(.el-tag),
.widget-root :deep(.el-slider) {
	font-family: inherit;
}

.widget-body {
	flex: 1;
	display: flex;
	flex-direction: column;
	gap: 6px;
	padding: 6px 5px;
	min-height: 0;
}

.info-empty {
	justify-content: space-between;
}

.widget-toolbar {
	display: flex;
	align-items: center;
	gap: 8px;
	min-width: 0;
}

.adapter-picker {
	position: relative;
	flex: 0 0 auto;
	width: 100%;
	min-width: 0;
}

.adapter-picker::before {
	content: '';
	position: absolute;
	left: 7px;
	right: 7px;
	top: -4px;
	height: 1px;
	background: rgba(255, 255, 255, 0.18);
	pointer-events: none;
}

.adapter-trigger {
	box-sizing: border-box;
	width: 100%;
	min-width: 0;
	height: 28px;
	display: flex;
	align-items: center;
	gap: 8px;
	padding: 0 9px;
	border: 1px solid transparent;
	border-radius: 8px;
	background: transparent;
	color: #e6e8eb;
	font: inherit;
	font-size: 10px;
	text-align: left;
	cursor: pointer;
	transition:
		border-color 0.16s ease,
		background 0.16s ease;
}
.adapter-trigger:hover,
.adapter-trigger.is-open {
	border-color: transparent;
	background: transparent;
	color: #8db6ff;
}
.adapter-trigger:hover .adapter-arrow,
.adapter-trigger.is-open .adapter-arrow {
	color: #8db6ff;
}
.adapter-current {
	flex: 1;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.adapter-arrow {
	flex: 0 0 auto;
	color: #9aa3b1;
	font-size: 10px;
	transition: transform 0.16s ease;
}
.adapter-arrow.is-open {
	transform: rotate(180deg);
}
.adapter-menu {
	position: absolute;
	z-index: 25;
	left: 0;
	right: 0;
	top: 34px;
	bottom: auto;
	max-height: 180px;
	padding: 4px;
	overflow-y: auto;
	overscroll-behavior: contain;
	border-radius: 10px;
	background: transparent;
	box-shadow: none;
}
.adapter-option {
	box-sizing: border-box;
	width: 100%;
	height: 38px;
	min-height: 38px;
	display: flex;
	align-items: center;
	gap: 8px;
	padding: 3px 7px;
	border: 0;
	border-radius: 7px;
	background: transparent;
	color: #dce1e9;
	font: inherit;
	text-align: left;
	cursor: pointer;
}
.adapter-option:hover {
	background: rgba(255, 255, 255, 0.07);
}
.adapter-option.is-selected {
	background: rgba(76, 141, 255, 0.16);
	color: #8db6ff;
}
.adapter-option-copy {
	flex: 1;
	min-width: 0;
	display: flex;
	flex-direction: column;
	gap: 1px;
}
.adapter-option-copy strong,
.adapter-option-copy small {
	display: block;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.adapter-option-copy strong {
	font-size: 10px;
	font-weight: 600;
}
.adapter-option-copy small {
	color: #929baa;
	font-size: 9px;
}
.adapter-option-check {
	flex: 0 0 auto;
	color: #4c8dff;
	font-size: 11px;
}
.adapter-empty {
	padding: 10px 8px;
	color: #929baa;
	font-size: 10px;
	text-align: center;
}

.icon-btn {
	flex-shrink: 0;
	margin-left: auto;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	border: none;
	background: transparent;
	color: #c3c8cf;
	cursor: pointer;
	font-size: 13px;
	line-height: 1;
	padding: 4px 5px;
	border-radius: 4px;
}

.icon-btn:hover {
	color: #4c8dff;
	background: rgba(76, 141, 255, 0.16);
}

.icon-btn.is-active {
	color: #4c8dff;
	background: rgba(76, 141, 255, 0.22);
}

.info-actions {
	flex: 0 0 auto;
	display: flex;
	align-items: center;
	gap: 4px;
	margin-left: auto;
	white-space: nowrap;
}

.info-actions .icon-btn {
	margin-left: 0;
}

.opacity-control {
	box-sizing: border-box;
	flex: 0 0 100px;
	width: 100px;
	min-width: 0;
	/* 最小值时滑块手柄中心与“状态”两字的中心落在同一竖线上 */
	margin-left: 15px;
	padding: 0;
}

.status-summary {
	box-sizing: border-box;
	position: relative;
	min-height: 28px;
	display: flex;
	align-items: center;
	gap: 6px;
	padding: 1px 7px;
	border: 1px solid transparent;
	border-radius: 8px;
	background: transparent;
	font-size: 10px;
	user-select: none;
	-webkit-user-select: none;
}

/* 即使背景调至全透，底部分隔线仍标识状态区与网络信息区的边界 */
.status-summary::after {
	content: '';
	position: absolute;
	left: 7px;
	right: 7px;
	bottom: 0;
	height: 1px;
	background: rgba(255, 255, 255, 0.18);
	pointer-events: none;
}

.summary-label {
	flex: 0 0 auto;
	color: #9199a6;
	font-size: 9px;
	letter-spacing: 0.01em;
}

.status-summary .el-tag {
	box-sizing: border-box;
	height: 19px;
	line-height: 17px;
	padding: 0 5px;
	font-size: 9px;
}

.summary-metrics {
	flex: 1;
	min-width: 0;
	display: flex;
	align-items: center;
	justify-content: flex-end;
	gap: 8px;
}

.info-list {
	box-sizing: border-box;
	min-width: 0;
	padding: 6px 7px 6px 8px;
	display: flex;
	flex-direction: column;
	justify-content: space-between;
	gap: 2px;
	font-size: 10px;
	cursor: default;
	user-select: none;
	-webkit-user-select: none;
}

.detail-grid {
	display: grid;
	grid-template-columns: minmax(0, 1fr) minmax(70px, 0.68fr);
	align-items: stretch;
	gap: 8px;
	min-height: 72px;
}

.detail-grid.without-traffic {
	grid-template-columns: minmax(0, 1fr);
}

.info-row {
	display: flex;
	align-items: center;
	gap: 5px;
	line-height: 1.35;
}

.info-label {
	width: 36px;
	flex-shrink: 0;
	color: #9199a6;
	font-size: 9px;
	letter-spacing: 0.01em;
}

.info-row > span:not(.info-label) {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	color: #e6e8eb;
	font-variant-numeric: tabular-nums;
}

.traffic-panel {
	box-sizing: border-box;
	min-width: 0;
	display: flex;
	flex-direction: column;
	justify-content: center;
	gap: 14px;
	padding: 6px 0 6px 10px;
	border-left: 1px solid rgba(255, 255, 255, 0.16);
}

.traffic-metric {
	min-width: 0;
	width: 100%;
	display: flex;
	flex-direction: column;
	align-items: center;
	gap: 2px;
	font-variant-numeric: tabular-nums;
	text-align: center;
}

.traffic-label {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	gap: 3px;
	color: #9aa3b1;
	font-size: 9px;
}

.traffic-label .el-icon {
	font-size: 10px;
}

.traffic-metric strong {
	width: 100%;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	font-size: 10px;
	font-weight: 500;
	text-align: center;
}

.traffic-up .traffic-label .el-icon,
.traffic-up strong {
	color: #78aaff;
}

.traffic-down .traffic-label .el-icon,
.traffic-down strong {
	color: #4adea0;
}

/* 状态信息与操作栏保持同一字体体系 */
.status-summary .status-tag {
	display: inline-flex;
	align-items: center;
	height: auto;
	line-height: 1.2;
	padding: 0;
	background: transparent !important;
	border: none !important;
	box-shadow: none !important;
}
/* el-tag 内层 content：图标与文字横向同行 */
.status-summary .status-tag :deep(.el-tag__content) {
	display: inline-flex;
	flex-direction: row;
	align-items: center;
	gap: 4px;
}
.status-summary .status-tag.status-on :deep(.el-tag__content) {
	color: #4ade80 !important;
}
.status-summary .status-tag.status-off :deep(.el-tag__content) {
	color: #f56c6c !important;
}
.status-summary .status-tag .el-icon {
	width: 14px;
	flex-shrink: 0;
	font-size: 11px;
}

/* 链路速率行：图标 + 文字同行，橙红色标识 */
.summary-metrics .speed-row {
	display: inline-flex;
	align-items: center;
	gap: 4px;
	font-size: 10px;
	color: #ffad66;
}
.summary-metrics .speed-row .el-icon {
	width: 14px;
	flex-shrink: 0;
	font-size: 11px;
	color: #ffad66;
}

/* DHCP 标签：与主窗一致的亮绿配色（:deep 强制覆盖 element 内部变量） */
.status-summary :deep(.tag-dhcp) {
	background: rgba(74, 222, 128, 0.18) !important;
	border-color: rgba(74, 222, 128, 0.45) !important;
	color: #4ade80 !important;
}

.status-summary :deep(.tag-static) {
	background: rgba(245, 108, 108, 0.16) !important;
	border-color: rgba(245, 108, 108, 0.4) !important;
	color: #f56c6c !important;
}

.actions {
	position: relative;
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	gap: 10px;
	margin-top: 0;
	padding: 0 4px;
}

.actions::before {
	content: '';
	position: absolute;
	left: 7px;
	right: 7px;
	top: -4px;
	height: 1px;
	background: rgba(255, 255, 255, 0.18);
	pointer-events: none;
}

.actions .el-button {
	position: relative;
	width: 100%;
	height: 29px;
	min-height: 29px;
	margin: 0;
	padding: 2px 8px 3px;
	border: 0;
	border-bottom: 0;
	background: transparent !important;
	box-shadow: none !important;
	color: #aeb6c2;
	font-size: 10px;
	font-weight: 600;
	border-radius: 0;
	transition: color 0.15s ease;
}

.actions .el-button.is-current {
	color: #78aaff;
}

.actions .el-button.is-current::after {
	content: '';
	position: absolute;
	left: 18%;
	right: 18%;
	bottom: 0;
	height: 2px;
	border-radius: 999px;
	background: linear-gradient(
		90deg,
		rgba(76, 141, 255, 0),
		#4c8dff 24%,
		#4c8dff 76%,
		rgba(76, 141, 255, 0)
	);
}

.actions .el-button:not(.is-current):hover:not(:disabled) {
	color: #fff;
}

.actions .el-button.is-current:hover:not(:disabled) {
	color: #a9c9ff;
}

.actions .el-button:disabled {
	color: #78808d;
	opacity: 0.65;
}

.opacity-slider {
	width: 100%;
	margin: 0;
}

/* 透明度范围以渐变呈现；手柄以低幅呼吸光提示可调 */
.opacity-slider :deep(.el-slider__runway) {
	height: 1px;
	border-radius: 999px;
	background: linear-gradient(
		90deg,
		rgba(255, 255, 255, 0.08),
		#d9e7ff
	) !important;
}

.opacity-slider :deep(.el-slider__bar) {
	height: 1px;
	background: transparent !important;
}

.opacity-slider :deep(.el-slider__button) {
	width: 8px;
	height: 8px;
  margin-bottom: 4px;
	border: 2px solid #8db6ff;
	background: #eaf2ff;
	position: relative;
}
.opacity-slider :deep(.el-slider__button::after) {
	content: '';
	position: absolute;
	inset: -5px;
	border-radius: 50%;
	background: rgba(76, 141, 255, 0.32);
	pointer-events: none;
	animation: opacity-handle-pulse 2.4s ease-in-out infinite;
}

@keyframes opacity-handle-pulse {
	0%,
	100% {
		opacity: 0.35;
		transform: scale(0.8);
	}
	50% {
		opacity: 0.8;
		transform: scale(1.4);
	}
}
@media (prefers-reduced-motion: reduce) {
	.opacity-slider :deep(.el-slider__button::after) { animation: none; }
}

/* 边缘缩放热区 */
.resize-edge {
	position: absolute;
	z-index: 5;
}
.resize-e {
	top: 12px;
	right: 0;
	bottom: 12px;
	width: 6px;
	cursor: ew-resize;
}

/* 固定（穿透）态：停用边缘缩放热区 */
.widget-root.pinned .resize-edge {
	display: none;
}
</style>

<!-- 全局 WebView 细节与提示框样式 -->
<style>
/* 隐藏 WebView2 / 浏览器默认的调整大小手柄，边缘拖拽缩放仍保留 */
::-webkit-resizer {
	display: none !important;
}
/* 挂件内 ElMessage 提示整体缩小 */
.el-message {
	padding: 6px 12px;
	font-size: 10px;
	border-radius: 6px;
}
.el-message .el-message__content {
	font-size: 10px;
}
.el-message .el-message__icon {
	font-size: 12px;
	margin-right: 5px;
}
/* 暂无方案提示：组件内绝对定位浮层，居中于挂件，不参与文档流 */
.np-mask {
	position: absolute;
	inset: 0;
	z-index: 30;
	display: flex;
	align-items: center;
	justify-content: center;
	background: rgba(0, 0, 0, 0.42);
}
.np-card {
	width: 160px;
	max-width: 84%;
	padding: 9px;
	border-radius: 7px;
	background: #26282f;
	box-shadow: none;
	user-select: none;
	-webkit-user-select: none;
}
.np-body {
	display: flex;
	align-items: flex-start;
	gap: 5px;
	font-size: 9.5px;
	line-height: 1.35;
	color: #c3c8cf;
}
.np-icon {
	flex-shrink: 0;
	font-size: 12px;
	color: #e6a23c;
	margin-top: 1px;
}
.np-actions {
	display: flex;
	justify-content: flex-end;
	padding-top: 6px;
}
.np-actions .el-button {
	min-height: 16px;
	padding: 1px 8px;
	font-size: 9px;
}
</style>
