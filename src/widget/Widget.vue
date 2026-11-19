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
} from '@element-plus/icons-vue';
import {
	getCurrentWindow,
	getAllWindows,
	LogicalSize,
	LogicalPosition,
} from '@tauri-apps/api/window';
import {
	applyDhcp,
	applyStatic,
	elevateAction,
	getConfig,
	listAdapters,
	onAdaptersChanged,
	emitAdaptersChanged,
	onConfigChanged,
	saveConfig,
	setWidgetOpacity,
	saveWidgetSize,
	setWidgetPinned,
	onWidgetPinChanged,
	openDevTools,
} from '@/api/ipc';
import type { AdapterInfo, AppConfig, IpProfile, PendingAction } from '@/types';
import { normalizeConfig } from '@/composables/useStore';
import type { UnlistenFn } from '@tauri-apps/api/event';

const adapters = ref<AdapterInfo[]>([]);
const currentName = ref<string | null>(null);
const appConfig = ref<AppConfig>({ default_adapter: null, profiles: [] });
const applying = ref<null | 'dhcp' | 'static'>(null);
/// 背景不透明度（0.0 ~ 1.0），仅作用于卡片背景，字体始终保持不透明
const opacity = ref<number>(0.92);
/// 顶部透明度面板是否展开（鼠标悬停上边界时显示）
const panelOpen = ref<boolean>(false);
/// 面板高度（逻辑像素），展开时据此临时增高窗口
const PANEL_HEIGHT = 34;
/// 挂件固定基线高度，纵向不可调整
const BASE_HEIGHT = 194;
/// 横向拉宽区间（逻辑像素）
const WIDTH_MIN = 240;
const WIDTH_MAX = 640;
/// 网卡下拉是否展开；展开时临时向下增高窗口以容纳向下弹出的列表
const selectOpen = ref<boolean>(false);
/// 固定（鼠标穿透）状态：固定后位置锁定、停用拖拽缩放与悬停交互
const pinned = ref<boolean>(false);
/// 「暂无固定 IP 方案」自绘提示框是否显示
const showNoProfile = ref<boolean>(false);

/// 由背景透明度派生各处 alpha，以数值直接写入 CSS 变量（rgba alpha 内 calc 乘法在 WebView 中会失效）
const styleVars = computed(() => ({
	'--widget-bg-alpha': opacity.value,
	'--root-border-alpha': 0.12 * opacity.value,
	'--select-bg-alpha': 0.06 * opacity.value,
	'--select-border-alpha': 0.14 * opacity.value,
	'--btn-bg-alpha': 0.06 * opacity.value,
	'--btn-border-alpha': Math.max(0.18, 0.16 * opacity.value),
	'--primary-bg-alpha': 0.9 * opacity.value,
	'--primary-border-alpha': Math.max(0.45, 0.9 * opacity.value),
	'--panel-border-alpha': 0.1 * opacity.value,
}));

/// 主体基础几何（不含面板/下拉），浮层展开时临时改几何、收起后恢复；两个浮层各自保存基线
let panelGeom: { x: number; y: number; h: number } | null = null;
let selectGeom: { x: number; y: number; h: number } | null = null;
/// 浮层开合引起的尺寸/位置变化不落盘
let suppressPersist = false;

const adapter = computed(
	() => adapters.value.find((a) => a.name === currentName.value) ?? null,
);

/// 当前网卡可套用的静态方案（挂件仅取第一个静态方案）
const staticProfile = computed<IpProfile | null>(() => {
	if (!adapter.value) return null;
	return (
		appConfig.value.profiles.find(
			(p) => p.mode === 'static' && p.ip && p.mask,
		) ?? null
	);
});

async function refresh() {
	adapters.value = await listAdapters();
	if (
		!currentName.value ||
		!adapters.value.some((a) => a.name === currentName.value)
	) {
		currentName.value =
			appConfig.value.widget_adapter ??
			appConfig.value.default_adapter ??
			adapters.value[0]?.name ??
			null;
	}
	// 注意：挂件自身刷新后不再广播 adapters-changed，避免主窗/挂件互相触发形成刷新环
}

let widgetTimers: number[] = [];

