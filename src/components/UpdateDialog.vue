<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
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
    width="480px"
    class="update-dialog"
    :close-on-click-modal="false"
    :close-on-press-escape="phase !== 'downloading'"
    :show-close="phase !== 'downloading'"
    @update:model-value="(v) => emit('update:visible', v)"
    @close="close">
    <div v-if="phase === 'confirm'" class="update-body">
      <div class="update-headline">发现新版本 {{ info?.latestVersion }}</div>
      <div class="update-sub">当前版本 v{{ info?.currentVersion }}</div>
      <div class="update-notes-title">更新说明</div>
      <el-scrollbar class="update-notes">
        <pre class="update-notes-text">{{ info?.notes || '（无更新说明）' }}</pre>
      </el-scrollbar>
      <div v-if="info?.message" class="update-warn">{{ info.message }}</div>
    </div>

    <div v-else-if="phase === 'downloading'" class="update-body">
      <div class="update-headline">正在下载更新包</div>
      <el-progress :percentage="progressPercent" :stroke-width="10" />
      <div class="update-size">
        {{ formatSize(downloaded) }} / {{ formatSize(total) }}
      </div>
    </div>

    <div v-else class="update-body">
      <div class="update-headline">更新失败</div>
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
  gap: 10px;
}

.update-headline {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
}

.update-sub {
  font-size: 13px;
  color: var(--text-secondary);
}

.update-notes-title {
  font-size: 13px;
  color: var(--text-secondary);
  margin-top: 4px;
}

.update-notes {
  max-height: 220px;
  border: 1px solid var(--panel-border);
  border-radius: 8px;
  background: var(--app-bg);
  padding: 10px 12px;
}

.update-notes-text {
  margin: 0;
  font-family: inherit;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-primary);
  white-space: pre-wrap;
  word-break: break-word;
}

.update-warn {
  font-size: 12px;
  color: var(--danger);
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
</style>
