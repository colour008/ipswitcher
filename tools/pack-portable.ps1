# 归集 release 产物到版本文件夹：release\v{Version}_{yyyyMMdd}\
# 含免安装 exe、NSIS 安装包、绿色便携 zip；保留历史旧版本，互不覆盖。
# 用法：tools\pack-portable.ps1 -Version 2.0.1
param(
	[Parameter(Mandatory = $true)]
	[string]$Version
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$releaseExe = Join-Path $root "src-tauri\target\release\IPSwitcher.exe"
$setupExe = Join-Path $root "src-tauri\target\release\bundle\nsis\IPSwitcher_${Version}_x64-setup.exe"

if (-not (Test-Path $releaseExe)) {
	throw "未找到免安装 exe：$releaseExe，请先执行 npm run app:build"
}

$date = Get-Date -Format "yyyyMMdd"
$dir = Join-Path $root "release\v${Version}_$date"
New-Item -ItemType Directory -Path $dir -Force | Out-Null

# 1. 免安装 exe
Copy-Item $releaseExe (Join-Path $dir "IPSwitcher.exe") -Force

# 2. NSIS 安装包
if (Test-Path $setupExe) {
	Copy-Item $setupExe (Join-Path $dir "IPSwitcher_${Version}_x64-setup.exe") -Force
} else {
	Write-Warning "未找到 NSIS 安装包：$setupExe"
}

# 3. 绿色便携包：exe + portable.marker + 默认空配置 -> zip
$stageDir = Join-Path $root "dist-portable"
$stageAppDir = Join-Path $stageDir "IPSwitcher"
if (Test-Path $stageDir) { Remove-Item $stageDir -Recurse -Force }
New-Item -ItemType Directory -Path $stageAppDir -Force | Out-Null
Copy-Item $releaseExe (Join-Path $stageAppDir "IPSwitcher.exe")
New-Item -ItemType File -Path (Join-Path $stageAppDir "portable.marker") | Out-Null

$defaultConfig = [ordered]@{
	default_adapter = $null
	profiles        = @()
	adapter_order   = @()
	pinned          = @()
	sidebar_width   = $null
	close_action    = $null
	autostart       = $null
	widget_visible  = $null
	widget_adapter  = $null
	widget_opacity  = $null
	widget_width    = $null
	widget_height   = $null
	theme           = $null
	admin_prompt_dismissed = $null
}
$json = $defaultConfig | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $stageAppDir "config.json"), $json, (New-Object System.Text.UTF8Encoding($false)))

$zipPath = Join-Path $dir "IPSwitcher_v${Version}_portable.zip"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Compress-Archive -Path $stageAppDir -DestinationPath $zipPath -CompressionLevel Optimal -Force

Write-Output "已归集到：$dir"
Get-ChildItem $dir | Select-Object Name, @{ N = 'KB'; E = { [math]::Round($_.Length / 1KB, 1) } } | Format-Table -AutoSize
