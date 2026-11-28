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
const creatingProfile = ref(false);
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

/** 顶部信息表当前展示的协议族（IPv6 仅只读展示） */
const ipFamily = ref<'ipv4' | 'ipv6'>('ipv4');

const ipv6List = computed(() => adapter.value?.ipv6 ?? []);
const ipv6Prefix = computed(() => adapter.value?.ipv6_prefix ?? []);
const hasIpv6 = computed(() => ipv6List.value.length > 0);

/** IPv6 视图链路状态：未获取到 IPv6 地址视为未连接 */
const ipv6Status = computed(() => (hasIpv6.value ? 'connected' : 'disconnected'));

const shownStatus = computed(() =>
	ipFamily.value === 'ipv6' ? ipv6Status.value : adapter.value?.status,
);

const shownAddress = computed(() =>
	ipFamily.value === 'ipv6'
		? ipv6List.value.join('、')
		: adapter.value?.ipv4.join('、') || '',
);
const shownPrefix = computed(() => {
	if (ipFamily.value === 'ipv6') {
		return ipv6Prefix.value.map((p) => `/${p}`).join('、');
	}
	return adapter.value?.subnet_mask.join('、') || '';
});
const shownGateway = computed(() =>
	ipFamily.value === 'ipv6'
		? adapter.value?.ipv6_gateway ?? ''
		: adapter.value?.gateway ?? '',
);
const shownDns1 = computed(() =>
	ipFamily.value === 'ipv6'
		? adapter.value?.ipv6_dns[0] ?? ''
		: adapter.value?.dns[0] ?? '',
);
const shownDns2 = computed(() =>
	ipFamily.value === 'ipv6'
		? adapter.value?.ipv6_dns[1] ?? ''
		: adapter.value?.dns[1] ?? '',
);

const adapterKind = computed(() => {
	const identity = `${adapter.value?.name ?? ''} ${adapter.value?.description ?? ''}`.toLowerCase();
	if (/bluetooth|蓝牙|personal area network|\bpan\b/.test(identity)) {
		return { kind: 'bluetooth', label: '蓝牙网络' };
	}
	if (/wi-?fi|wlan|wireless|802\.11|无线网络|无线网卡/.test(identity)) {
		return { kind: 'wifi', label: '无线网络' };
	}
	if (/cellular|mobile broadband|wwan|\b(?:4g|5g|lte|nr)\b|蜂窝网络|移动宽带/.test(identity)) {
		return { kind: 'cellular', label: '移动网络' };
	}
	if (/rndis|usb tether|usb network|网络共享|usb 网卡/.test(identity)) {
		return { kind: 'usb', label: 'USB 网络' };
	}
	if (/virtual|vmnet|hyper-v|\bvethernet\b|\btap\b|\btun\b|vpn|wireguard|openvpn|zerotier|tailscale|loopback|docker|wsl|虚拟网卡|虚拟以太网|隧道/.test(identity)) {
		return { kind: 'virtual', label: '虚拟网络' };
	}
	if (/ethernet|有线|以太网|local area connection|\blan\b|\d+(?:\.\d+)?\s*gbe\b|\bgigabit\b|fast ethernet|realtek/.test(identity)) {
		return { kind: 'wired', label: '有线网络' };
	}
	if (/\busb\b/.test(identity)) {
		return { kind: 'usb', label: 'USB 网络' };
	}
	return { kind: 'default', label: '网络设备' };
});

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
	creatingProfile.value = false;
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

function openNewProfile() {
	creatingProfile.value = true;
	editProfileId.value = '';
	editForm.name = '';
	editForm.mode = mode.value;
	editForm.ip = form.ip;
	editForm.mask = form.mask;
	editForm.gateway = form.gateway;
	editForm.dns1 = form.dns1;
	editForm.dns2 = form.dns2;
	editVisible.value = true;
}

