use crate::encoding::decode_bytes;
use crate::process::suppress_window;
use crate::types::AdapterInfo;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(target_os = "windows")]
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfEntry2, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};

/// 判断当前进程是否具备管理员权限
#[cfg(target_os = "windows")]
pub fn is_admin() -> bool {
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::{
        CheckTokenMembership, CreateWellKnownSid, WinBuiltinAdministratorsSid, SECURITY_MAX_SID_SIZE,
    };

    let mut sid = [0u32; (SECURITY_MAX_SID_SIZE as usize).div_ceil(4)];
    let mut size = SECURITY_MAX_SID_SIZE;
    let mut member = 0;
    let sid = sid.as_mut_ptr().cast();
    unsafe {
        CreateWellKnownSid(WinBuiltinAdministratorsSid, null_mut(), sid, &mut size) != 0
            && CheckTokenMembership(null_mut(), sid, &mut member) != 0
            && member != 0
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_admin() -> bool {
    false
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

const LIST_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
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
ConvertTo-Json -InputObject @($result) -Depth 4 -Compress
"#;

#[derive(Clone, Serialize)]
pub struct AdapterSnapshot {
    revision: u64,
    adapters: Vec<AdapterInfo>,
}

#[derive(Default)]
pub struct AdapterSnapshots {
    cache: Mutex<Option<(Instant, AdapterSnapshot)>>,
    refresh_due: Mutex<Option<Instant>>,
}

fn enumerate_adapters() -> Result<Vec<AdapterInfo>, String> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", LIST_SCRIPT]);
    suppress_window(&mut cmd);
    let output = cmd.output().map_err(|e| format!("网卡信息读取失败：{e}"))?;
    if !output.status.success() {
        return Err(format!("网卡信息读取失败：{}", decode_bytes(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("网卡信息解析失败：{e}"))
}

fn refresh_adapter_snapshot(app: &AppHandle, force: bool, requested_at: Instant) -> Result<AdapterSnapshot, String> {
        let state = app.state::<AdapterSnapshots>();
        let mut cache = state.cache.lock().map_err(|_| "网卡缓存不可用".to_string())?;
        if let Some((updated, snapshot)) = cache.as_ref() {
            if *updated >= requested_at || (!force && updated.elapsed() < Duration::from_secs(2)) {
                return Ok(snapshot.clone());
            }
        }

        let adapters = enumerate_adapters()?;
        let completed_at = Instant::now();
        if let Some((updated, existing)) = cache.as_mut() {
            if existing.adapters == adapters {
                *updated = completed_at;
                return Ok(existing.clone());
            }
        }
        let revision = cache.as_ref().map_or(1, |(_, snapshot)| snapshot.revision + 1);
        let snapshot = AdapterSnapshot { revision, adapters };
        *cache = Some((completed_at, snapshot.clone()));
        let _ = app.emit("adapters-changed", &snapshot);
        Ok(snapshot)
}

/// 两个窗口共用缓存。请求若与上一次实际查询重叠，则直接复用其结果。
#[tauri::command]
pub async fn list_adapters(app: AppHandle, force: bool) -> Result<AdapterSnapshot, String> {
    let requested_at = Instant::now();
    tauri::async_runtime::spawn_blocking(move || refresh_adapter_snapshot(&app, force, requested_at))
    .await
    .map_err(|e| format!("网卡查询任务失败：{e}"))?
}

/// 共用一次延迟复查，发起操作的窗口关闭后仍会执行。
#[tauri::command]
pub fn schedule_adapter_refresh(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AdapterSnapshots>();
    let due = Instant::now() + Duration::from_millis(3500);
    let already_pending = {
        let mut pending = state.refresh_due.lock().map_err(|_| "网卡缓存不可用".to_string())?;
        pending.replace(due).is_some()
    };
    if already_pending {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AdapterSnapshots>();
        loop {
            let delay = {
                let Ok(mut pending) = state.refresh_due.lock() else { return };
                let Some(due) = *pending else { return };
                let delay = due.saturating_duration_since(Instant::now());
                if delay.is_zero() {
                    *pending = None;
                }
                delay
            };
            if delay.is_zero() {
                let _ = refresh_adapter_snapshot(&app, true, Instant::now());
                break;
            }
            std::thread::sleep(delay);
        }
    });
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AdapterTrafficCounters {
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub sampled_at: u64,
}

#[cfg(target_os = "windows")]
type TrafficSamples = std::collections::HashMap<(Option<u32>, String), (Instant, AdapterTrafficCounters)>;

#[derive(Default)]
pub struct TrafficCache {
    #[cfg(target_os = "windows")]
    samples: Mutex<TrafficSamples>,
}

/// GetIfTable2 分配的网卡表由 FreeMibTable 释放。
#[cfg(target_os = "windows")]
struct IfTable(*mut MIB_IF_TABLE2);

#[cfg(target_os = "windows")]
impl Drop for IfTable {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn read_interface(name: &str, if_index: Option<u32>) -> Result<MIB_IF_ROW2, String> {
    let name_utf16: Vec<u16> = name.encode_utf16().collect();
    let matches_alias = |row: &MIB_IF_ROW2| {
        let end = row.Alias.iter().position(|unit| *unit == 0).unwrap_or(row.Alias.len());
        row.Alias[..end] == name_utf16
    };
    if let Some(index) = if_index.filter(|index| *index != 0) {
        let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
        row.InterfaceIndex = index;
        if unsafe { GetIfEntry2(&mut row) } == 0 && matches_alias(&row) {
            return Ok(row);
        }
    }

    // 旧快照的索引可能在网卡重启后变化，按别名保留兼容路径。
    let mut raw_table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    let result = unsafe { GetIfTable2(&mut raw_table) };
    let table = IfTable(raw_table);
    if result != 0 {
        return Err(format!("读取网卡统计失败（Windows 错误码 {result}）"));
    }
    if table.0.is_null() {
        return Err("Windows 未返回网卡统计表".to_string());
    }

    unsafe {
        let table_ref = &*table.0;
        std::slice::from_raw_parts(
            table_ref.Table.as_ptr(),
            table_ref.NumEntries as usize,
        )
        .iter()
        .find(|row| matches_alias(row))
        .copied()
    }
    .ok_or_else(|| format!("未找到网卡“{name}”的统计信息"))
}

/// 以约一秒的粒度共享同一接口的原生采样，并返回真实采样时间。
#[cfg(target_os = "windows")]
#[tauri::command]
pub fn get_adapter_traffic(
    state: tauri::State<'_, TrafficCache>,
    name: String,
    if_index: Option<u32>,
) -> Result<AdapterTrafficCounters, String> {
    let mut samples = state.samples.lock().map_err(|_| "流量缓存不可用".to_string())?;
    samples.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(5));
    let key = (if_index, name);
    if let Some((at, counters)) = samples.get(&key) {
        if at.elapsed() < Duration::from_millis(900) {
            return Ok(*counters);
        }
    }
    let row = read_interface(&key.1, if_index)?;
    let counters = AdapterTrafficCounters {
        sent_bytes: row.OutOctets,
        received_bytes: row.InOctets,
        sampled_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
    };
    samples.insert(key, (Instant::now(), counters));
    Ok(counters)
}

/// 桌面版本面向 Windows；该占位实现让其他平台仍可完成编译。
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn get_adapter_traffic(name: String, if_index: Option<u32>) -> Result<AdapterTrafficCounters, String> {
    let _ = (name, if_index);
    Err("流量监控仅支持 Windows".to_string())
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