/// 切换配置后立即 + 延时两次刷新本地列表，最后一次刷新后广播给主窗，等待 DHCP 租约 / IP 配置收敛
function scheduleWidgetRefresh() {
	void refresh();
	widgetTimers.forEach((t) => window.clearTimeout(t));
	widgetTimers = [];
	const delays = [1500, 3500];
	delays.forEach((ms, i) => {
		const t = window.setTimeout(() => {
			const last = i === delays.length - 1;
			void refresh().then(() => {
				if (last) emitAdaptersChanged();
			});
		}, ms);
		widgetTimers.push(t);
	});
}

async function init() {
	appConfig.value = normalizeConfig(await getConfig());
	opacity.value = appConfig.value.widget_opacity ?? 0.92;
	await refresh();
}

/// 主窗保存方案 / 默认网卡后，重载本地配置并刷新网卡列表
async function reloadConfig() {
	appConfig.value = normalizeConfig(await getConfig());
	opacity.value = appConfig.value.widget_opacity ?? 0.92;
	await refresh();
	// 主窗显式设置默认网卡后，挂件跟随切换到该网卡（网卡存在才生效）
	const def = appConfig.value.default_adapter;
	if (def && adapters.value.some((a) => a.name === def)) {
		currentName.value = def;
	}
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
	if (!devMenuVisible.value) return;
	if ((e.target as HTMLElement).closest('.dev-context-menu')) return;
	closeDevMenu();
}

function onDocKeydown(e: KeyboardEvent) {
	if (e.key === 'Escape') closeDevMenu();
}

onMounted(init);

let unlisten: UnlistenFn | null = null;
let pinUnlisten: UnlistenFn | null = null;
let configUnlisten: UnlistenFn | null = null;

onMounted(async () => {
	unlisten = await onAdaptersChanged(() => void refresh());
	configUnlisten = await onConfigChanged(() => void reloadConfig());
	pinUnlisten = await onWidgetPinChanged((p) => {
		pinned.value = p;
	});
	document.addEventListener('pointerdown', onDocPointerDown);
	document.addEventListener('keydown', onDocKeydown);
	// 监听窗口尺寸变化，缩放停止后防抖把尺寸写回配置
	const win = getCurrentWindow();
	let sizeTimer: number | null = null;
	await win.onResized(() => {
		if (sizeTimer) window.clearTimeout(sizeTimer);
		sizeTimer = window.setTimeout(() => void persistSize(), 250);
	});
});

onUnmounted(() => {
	if (unlisten) void unlisten();
	if (pinUnlisten) void pinUnlisten();
	if (configUnlisten) void configUnlisten();
	document.removeEventListener('pointerdown', onDocPointerDown);
	document.removeEventListener('keydown', onDocKeydown);
	widgetTimers.forEach((t) => window.clearTimeout(t));
	widgetTimers = [];
});

watch(currentName, async (name) => {
	if (!name) return;
	appConfig.value.widget_adapter = name;
	// 挂件选中的网卡同时作为默认网卡，便于主窗口与快速切换联动
	appConfig.value.default_adapter = name;
	await saveConfig(appConfig.value);
});

async function persistSize() {
	// 浮层（面板/下拉）展开期间窗口处于临时扩窗态，不把临时尺寸落盘
	if (suppressPersist || panelGeom || selectGeom) return;
	const win = getCurrentWindow();
	const size = await win.innerSize();
	const scale = await win.scaleFactor();
	// 高度固定基线，仅宽度参与持久化
	await saveWidgetSize(size.width / scale, BASE_HEIGHT);
}

/// 下拉列表所需高度（逻辑像素）：网卡名+描述多为两行，每项预留约 44px 并限幅
function listHeight() {
	const rows = adapters.value.length || 1;
	return Math.min(240, Math.max(140, rows * 44 + 20));
}

