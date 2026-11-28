/** 网卡信息（与 Rust 侧 AdapterInfo 对应） */
export interface AdapterInfo {
  name: string;
  description: string;
  interface_guid: string | null;
  if_index: number | null;
  mac: string | null;
  status: string; // connected / disconnected / disabled
  link_speed: string | null;
  dhcp_enabled: boolean | null;
  ipv4: string[];
  subnet_mask: string[];
  gateway: string | null;
  dns: string[];
  ipv6: string[];
  ipv6_prefix: string[];
  ipv6_gateway: string | null;
  ipv6_dns: string[];
}

/** Network interface counters reported by Windows. */
export interface AdapterSnapshot {
  revision: number;
  adapters: AdapterInfo[];
}

export interface AdapterTrafficCounters {
  sent_bytes: number;
  received_bytes: number;
  sampled_at: number;
}

/** 保存的 IP 方案 */
export interface IpProfile {
  id: string;
  name: string;
  mode: "dhcp" | "static";
  ip?: string | null;
  mask?: string | null;
  gateway?: string | null;
  dns1?: string | null;
  dns2?: string | null;
}

/** 本地持久化配置 */
export interface AppConfig {
  default_adapter: string | null;
  profiles: IpProfile[];
  adapter_order?: string[];
  pinned?: string[];
  sidebar_width?: number;
  close_action?: "exit" | "tray" | null;
  autostart?: boolean | null;
  widget_visible?: boolean | null;
  traffic_monitor_enabled?: boolean | null;
  widget_adapter?: string | null;
  widget_opacity?: number | null;
  widget_width?: number | null;
  widget_height?: number | null;
  theme?: "light" | "dark" | null;
  /// 挂件独立主题偏好；为 null 时跟随主界面全局 theme
  widget_theme?: "light" | "dark" | null;
  /** 启动管理员询问是否已选“暂不”（true 后不再自动弹窗，可在设置页手动启用） */
  admin_prompt_dismissed?: boolean | null;
  elevate_on_startup?: boolean | null;
}

export interface ScanHostResult {
  ip: string;
  status: "online" | "offline" | "undiscovered";
  last_seen: number | null;
  hostname: string | null;
  mac: string | null;
}

export interface ScanBatch {
  task_id: string;
  scanned: number;
  rows: ScanHostResult[];
}

export interface ScanDoneResult {
  task_id: string;
  scanned: number;
  cancelled: boolean;
  error: string | null;
}

export interface StaticInput {
  name: string;
  ip: string;
  mask: string;
  gateway?: string | null;
  dns1?: string | null;
  dns2?: string | null;
}

export type PendingAction =
  | {
      kind: "Static";
      names: string[];
      ip: string;
      mask: string;
      gateway?: string | null;
      dns1?: string | null;
      dns2?: string | null;
    }
  | { kind: "Dhcp"; names: string[]; reset_dns: boolean }
  | { kind: "Dns"; names: string[]; dns1: string; dns2?: string | null }
  | { kind: "Toggle"; names: string[]; enabled: boolean }
  | { kind: "Rename"; name: string; new_name: string };

/** 检查更新结果（与 Rust 侧 UpdateInfo 对应） */
export interface UpdateInfo {
  hasUpdate: boolean;
  currentVersion: string;
  latestVersion: string;
  /** Release Notes 正文（Markdown，前端渲染） */
  notes: string;
  /** 当前分发形态：installed / portable / standalone */
  flavor: string;
  assetName: string | null;
  assetUrl: string | null;
  assetSize: number;
  assetDigest: string | null;
  message: string | null;
}

/** 更新包下载进度 */
export interface UpdateProgress {
  downloaded: number;
  total: number;
}
