use crate::config::{install_flavor, InstallFlavor};
use crate::process::suppress_window;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const REPO_OWNER: &str = "colour008";
const REPO_NAME: &str = "ipswitcher";
const USER_AGENT: &str = "ipswitcher-updater";
/// 更新包临时目录（系统临时目录下）
const UPDATE_TMP_DIR: &str = "ipswitcher-update";
/// GitHub 加速前缀：直连失败后按顺序回退。公益代理可用性会变化，集中在此便于后续调整。
const PROXY_PREFIXES: &[&str] = &[
    "https://gh-proxy.com",
    "https://ghproxy.net",
    "https://ghfast.top",
];

// ---------- 向前端返回的数据结构 ----------

/// 检查更新结果（camelCase 与前端类型对应）
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    /// Release Notes 正文（纯文本展示）
    pub notes: String,
    /// 当前分发形态：installed / portable / standalone
    pub flavor: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub asset_size: u64,
    pub asset_digest: Option<String>,
    /// 无可匹配资产或其它需要提示前端的信息
    pub message: Option<String>,
}

/// 下载进度事件载荷
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: u64,
}

/// 下载被用户取消时返回的固定错误文案，前端据此区分“取消”与“失败”
pub const DOWNLOAD_CANCELLED: &str = "更新已取消";

/// 跨命令共享的下载取消标志：`download_update` 开始时复位，
/// `cancel_download` 置位后进行中的下载在下一数据块退出。
pub struct DownloadCancel(pub AtomicBool);

// ---------- GitHub API 响应结构 ----------

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    prerelease: Option<bool>,
    draft: Option<bool>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn flavor_str(flavor: InstallFlavor) -> &'static str {
    match flavor {
        InstallFlavor::Installed => "installed",
        InstallFlavor::Portable => "portable",
        InstallFlavor::Standalone => "standalone",
    }
}

/// 解析语义化版本为 (major, minor, patch)，忽略前缀 v 与预发布/构建后缀
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().strip_prefix('v').unwrap_or(v.trim()).trim();
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// 按当前分发形态挑选匹配的更新资产
fn pick_asset<'a>(flavor: InstallFlavor, assets: &'a [GithubAsset]) -> Option<&'a GithubAsset> {
    match flavor {
        InstallFlavor::Installed => assets.iter().find(|a| {
            let n = a.name.to_lowercase();
            n.ends_with(".exe") && n.contains("setup")
        }),
        InstallFlavor::Portable => assets
            .iter()
            .find(|a| a.name.to_lowercase().ends_with("_portable.zip")),
        InstallFlavor::Standalone => assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case("ipswitcher.exe")),
    }
}

/// 生成候选访问地址：先直连原始地址，再依次拼接加速前缀（前缀 + 完整原始 URL）
fn fallback_urls(original: &str) -> Vec<String> {
    let mut urls = Vec::with_capacity(1 + PROXY_PREFIXES.len());
    urls.push(original.to_string());
    for prefix in PROXY_PREFIXES {
        urls.push(format!("{prefix}/{original}"));
    }
    urls
}

async fn fetch_release() -> Result<GithubRelease, String> {
    let api_url = format!(
        "https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest"
    );
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;

    let mut last_err = String::from("检查更新失败：直连与加速节点均不可用");
    for url in fallback_urls(&api_url) {
        let resp = match client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("检查更新失败：{e}");
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("检查更新失败：HTTP {}", resp.status().as_u16());
            continue;
        }
        match resp.json::<GithubRelease>().await {
            Ok(release) => {
                if release.prerelease.unwrap_or(false) || release.draft.unwrap_or(false) {
                    return Err("当前最新版本为预发布版本，已跳过".to_string());
                }
                return Ok(release);
            }
            Err(e) => {
                last_err = format!("解析版本信息失败：{e}");
                continue;
            }
        }
    }
    Err(last_err)
}

