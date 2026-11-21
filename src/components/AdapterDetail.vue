<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import {
	applyDhcp,
	applyDns,
	applyStatic,
	elevateAction,
	saveConfig,
	emitConfigChanged,
} from '@/api/ipc';
import {
	appConfig,
	currentAdapter,
	refreshAdapters,
	scheduleDelayedRefresh,
} from '@/composables/useStore';
import type { IpProfile, PendingAction, StaticInput } from '@/types';
import { DocumentCopy } from '@element-plus/icons-vue';

const mode = ref<'dhcp' | 'static'>('dhcp');
const form = reactive({ ip: '', mask: '', gateway: '', dns1: '', dns2: '' });
const dnsOnly = reactive({ dns1: '', dns2: '' });
const applying = ref(false);
const profileId = ref('');
const editVisible = ref(false);
const editProfileId = ref('');
const editForm = reactive({
	name: '',
	mode: 'dhcp' as 'dhcp' | 'static',
	ip: '',
	mask: '',
	gateway: '',
	dns1: '',
	dns2: '',
});

const adapter = computed(() => currentAdapter());

function fillFromAdapter() {
	const a = adapter.value;
	if (!a) return;
	mode.value = a.dhcp_enabled ? 'dhcp' : 'static';
	form.ip = a.ipv4[0] ?? '';
	form.mask = a.subnet_mask[0] ?? '';
	form.gateway = a.gateway ?? '';
	form.dns1 = a.dns[0] ?? '';
	form.dns2 = a.dns[1] ?? '';
	dnsOnly.dns1 = a.dns[0] ?? '';
	dnsOnly.dns2 = a.dns[1] ?? '';
}

watch(adapter, fillFromAdapter, { immediate: true });

const targets = computed(() => (adapter.value ? [adapter.value.name] : []));

async function copyInfo() {
	const a = adapter.value;
	if (!a) {
		ElMessage.warning('请先选择网卡');
		return;
	}
	const mode = a.dhcp_enabled ? 'DHCP 自动获取' : '固定 IP';
	const status = a.status === 'connected' ? '已连接' : '未连接';
	const lines = [
		`网卡：${a.name}`,
		`设备：${a.description || '-'}`,
		`获取方式：${mode}`,
		`链路状态：${status}`,
		`IPv4 地址：${a.ipv4.join('、') || '-'}`,
		`子网掩码：${a.subnet_mask.join('、') || '-'}`,
		`默认网关：${a.gateway || '-'}`,
		`链路速率：${a.link_speed || '-'}`,
		`首选 DNS：${a.dns[0] || '-'}`,
		`备用 DNS：${a.dns[1] || '-'}`,
		`MAC 地址：${a.mac || '-'}`,
	];
	try {
		await navigator.clipboard.writeText(lines.join('\n'));
		ElMessage.success('已复制网卡及网络信息');
	} catch {
		ElMessage.error('复制失败，请重试');
	}
}

const ipRe = /^(\d{1,3}\.){3}\d{1,3}$/;

function validateStatic(): string | null {
	if (!ipRe.test(form.ip)) return '请填写合法的 IP 地址';
	if (!form.mask.trim()) return '请填写子网掩码';
	return null;
}

async function handleApplyError(e: unknown) {
	const msg = typeof e === 'string' ? e : String(e);
	if (msg === 'ELEVATION_REQUIRED') {
		const action: PendingAction =
			mode.value === 'dhcp'
				? { kind: 'Dhcp', names: targets.value, reset_dns: true }
				: {
						kind: 'Static',
						names: targets.value,
						ip: form.ip,
						mask: form.mask,
						gateway: form.gateway || null,
						dns1: form.dns1 || null,
						dns2: form.dns2 || null,
					};
		await elevateAction(action);
		ElMessage.info('已提交提权执行，请查看弹出的提示框');
		scheduleDelayedRefresh();
	} else {
		ElMessage.error(msg || '操作失败');
	}
}

