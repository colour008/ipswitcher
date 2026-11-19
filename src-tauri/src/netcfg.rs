use crate::encoding::decode_bytes;
use crate::process::suppress_window;
use crate::types::AdapterInfo;
use serde::Deserialize;
use std::fs;
use std::process::Command;

/// 判断当前进程是否具备管理员权限
pub fn is_admin() -> bool {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-Command",
        "[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
    ]);
    suppress_window(&mut cmd);
    match cmd.output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "True",
        Err(_) => false,
    }
}

fn netsh(args: &[&str]) -> Result<(bool, String), String> {
    let mut cmd = Command::new("netsh");
    cmd.args(args);
    suppress_window(&mut cmd);
    let output = cmd.output().map_err(|e| format!("无法调用 netsh：{e}"))?;
    let stdout = decode_bytes(&output.stdout).trim().to_string();
    let stderr = decode_bytes(&output.stderr).trim().to_string();
    let combined = if stdout.is_empty() { stderr } else { stdout };
    let denied = combined.contains("拒绝访问")
        || combined.contains("Access is denied")
        || combined.contains("需要提升")
        || combined.contains("elevation");
    if denied && !output.status.success() {
        return Err("ELEVATION_REQUIRED".to_string());
    }
    Ok((output.status.success(), combined))
}

const LIST_SCRIPT: &str = r#"$ErrorActionPreference = 'SilentlyContinue'
$nics = Get-NetAdapter | Select-Object Name, InterfaceDescription, Status, AdminStatus, MacAddress, LinkSpeed, InterfaceGuid, ifIndex
$wmi = @(Get-CimInstance Win32_NetworkAdapterConfiguration)
$result = foreach ($n in $nics) {
  $w = $wmi | Where-Object { $_.InterfaceIndex -eq $n.ifIndex } | Select-Object -First 1
  $gw = $null
  if ($w -and $w.DefaultIPGateway) { $gw = @($w.DefaultIPGateway)[0] }
  [PSCustomObject]@{
    name = [string]$n.Name
    description = [string]$n.InterfaceDescription
    interface_guid = if ($n.InterfaceGuid) { [string]$n.InterfaceGuid } else { $null }
    if_index = $n.ifIndex
    mac = if ($n.MacAddress) { [string]$n.MacAddress } else { $null }
    status = if ($n.AdminStatus -eq 'Down' -or $n.Status -eq 'Disabled') { 'disabled' } elseif ($n.Status -eq 'Up') { 'connected' } else { 'disconnected' }
    link_speed = if ($n.LinkSpeed) { [string]$n.LinkSpeed } else { $null }
    dhcp_enabled = if ($w) { [bool]$w.DHCPEnabled } else { $null }
    ipv4 = @($w.IPAddress | Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' })
    subnet_mask = @($w.IPSubnet | Where-Object { $_ })
    gateway = $gw
    dns = @($w.DNSServerSearchOrder | Where-Object { $_ })
  }
}
$result = @($result | Group-Object if_index | ForEach-Object { $_.Group[0] })
$json = $result | ConvertTo-Json -Depth 4 -Compress
[IO.File]::WriteAllText($env:IPS_LIST_OUT, $json, (New-Object System.Text.UTF8Encoding($false)))
"#;

/// 枚举系统全部网卡及其当前配置
#[tauri::command]
pub fn list_adapters() -> Result<Vec<AdapterInfo>, String> {
    let script_path = std::env::temp_dir().join("ipswitcher_list_adapters.ps1");
    let json_path = std::env::temp_dir().join("ipswitcher_list_adapters.json");
    fs::write(&script_path, LIST_SCRIPT).map_err(|e| format!("临时脚本写入失败：{e}"))?;

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_path.to_string_lossy(),
    ]);
    cmd.env("IPS_LIST_OUT", &json_path);
    suppress_window(&mut cmd);
    let output = cmd.output().map_err(|e| format!("网卡信息读取失败：{e}"))?;

    let _ = fs::remove_file(&script_path);

    let text = match fs::read_to_string(&json_path) {
        Ok(t) => t,
        Err(e) => {
            let err = decode_bytes(&output.stderr).trim().to_string();
            return Err(format!("网卡信息读取失败：{e}；{err}"));
        }
    };
    let _ = fs::remove_file(&json_path);

    let text = text.trim_start_matches('\u{feff}').trim().to_string();
    if text.is_empty() {
        let err = decode_bytes(&output.stderr).trim().to_string();
        return Err(format!("未读取到网卡信息：{err}"));
    }

    let trimmed = text.trim();
    let arr_text = if trimmed.starts_with('[') {
        trimmed.to_string()
    } else {
        format!("[{trimmed}]")
    };
    let adapters: Vec<AdapterInfo> =
        serde_json::from_str(&arr_text).map_err(|e| format!("网卡信息解析失败：{e}"))?;
    Ok(adapters)
}