/// 检查是否存在更新；网络/解析失败时返回 Err，由调用方决定静默忽略或提示
#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let flavor = install_flavor();
    let release = fetch_release().await?;

    let has_update = match (parse_version(&current), parse_version(&release.tag_name)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    };

    let asset = pick_asset(flavor, &release.assets);
    let mut result = UpdateInfo {
        has_update,
        current_version: current,
        latest_version: release.tag_name,
        notes: release.body.unwrap_or_default(),
        flavor: flavor_str(flavor).to_string(),
        asset_name: asset.map(|a| a.name.clone()),
        asset_url: asset.map(|a| a.browser_download_url.clone()),
        asset_size: asset.map(|a| a.size).unwrap_or(0),
        asset_digest: asset.and_then(|a| a.digest.clone()),
        message: None,
    };

    if has_update && asset.is_none() {
        result.message = Some(
            "当前分发形态暂未匹配到更新包，请前往 Release 页面手动下载".to_string(),
        );
    }
    Ok(result)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("打开临时文件失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("读取临时文件失败：{e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 校验已落盘的更新包：优先 SHA256，digest 缺失时回退文件大小比对
fn verify_asset(path: &Path, digest: Option<&str>, expected_size: u64) -> Result<(), String> {
    if let Some(d) = digest.filter(|s| !s.is_empty()) {
        let expected = d.strip_prefix("sha256:").unwrap_or(d).to_lowercase();
        let actual = sha256_file(path)?;
        if actual != expected {
            return Err("更新包校验失败：哈希不一致，文件可能已损坏或被篡改".to_string());
        }
    } else if expected_size > 0 {
        let meta = fs::metadata(path).map_err(|e| format!("读取临时文件失败：{e}"))?;
        if meta.len() != expected_size {
            return Err(format!(
                "更新包大小不一致：期望 {expected_size} 字节，实际 {} 字节",
                meta.len()
            ));
        }
    }
    Ok(())
}

/// 从单个候选地址流式下载到目标文件，过程中回传进度；失败时由调用方清理半截文件
async fn stream_to_file(
    client: &reqwest::Client,
    app: &AppHandle,
    url: &str,
    dest: &Path,
    expected_size: u64,
    cancel: &DownloadCancel,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载更新包失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载更新包失败：HTTP {}", resp.status().as_u16()));
    }
    let total = resp.content_length().unwrap_or(expected_size);

    let mut file = fs::File::create(dest).map_err(|e| format!("临时文件创建失败：{e}"))?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.0.load(Ordering::Relaxed) {
            return Err(DOWNLOAD_CANCELLED.to_string());
        }
        let chunk = chunk.map_err(|e| format!("下载中断：{e}"))?;
        file.write_all(&chunk).map_err(|e| format!("写入临时文件失败：{e}"))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "update-download-progress",
            UpdateProgress { downloaded, total },
        );
    }
    file.flush().map_err(|e| format!("刷新临时文件失败：{e}"))?;
    drop(file);
    Ok(())
}

/// 下载更新包到系统临时目录，直连失败后依次回退加速节点；流式回传进度，优先校验 SHA256。
/// 用户取消时立即中止，不继续回退其它节点，并清理半截临时文件。
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    cancel_state: State<'_, DownloadCancel>,
    url: String,
    file_name: String,
    digest: Option<String>,
    expected_size: u64,
) -> Result<String, String> {
    cancel_state.0.store(false, Ordering::Relaxed);
    let tmp_dir = std::env::temp_dir().join(UPDATE_TMP_DIR);
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("临时目录创建失败：{e}"))?;
    let dest = tmp_dir.join(&file_name);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;

    let mut last_err = String::from("下载更新包失败：直连与加速节点均不可用");
    let mut verified = false;

    for dl_url in fallback_urls(&url) {
        let _ = fs::remove_file(&dest);
        match stream_to_file(&client, &app, &dl_url, &dest, expected_size, cancel_state.inner()).await {
            Ok(_) => match verify_asset(&dest, digest.as_deref(), expected_size) {
                Ok(()) => {
                    verified = true;
                    break;
                }
                Err(e) => {
                    last_err = e;
                    let _ = fs::remove_file(&dest);
                }
            },
            Err(e) => {
                let cancelled = e == DOWNLOAD_CANCELLED;
                last_err = e;
                let _ = fs::remove_file(&dest);
                // 取消属于用户主动中止，不再回退其它节点
                if cancelled {
                    break;
                }
            }
        }
    }

    if !verified {
        let _ = fs::remove_file(&dest);
        return Err(last_err);
    }
    Ok(dest.to_string_lossy().to_string())
}

