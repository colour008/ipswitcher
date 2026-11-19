import { ref } from "vue";
import { listAdapters, emitAdaptersChanged } from "@/api/ipc";
import type { AdapterInfo, AppConfig } from "@/types";

export const adapters = ref<AdapterInfo[]>([]);
export const currentName = ref<string | null>(null);
export const appConfig = ref<AppConfig>({ default_adapter: null, profiles: [] });
export const refreshing = ref(false);

/// 规范化配置：置顶列表去重、去除空串，避免同一网卡被渲染多次
export function normalizeConfig(cfg: AppConfig): AppConfig {
  if (Array.isArray(cfg.pinned)) {
    cfg.pinned = [...new Set(cfg.pinned.filter((n) => typeof n === "string" && n.trim()))];
  }
  return cfg;
}

export function currentAdapter(): AdapterInfo | null {
  return adapters.value.find((a) => a.name === currentName.value) ?? null;
}

export async function refreshAdapters(broadcast = true): Promise<void> {
  refreshing.value = true;
  try {
    const list = await listAdapters();
    // 改名 / 状态收敛期，同一网卡可能被短暂枚举两次；按 ifIndex（无则 MAC）去重，避免列表出现重复行
    const seen = new Set<string>();
    adapters.value = list.filter((a) => {
      const key = a.if_index != null ? `i${a.if_index}` : `m${a.mac ?? a.name}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    // 仅主动切换后才广播；被动同步刷新不再回播，避免主窗/挂件互相触发形成刷新环
    if (broadcast) emitAdaptersChanged();
  } finally {
    refreshing.value = false;
  }
}

let refreshTimers: number[] = [];

/// 应用配置后立即刷新，并在延时后再次刷新，等待 DHCP 租约 / 网卡状态收敛。
/// broadcast=false 表示这是被动同步刷新，不向另一窗口回播。
export function scheduleDelayedRefresh(broadcast = true): void {
  refreshTimers.forEach((t) => window.clearTimeout(t));
  refreshTimers = [];
  for (const ms of [1500, 3500]) {
    const t = window.setTimeout(() => {
      void refreshAdapters(broadcast);
    }, ms);
    refreshTimers.push(t);
  }
}
