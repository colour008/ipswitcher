use serde::{Deserialize, Serialize};

/// 网卡信息（前端列表与详情展示使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterInfo {
    /// 接口别名（netsh / PowerShell 使用的名称，作为操作标识）
    pub name: String,
    /// 设备描述
    pub description: String,
    pub interface_guid: Option<String>,
    pub if_index: Option<u32>,
    pub mac: Option<String>,
    /// connected / disconnected
    pub status: String,
    pub link_speed: Option<String>,
    pub dhcp_enabled: Option<bool>,
    pub ipv4: Vec<String>,
    pub subnet_mask: Vec<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}

/// 保存的静态 IP 方案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpProfile {
    pub id: String,
    pub name: String,
    /// dhcp / static
    pub mode: String,
    pub ip: Option<String>,
    pub mask: Option<String>,
    pub gateway: Option<String>,
    pub dns1: Option<String>,
    pub dns2: Option<String>,
}

/// 本地持久化配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub default_adapter: Option<String>,
    pub profiles: Vec<IpProfile>,
    /// 网卡列表展示顺序（按接口别名排列，未记录的网卡追加在末尾）
    #[serde(default)]
    pub adapter_order: Vec<String>,
    /// 已置顶（固定在列表顶部）的网卡接口别名
    #[serde(default)]
    pub pinned: Vec<String>,
    /// 左侧网卡列表宽度（像素）
    #[serde(default)]
    pub sidebar_width: Option<f64>,
    /// 关闭按钮行为：exit 直接退出，tray 最小化到托盘
    #[serde(default)]
    pub close_action: Option<String>,
    /// 开机是否随系统启动（缺省 true）
    #[serde(default)]
    pub autostart: Option<bool>,
    /// 是否显示桌面挂件（透明悬浮小窗）
    #[serde(default)]
    pub widget_visible: Option<bool>,
    /// 挂件选中的网卡接口别名
    #[serde(default)]
    pub widget_adapter: Option<String>,
    /// 挂件背景不透明度（0.0 ~ 1.0，0 为全透明），缺省 0.92
    #[serde(default)]
    pub widget_opacity: Option<f64>,
    /// 挂件窗口逻辑宽度（像素）
    #[serde(default)]
    pub widget_width: Option<f64>,
    /// 挂件窗口逻辑高度（像素）
    #[serde(default)]
    pub widget_height: Option<f64>,
    /// 界面主题：light / dark，缺省按深色处理
    #[serde(default)]
    pub theme: Option<String>,
    /// 启动管理员权限询问是否已被用户选择“暂不”（置 true 后不再自动弹窗，可在设置页手动启用）
    #[serde(default)]
    pub admin_prompt_dismissed: Option<bool>,
    /// Previously responsive hosts, used to distinguish offline from never discovered.
    #[serde(default)]
    pub host_history: Vec<HostHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostHistory {
    pub adapter: String,
    pub network: String,
    pub ip: String,
    pub last_seen: u64,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub mac: Option<String>,
}