/// 请求取消当前进行中的更新包下载；未在下载时置位也无副作用。
#[tauri::command]
pub fn cancel_download(cancel_state: State<'_, DownloadCancel>) {
    cancel_state.0.store(true, Ordering::Relaxed);
}

fn unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("打开更新包失败：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("解压更新包失败：{e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("解压条目失败：{e}"))?;
        let out = dest.join(entry.mangled_name());
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| format!("创建目录失败：{e}"))?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
            }
            let mut outfile = fs::File::create(&out).map_err(|e| format!("写出文件失败：{e}"))?;
            std::io::copy(&mut entry, &mut outfile).map_err(|e| format!("解压拷贝失败：{e}"))?;
        }
    }
    Ok(())
}

/// 递归查找解压目录内的主程序 IPSwitcher.exe
fn find_exe(dir: &Path) -> Result<PathBuf, String> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Ok(found) = find_exe(&p) {
                    return Ok(found);
                }
            } else if p
                .file_name()
                .map(|n| n.eq_ignore_ascii_case("IPSwitcher.exe"))
                .unwrap_or(false)
            {
                return Ok(p);
            }
        }
    }
    Err("更新包内未找到主程序".to_string())
}

/// 生成退出后执行替换的 PowerShell 辅助脚本（UTF-8 BOM，路径全部经参数传入；兼容 Windows PowerShell 5.1）。
/// 替换策略：等待所有 IPSwitcher 进程退出 → 等待目标 exe 句柄释放 → 先重命名旧 exe 再放入新 exe
/// （运行中映像可被重命名，规避直接覆盖占用）；失败按“权限不足 / 文件被占用”分类提示并回滚。
fn write_helper_ps1(path: &Path, clean_extract: bool) -> Result<(), String> {
    let extract_cleanup = if clean_extract {
        r#"
if ($ExtractDir -and (Test-Path -LiteralPath $ExtractDir)) {
    Remove-Item -LiteralPath $ExtractDir -Recurse -Force -ErrorAction SilentlyContinue
}"#
    } else {
        ""
    };
    let script = format!(
        r#"param(
    [int]$OldPid,
    [string]$TargetExe,
    [string]$NewExe,
    [string]$Backup,
    [string]$ExtractDir
)
$ErrorActionPreference = 'Stop'
$TargetDir = Split-Path -Parent $TargetExe
$OldExe = Join-Path $TargetDir 'IPSwitcher.old.exe'
$LogPath = Join-Path $env:TEMP 'ipswitcher-update\update-helper.log'

function Write-Log($msg) {{
    try {{
        $line = (Get-Date -Format 'yyyy-MM-dd HH:mm:ss') + ' ' + $msg
        Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8
    }} catch {{}}
}}

# 判断目标文件是否仍被独占占用（能以 ReadWrite/None 共享打开即视为已释放）
function Test-FileLocked($path) {{
    if (-not (Test-Path -LiteralPath $path)) {{ return $false }}
    try {{
        $fs = [System.IO.File]::Open($path, 'Open', 'ReadWrite', 'None')
        $fs.Close()
        return $false
    }} catch {{ return $true }}
}}

# 第一步：等待所有 IPSwitcher 实例退出（不仅是触发更新的 OldPid），最多 30 秒
$waitDeadline = (Get-Date).AddSeconds(30)
while ($true) {{
    $procs = Get-Process -Name 'IPSwitcher' -ErrorAction SilentlyContinue
    if (-not $procs) {{ break }}
    if ((Get-Date) -gt $waitDeadline) {{
        Write-Log 'wait timeout: another instance still running'
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.MessageBox]::Show('更新失败：检测到 IPSwitcher 其他实例仍在运行，请全部关闭后重试') | Out-Null
        exit 1
    }}
    Start-Sleep -Milliseconds 300
}}

