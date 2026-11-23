<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { ElProgress } from 'element-plus';
import 'element-plus/es/components/progress/style/css';
import {
  downloadUpdate,
  cancelDownload,
  applyUpdate,
  onUpdateProgress,
  type UnlistenFn,
} from '@/api/ipc';
import type { UpdateInfo } from '@/types';

/** 与后端 update.rs 的 DOWNLOAD_CANCELLED 保持一致，用于区分“取消”与“失败” */
const DOWNLOAD_CANCELLED = '更新已取消';

const props = defineProps<{
  visible: boolean;
  info: UpdateInfo | null;
}>();

const emit = defineEmits<{
  'update:visible': [v: boolean];
}>();

type Phase = 'confirm' | 'downloading' | 'error';

const phase = ref<Phase>('confirm');
const errorMsg = ref('');
const downloaded = ref(0);
const total = ref(0);
/** 是否保留现有网卡配置（当前仅记录，升级默认不删除配置文件） */
const keepConfig = ref(true);
/** 下载取消请求已发出，等待下载协程退出 */
const cancelling = ref(false);
let unlistenProgress: UnlistenFn | null = null;

/// 版本号展示：后端 tag_name 可能自带 v 前缀，统一为“至多一个 v”
function displayVersion(v?: string | null): string {
  const s = (v ?? '').trim();
  if (!s) return '';
  return /^[vV]/.test(s) ? s : `v${s}`;
}

const progressPercent = computed(() => {
  if (!total.value) return 0;
  return Math.min(100, Math.round((downloaded.value / total.value) * 100));
});

// Release Notes（Markdown）-> 消毒后 HTML，避免原始标记裸显与 XSS
const renderedNotes = computed(() => {
  const raw = props.info?.notes?.trim();
  if (!raw) return '';
  const html = marked.parse(raw, { gfm: true, breaks: false, async: false }) as string;
  return DOMPurify.sanitize(html);
});

const hasNotes = computed(() => renderedNotes.value.length > 0);

