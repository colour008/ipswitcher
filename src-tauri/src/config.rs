use crate::types::{AppConfig, HostHistory};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// 是否位于标准受保护安装目录（Program Files / Windows），据此判定为安装版
fn under_protected_install_dir(dir: &Path) -> bool {
    let lower = dir.to_string_lossy().to_lowercase();
    lower.contains("program files") || lower.starts_with("c:\\windows")
}

/// 程序分发形态：安装版（Program Files 等受保护目录）/ 便携目录（含 portable.marker）/ 绿色单文件
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallFlavor {
    Installed,
    Portable,
    Standalone,
}

/// 判断当前安装形态：
/// - exe 同级存在 `installed.marker`：强制按安装版处理；
/// - exe 同级存在 `portable.marker`：强制按便携目录处理；
/// - 无标记时：exe 位于 Program Files / Windows 等受保护目录视为安装版，
///   其余普通目录（桌面、下载、U盘、解压目录等）视为绿色单文件。
pub fn install_flavor() -> InstallFlavor {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("installed.marker").exists() {
                return InstallFlavor::Installed;
            }
            if dir.join("portable.marker").exists() {
                return InstallFlavor::Portable;
            }
            if under_protected_install_dir(dir) {
                return InstallFlavor::Installed;
            }
            return InstallFlavor::Standalone;
        }
    }
    InstallFlavor::Installed
}

/// 判断当前是否为便携/绿色版（配置随 exe 走）：安装版之外均按绿色存储处理
fn is_portable() -> bool {
    !matches!(install_flavor(), InstallFlavor::Installed)
}

/// 在 Tauri AppHandle 创建前读取启动偏好，供 Windows 入口决定是否需要 UAC 提权。
pub fn load_startup_config() -> AppConfig {
    let path = if is_portable() {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("config.json")))
    } else {
        std::env::var_os("APPDATA").map(|app_data| {
            PathBuf::from(app_data)
                .join("com.jamhoo.ipswitcher")
                .join("config.json")
        })
    };
    path.and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<AppConfig>(&text).ok())
        .unwrap_or_default()
}

/// 更新 Tauri 启动前使用的配置文件；便携版与安装版路径和 `config_path` 保持一致。
pub fn save_startup_config(config: &AppConfig) -> Result<(), String> {
    let path = if is_portable() {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("config.json")))
    } else {
        std::env::var_os("APPDATA").map(|app_data| {
            PathBuf::from(app_data)
                .join("com.jamhoo.ipswitcher")
                .join("config.json")
        })
    }
    .ok_or_else(|| "无法定位软件配置目录".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("配置写入失败：{e}"))
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

static HOST_HISTORY_LOCK: Mutex<()> = Mutex::new(());

fn history_path(app: &AppHandle) -> PathBuf {
    config_path(app).with_file_name("host-history.json")
}

pub fn load_host_history(app: &AppHandle) -> Result<Vec<HostHistory>, String> {
    match fs::read(history_path(app)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(load_config(app).host_history),
        Err(e) => Err(e.to_string()),
    }
}

fn write_host_history(app: &AppHandle, history: &[HostHistory]) -> Result<(), String> {
    let path = history_path(app);
    let temporary = path.with_extension("json.tmp");
    let data = serde_json::to_vec(history).map_err(|e| e.to_string())?;
    fs::write(&temporary, data).map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

pub fn migrate_host_history(app: &AppHandle) -> Result<(), String> {
    let mut config = load_config(app);
    if config.host_history.is_empty() {
        return Ok(());
    }
    // 若上次在写入新文件后退出，验证新文件后即可继续清理旧字段。
    if history_path(app).try_exists().map_err(|e| e.to_string())? {
        let _ = load_host_history(app)?;
    } else {
        write_host_history(app, &config.host_history)?;
    }
    config.host_history.clear();
    save_config(app, &config)
}

pub fn update_host_history(
    app: &AppHandle,
    update: impl FnOnce(&mut Vec<HostHistory>),
) -> Result<(), String> {
    let _guard = HOST_HISTORY_LOCK.lock().map_err(|_| "扫描历史不可用".to_string())?;
    let mut history = load_host_history(app)?;
    update(&mut history);
    write_host_history(app, &history)
}