# 第二步：等待目标 exe 文件句柄释放，最多 10 秒
$lockDeadline = (Get-Date).AddSeconds(10)
while ((Test-FileLocked $TargetExe) -and (Get-Date) -lt $lockDeadline) {{
    Start-Sleep -Milliseconds 300
}}

# 第三步：先重命名旧 exe（运行中映像允许重命名），再放入新 exe；失败回滚
$replaced = $false
try {{
    if (Test-Path -LiteralPath $OldExe) {{
        Remove-Item -LiteralPath $OldExe -Force -ErrorAction SilentlyContinue
    }}
    Move-Item -LiteralPath $TargetExe -Destination $OldExe -Force
    Copy-Item -LiteralPath $NewExe -Destination $TargetExe -Force
    $replaced = $true
    Write-Log 'copy ok (rename fallback)'
}} catch {{
    Write-Log ('copy failed: ' + $_.Exception.Message)
    # 主程序缺失时从旧 exe 或备份还原
    if (-not (Test-Path -LiteralPath $TargetExe)) {{
        $restore = if ((Test-Path -LiteralPath $OldExe)) {{ $OldExe }} else {{ $Backup }}
        if ($restore -and (Test-Path -LiteralPath $restore)) {{
            Copy-Item -LiteralPath $restore -Destination $TargetExe -Force -ErrorAction SilentlyContinue
        }}
    }}
    Add-Type -AssemblyName System.Windows.Forms
    $msg = $_.Exception.Message
    if ($msg -match '拒绝访问|Access is denied|UnauthorizedAccess|being used by another process') {{
        if ($msg -match '拒绝访问|Access is denied|UnauthorizedAccess') {{
            [System.Windows.Forms.MessageBox]::Show('更新失败：程序目录需要管理员权限，请以管理员身份运行后重试，或将软件移出 Program Files 目录') | Out-Null
        }} else {{
            [System.Windows.Forms.MessageBox]::Show('更新失败：主程序仍被占用，请关闭其他 IPSwitcher 实例后手动前往 Release 页面下载') | Out-Null
        }}
    }} else {{
        [System.Windows.Forms.MessageBox]::Show('更新失败：未能替换主程序，请手动前往 Release 页面下载') | Out-Null
    }}
}}

# 第四步：复制成功后清理备份与旧 exe，再尝试重启；重启失败不回滚、不判为更新失败
if ($replaced) {{
    if (Test-Path -LiteralPath $Backup) {{
        Remove-Item -LiteralPath $Backup -Force -ErrorAction SilentlyContinue
    }}
    if (Test-Path -LiteralPath $OldExe) {{
        Remove-Item -LiteralPath $OldExe -Force -ErrorAction SilentlyContinue
    }}
    try {{
        Start-Process -FilePath $TargetExe -WorkingDirectory $TargetDir
        Write-Log 'restart ok'
    }} catch {{
        Write-Log ('restart failed: ' + $_.Exception.Message)
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.MessageBox]::Show('更新已完成，请手动启动 IPSwitcher') | Out-Null
    }}
}}

Remove-Item -LiteralPath $NewExe -Force -ErrorAction SilentlyContinue
{extract_cleanup}
Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
"#
    );
    let mut file = fs::File::create(path).map_err(|e| format!("创建辅助脚本失败：{e}"))?;
    file.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| format!("写入脚本 BOM 失败：{e}"))?;
    file.write_all(script.as_bytes())
        .map_err(|e| format!("写入辅助脚本失败：{e}"))?;
    Ok(())
}