/// 读取浮层内各选项实际布局高度求和，得到列表所需高度；offsetHeight 不受入场位移动画影响
function measureListHeight(): number {
	const popper = document.querySelector<HTMLElement>('.widget-select-popper');
	if (!popper) return listHeight();
	const items = [
		...popper.querySelectorAll<HTMLElement>('.el-select-dropdown__item'),
	];
	if (!items.length) return listHeight();
	let itemsH = 0;
	for (const it of items) {
		const cs = getComputedStyle(it);
		itemsH +=
			it.offsetHeight +
			parseFloat(cs.marginTop) +
			parseFloat(cs.marginBottom);
	}
	const popperPad = 8; // 浮层上下 padding 4px*2
	const listPad = 4; // 列表上下 padding 2px*2
	const gap = 8; // 下拉框到浮层的间隙
	return Math.min(240, Math.max(140, itemsH + popperPad + listPad + gap));
}

async function captureGeom() {
	const win = getCurrentWindow();
	const scale = await win.scaleFactor();
	const size = await win.innerSize();
	const pos = await win.outerPosition();
	return {
		x: pos.x / scale,
		y: pos.y / scale,
		h: size.height / scale,
	};
}

/// 面板在顶部向上探出：展开时增高并上移窗口，收起后恢复主体几何；该过程不落盘
watch(panelOpen, async (open) => {
	const win = getCurrentWindow();
	const scale = await win.scaleFactor();
	const size = await win.innerSize();
	const w = size.width / scale;
	if (open) {
		if (!panelGeom) panelGeom = await captureGeom();
		const g = panelGeom;
		suppressPersist = true;
		await win.setSize(new LogicalSize(w, g.h + PANEL_HEIGHT));
		await win.setPosition(
			new LogicalPosition(g.x, Math.max(0, g.y - PANEL_HEIGHT)),
		);
		window.setTimeout(() => {
			suppressPersist = false;
		}, 300);
	} else if (panelGeom) {
		const g = panelGeom;
		suppressPersist = true;
		await win.setSize(new LogicalSize(w, g.h));
		await win.setPosition(new LogicalPosition(g.x, g.y));
		panelGeom = null;
		window.setTimeout(() => {
			suppressPersist = false;
		}, 300);
	}
});

/// 下拉在底部向下弹出：展开时向下增高窗口，收起后恢复主体几何；该过程不落盘
watch(selectOpen, async (open) => {
	const win = getCurrentWindow();
	const scale = await win.scaleFactor();
	const size = await win.innerSize();
	const w = size.width / scale;
	if (open) {
		if (!selectGeom) selectGeom = await captureGeom();
		const g = selectGeom;
		suppressPersist = true;
		await nextTick();
		await new Promise((r) => requestAnimationFrame(() => r(null)));
		// 整体高度 = 基线组件高度 + 下拉列表实测高度
		const listH = measureListHeight();
		await win.setSize(new LogicalSize(w, g.h + listH));
		window.setTimeout(() => {
			if (selectOpen.value) suppressPersist = false;
		}, 300);
	} else if (selectGeom) {
		const g = selectGeom;
		suppressPersist = true;
		await win.setSize(new LogicalSize(w, g.h));
		selectGeom = null;
		window.setTimeout(() => {
			suppressPersist = false;
		}, 300);
	}
});

async function onOpacityChange(v: number) {
	opacity.value = v;
	appConfig.value.widget_opacity = v;
	await saveConfig(appConfig.value);
	// 持久化到后端配置（保持命令契约，便于重启后恢复背景透明度）
	await setWidgetOpacity(v);
	applyPopperAlpha();
}

/// 下拉弹层挂在 body 下，不继承挂件 CSS 变量；展开/透明度变化时同步其背景与边框透明度
function applyPopperAlpha() {
	document
		.querySelectorAll<HTMLElement>('.widget-select-popper')
		.forEach((el) => {
			el.style.setProperty('--popper-alpha', String(opacity.value));
			el.style.setProperty(
				'--popper-border-alpha',
				String(0.14 * opacity.value),
			);
		});
}

