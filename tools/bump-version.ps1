# 版本号递增并同步三处：package.json / tauri.conf.json / Cargo.toml
# 用法：
#   tools\bump-version.ps1 -Bump patch     # 常规修复/样式：末位 +1（2.0.0 -> 2.0.1）
#   tools\bump-version.ps1 -Bump minor     # 新增功能：中位 +1（2.0.0 -> 2.1.0）
#   tools\bump-version.ps1 -Version 2.1.0  # 直接指定版本号（大版本仅由人工决定）
param(
	[ValidateSet("patch", "minor")]
	[string]$Bump = "patch",
	[string]$Version
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$pkgPath = Join-Path $root "package.json"
$tauriPath = Join-Path $root "src-tauri\tauri.conf.json"
$cargoPath = Join-Path $root "src-tauri\Cargo.toml"

# 从 package.json 读取当前版本
$pkgRaw = Get-Content $pkgPath -Raw -Encoding UTF8
$m = [regex]::Match($pkgRaw, '"version"\s*:\s*"([^"]+)"')
if (-not $m.Success) { throw "未在 package.json 找到 version 字段" }
$current = $m.Groups[1].Value

if ($Version) {
	$new = $Version
} else {
	$parts = $current.Split(".")
	if ($parts.Count -ne 3) { throw "当前版本号非语义化格式：$current" }
	$major = [int]$parts[0]
	$minor = [int]$parts[1]
	$patch = [int]$parts[2]
	if ($Bump -eq "minor") { $minor++; $patch = 0 } else { $patch++ }
	$new = "$major.$minor.$patch"
}

if ($new -notmatch '^\d+\.\d+\.\d+$') { throw "版本号格式非法：$new" }

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

# package.json
$pkgRaw = [regex]::Replace($pkgRaw, '"version"\s*:\s*"[^"]+"', "`"version`": `"$new`"")
[IO.File]::WriteAllText($pkgPath, $pkgRaw, $utf8NoBom)

# tauri.conf.json
$tauriRaw = Get-Content $tauriPath -Raw -Encoding UTF8
$tauriRaw = [regex]::Replace($tauriRaw, '"version"\s*:\s*"[^"]+"', "`"version`": `"$new`"")
[IO.File]::WriteAllText($tauriPath, $tauriRaw, $utf8NoBom)

# Cargo.toml
$cargoRaw = Get-Content $cargoPath -Raw -Encoding UTF8
$cargoRaw = [regex]::Replace($cargoRaw, '(?m)^version\s*=\s*"[^"]+"', "version = `"$new`"")
[IO.File]::WriteAllText($cargoPath, $cargoRaw, $utf8NoBom)

Write-Output "版本已更新：$current -> $new"