async function confirmEdit() {
	if (!editForm.name.trim()) {
		ElMessage.warning('请填写配置名称');
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
	if (creatingProfile.value) {
		appConfig.value.profiles.push({
			id: crypto.randomUUID(), name: editForm.name.trim(), mode: editForm.mode,
			ip: editForm.mode === 'static' ? editForm.ip || null : null,
			mask: editForm.mode === 'static' ? editForm.mask || null : null,
			gateway: editForm.mode === 'static' ? editForm.gateway || null : null,
			dns1: editForm.dns1 || null, dns2: editForm.dns2 || null,
		});
		await saveConfig(appConfig.value);
		emitConfigChanged();
		ElMessage.success('配置已新增');
		editVisible.value = false;
		return;
	}
	if (!existing) {
		ElMessage.warning('配置不存在或已被删除');
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
	ElMessage.success('配置已更新');
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
	ElMessage.success('配置已删除');
}

async function saveProfile() {
	const defaultName =
		mode.value === 'dhcp' ? 'DHCP' : `固定IP ${form.ip || ''}`.trim();
	let name: string | undefined;
	try {
		const ret = await ElMessageBox.prompt(
			'请输入配置名称',
			'保存 IP 配置',
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
	ElMessage.success('配置已保存');
}
</script>

<template>
	<div class="detail-wrap">
		<div class="panel current-card">
			<div class="current-head">
				<div class="current-info">
					<div class="current-title">
						<span
							class="current-kind-icon"
							:class="[`kind-${adapterKind.kind}`, { 'is-disconnected': adapter?.status !== 'connected' }]"
							:title="adapterKind.label"
							:aria-label="adapterKind.label"
							aria-hidden="false">
							<svg v-if="adapterKind.kind === 'wired'" viewBox="0 0 24 24" aria-hidden="true">
								<rect x="4" y="4" width="16" height="16" rx="3" />
								<path d="M8 4v5m4-5v5m4-5v5M4 11h16M9 15h.01M12 15h.01M15 15h.01" />
							</svg>
							<svg v-else-if="adapterKind.kind === 'wifi'" viewBox="0 0 24 24" aria-hidden="true">
								<path d="M4.5 9a12 12 0 0 1 15 0M7.5 12a7.5 7.5 0 0 1 9 0M10.5 15a3 3 0 0 1 3 0" />
								<circle cx="12" cy="18" r="1" />
							</svg>
							<svg v-else-if="adapterKind.kind === 'bluetooth'" viewBox="0 0 24 24" aria-hidden="true">
								<path d="m7 7 10 10-5 4V3l5 4L7 17" />
							</svg>
							<svg v-else-if="adapterKind.kind === 'cellular'" viewBox="0 0 24 24" aria-hidden="true">
								<path d="M4 20v-4m5 4v-7m5 7V9m5 11V4" />
							</svg>
							<svg v-else-if="adapterKind.kind === 'usb'" viewBox="0 0 24 24" aria-hidden="true">
								<path d="M12 21V5m0 0L8.5 8.5M12 5l3.5 3.5M12 13l-4 4m4-4 4 4" />
								<circle cx="12" cy="3" r="1.5" />
								<rect x="14.5" y="16" width="3" height="3" rx=".5" />
								<circle cx="7" cy="18" r="1.5" />
							</svg>
							<svg v-else-if="adapterKind.kind === 'virtual'" viewBox="0 0 24 24" aria-hidden="true">
								<rect x="4" y="4" width="16" height="6" rx="2" />
								<rect x="4" y="14" width="16" height="6" rx="2" />
								<path d="M8 7h.01M8 17h.01M12 10v4" />
							</svg>
							<svg v-else viewBox="0 0 24 24" aria-hidden="true">
								<circle cx="12" cy="12" r="3" />
								<circle cx="5" cy="6" r="1.5" />
								<circle cx="19" cy="6" r="1.5" />
								<circle cx="5" cy="18" r="1.5" />
								<circle cx="19" cy="18" r="1.5" />
								<path d="m9.5 10-3-3m8 3 3-3m-8 7-3 3m8-3 3 3" />
							</svg>
						</span>
						<span class="current-name">{{ adapter?.name || '未选择网卡' }}</span>
						<span v-if="adapter?.description" class="current-sub" :title="adapter.description">（{{ adapter.description }}）</span>
						<div class="family-switch" role="tablist" aria-label="协议族切换">
							<span
								class="family-thumb"
								:class="{ 'is-right': ipFamily === 'ipv6' }"
								aria-hidden="true"></span>
							<button
								type="button"
								class="family-option"
								:class="{ 'is-on': ipFamily === 'ipv4' }"
								@click="ipFamily = 'ipv4'">
								IPv4
							</button>
							<button
								type="button"
								class="family-option"
								:class="{ 'is-on': ipFamily === 'ipv6' }"
								@click="ipFamily = 'ipv6'">
								IPv6
							</button>
						</div>
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
			<div class="desc-frame">
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
						v-if="shownStatus === 'connected'"
						type="success"
						size="small"
						>已连接</el-tag
					>
					<el-tag v-else type="danger" size="small">未连接</el-tag>
				</el-descriptions-item>
				<el-descriptions-item :label="ipFamily === 'ipv6' ? 'IPv6 地址' : 'IPv4 地址'">
					{{ shownAddress || '-' }}
				</el-descriptions-item>
				<el-descriptions-item :label="ipFamily === 'ipv6' ? '前缀长度' : '子网掩码'">
					{{ shownPrefix || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="默认网关">
					{{ shownGateway || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="链路速率">
					{{ adapter?.link_speed || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="首选 DNS">
					{{ shownDns1 || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="备用 DNS">
					{{ shownDns2 || '-' }}
				</el-descriptions-item>
				<el-descriptions-item label="MAC 地址" :span="2">
					<span>{{ adapter?.mac || '-' }}</span>
				</el-descriptions-item>
			</el-descriptions>
			</div>
		</div>

		<div class="target-grid">
			<div class="panel form-col">
				<div class="card-title">IP 配置</div>
				<div class="form-body">
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

					<div class="dns-only">
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
					</div>
				</div>

				<div class="actions">
					<el-button
						type="primary"
						:loading="applying"
						@click="handleApply">
						应用
					</el-button>
					<el-button :disabled="!adapter" @click="saveProfile">保存</el-button>
				</div>
			</div>

			<div class="panel profile-col">
				<div class="profile-header">
					<div class="card-title">配置库</div>
					<el-button size="small" type="primary" plain @click="openNewProfile">＋ 新增</el-button>
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
					暂无保存的配置，配置好参数后点击「保存当前」
				</div>
			</div>
		</div>

		<el-dialog
			v-model="editVisible"
			:title="creatingProfile ? '新增配置' : '编辑配置'"
			class="profile-dialog"
			width="560px"
			destroy-on-close>
			<el-form label-position="top" class="profile-dialog-form">
				<el-form-item label="配置名称">
					<el-input
						v-model="editForm.name"
						placeholder="请输入配置名称" />
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
					>{{ creatingProfile ? '创建配置' : '保存修改' }}</el-button
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
	padding: 10px;
}
.current-card {
	overflow: hidden;
	background: var(--panel-glass);
}
.desc-frame {
	border: 1px solid var(--description-border);
	border-radius: 9px;
	background: var(--panel-bg);
	overflow: hidden;
}
.desc {
	width: 100%;
}
.card-title {
	font-size: 14px;
	font-weight: 600;
	margin-bottom: 6px;
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
	font-size: 14px;
	font-weight: 650;
}
.current-kind-icon {
	display: inline-grid;
	place-items: center;
	width: 17px;
	height: 17px;
	flex: none;
}
.current-kind-icon svg {
	width: 100%;
	height: 100%;
	fill: none;
	stroke: currentColor;
	stroke-width: 1.8;
	stroke-linecap: round;
	stroke-linejoin: round;
}
.current-kind-icon.kind-wired {
	color: #169b63;
}
.current-kind-icon.kind-wifi {
	color: #3478e5;
}
.current-kind-icon.kind-bluetooth {
	color: #7756d8;
}
.current-kind-icon.kind-cellular {
	color: #0b91b2;
}
.current-kind-icon.kind-usb {
	color: #138e9e;
}
.current-kind-icon.kind-virtual {
	color: #8862d7;
}
.current-kind-icon.kind-default {
	color: var(--text-secondary);
}
.current-kind-icon.is-disconnected {
	opacity: 0.55;
}
.current-name {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.current-sub {
	flex: 0 1 auto;
	max-width: 62%;
	min-width: 0;
	font-size: 12px;
	font-weight: 400;
	color: var(--text-secondary);
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.family-switch {
	flex: none;
	position: relative;
	display: grid;
	grid-template-columns: 1fr 1fr;
	align-items: center;
	gap: 2px;
	margin-left: 2px;
	padding: 2px;
	background: var(--app-bg);
	border: 1px solid var(--panel-border);
	border-radius: 999px;
}
.family-thumb {
	position: absolute;
	top: 2px;
	bottom: 2px;
	left: 2px;
	width: calc((100% - 6px) / 2);
	border-radius: 999px;
	background: var(--accent);
	box-shadow: 0 2px 8px color-mix(in srgb, var(--accent) 38%, transparent);
	transition: transform 0.3s cubic-bezier(0.34, 0.8, 0.4, 1);
}
.family-thumb.is-right {
	transform: translateX(calc(100% + 2px));
}
.family-option {
	position: relative;
	z-index: 1;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	height: 24px;
	padding: 0 12px;
	border: 0;
	border-radius: 999px;
	background: transparent;
	color: var(--text-secondary);
	font-size: 11px;
	font-weight: 550;
	line-height: 1;
	cursor: pointer;
	transition: color 0.28s ease;
}
.family-option:hover:not(.is-on) {
	color: var(--text-primary);
}
.family-option.is-on {
	color: #fff;
}
.family-option:focus-visible {
	outline: 2px solid var(--accent);
	outline-offset: 1px;
}
.copy-btn {
	flex: none;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	width: 28px;
	height: 28px;
	padding: 0;
	border: 0;
	background: transparent;
	border-radius: 6px;
	color: var(--text-secondary);
	cursor: pointer;
	transition:
		color 0.15s,
		background 0.15s;
}
.copy-btn:hover:not(:disabled) {
	color: var(--accent);
}
.copy-btn:focus-visible {
	outline: 2px solid var(--accent);
	outline-offset: 2px;
}
.copy-btn:disabled {
	opacity: 0.4;
	cursor: not-allowed;
}
.desc :deep(.el-descriptions__label) {
	width: 110px;
	background: var(--app-bg) !important;
	color: var(--text-secondary);
	font-size: 11px;
	font-weight: 500;
}
.desc :deep(.el-descriptions__table) {
	width: 100%;
	border: 0;
	border-collapse: collapse;
}
.desc :deep(.el-descriptions__cell) {
	border-color: var(--description-border) !important;
	padding: 8px 12px;
	font-size: 12px;
}
.desc :deep(.el-descriptions__table tr:first-child > td) {
	border-top: 0 !important;
}
.desc :deep(.el-descriptions__table tr:last-child > td) {
	border-bottom: 0 !important;
}
.desc :deep(.el-descriptions__table tr > td:first-child) {
	border-left: 0 !important;
}
.desc :deep(.el-descriptions__table tr > td:last-child) {
	border-right: 0 !important;
}
.desc :deep(.el-descriptions__content) {
	background: var(--panel-bg) !important;
	color: var(--text-primary);
	font-variant-numeric: tabular-nums;
}

.target-grid {
	display: grid;
	grid-template-columns: minmax(0, 1fr) 350px;
	grid-template-rows: minmax(0, 1fr);
	gap: 10px;
	align-items: stretch;
	flex: 1;
	min-height: 0;
}
.form-col {
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
}
.form-body {
	display: flex;
	flex: 1;
	flex-direction: column;
	min-height: 0;
	overflow-y: auto;
	gap: 8px;
}
.form-body > * {
	flex-shrink: 0;
}
.form-col > .card-title {
	padding-bottom: 8px;
	border-bottom: 1px solid var(--panel-border);
}
.target-form {
	display: grid;
	flex: 1 0 auto;
	max-height: 180px;
}
.form-grid {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	gap: 2px 10px;
}
.target-form :deep(.el-form-item) {
	margin-bottom: 0;
}
.target-form :deep(.el-form-item--label-top > .el-form-item__label) {
	display: block;
	margin-bottom: 3px !important;
  margin-left: 3px !important;
	font-size: 12px;
}
.target-form :deep(.el-input__wrapper),
.dns-input :deep(.el-input__wrapper) {
	background: var(--input-bg) !important;
	box-shadow: 0 0 0 1px var(--input-border) inset;
	transition: box-shadow 0.16s ease;
}
.target-form :deep(.el-input__wrapper:hover),
.dns-input :deep(.el-input__wrapper:hover) {
	box-shadow: 0 0 0 1px var(--accent) inset;
}
.target-form :deep(.el-input__wrapper.is-focus),
.dns-input :deep(.el-input__wrapper.is-focus) {
	box-shadow: 0 0 0 1px var(--accent) inset, 0 0 0 3px var(--accent-soft);
}
.target-form :deep(.el-input__inner),
.dns-input :deep(.el-input__inner) {
	color: var(--input-text);
}
.dns-section {
	display: flex;
	align-items: center;
	gap: 12px;
	margin-bottom: 6px;
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
}
.dns-input {
	flex: 1;
}
.dns-input :deep(.el-input__wrapper) {
	min-height: 32px;
}
.actions {
	display: flex;
	flex: none;
	align-items: center;
	gap: 8px;
	margin-bottom: 8px;
	padding-top: 18px;
	border-top: 1px solid var(--panel-border);
}
/* 次按钮去掉浅色实底，保持透明（主按钮保留实色） */
.dns-row :deep(.el-button),
.actions :deep(.el-button:not(.el-button--primary)) {
	background: transparent;
}

.profile-col {
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
}
.profile-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	margin-bottom: 8px;
	padding-bottom: 6px;
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

/* 方案编辑/创建弹窗：字段成组排列，缩短滚动距离 */
:global(.profile-dialog.el-dialog) {
	max-width: calc(100vw - 40px);
	border: 1px solid var(--panel-border-glass);
	border-radius: 16px;
	background: var(--panel-bg);
	box-shadow: 0 24px 72px rgba(0, 0, 0, 0.42);
}
:global(.profile-dialog .el-dialog__header) {
	margin: 0;
	padding: 20px 22px 14px;
	border-bottom: 1px solid var(--panel-border);
}
:global(.profile-dialog .el-dialog__title) {
	font-size: 17px;
	font-weight: 650;
	letter-spacing: 0.01em;
}
:global(.profile-dialog .el-dialog__body) {
	max-height: min(68vh, 620px);
	overflow-y: auto;
	padding: 18px 22px 8px;
}
:global(.profile-dialog .el-dialog__footer) {
	padding: 12px 22px 18px;
	border-top: 1px solid var(--panel-border);
}
.profile-dialog-form {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	column-gap: 14px;
}
.profile-dialog-form :deep(.el-form-item) {
	min-width: 0;
	margin-bottom: 13px;
}
.profile-dialog-form :deep(.el-form-item:nth-child(-n + 2)) {
	grid-column: 1 / -1;
}
.profile-dialog-form :deep(.el-form-item__label) {
	padding-bottom: 5px;
	color: var(--text-secondary);
	font-size: 11px;
	line-height: 1.4;
}
.profile-dialog-form :deep(.el-input__wrapper) {
	min-height: 38px;
	border-radius: 8px;
	background: var(--input-bg) !important;
	box-shadow: 0 0 0 1px var(--input-border) inset;
	transition: box-shadow 0.16s ease;
}
.profile-dialog-form :deep(.el-input__wrapper:hover) {
	box-shadow: 0 0 0 1px var(--accent) inset;
}
.profile-dialog-form :deep(.el-input__wrapper.is-focus) {
	box-shadow: 0 0 0 1px var(--accent) inset, 0 0 0 3px var(--accent-soft);
}
.profile-dialog-form :deep(.el-radio-group) {
	display: flex;
	gap: 8px;
}
.profile-dialog-form :deep(.el-radio) {
	margin: 0;
	padding: 7px 10px;
	border: 1px solid var(--panel-border);
	border-radius: 8px;
	background: var(--app-bg);
}
.profile-dialog-form :deep(.el-radio.is-checked) {
	border-color: color-mix(in srgb, var(--accent) 55%, transparent);
	background: var(--accent-soft);
}
.profile-dialog-form :deep(.el-radio__label) {
	font-size: 11px;
}
:global(.profile-dialog .el-dialog__footer .el-button) {
	min-width: 88px;
	height: 34px;
	border-radius: 8px;
	font-size: 11px;
	font-weight: 550;
}

@media (max-width: 620px) {
	.profile-dialog-form {
		grid-template-columns: 1fr;
	}
	.profile-dialog-form :deep(.el-form-item) {
		grid-column: 1 / -1;
	}
}

@media (max-width: 1000px) {
	.detail-wrap {
		overflow-y: auto;
	}
	.current-card {
		flex: none;
	}
	.target-grid {
		grid-template-columns: 1fr;
		grid-template-rows: auto auto;
		flex: none;
	}
	.form-body, .profile-list {
		flex: none;
	}
}
</style>