/// 下拉展开后弹层才挂载，需在可见性变化时补刷一次背景透明度
function onSelectVisible(visible: boolean) {
	selectOpen.value = visible;
	if (visible) requestAnimationFrame(applyPopperAlpha);
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
		suppressPersist = true;
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

/// 阈值拖拽：单击保留按钮/下拉等控件交互，按住移动超过阈值才拖动窗口，组件内任意位置均可拖动
function onDragStart(e: PointerEvent) {
	if (pinned.value) return;
	if (e.button !== 0) return;
	const el = e.target as HTMLElement;
	// 滑块轨道/手柄保留自身拖拽，不参与窗口拖动
	if (el.closest('.el-slider')) return;
	const start = { x: e.clientX, y: e.clientY, moved: false };
	const onMove = (ev: PointerEvent) => {
		if (!start || start.moved) return;
		if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) > 5) {
			start.moved = true;
			void getCurrentWindow().startDragging();
		}
	};
	const onUp = () => {
		window.removeEventListener('pointermove', onMove);
		window.removeEventListener('pointerup', onUp);
	};
	window.addEventListener('pointermove', onMove);
	window.addEventListener('pointerup', onUp);
}

/// 右侧热区手动横向拖拽：仅改宽度、高度固定基线（系统原生缩放已关闭，不调用 startResizeDragging）
function onResizeStart(e: PointerEvent) {
	if (pinned.value) return;
	e.preventDefault();
	e.stopPropagation();
	const win = getCurrentWindow();
	void (async () => {
		const scale = await win.scaleFactor();
		const size = await win.innerSize();
		const startX = e.clientX;
		const startW = size.width / scale;
		const onMove = (ev: PointerEvent) => {
			const dx = (ev.clientX - startX) / scale;
			const w = Math.min(WIDTH_MAX, Math.max(WIDTH_MIN, startW + dx));
			void win.setSize(new LogicalSize(w, BASE_HEIGHT));
		};
		const onUp = () => {
			window.removeEventListener('pointermove', onMove);
			window.removeEventListener('pointerup', onUp);
			void persistSize();
		};
		window.addEventListener('pointermove', onMove);
		window.addEventListener('pointerup', onUp);
	})();
}

