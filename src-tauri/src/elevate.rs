use crate::config::config_path;
use crate::netcfg::{apply_dhcp, apply_dns, apply_static, rename_adapter, set_adapter_enabled, StaticInput};
use crate::process::suppress_window;
use serde::{Deserialize, Serialize};
use std::fs;
use tauri::AppHandle;

/// 提权后需要执行的待决操作（names 支持批量网卡）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PendingAction {
    Static {
        names: Vec<String>,
        ip: String,
        mask: String,
        gateway: Option<String>,
        dns1: Option<String>,
        dns2: Option<String>,
    },
    Dhcp {
        names: Vec<String>,
        reset_dns: bool,
    },
    Dns {
        names: Vec<String>,
        dns1: String,
        dns2: Option<String>,
    },
    Toggle {
        names: Vec<String>,
        enabled: bool,
    },
    Rename {
        name: String,
        new_name: String,
    },
}

#[derive(Debug, Serialize)]
struct PendingResult {
    ok: bool,
    message: String,
}

fn pending_path(app: &AppHandle) -> std::path::PathBuf {
    config_path(app).with_file_name("pending_action.json")
}

fn result_path(app: &AppHandle) -> std::path::PathBuf {
    config_path(app).with_file_name("pending_result.json")
}

/// 把待决操作落盘后，以管理员权限重新启动本程序（触发 UAC 提权）。
#[tauri::command]
pub fn elevate_action(app: AppHandle, action: PendingAction) -> Result<(), String> {
    let path = pending_path(&app);
    let text = serde_json::to_string_pretty(&action).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("待执行操作写入失败：{e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("定位程序路径失败：{e}"))?;
    let exe_q = exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "Start-Process -FilePath '{}' -ArgumentList '--elevate' -Verb RunAs",
        exe_q
    );
    let mut child_cmd = std::process::Command::new("powershell");
    child_cmd.args(["-NoProfile", "-Command", &ps]);
    suppress_window(&mut child_cmd);
    let mut child = child_cmd
        .spawn()
        .map_err(|e| format!("提权启动失败：{e}"))?;
    child.wait().map_err(|e| format!("提权启动失败：{e}"))?;
    Ok(())
}

/// 以管理员权限重启本程序（无待决操作）。已是管理员时返回 Ok(false)，不重启；
/// 成功发起提权后直接退出当前实例，避免标准权限与提权实例双开占用文件。
/// 提权后的实例带 `--elevated-restart` 标记，不再重复弹询问。
#[tauri::command]
pub fn restart_as_admin() -> Result<bool, String> {
    if crate::netcfg::is_admin() {
        return Ok(false);
    }
    let exe = std::env::current_exe().map_err(|e| format!("定位程序路径失败：{e}"))?;
    let exe_q = exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "Start-Process -FilePath '{}' -ArgumentList '--elevated-restart' -Verb RunAs",
        exe_q
    );
    let mut child_cmd = std::process::Command::new("powershell");
    child_cmd.args(["-NoProfile", "-Command", &ps]);
    suppress_window(&mut child_cmd);
    let mut child = child_cmd
        .spawn()
        .map_err(|e| format!("提权重启失败：{e}"))?;
    let status = child
        .wait()
        .map_err(|e| format!("提权重启失败：{e}"))?;
    if !status.success() {
        return Err("提权已取消：未通过管理员权限授权".to_string());
    }
    // 提权实例已由独立进程拉起，退出当前标准权限实例
    std::process::exit(0);
}

/// 启动时是否应弹出“是否以管理员身份运行”询问：非管理员、非提权重启实例，
/// 且用户尚未在询问中选择“暂不”（该选择记入配置，可在设置页手动启用管理员权限）。
#[tauri::command]
pub fn should_prompt_admin(app: AppHandle) -> Result<bool, String> {
    let elevated_restart = std::env::args().any(|a| a == "--elevated-restart");
    if elevated_restart || crate::netcfg::is_admin() {
        return Ok(false);
    }
    let cfg = crate::config::load_config(&app);
    Ok(!cfg.admin_prompt_dismissed.unwrap_or(false))
}

/// 提权实例启动后执行待决操作，写回结果并弹出提示框。
pub fn process_pending_action(app: &AppHandle) {
    let path = pending_path(app);
    let action: Option<PendingAction> = match fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str::<PendingAction>(&t).ok(),
        Err(_) => None,
    };

    if let Some(action) = action {
        let mut failures: Vec<String> = Vec::new();
        let mut success_count = 0;

        match &action {
            PendingAction::Static {
                names,
                ip,
                mask,
                gateway,
                dns1,
                dns2,
            } => {
                for name in names {
                    let input = StaticInput {
                        name: name.clone(),
                        ip: ip.clone(),
                        mask: mask.clone(),
                        gateway: gateway.clone(),
                        dns1: dns1.clone(),
                        dns2: dns2.clone(),
                    };
                    match apply_static(input) {
                        Ok(_) => success_count += 1,
                        Err(e) => failures.push(format!("{name}：{e}")),
                    }
                }
            }
            PendingAction::Dhcp { names, reset_dns } => {
                for name in names {
                    match apply_dhcp(name.clone(), *reset_dns) {
                        Ok(_) => success_count += 1,
                        Err(e) => failures.push(format!("{name}：{e}")),
                    }
                }
            }
            PendingAction::Dns {
                names,
                dns1,
                dns2,
            } => {
                for name in names {
                    match apply_dns(name.clone(), dns1.clone(), dns2.clone()) {
                        Ok(_) => success_count += 1,
                        Err(e) => failures.push(format!("{name}：{e}")),
                    }
                }
            }
            PendingAction::Toggle { names, enabled } => {
                for name in names {
                    match set_adapter_enabled(name.clone(), *enabled) {
                        Ok(_) => success_count += 1,
                        Err(e) => failures.push(format!("{name}：{e}")),
                    }
                }
            }
            PendingAction::Rename { name, new_name } => {
                match rename_adapter(name.clone(), new_name.clone()) {
                    Ok(_) => success_count += 1,
                    Err(e) => {
                        let friendly = if e == "NAME_IN_USE_RESERVED" {
                            "该名称已被残留网络连接占用，请更换名称，或先在设备管理器卸载隐藏的旧适配器".to_string()
                        } else {
                            e
                        };
                        failures.push(format!("{name}：{friendly}"));
                    }
                }
            }
        }

        let (ok, message) = if failures.is_empty() {
            (true, format!("操作成功，共应用到 {success_count} 张网卡"))
        } else {
            (false, failures.join("；"))
        };
        let _ = fs::remove_file(&path);
        let _ = fs::write(
            result_path(app),
            serde_json::to_string_pretty(&PendingResult {
                ok,
                message: message.clone(),
            })
            .unwrap_or_default(),
        );
        show_message_box(ok, &message);
    }
}

fn show_message_box(ok: bool, message: &str) {
    let title = if ok { "操作成功" } else { "操作失败" };
    let icon = if ok { "Information" } else { "Exclamation" };
    let body = message.replace('\'', "''");
    let title_q = title.replace('\'', "''");
    let ps = format!(
        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('{}','{}','OK','{}') | Out-Null",
        body, title_q, icon
    );
    let mut msg_cmd = std::process::Command::new("powershell");
    msg_cmd.args(["-NoProfile", "-Command", &ps]);
    suppress_window(&mut msg_cmd);
    let _ = msg_cmd.spawn();
}