function formatSize(bytes: number): string {
  if (!bytes || bytes <= 0) return '--';
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

function cleanupProgress() {
  if (unlistenProgress) {
    void unlistenProgress();
    unlistenProgress = null;
  }
}

function close() {
  cleanupProgress();
  emit('update:visible', false);
}

// 对话框每次打开时回到确认态，关闭时清理进度订阅
watch(
  () => props.visible,
  (v) => {
    if (v) {
      phase.value = 'confirm';
      errorMsg.value = '';
      downloaded.value = 0;
      total.value = props.info?.assetSize ?? 0;
      keepConfig.value = true;
      cancelling.value = false;
    } else {
      cleanupProgress();
    }
  },
  { immediate: true }
);

async function startUpdate() {
  if (!props.info?.assetUrl || !props.info.assetName) {
    phase.value = 'error';
    errorMsg.value = props.info?.message ?? '未找到可用的更新包';
    return;
  }
  phase.value = 'downloading';
  errorMsg.value = '';
  downloaded.value = 0;
  total.value = props.info.assetSize;
  cancelling.value = false;
  cleanupProgress();
  unlistenProgress = await onUpdateProgress((p) => {
    if (phase.value !== 'downloading') return;
    downloaded.value = p.downloaded;
    if (p.total > 0) total.value = p.total;
  });
  try {
    const localPath = await downloadUpdate(
      props.info.assetUrl,
      props.info.assetName,
      props.info.assetDigest,
      props.info.assetSize
    );
    // 后端成功后会退出主程序，由辅助进程完成覆盖并重启
    await applyUpdate(localPath, props.info.assetName);
  } catch (e) {
    cleanupProgress();
    const msg = typeof e === 'string' ? e : '更新失败，请稍后重试';
    if (msg === DOWNLOAD_CANCELLED) {
      // 用户主动取消：回到确认页，进度清零，可再次更新或稍后再说
      phase.value = 'confirm';
      errorMsg.value = '';
      downloaded.value = 0;
      total.value = props.info?.assetSize ?? 0;
      cancelling.value = false;
    } else {
      phase.value = 'error';
      errorMsg.value = msg;
    }
  }
}

/** 取消进行中的下载；收尾由 startUpdate 的 catch 收到取消错误后完成 */
async function onCancelDownload() {
  if (cancelling.value) return;
  cancelling.value = true;
  try {
    await cancelDownload();
  } catch {
    // 命令异常不阻断：下载协程仍会在取消标志下退出
  }
}

onUnmounted(cleanupProgress);
</script>

<template>
  <el-dialog
    :model-value="visible"
    width="500px"
    class="update-dialog"
    :close-on-click-modal="false"
    :close-on-press-escape="phase !== 'downloading'"
    :show-close="phase !== 'downloading'"
    @update:model-value="(v) => emit('update:visible', v)"
    @close="close">
    <div v-if="phase === 'confirm'" class="update-body">
      <div class="update-title-row">
        <span class="update-title">发现新版本</span>
        <span class="update-version-badge">{{ displayVersion(info?.latestVersion) }}</span>
      </div>
      <div class="update-sub">当前版本 {{ displayVersion(info?.currentVersion) }}</div>

      <div class="update-notes-title">更新说明</div>
      <div v-if="hasNotes" class="update-notes">
        <div class="markdown-body" v-html="renderedNotes"></div>
      </div>
      <div v-else class="update-notes update-notes-empty">（无更新说明）</div>

      <label class="keep-config" :class="{ 'is-checked': keepConfig }">
        <input v-model="keepConfig" type="checkbox" />
        <span class="keep-config-check" aria-hidden="true">
          <svg v-if="keepConfig" viewBox="0 0 16 16"><path d="m3.5 8.2 3 3 6-6.4" /></svg>
        </span>
        <span class="keep-config-copy">
          <strong>保留现有网卡配置</strong>
          <small>更新后继续使用已保存的方案与设置</small>
        </span>
        <span class="keep-config-default">默认开启</span>
      </label>

      <div v-if="info?.message" class="update-warn">{{ info.message }}</div>
    </div>

    <div v-else-if="phase === 'downloading'" class="update-body update-body--progress">
      <div class="update-title-row">
        <span class="update-title">正在下载更新包</span>
        <span class="update-version-badge">{{ displayVersion(info?.latestVersion) }}</span>
      </div>
      <el-progress :percentage="progressPercent" :stroke-width="10" />
      <div class="update-size">
        {{ formatSize(downloaded) }} / {{ formatSize(total) }}
      </div>
    </div>

    <div v-else class="update-body">
      <div class="update-title">更新失败</div>
      <div class="update-error">{{ errorMsg }}</div>
    </div>

    <template #footer>
      <template v-if="phase === 'confirm'">
        <el-button @click="close">稍后再说</el-button>
        <el-button type="primary" :disabled="!info?.assetUrl" @click="startUpdate"
          >立即更新</el-button
        >
      </template>
      <template v-else-if="phase === 'downloading'">
        <el-button :loading="cancelling" @click="onCancelDownload">取消</el-button>
      </template>
      <template v-else-if="phase === 'error'">
        <el-button @click="close">关闭</el-button>
        <el-button type="primary" @click="startUpdate">重试</el-button>
      </template>
    </template>
  </el-dialog>
</template>

<style scoped>
.update-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.update-body--progress {
  gap: 16px;
  padding: 4px 0;
}

.update-title-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.update-title {
  font-size: 17px;
  font-weight: 600;
  color: var(--text-primary);
}

.update-version-badge {
  font-size: 13px;
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
  border-radius: var(--radius-sm);
  padding: 2px 10px;
}

.update-sub {
  font-size: 13px;
  color: var(--text-secondary);
}

.update-notes-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-secondary);
  margin-top: 2px;
}

.update-notes {
  max-height: 260px;
  overflow-y: auto;
  border: 1px solid var(--panel-border);
  border-radius: var(--radius);
  background: var(--panel-bg);
  padding: 12px 14px;
}