/// 关闭无方案提示框，并恢复尺寸落盘
function closeNoProfile() {
	showNoProfile.value = false;
	suppressPersist = false;
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
	const wins = await getAllWindows();
	const main = wins.find((w) => w.label === 'main');
	if (main) {
		await main.show();
		await main.setFocus();
	}
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
				:style="{ left: devMenuPos.x + 'px', top: devMenuPos.y + 'px' }"
				@contextmenu.stop.prevent
			>
				<div class="dev-menu-item" @click="handleOpenDevTools">检查元素</div>
			</div>
		</teleport>
		<!-- 顶部边缘热区 + 透明度面板：仅鼠标移入顶部窄条才展开，主体区域悬停不触发 -->
		<div
			class="panel-wrap"
			@mouseenter="!pinned && (panelOpen = true)"
			@mouseleave="panelOpen = false">
			<div class="panel-hotzone" />
			<div class="opacity-panel" :class="{ open: panelOpen }">
				<span class="opacity-label">透明度</span>
				<el-slider
					v-model="opacity"
					class="opacity-slider"
					tooltip-class="app-tip"
					:min="0"
					:max="1"
					:step="0.05"
					size="small"
					@input="onOpacityChange" />
			</div>
		</div>

		<div class="widget-body">
			<div class="info" v-if="adapter">
				<div class="info-row">
					<span class="info-label">获取方式</span>
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
					<span class="info-actions">
						<el-tooltip
							:content="
								pinned
									? '已固定（在托盘菜单取消固定）'
									: '固定挂件（鼠标穿透）'
							"
							placement="top"
							popper-class="app-tip">
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
							popper-class="app-tip">
							<button class="icon-btn" @click="copyInfo">
								<el-icon><DocumentCopy /></el-icon>
							</button>
						</el-tooltip>
						<el-tooltip
							content="打开主窗口"
							placement="top"
							popper-class="app-tip">
							<button class="icon-btn" @click="openMain">
								⤢
							</button>
						</el-tooltip>
					</span>
				</div>
				<div class="info-row">
					<span class="info-label">IPv4 地址</span>
					<span class="info-value">{{
						adapter.ipv4.join('、') || '-'
					}}</span>
				</div>
				<div class="info-row">
					<span class="info-label">子网掩码</span>
					<span>{{ adapter.subnet_mask.join('、') || '-' }}</span>
				</div>
				<div class="info-row">
					<span class="info-label">默认网关</span>
					<span>{{ adapter.gateway || '-' }}</span>
				</div>
				<div class="info-row">
					<span class="info-label">首选DNS</span>
					<span>{{ adapter.dns[0] || '-' }}</span>
				</div>
				<div class="info-row">
					<span class="info-label">备用DNS</span>
					<span>{{ adapter.dns[1] || '-' }}</span>
				</div>
				<div class="status-col">
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
								v-if="adapter.status === 'connected'" /><Link
								v-else
						/></el-icon>
						<span>{{
							adapter.status === 'connected' ? '已连接' : '未连接'
						}}</span>
					</el-tag>
					<div class="speed-row">
						<el-icon><Odometer /></el-icon>
						<span>{{ adapter.link_speed || '-' }}</span>
					</div>
				</div>
			</div>
			<div class="info-row info-empty" v-else>
				<span class="muted">未检测到可用网卡</span>
				<el-tooltip
					content="打开主窗口"
					placement="top"
					popper-class="app-tip">
					<button class="icon-btn" @click="openMain">⤢</button>
				</el-tooltip>
			</div>

			<div class="actions">
				<el-button
					size="small"
					:loading="applying === 'dhcp'"
					:disabled="!adapter"
					@click="applyDhcpNow">
					DHCP
				</el-button>
				<el-button
					size="small"
					type="primary"
					:loading="applying === 'static'"
					:disabled="!adapter"
					@click="applyStaticNow">
					固定IP
				</el-button>
			</div>

			<el-select
				v-model="currentName"
				size="small"
				class="adapter-select"
				popper-class="widget-select-popper"
				placement="bottom-start"
				:fallback-placements="['bottom-start']"
				placeholder="选择网卡"
				@visible-change="onSelectVisible">
				<el-option
					v-for="a in adapters"
					:key="a.name"
					:label="`${a.name}（${a.description}）`"
					:value="a.name" />
			</el-select>
		</div>

		<!-- 边缘缩放热区：仅东侧横向拉宽，高度固定默认值 -->
		<div
			class="resize-edge resize-e"
			@pointerdown="onResizeStart($event)" />

		<!-- 暂无固定 IP 方案：组件内定位浮层，不进文档流、不撑大窗口 -->
		<div v-if="showNoProfile" class="np-mask" @click.self="closeNoProfile">
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
	background: rgba(18, 19, 23, var(--widget-bg-alpha, 0.92));
	border: 1px solid rgba(255, 255, 255, var(--root-border-alpha, 0.12));
	border-radius: var(--radius-sm);
	overflow: hidden;
	font-size: 12px;
	position: relative;
	color: #e6e8eb;
}

.widget-body {
	flex: 1;
	display: flex;
	flex-direction: column;
	gap: 3px;
	padding: 8px 8px 8px;
	min-height: 0;
}

.info-empty {
	justify-content: space-between;
}

.adapter-select {
	flex: 1;
	min-width: 0;
	margin-top: 2px;
}

/* 压缩挂件内 el-select 输入框，背景/边框跟随全局透明度 */
.adapter-select :deep(.el-select__wrapper) {
	min-height: 26px;
	padding: 0 8px;
	font-size: 10px;
	background: rgba(255, 255, 255, var(--select-bg-alpha, 0.06));
	box-shadow: 0 0 0 1px rgba(255, 255, 255, var(--select-border-alpha, 0.14))
		inset;
}

