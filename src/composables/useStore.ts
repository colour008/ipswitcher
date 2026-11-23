import { ref } from "vue";
import { listAdapters, scheduleAdapterRefresh } from "@/api/ipc";
import type { AdapterInfo, AdapterSnapshot, AppConfig } from "@/types";

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

let adapterRevision = 0;
let pendingRefresh: Promise<void> | null = null;

export function applyAdapterSnapshot(snapshot: AdapterSnapshot): void {
  if (snapshot.revision <= adapterRevision) return;
  adapterRevision = snapshot.revision;
  const seen = new Set<string>();
  adapters.value = snapshot.adapters.filter((a) => {
    const key = a.if_index != null ? `i${a.if_index}` : `m${a.mac ?? a.name}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function refreshAdapters(force = true): Promise<void> {
  if (pendingRefresh) return pendingRefresh;
  refreshing.value = true;
  pendingRefresh = listAdapters(force).then(applyAdapterSnapshot).finally(() => {
    refreshing.value = false;
    pendingRefresh = null;
  });
  return pendingRefresh;
}

/// 给 DHCP 租约和网卡状态一次延后的复查机会。
export function scheduleDelayedRefresh(): void {
  void scheduleAdapterRefresh().catch(() => undefined);
}