.update-notes-empty {
  display: flex;
  align-items: center;
  font-size: 13px;
  color: var(--text-secondary);
}

.update-warn {
  font-size: 12px;
  color: var(--danger);
  line-height: 1.5;
}

.keep-config {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 52px;
  padding: 9px 12px;
  border: 1px solid var(--panel-border);
  border-radius: var(--radius);
  background: color-mix(in srgb, var(--panel-bg) 55%, transparent);
  cursor: pointer;
  transition: border-color 160ms ease, background-color 160ms ease;
}

.keep-config:hover,
.keep-config.is-checked {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--panel-border));
}

.keep-config.is-checked {
  background: color-mix(in srgb, var(--accent-soft) 42%, transparent);
}

.keep-config input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
}

.keep-config input:focus-visible + .keep-config-check {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.keep-config-check {
  display: grid;
  flex: 0 0 18px;
  width: 18px;
  height: 18px;
  place-items: center;
  border: 1.5px solid var(--text-secondary);
  border-radius: 5px;
  color: #fff;
  transition: background-color 160ms ease, border-color 160ms ease;
}

.is-checked .keep-config-check {
  border-color: var(--accent);
  background: var(--accent);
}

.keep-config-check svg {
  width: 13px;
  height: 13px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.keep-config-copy {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.keep-config-copy strong {
  color: var(--text-primary);
  font-size: 12px;
  font-weight: 600;
}

.keep-config-copy small,
.keep-config-default {
  color: var(--text-secondary);
  font-size: 11px;
}

.keep-config-default {
  flex: none;
  padding: 3px 7px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
}

.update-size {
  font-size: 13px;
  color: var(--text-secondary);
  text-align: right;
}

.update-error {
  font-size: 13px;
  color: var(--danger);
  line-height: 1.6;
  white-space: pre-wrap;
}

/* ---------- Markdown 渲染样式 ---------- */
.markdown-body {
  font-size: 12.5px;
  line-height: 1.7;
  color: var(--text-primary);
  word-break: break-word;
}

.markdown-body :deep(h2),
.markdown-body :deep(h3) {
  margin: 14px 0 8px;
  font-weight: 600;
  line-height: 1.4;
  color: var(--text-primary);
}

.markdown-body :deep(h2) {
  font-size: 14px;
}

.markdown-body :deep(h3) {
  font-size: 13px;
}

.markdown-body :deep(h2:first-child),
.markdown-body :deep(h3:first-child) {
  margin-top: 0;
}

.markdown-body :deep(p) {
  margin: 8px 0;
}

.markdown-body :deep(p:first-child) {
  margin-top: 0;
}

.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  margin: 8px 0;
  padding-left: 20px;
}

.markdown-body :deep(li) {
  margin: 6px 0;
}

.markdown-body :deep(li::marker) {
  color: var(--text-secondary);
}

.markdown-body :deep(strong) {
  font-weight: 600;
  color: var(--text-primary);
}

.markdown-body :deep(a) {
  color: var(--accent);
  text-decoration: none;
}

.markdown-body :deep(a:hover) {
  text-decoration: underline;
}

.markdown-body :deep(code) {
  font-family: inherit;
  font-size: 11.5px;
  color: var(--accent);
  background: var(--accent-soft);
  border-radius: 4px;
  padding: 1px 5px;
}

.markdown-body :deep(blockquote) {
  margin: 8px 0;
  padding: 2px 12px;
  border-left: 3px solid var(--panel-border);
  color: var(--text-secondary);
}

.markdown-body :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin: 10px 0;
  font-size: 12px;
}

.markdown-body :deep(th),
.markdown-body :deep(td) {
  border: 1px solid var(--panel-border);
  padding: 6px 10px;
  text-align: left;
}

.markdown-body :deep(th) {
  font-weight: 600;
  color: var(--text-primary);
  background: var(--accent-soft);
}
</style>
