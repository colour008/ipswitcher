use crate::config::{load_config, save_config};
use crate::process::suppress_window;
use std::process::Command;
use tauri::{menu::CheckMenuItem, AppHandle, Manager};

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "IPSwitcher";

/// 托盘「开机自启」勾选项句柄，用于设置面板/命令切换时同步勾选状态
pub struct AutostartMenu(pub CheckMenuItem<tauri::Wry>);

fn current_exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("定位程序路径失败：{e}"))?;
    Ok(exe.to_string_lossy().into_owned())
}

/// 注册表中是否已登记开机自启（以实际登记状态为准）
pub fn is_registered() -> bool {
    let mut cmd = Command::new("reg");
    cmd.args(["query", RUN_KEY, "/v", VALUE_NAME]);
    suppress_window(&mut cmd);
    match cmd.output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

fn register() -> Result<(), String> {
    let exe = current_exe()?;
    let mut cmd = Command::new("reg");
    cmd.args([
        "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &exe, "/f",
    ]);
    suppress_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("写入注册表失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "写入注册表失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn unregister() -> Result<(), String> {
    // 项本就不存在时直接视为已移除，避免 reg delete 报错
    if !is_registered() {
        return Ok(());
    }
    let mut cmd = Command::new("reg");
    cmd.args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"]);
    suppress_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("移除注册表项失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "移除注册表项失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// 按目标状态登记/移除开机自启，持久化配置并同步托盘勾选
pub fn apply(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        register()?;
    } else {
        unregister()?;
    }
    let mut cfg = load_config(app);
    cfg.autostart = Some(enabled);
    let _ = save_config(app, &cfg);
    if let Some(menu) = app.try_state::<AutostartMenu>() {
        let _ = menu.0.set_checked(enabled);
    }
    Ok(())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    apply(&app, enabled)
}

#[tauri::command]
pub fn get_autostart() -> Result<bool, String> {
    Ok(is_registered())
}
