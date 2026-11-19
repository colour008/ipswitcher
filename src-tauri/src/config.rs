use crate::types::AppConfig;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// 是否位于标准受保护安装目录（Program Files / Windows），据此判定为安装版
fn under_protected_install_dir(dir: &Path) -> bool {
    let lower = dir.to_string_lossy().to_lowercase();
    lower.contains("program files") || lower.starts_with("c:\\windows")
}

/// 判断当前是否为便携版（绿色版，配置随 exe 走）：
/// - exe 同级存在 `installed.marker`：强制按安装版处理；
/// - exe 同级存在 `portable.marker`：强制按便携版处理；
/// - 无标记时：exe 位于 Program Files / Windows 等受保护目录视为安装版，
///   其余普通目录（桌面、下载、U盘、解压目录等）视为绿色版便携存储。
fn is_portable() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("installed.marker").exists() {
                return false;
            }
            if dir.join("portable.marker").exists() {
                return true;
            }
            return !under_protected_install_dir(dir);
        }
    }
    false
}

/// 配置文件路径：
/// - 便携版：软件同级目录 config.json（U 盘携带，配置随包走）
/// - 安装版：系统 AppData 配置目录
pub fn config_path(app: &AppHandle) -> PathBuf {
    if is_portable() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("config.json");
            }
        }
    }
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = fs::create_dir_all(&dir);
    dir.join("config.json")
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    let path = config_path(app);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<AppConfig>(&text).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app);
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("配置写入失败：{e}"))
}