/// 应用更新：安装版拉起安装器；便携/绿色版由辅助脚本在主进程退出后覆盖主程序并重启
#[tauri::command]
pub async fn apply_update(
    app: AppHandle,
    asset_path: String,
    asset_name: String,
) -> Result<(), String> {
    let flavor = install_flavor();
    let current_exe = std::env::current_exe().map_err(|e| format!("定位程序路径失败：{e}"))?;
    let prog_dir = current_exe
        .parent()
        .ok_or_else(|| "无法定位程序目录".to_string())?
        .to_path_buf();
    let pid = std::process::id();
    let asset = PathBuf::from(&asset_path);

    match flavor {
        InstallFlavor::Installed => {
            // 安装版：perMachine 安装器要求管理员权限，标准权限直接 CreateProcess 会报 740，
            // 因此经 ShellExecute RunAs 触发 UAC 提权拉起；/S 静默安装、/R 装后自动启动新版。
            // 随后退出主程序释放文件占用。
            let setup_q = asset.to_string_lossy().replace('\'', "''");
            let ps = format!(
                "try {{ Start-Process -FilePath '{}' -ArgumentList '/S /R' -Verb RunAs }} catch {{ exit 1 }}",
                setup_q
            );
            let mut child_cmd = std::process::Command::new("powershell");
            child_cmd.args(["-NoProfile", "-Command", &ps]);
            suppress_window(&mut child_cmd);
            let mut child = child_cmd
                .spawn()
                .map_err(|e| format!("启动安装器失败：{e}"))?;
            let status = child
                .wait()
                .map_err(|e| format!("启动安装器失败：{e}"))?;
            if !status.success() {
                return Err("更新已取消：未通过管理员权限授权".to_string());
            }
            app.exit(0);
            Ok(())
        }
        InstallFlavor::Portable | InstallFlavor::Standalone => {
            let is_zip = asset_name.to_lowercase().ends_with(".zip");
            let extract_dir = if is_zip {
                let dir = std::env::temp_dir()
                    .join(UPDATE_TMP_DIR)
                    .join("extract");
                let _ = fs::remove_dir_all(&dir);
                fs::create_dir_all(&dir).map_err(|e| format!("解压目录创建失败：{e}"))?;
                unzip(&asset, &dir)?;
                Some(dir)
            } else {
                None
            };
            let new_exe = match &extract_dir {
                Some(dir) => find_exe(dir)?,
                None => asset.clone(),
            };

            // zip 形态解压完成后删除临时下载的压缩包；单文件形态由辅助脚本删除 NewExe
            if is_zip {
                let _ = fs::remove_file(&asset);
            }

            // 备份当前主程序，便于替换失败时还原
            let backup = prog_dir.join("IPSwitcher.exe.bak");
            fs::copy(&current_exe, &backup)
                .map_err(|e| format!("备份旧版本失败：{e}"))?;

            let tmp_dir = std::env::temp_dir().join(UPDATE_TMP_DIR);
            fs::create_dir_all(&tmp_dir).map_err(|e| format!("临时目录创建失败：{e}"))?;
            let ps1 = tmp_dir.join("update-helper.ps1");
            write_helper_ps1(&ps1, extract_dir.is_some())?;

            let mut cmd = std::process::Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &ps1.to_string_lossy(),
                "-OldPid",
                &pid.to_string(),
                "-TargetExe",
                &current_exe.to_string_lossy(),
                "-NewExe",
                &new_exe.to_string_lossy(),
                "-Backup",
                &backup.to_string_lossy(),
                "-ExtractDir",
                &extract_dir
                    .as_ref()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ]);
            suppress_window(&mut cmd);
            cmd.spawn().map_err(|e| format!("启动更新辅助进程失败：{e}"))?;
            app.exit(0);
            Ok(())
        }
    }
}
