import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AdapterInfo,
  AdapterTrafficCounters,
  AppConfig,
  IpProfile,
  PendingAction,
  StaticInput,
  ScanHostResult,
  ScanDoneResult,
  UpdateInfo,
  UpdateProgress,
} from "@/types";

export const listAdapters = () => invoke<AdapterInfo[]>("list_adapters");
export const getAdapterTraffic = (name: string) =>
  invoke<AdapterTrafficCounters>("get_adapter_traffic", { name });

export const applyStatic = (input: StaticInput) =>
  invoke<string>("apply_static", { input });

export const applyDhcp = (name: string, resetDns: boolean) =>
  invoke<string>("apply_dhcp", { name, resetDns });

export const applyDns = (name: string, dns1: string, dns2?: string | null) =>
  invoke<string>("apply_dns", { name, dns1, dns2 });

export const setAdapterEnabled = (name: string, enabled: boolean) =>
  invoke<string>("set_adapter_enabled", { name, enabled });

export const renameAdapter = (name: string, newName: string) =>
  invoke<string>("rename_adapter", { name, newName });

export const openNetworkConnections = () =>
  invoke<void>("open_network_connections");

export const runDiagnostic = (
  taskId: string,
  preset: string,
  target: string | null,
  extraArgs: string[]
) =>
  invoke<void>("run_diagnostic", { taskId, preset, target, extraArgs });

export const stopDiagnostic = (taskId: string) =>
  invoke<void>("stop_diagnostic", { taskId });

export const runHostScan = (taskId: string, adapter: string, ip: string, mask: string) =>
  invoke<void>("run_host_scan", { taskId, adapter, ip, mask });

export const stopHostScan = (taskId: string) =>
  invoke<void>("stop_host_scan", { taskId });

export const onScanHostResult = (cb: (p: ScanHostResult) => void) =>
  listen<ScanHostResult>("scan-host-result", (e) => cb(e.payload));

export const onScanDone = (cb: (p: ScanDoneResult) => void) =>
  listen<ScanDoneResult>("scan-done", (e) => cb(e.payload));

export const elevateAction = (action: PendingAction) =>
  invoke<void>("elevate_action", { action });

/** 以管理员权限重启本程序；返回 false 表示当前已是管理员 */
export const restartAsAdmin = () => invoke<boolean>("restart_as_admin");

/** 启动时是否应弹出管理员权限询问 */
export const shouldPromptAdmin = () => invoke<boolean>("should_prompt_admin");

export const getConfig = () => invoke<AppConfig>("get_config");

export const saveConfig = (config: AppConfig) =>
  invoke<void>("save_config_cmd", { config });

export const checkAdmin = () => invoke<boolean>("check_admin");

export const openDevTools = () => invoke<void>("open_devtools");

export const setWindowTheme = (dark: boolean) =>
  invoke<void>("set_window_theme", { dark });

export const showWidget = () => invoke<void>("show_widget");

export const hideWidget = () => invoke<void>("hide_widget");

export const setWidgetOpacity = (opacity: number) =>
  invoke<void>("set_widget_opacity", { opacity });

export const saveWidgetSize = (width: number, height: number) =>
  invoke<void>("save_widget_size", { width, height });

export const setWidgetPinned = (pinned: boolean) =>
  invoke<void>("set_widget_pinned", { pinned });

export const setAutostart = (enabled: boolean) =>
  invoke<void>("set_autostart", { enabled });

export const getAutostart = () => invoke<boolean>("get_autostart");

export const createDesktopShortcut = () =>
  invoke<void>("create_desktop_shortcut");

/// 挂件固定/穿透状态变化（托盘菜单或挂件图标触发）
export function onWidgetPinChanged(cb: (pinned: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>("widget-pin-changed", (e) => cb(e.payload));
}

/// 广播网卡信息已变化，挂件收到后刷新
export function emitAdaptersChanged() {
  void emit("adapters-changed", null);
}

/// 挂件订阅网卡变化事件
export function onAdaptersChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("adapters-changed", () => cb());
}

/// 广播配置（方案 / 默认网卡等）已变化，挂件收到后重载配置
export function emitConfigChanged() {
  void emit("config-changed", null);
}

/// 挂件订阅配置变化事件
export function onConfigChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("config-changed", () => cb());
}

export type DiagLinePayload = { task_id: string; line: string };
export type DiagDonePayload = { task_id: string; code: number };
export type TrayResultPayload = { ok: boolean; message: string };

export function onDiagLine(
  cb: (p: DiagLinePayload) => void
): Promise<UnlistenFn> {
  return listen<DiagLinePayload>("diag-line", (e) => cb(e.payload));
}

export function onDiagDone(
  cb: (p: DiagDonePayload) => void
): Promise<UnlistenFn> {
  return listen<DiagDonePayload>("diag-done", (e) => cb(e.payload));
}

export function onTrayApplyResult(
  cb: (p: TrayResultPayload) => void
): Promise<UnlistenFn> {
  return listen<TrayResultPayload>("tray-apply-result", (e) => cb(e.payload));
}

/** 检查 GitHub Release 是否存在更新 */
export const checkUpdate = () => invoke<UpdateInfo>("check_update");

/** 下载更新包到临时目录，返回本地临时文件路径 */
export const downloadUpdate = (
  url: string,
  fileName: string,
  digest: string | null,
  expectedSize: number
) =>
  invoke<string>("download_update", { url, fileName, digest, expectedSize });

/** 请求取消当前进行中的更新包下载 */
export const cancelDownload = () => invoke<void>("cancel_download");

/** 应用更新：安装版拉起安装器，绿色版退出后由辅助进程覆盖并重启 */
export const applyUpdate = (assetPath: string, assetName: string) =>
  invoke<void>("apply_update", { assetPath, assetName });

/** 订阅更新包下载进度 */
export const onUpdateProgress = (cb: (p: UpdateProgress) => void) =>
  listen<UpdateProgress>("update-download-progress", (e) => cb(e.payload));

export type { IpProfile };
export type { UnlistenFn } from "@tauri-apps/api/event";