async function handleApply() {
	if (targets.value.length === 0) {
		ElMessage.warning('请先选择网卡');
		return;
	}
	if (mode.value === 'static') {
		const err = validateStatic();
		if (err) {
			ElMessage.warning(err);
			return;
		}
	}
	applying.value = true;
	try {
		if (mode.value === 'dhcp') {
			for (const name of targets.value) await applyDhcp(name, true);
		} else {
			for (const name of targets.value) {
				const input: StaticInput = {
					name,
					ip: form.ip,
					mask: form.mask,
					gateway: form.gateway || null,
					dns1: form.dns1 || null,
					dns2: form.dns2 || null,
				};
				await applyStatic(input);
			}
		}
		ElMessage.success('配置已应用');
		await refreshAdapters();
		scheduleDelayedRefresh();
	} catch (e) {
		await handleApplyError(e);
	} finally {
		applying.value = false;
	}
}

async function applyDnsOnly() {
	if (!dnsOnly.dns1.trim()) {
		ElMessage.warning('请填写首选 DNS 服务器');
		return;
	}
	if (targets.value.length === 0) {
		ElMessage.warning('请先选择网卡');
		return;
	}
	try {
		for (const name of targets.value) {
			await applyDns(name, dnsOnly.dns1.trim(), dnsOnly.dns2 || null);
		}
		ElMessage.success('DNS 服务器已更新');
		await refreshAdapters();
		scheduleDelayedRefresh();
	} catch (e) {
		await handleApplyError(e);
	}
}

function applyProfileToForm(id: string) {
	const p = appConfig.value.profiles.find((x) => x.id === id);
	if (!p) return;
	mode.value = p.mode;
	form.ip = p.ip ?? '';
	form.mask = p.mask ?? '';
	form.gateway = p.gateway ?? '';
	form.dns1 = p.dns1 ?? '';
	form.dns2 = p.dns2 ?? '';
}

function selectProfile(id: string) {
	profileId.value = id;
	applyProfileToForm(id);
}

function openEdit(p: IpProfile) {
	editProfileId.value = p.id;
	editForm.name = p.name;
	editForm.mode = p.mode;
	editForm.ip = p.ip ?? '';
	editForm.mask = p.mask ?? '';
	editForm.gateway = p.gateway ?? '';
	editForm.dns1 = p.dns1 ?? '';
	editForm.dns2 = p.dns2 ?? '';
	editVisible.value = true;
}

async function confirmEdit() {
	if (!editForm.name.trim()) {
		ElMessage.warning('请填写方案名称');
		return;
	}
	if (editForm.mode === 'static') {
		if (!ipRe.test(editForm.ip)) {
			ElMessage.warning('请填写合法的 IP 地址');
			return;
		}
		if (!editForm.mask.trim()) {
			ElMessage.warning('请填写子网掩码');
			return;
		}
	}
	const existing = appConfig.value.profiles.find(
		(x) => x.id === editProfileId.value,
	);
	if (!existing) {
		ElMessage.warning('方案不存在或已被删除');
		editVisible.value = false;
		return;
	}
	existing.name = editForm.name.trim();
	existing.mode = editForm.mode;
	existing.ip = editForm.mode === 'static' ? editForm.ip || null : null;
	existing.mask = editForm.mode === 'static' ? editForm.mask || null : null;
	existing.gateway =
		editForm.mode === 'static' ? editForm.gateway || null : null;
	existing.dns1 = editForm.dns1 || null;
	existing.dns2 = editForm.dns2 || null;
	await saveConfig(appConfig.value);
	emitConfigChanged();
	ElMessage.success('方案已更新');
	editVisible.value = false;
}

async function quickApply(p: IpProfile) {
	profileId.value = p.id;
	applyProfileToForm(p.id);
	await handleApply();
}

function removeProfile(id: string) {
	appConfig.value.profiles = appConfig.value.profiles.filter(
		(x) => x.id !== id,
	);
	if (profileId.value === id) profileId.value = '';
	saveConfig(appConfig.value);
	emitConfigChanged();
	ElMessage.success('方案已删除');
}

async function saveProfile() {
	const defaultName =
		mode.value === 'dhcp' ? 'DHCP' : `固定IP ${form.ip || ''}`.trim();
	let name: string | undefined;
	try {
		const ret = await ElMessageBox.prompt(
			'请输入方案名称',
			'保存 IP 方案',
			{
				inputPlaceholder: defaultName,
				inputValue: defaultName,
			},
		);
		name = ret.value?.trim() || defaultName;
	} catch {
		return;
	}
	if (!name) return;
	const profile: IpProfile = {
		id: crypto.randomUUID(),
		name,
		mode: mode.value,
		ip: form.ip || null,
		mask: form.mask || null,
		gateway: form.gateway || null,
		dns1: form.dns1 || null,
		dns2: form.dns2 || null,
	};
	profileId.value = profile.id;
	appConfig.value.profiles.push(profile);
	await saveConfig(appConfig.value);
	emitConfigChanged();
	ElMessage.success('方案已保存');
}
</script>