#[derive(Debug, Deserialize)]
pub struct StaticInput {
    pub name: String,
    pub ip: String,
    pub mask: String,
    pub gateway: Option<String>,
    pub dns1: Option<String>,
    pub dns2: Option<String>,
}

/// 应用静态 IP 配置（可同步配置 DNS）
#[tauri::command]
pub fn apply_static(input: StaticInput) -> Result<String, String> {
    if !is_admin() {
        return Err("ELEVATION_REQUIRED".to_string());
    }

    let name_arg = format!("name={}", input.name);
    let mut addr_args: Vec<&str> = vec![
        "interface",
        "ipv4",
        "set",
        "address",
        &name_arg,
        "static",
        &input.ip,
        &input.mask,
    ];
    if let Some(gw) = input.gateway.as_deref().filter(|g| !g.is_empty()) {
        addr_args.push(gw);
        addr_args.push("1");
    }
    let (ok, msg) = netsh(&addr_args)?;
    if !ok {
        return Err(format!("IP 地址设置失败：{msg}"));
    }

    if let Some(dns1) = input.dns1.as_deref().filter(|d| !d.is_empty()) {
        let dns_args = vec![
            "interface",
            "ipv4",
            "set",
            "dns",
            &name_arg,
            "static",
            dns1,
            "primary",
        ];
        let (ok, msg) = netsh(&dns_args)?;
        if !ok {
            return Err(format!("首选 DNS 设置失败：{msg}"));
        }
        if let Some(dns2) = input.dns2.as_deref().filter(|d| !d.is_empty()) {
            let add_args = vec![
                "interface",
                "ipv4",
                "add",
                "dns",
                &name_arg,
                dns2,
                "index=2",
            ];
            let (ok, msg) = netsh(&add_args)?;
            if !ok {
                return Err(format!("备用 DNS 设置失败：{msg}"));
            }
        }
    }

    Ok(format!("已为「{}」应用静态 IP：{}", input.name, input.ip))
}

/// 切换为 DHCP 自动获取；reset_dns 同时将 DNS 恢复为自动获取
#[tauri::command]
pub fn apply_dhcp(name: String, reset_dns: bool) -> Result<String, String> {
    if !is_admin() {
        return Err("ELEVATION_REQUIRED".to_string());
    }
    let name_arg = format!("name={name}");

    let (ok, msg) = netsh(&[
        "interface",
        "ipv4",
        "set",
        "address",
        &name_arg,
        "source=dhcp",
    ])?;
    if !ok {
        return Err(format!("DHCP 切换失败：{msg}"));
    }

    if reset_dns {
        let (ok, msg) = netsh(&[
            "interface",
            "ipv4",
            "set",
            "dns",
            &name_arg,
            "source=dhcp",
        ])?;
        if !ok {
            return Err(format!("DNS 自动获取切换失败：{msg}"));
        }
    }

    Ok(format!("已为「{name}」切换为 DHCP 自动获取"))
}

/// 仅修改 DNS 服务器
#[tauri::command]
pub fn apply_dns(name: String, dns1: String, dns2: Option<String>) -> Result<String, String> {
    if !is_admin() {
        return Err("ELEVATION_REQUIRED".to_string());
    }
    let name_arg = format!("name={name}");

    let (ok, msg) = netsh(&[
        "interface",
        "ipv4",
        "set",
        "dns",
        &name_arg,
        "static",
        &dns1,
        "primary",
    ])?;
    if !ok {
        return Err(format!("首选 DNS 设置失败：{msg}"));
    }
    if let Some(dns2) = dns2.as_deref().filter(|d| !d.is_empty()) {
        let (ok, msg) = netsh(&[
            "interface",
            "ipv4",
            "add",
            "dns",
            &name_arg,
            dns2,
            "index=2",
        ])?;
        if !ok {
            return Err(format!("备用 DNS 设置失败：{msg}"));
        }
    }
    Ok(format!("已为「{name}」更新 DNS 服务器"))
}