.adapter-select :deep(.el-select__selected-item),
.adapter-select :deep(.el-select__placeholder) {
	font-size: 10px;
	color: #e6e8eb;
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
	font-size: 14px;
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

.info {
	position: relative;
	display: flex;
	flex-direction: column;
	gap: 2px;
	font-size: 11px;
	cursor: default;
	user-select: none;
	-webkit-user-select: none;
}

.info-actions {
	margin-left: auto;
	display: flex;
	align-items: center;
	gap: 2px;
}

.info-actions .el-icon {
	font-size: 13px;
}

.info-actions .icon-btn {
	margin-left: 0;
}

.info-row {
	display: flex;
	align-items: center;
	gap: 8px;
	line-height: 1.35;
}

.info-label {
	width: 56px;
	flex-shrink: 0;
	color: #9aa0a8;
	font-size: 10px;
}

.info-value {
	font-size: 11px;
	font-weight: 400;
	color: #f2f4f7;
	text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
}

.info .info-row > span:not(.info-label):not(.info-value) {
	color: #e6e8eb;
	text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
}

.info .el-tag {
	height: 18px;
	line-height: 16px;
	padding: 0 6px;
	font-size: 11px;
}

/* 右侧状态列：链路状态在上、链路速率在下，整体垂直居中，两行左对齐 */
.info .status-col {
	position: absolute;
	right: 1%;
	top: 60%;
	transform: translateY(-50%);
	display: flex;
	flex-direction: column;
	align-items: flex-start;
	gap: 15px;
}

/* 链路状态标签：图标与文字同一行，无底色无边框 */
.info .status-tag {
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
.info .status-tag :deep(.el-tag__content) {
	display: inline-flex;
	flex-direction: row;
	align-items: center;
	gap: 4px;
}
.info .status-tag.status-on :deep(.el-tag__content) {
	color: #4ade80 !important;
}
.info .status-tag.status-off :deep(.el-tag__content) {
	color: #f56c6c !important;
}
.info .status-tag .el-icon {
	width: 14px;
	flex-shrink: 0;
	font-size: 12px;
}

/* 链路速率行：图标 + 文字同行，橙红色标识 */
.info .speed-row {
	display: inline-flex;
	align-items: center;
	gap: 4px;
	font-size: 10px;
	color: #f77234;
}
.info .speed-row .el-icon {
	width: 14px;
	flex-shrink: 0;
	font-size: 12px;
	color: #f77234;
}

/* DHCP 标签：与主窗一致的亮绿配色（:deep 强制覆盖 element 内部变量） */
.info :deep(.tag-dhcp) {
	background: rgba(74, 222, 128, 0.18) !important;
	border-color: rgba(74, 222, 128, 0.45) !important;
	color: #4ade80 !important;
}

.info :deep(.tag-static) {
	background: rgba(245, 108, 108, 0.16) !important;
	border-color: rgba(245, 108, 108, 0.4) !important;
	color: #f56c6c !important;
}

.actions {
	display: flex;
	gap: 8px;
	margin-top: 2px;
	justify-content: space-between;
}

.actions .el-button {
	flex: 0 0 auto;
	width: 72px;
	min-height: 24px;
	padding: 3px 5px;
	font-size: 10.5px;
	transition:
		background 0.15s ease,
		border-color 0.15s ease,
		color 0.15s ease;
}

/* 按钮背景跟随全局透明度；边框保留最低可见度，全透明时仍可辨识 */
.actions .el-button:not(.el-button--primary) {
	background: rgba(255, 255, 255, var(--btn-bg-alpha, 0.06));
	border-color: rgba(255, 255, 255, var(--btn-border-alpha, 0.18));
	color: #e6e8eb;
}

.actions .el-button:not(.el-button--primary):hover:not(:disabled) {
	background: rgba(255, 255, 255, 0.14);
	border-color: rgba(255, 255, 255, 0.32);
	color: #ffffff;
}

.actions .el-button--primary {
	background: rgba(76, 141, 255, var(--primary-bg-alpha, 0.9));
	border-color: rgba(76, 141, 255, var(--primary-border-alpha, 0.45));
}

.actions .el-button--primary:hover:not(:disabled) {
	background: rgba(76, 141, 255, 1);
	border-color: rgba(76, 141, 255, 0.7);
}

/* 顶部边缘热区：仅这条窄条悬停才展开面板，主体区域不触发 */
.panel-wrap {
	width: 100%;
	flex-shrink: 0;
}

.panel-hotzone {
	width: 100%;
	height: 8px;
}

/* 顶部透明度面板：文档流内占顶部一行，展开时随窗口上移探出；增高部分被面板占用，底部不堆空白 */
.opacity-panel {
	z-index: 50;
	display: flex;
	align-items: center;
	gap: 8px;
	height: 0;
	padding: 0 8px;
	background: rgba(18, 19, 23, var(--widget-bg-alpha, 0.92));
	border-bottom: 1px solid rgba(255, 255, 255, var(--panel-border-alpha, 0.1));
	opacity: 0;
	transition:
		height 0.18s ease,
		opacity 0.18s ease,
		padding 0.18s ease;
	overflow: hidden;
	flex-shrink: 0;
}

.opacity-panel.open {
	height: 34px;
	padding: 6px 8px;
	opacity: 1;
}

.opacity-label {
	flex-shrink: 0;
	width: 44px;
	color: #9aa0a8;
	font-size: 12px;
}

.opacity-slider {
	flex: 1;
	margin: 0;
}

/* 调细滑块轨道与手柄 */
.opacity-slider :deep(.el-slider__runway) {
	height: 3px;
}

.opacity-slider :deep(.el-slider__bar) {
	height: 3px;
}

.opacity-slider :deep(.el-slider__button) {
	width: 10px;
	height: 10px;
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

<!-- 下拉弹层挂在 body 下，背景透明度由 JS 写入 --popper-alpha，选项紧凑且长名称换行完整 -->
<style>
/* 隐藏 WebView2 / 浏览器默认的调整大小手柄，边缘拖拽缩放仍保留 */
::-webkit-resizer {
	display: none !important;
}
/* 挂件内 ElMessage 提示整体缩小 */
.el-message {
	padding: 6px 12px;
	font-size: 11px;
	border-radius: 6px;
}
.el-message .el-message__content {
	font-size: 11px;
}
.el-message .el-message__icon {
	font-size: 13px;
	margin-right: 5px;
}
.widget-select-popper {
	width: 224px;
	min-width: 0;
	padding: 4px 4px;
	background: rgba(18, 19, 23, var(--popper-alpha, 0.92)) !important;
	/* 去掉浮层外框、投影与圆角，视觉上作为组件内部列表展开 */
	border: none !important;
	box-shadow: none !important;
	border-radius: 0 !important;
}
/* 选项超出最大高度时在列表内部滚动 */
.widget-select-popper .el-scrollbar {
	max-height: 240px;
}
/* 隐藏浮层小三角，避免露出独立弹层痕迹 */
.widget-select-popper .el-popper__arrow,
.widget-select-popper .el-select-dropdown__arrow {
	display: none !important;
}
.widget-select-popper .el-scrollbar__wrap {
	overflow-y: auto;
}
/* 左右留白由列表容器提供，避免选项外边距被滚动容器裁掉、右侧圆角丢失 */
.widget-select-popper .el-select-dropdown__list {
	padding: 2px 4px;
}
.widget-select-popper .el-select-dropdown__item {
	height: auto;
	min-height: 18px;
	width: auto;
	line-height: 1.2;
	font-size: 9px;
	padding: 4px; /* 内边距 */
	margin: 5px 5px 3px 0; /* 外边距：上 右 下 左（增大右外边距以露出右侧圆角，调小左外边距） */
	border-radius: 5px;
	white-space: normal;
	word-break: break-word;
	overflow: visible;
	color: #e6e8eb;
}
.widget-select-popper .el-select-dropdown__item > span {
	white-space: normal;
	word-break: break-word;
}
.widget-select-popper .el-select-dropdown__item.is-hovering,
.widget-select-popper .el-select-dropdown__item:hover {
	background: rgba(76, 141, 255, 0.16);
}
.widget-select-popper .el-select-dropdown__item.is-selected {
	color: #4c8dff;
	background: rgba(76, 141, 255, 0.22);
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
	box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4);
	user-select: none;
	-webkit-user-select: none;
}
.np-body {
	display: flex;
	align-items: flex-start;
	gap: 5px;
	font-size: 10.5px;
	line-height: 1.35;
	color: #c3c8cf;
}
.np-icon {
	flex-shrink: 0;
	font-size: 13px;
	color: #e6a23c;
	margin-top: 1px;
}
.np-actions {
	display: flex;
	justify-content: flex-end;
	padding-top: 6px;
}
.np-actions .el-button {
	min-height: 18px;
	padding: 1px 8px;
	font-size: 10px;
}
</style>
