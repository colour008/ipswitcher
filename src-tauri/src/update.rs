use crate::config::{install_flavor, InstallFlavor};
use crate::process::suppress_window;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

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

/// 下载更新包到系统临时目录，直连失败后依次回退加速节点；流式回传进度，优先校验 SHA256
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    url: String,
    file_name: String,
    digest: Option<String>,
    expected_size: u64,
) -> Result<String, String> {
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
        match stream_to_file(&client, &app, &dl_url, &dest, expected_size).await {
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
                last_err = e;
                let _ = fs::remove_file(&dest);
            }
        }
    }

    if !verified {
        let _ = fs::remove_file(&dest);
        return Err(last_err);
    }
    Ok(dest.to_string_lossy().to_string())
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

/// 生成退出后执行替换的 PowerShell 辅助脚本（UTF-8 BOM，路径全部经参数传入）
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
while (Get-Process -Id $OldPid -ErrorAction SilentlyContinue) {{
    Start-Sleep -Milliseconds 300
}}
try {{
    Copy-Item -LiteralPath $NewExe -Destination $TargetExe -Force
    Start-Process -LiteralPath $TargetExe
}} catch {{
    if ((Test-Path -LiteralPath $Backup) -and -not (Test-Path -LiteralPath $TargetExe)) {{
        Copy-Item -LiteralPath $Backup -Destination $TargetExe -Force
    }}
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show('更新失败，已保留旧版本，请手动前往 Release 页面下载') | Out-Null
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
            // 安装版：直接拉起 NSIS 安装器，随后退出主程序释放文件占用
            let mut cmd = std::process::Command::new(&asset);
            suppress_window(&mut cmd);
            cmd.spawn().map_err(|e| format!("启动安装器失败：{e}"))?;
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
