<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import {
  downloadUpdate,
  applyUpdate,
  onUpdateProgress,
  type UnlistenFn,
} from '@/api/ipc';
import type { UpdateInfo } from '@/types';

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
let unlistenProgress: UnlistenFn | null = null;

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
    } else {
      cleanupProgress();
    }
  }
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
  cleanupProgress();
  unlistenProgress = await onUpdateProgress((p) => {
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
    phase.value = 'error';
    errorMsg.value = typeof e === 'string' ? e : '更新失败，请稍后重试';
    cleanupProgress();
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
        <span class="update-version-badge">v{{ info?.latestVersion }}</span>
      </div>
      <div class="update-sub">当前版本 v{{ info?.currentVersion }}</div>

      <div class="update-notes-title">更新说明</div>
      <el-scrollbar v-if="hasNotes" class="update-notes">
        <div class="markdown-body" v-html="renderedNotes"></div>
      </el-scrollbar>
      <div v-else class="update-notes update-notes-empty">（无更新说明）</div>

      <div v-if="info?.message" class="update-warn">{{ info.message }}</div>
    </div>

    <div v-else-if="phase === 'downloading'" class="update-body update-body--progress">
      <div class="update-title-row">
        <span class="update-title">正在下载更新包</span>
        <span class="update-version-badge">v{{ info?.latestVersion }}</span>
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
  font-size: 13px;
  color: var(--text-secondary);
  margin-top: 2px;
}

.update-notes {
  max-height: 260px;
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
  font-size: 13px;
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
  font-size: 15px;
}

.markdown-body :deep(h3) {
  font-size: 14px;
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
  font-size: 12px;
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
  font-size: 12.5px;
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
