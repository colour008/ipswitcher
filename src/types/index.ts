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
  widget_adapter?: string | null;
  widget_opacity?: number | null;
  widget_width?: number | null;
  widget_height?: number | null;
  theme?: "light" | "dark" | null;
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