<template>
	<div class="detail-wrap">
		<div class="panel current-card">
			<div class="current-head">
				<div class="current-info">
					<div class="current-title">
						<span
							class="current-status-dot"
							:class="adapter?.status === 'connected' ? 'is-connected' : 'is-disconnected'"></span>
						{{ adapter?.name || '未选择网卡' }}
					</div>
					<div class="current-desc muted">
						{{ adapter?.description || '请在左侧选择一张网卡' }}
					</div>
				</div>
				<el-tooltip
					content="复制网卡及网络信息"
					placement="top"
					popper-class="app-tip">
					<button
						class="copy-btn"
						:disabled="!adapter"
						@click="copyInfo">
						<el-icon><DocumentCopy /></el-icon>
					</button>
				</el-tooltip>
			</div>
			<el-descriptions :column="2" border size="small" class="desc">
				<el-descriptions-item label="获取方式">
					<el-tag
						v-if="adapter?.dhcp_enabled"
						type="success"
						size="small"
						>DHCP 自动获取</el-tag
					>
					<el-tag v-else type="warning" size="small">固定 IP</el-tag>
				</el-descriptions-item>
				<el-descriptions-item label="链路状态">
					<el-tag
						v-if="adapter?.status === 'connected'"
						type="success"
						size="small"
						>已连接</el-tag
					>
					<el-tag v-else type="danger" size="small">未连接</el-tag>
				</el-descriptions-item>
				<el-descriptions-item label="IPv4 地址">
					{{ adapter?.ipv4.join('、') || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="子网掩码">
					{{ adapter?.subnet_mask.join('、') || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="默认网关">
					{{ adapter?.gateway || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="链路速率">
					{{ adapter?.link_speed || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="首选 DNS">
					{{ adapter?.dns[0] || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="备用 DNS">
					{{ adapter?.dns[1] || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="MAC 地址" :span="2">
					<span>{{ adapter?.mac || '-' }}</span>
				</el-descriptions-item>
			</el-descriptions>
		</div>

		<div class="target-grid">
			<div class="panel form-col">
				<div class="card-title">目标配置</div>
				<el-radio-group v-model="mode" class="mode-radio">
					<el-radio value="dhcp">DHCP 自动获取</el-radio>
					<el-radio value="static">固定 IP</el-radio>
				</el-radio-group>

				<el-form
					v-if="mode === 'static'"
					label-position="top"
					class="target-form">
					<div class="form-grid">
						<el-form-item label="IP 地址">
							<el-input
								v-model="form.ip"
								placeholder="如 192.168.1.100" />
						</el-form-item>
						<el-form-item label="子网掩码">
							<el-input
								v-model="form.mask"
								placeholder="如 255.255.255.0" />
						</el-form-item>
						<el-form-item label="默认网关（可选）">
							<el-input
								v-model="form.gateway"
								placeholder="如 192.168.1.1" />
						</el-form-item>
						<el-form-item label="首选 DNS（可选）">
							<el-input
								v-model="form.dns1"
								placeholder="如 223.5.5.5" />
						</el-form-item>
						<el-form-item label="备用 DNS（可选）">
							<el-input
								v-model="form.dns2"
								placeholder="如 119.29.29.29" />
						</el-form-item>
					</div>
				</el-form>

				<div class="dns-section">
					<span class="dns-section-title">仅修改 DNS</span>
				</div>
				<div class="dns-row">
					<el-input
						v-model="dnsOnly.dns1"
						placeholder="首选 DNS"
						class="dns-input" />
					<el-input
						v-model="dnsOnly.dns2"
						placeholder="备用 DNS（可选）"
						class="dns-input" />
					<el-button @click="applyDnsOnly">应用 DNS</el-button>
				</div>

				<div class="actions">
					<el-button
						type="primary"
						:loading="applying"
						@click="handleApply">
						应用到当前网卡
					</el-button>
					<span class="muted tip">作用于当前选中的网卡</span>
				</div>
			</div>

			<div class="panel profile-col">
				<div class="profile-header">
					<div class="card-title">保存的方案</div>
					<el-button size="small" @click="saveProfile"
						>保存当前</el-button
					>
				</div>

				<div v-if="appConfig.profiles.length" class="profile-list">
					<div
						v-for="p in appConfig.profiles"
						:key="p.id"
						class="profile-item"
						:class="{ active: profileId === p.id }"
						@click="selectProfile(p.id)">
						<div class="profile-meta">
							<div class="profile-name">{{ p.name }}</div>
							<template v-if="p.mode === 'dhcp'">
								<div class="profile-sub muted">
									DHCP 自动获取
								</div>
							</template>
							<template v-else>
								<div class="profile-sub muted">
									{{ p.ip ?? '-' }}/{{ p.mask ?? '-' }}
								</div>
								<div v-if="p.gateway" class="profile-sub muted">
									网关 {{ p.gateway }}
								</div>
							</template>
						</div>
						<div class="profile-ops">
							<el-button
								link
								size="small"
								type="success"
								@click.stop="openEdit(p)"
								>编辑</el-button
							>
							<el-button
								link
								size="small"
								type="primary"
								@click.stop="quickApply(p)"
								>套用</el-button
							>
							<el-button
								link
								size="small"
								type="danger"
								@click.stop="removeProfile(p.id)"
								>删除</el-button
							>
						</div>
					</div>
				</div>
				<div v-else class="profile-empty muted">
					暂无保存的方案，配置好参数后点击「保存当前」
				</div>
			</div>
		</div>

		<el-dialog
			v-model="editVisible"
			title="编辑方案"
			width="460px"
			destroy-on-close>
			<el-form label-position="top">
				<el-form-item label="方案名称">
					<el-input
						v-model="editForm.name"
						placeholder="请输入方案名称" />
				</el-form-item>
				<el-form-item label="配置类型">
					<el-radio-group v-model="editForm.mode">
						<el-radio value="dhcp">DHCP 自动获取</el-radio>
						<el-radio value="static">固定 IP</el-radio>
					</el-radio-group>
				</el-form-item>
				<template v-if="editForm.mode === 'static'">
					<el-form-item label="IP 地址">
						<el-input
							v-model="editForm.ip"
							placeholder="如 192.168.1.100" />
					</el-form-item>
					<el-form-item label="子网掩码">
						<el-input
							v-model="editForm.mask"
							placeholder="如 255.255.255.0" />
					</el-form-item>
					<el-form-item label="默认网关（可选）">
						<el-input
							v-model="editForm.gateway"
							placeholder="如 192.168.1.1" />
					</el-form-item>
				</template>
				<el-form-item label="首选 DNS（可选）">
					<el-input
						v-model="editForm.dns1"
						placeholder="如 223.5.5.5" />
				</el-form-item>
				<el-form-item label="备用 DNS（可选）">
					<el-input
						v-model="editForm.dns2"
						placeholder="如 119.29.29.29" />
				</el-form-item>
			</el-form>
			<template #footer>
				<el-button @click="editVisible = false">取消</el-button>
				<el-button type="primary" @click="confirmEdit"
					>保存修改</el-button
				>
			</template>
		</el-dialog>
	</div>
</template>

<style scoped>
.detail-wrap {
	display: flex;
	flex-direction: column;
	gap: 10px;
	height: 100%;
	min-height: 0;
	overflow: hidden;
}
.current-card,
.form-col,
.profile-col {
	padding: 12px;
}
.current-card {
	overflow: hidden;
	background: linear-gradient(145deg, var(--panel-glass), var(--panel-bg));
}
.card-title {
	font-size: 15px;
	font-weight: 600;
	margin-bottom: 8px;
	letter-spacing: 0.015em;
}
.current-head {
	display: flex;
	align-items: flex-start;
	justify-content: space-between;
	gap: 10px;
	margin-bottom: 8px;
}
.current-info {
	flex: 1;
	min-width: 0;
}
.current-title {
	display: flex;
	align-items: center;
	gap: 8px;
	font-size: 16px;
	font-weight: 700;
}
.current-status-dot {
	width: 8px;
	height: 8px;
	flex: none;
	border-radius: 50%;
}
.current-status-dot.is-connected {
	background: var(--success);
	box-shadow: 0 0 8px rgba(74, 222, 128, 0.35);
}
.current-status-dot.is-disconnected {
	background: var(--text-secondary);
}
.current-desc {
	font-size: 12px;
	margin-top: 2px;
}
.copy-btn {
	flex: none;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	width: 28px;
	height: 28px;
	padding: 0;
	border: 1px solid var(--panel-border-glass);
	background: transparent;
	border-radius: 8px;
	color: var(--text-secondary);
	cursor: pointer;
	transition:
		color 0.15s,
		background 0.15s,
		border-color 0.15s;
}
.copy-btn:hover:not(:disabled) {
	color: var(--accent);
	background: var(--accent-soft);
}
.copy-btn:disabled {
	opacity: 0.4;
	cursor: not-allowed;
}
.desc :deep(.el-descriptions__label) {
	width: 110px;
	background: var(--app-bg);
	color: var(--text-secondary);
	font-size: 12px;
	font-weight: 600;
}
.desc :deep(.el-descriptions__table) {
	border-color: var(--panel-border);
	border-radius: 8px;
	overflow: hidden;
}
.desc :deep(.el-descriptions__cell) {
	border-color: var(--panel-border);
	padding: 8px 12px;
	font-size: 13px;
}
.desc :deep(.el-descriptions__content) {
	color: var(--text-primary);
	font-variant-numeric: tabular-nums;
}

.target-grid {
	display: grid;
	grid-template-columns: minmax(0, 1fr) 350px;
	gap: 10px;
	flex: 1;
	min-height: 0;
}
.form-col {
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
	height: 100%;
	overflow-y: auto;
}
.form-col > .card-title {
	padding-bottom: 8px;
	border-bottom: 1px solid var(--panel-border);
}
.mode-radio {
	margin-bottom: 8px;
}
.form-grid {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	gap: 0 12px;
}
.target-form :deep(.el-form-item) {
	margin-bottom: 8px;
}
.target-form :deep(.el-form-item__label) {
	padding-bottom: 2px;
	font-size: 12px;
}
.dns-section {
	display: flex;
	align-items: center;
	gap: 12px;
	margin: 8px 0;
}
.dns-section-title {
	flex: none;
	font-size: 12px;
	font-weight: 500;
	color: var(--text-secondary);
}
.dns-section::after {
	content: '';
	flex: 1;
	height: 1px;
	background: var(--panel-border);
}
.dns-row {
	display: flex;
	gap: 8px;
	align-items: center;
	padding: 4px 0;
}
.dns-input {
	flex: 1;
}
.dns-input :deep(.el-input__wrapper) {
	min-height: 36px;
}
.actions {
	display: flex;
	align-items: center;
	gap: 12px;
	margin-top: 8px;
}
.tip {
	font-size: 12px;
}

.profile-col {
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
	height: 100%;
}
.profile-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	margin-bottom: 10px;
	padding-bottom: 8px;
	border-bottom: 1px solid var(--panel-border);
}
.profile-header .card-title {
	margin-bottom: 0;
}
.profile-list {
	display: flex;
	flex-direction: column;
	gap: 8px;
	flex: 1;
	min-height: 0;
	overflow-y: auto;
}
.profile-item {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 6px;
	border: 1px solid var(--panel-border);
	border-radius: var(--radius-sm);
	padding: 8px;
	cursor: pointer;
	transition:
		border-color 0.15s,
		background 0.15s;
}
.profile-item:hover {
	border-color: var(--accent);
	background: var(--accent-soft);
}
.profile-item.active {
	border-color: var(--accent);
	background: var(--accent-soft);
}
.profile-meta {
	flex: 1;
	min-width: 0;
}
.profile-name {
	font-weight: 600;
	font-size: 13px;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.profile-sub {
	font-size: 12px;
	margin-top: 2px;
	line-height: 1.35;
	overflow: hidden;
	word-break: break-all;
}
.profile-ops {
	flex-shrink: 0;
	display: flex;
	align-items: center;
	gap: 0;
}
.profile-ops :deep(.el-button) {
	padding: 3px 3px;
	font-size: 12px;
	margin-left: 0;
}
.profile-ops :deep(.el-button--success) {
	color: var(--success);
}
.profile-ops :deep(.el-button--success:hover) {
	color: var(--success);
	opacity: 0.8;
}
.profile-empty {
	font-size: 12px;
	padding: 16px 8px;
	text-align: center;
}

@media (max-width: 1000px) {
	.target-grid {
		grid-template-columns: 1fr;
	}
}
</style>