/// 启用 / 禁用指定网卡（netsh interface set interface admin=enabled|disabled）
#[tauri::command]
pub fn set_adapter_enabled(name: String, enabled: bool) -> Result<String, String> {
    if !is_admin() {
        return Err("ELEVATION_REQUIRED".to_string());
    }
    let name_arg = format!("name={name}");
    let admin = if enabled { "enabled" } else { "disabled" };
    let (ok, msg) = netsh(&[
        "interface",
        "set",
        "interface",
        &name_arg,
        &format!("admin={admin}"),
    ])?;
    if !ok {
        return Err(format!(
            "网卡{}失败：{msg}",
            if enabled { "启用" } else { "禁用" }
        ));
    }
    Ok(format!(
        "已{}网卡「{name}」",
        if enabled { "启用" } else { "禁用" }
    ))
}

/// 打开系统「网络连接」面板（ncpa.cpl）
#[tauri::command]
pub fn open_network_connections() -> Result<(), String> {
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", "start", "", "ncpa.cpl"]);
    suppress_window(&mut cmd);
    cmd.spawn().map_err(|e| format!("打开网络连接失败：{e}"))?;
    Ok(())
}

/// 网卡名称中不允许出现的字符（Windows 连接名规则 + 命令行安全）
const RENAME_INVALID_CHARS: [char; 9] = ['\\', '/', ':', '"', '*', '?', '<', '>', '|'];

const RENAME_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
try {
  Rename-NetAdapter -Name $env:IPS_OLD_NAME -NewName $env:IPS_NEW_NAME -ErrorAction Stop
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 1
}
"#;

/// 重命名网卡（Rename-NetAdapter）
#[tauri::command]
pub fn rename_adapter(name: String, new_name: String) -> Result<String, String> {
    if !is_admin() {
        return Err("ELEVATION_REQUIRED".to_string());
    }
    let new_name = new_name.trim().to_string();
    if new_name.is_empty() {
        return Err("网卡名称不能为空".to_string());
    }
    if new_name.chars().count() > 32 {
        return Err("网卡名称不能超过 32 个字符".to_string());
    }
    if RENAME_INVALID_CHARS.iter().any(|c| new_name.contains(*c)) {
        return Err("网卡名称不能包含 \\ / : \" * ? < > | 字符".to_string());
    }
    if new_name == name {
        return Ok("名称未变更".to_string());
    }

    let script_path = std::env::temp_dir().join("ipswitcher_rename.ps1");
    fs::write(&script_path, RENAME_SCRIPT).map_err(|e| format!("临时脚本写入失败：{e}"))?;

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_path.to_string_lossy(),
    ]);
    cmd.env("IPS_OLD_NAME", &name);
    cmd.env("IPS_NEW_NAME", &new_name);
    suppress_window(&mut cmd);
    let output = cmd.output().map_err(|e| format!("重命名失败：{e}"))?;
    let _ = fs::remove_file(&script_path);

    let stderr = decode_bytes(&output.stderr).trim().to_string();
    let stdout = decode_bytes(&output.stdout).trim().to_string();
    let raw = if stderr.is_empty() { stdout.clone() } else { stderr.clone() };
    let lower = raw.to_lowercase();
    let looks_like_error = !output.status.success()
        || lower.contains("cimexception")
        || lower.contains("fullyqualifiederrorid")
        || lower.contains("object exists")
        || lower.contains("resourceexists")
        || raw.contains("698");
    if looks_like_error {
        let is_in_use = raw.contains("698")
            || lower.contains("object exists")
            || lower.contains("resourceexists")
            || raw.contains("已存在");
        if is_in_use {
            return Err("NAME_IN_USE_RESERVED".to_string());
        }
        return Err(format!("网卡重命名失败：{raw}"));
    }
    Ok(format!("已将「{name}」重命名为「{new_name}」"))
}
