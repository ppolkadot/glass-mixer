<#
.SYNOPSIS
  Builds the release binary and packages it.

.DESCRIPTION
  Produces, in .\dist:
    glass-mixer-<version>-portable.zip   always
    glass-mixer-<version>-setup.exe      when Inno Setup 6 (ISCC.exe) is installed

  Run from the repository root:  powershell -ExecutionPolicy Bypass -File scripts\package.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Glass Mixer $version"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
}
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# The build script writes the generated icon to OUT_DIR; copy it where the installer expects it.
$ico = Get-ChildItem target\release\build -Recurse -Filter app.ico | Sort-Object LastWriteTime | Select-Object -Last 1
Copy-Item $ico.FullName target\release\app.ico -Force

New-Item -ItemType Directory -Force dist | Out-Null
$staging = Join-Path dist "glass-mixer-$version"
Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item target\release\glass-mixer.exe, README.md $staging
$zip = Join-Path dist "glass-mixer-$version-portable.zip"
Remove-Item $zip -Force -ErrorAction SilentlyContinue
Compress-Archive -Path "$staging\*" -DestinationPath $zip
Remove-Item $staging -Recurse -Force
Write-Host "Portable zip: $zip"

$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    & $iscc "/DAppVersion=$version" installer\glass-mixer.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
    Write-Host "Installer: dist\glass-mixer-$version-setup.exe"
} else {
    Write-Warning "Inno Setup 6 not found; skipping installer. Install it with: winget install JRSoftware.InnoSetup"
}
