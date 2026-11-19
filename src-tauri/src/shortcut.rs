use crate::process::suppress_window;
use std::path::Path;
use std::process::Command;

fn current_exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("定位程序路径失败：{e}"))?;
    Ok(exe.to_string_lossy().into_owned())
}

/// PowerShell 单引号字符串内的单引号需双写转义
fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// 在当前用户桌面创建指向本程序的 IPSwitcher 快捷方式（.lnk）。
/// 安装版与便携版均可调用；便携版不经安装器，由用户手动触发。
#[tauri::command]
pub fn create_desktop_shortcut() -> Result<(), String> {
    let exe = current_exe()?;
    let exe_dir = Path::new(&exe)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$ws = New-Object -ComObject WScript.Shell
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop 'IPSwitcher.lnk'
$sc = $ws.CreateShortcut($lnk)
$sc.TargetPath = '{exe}'
$sc.WorkingDirectory = '{dir}'
$sc.IconLocation = '{exe},0'
$sc.Description = 'IPSwitcher 网卡IP快速切换工具'
$sc.Save()
"#,
        exe = ps_quote(&exe),
        dir = ps_quote(&exe_dir),
    );

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    suppress_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("创建快捷方式失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "创建快捷方式失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}
