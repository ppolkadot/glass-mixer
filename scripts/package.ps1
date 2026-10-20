<#
.SYNOPSIS
  Builds the release binary and packages it.

.DESCRIPTION
  Produces, in .\dist:
    glass-mixer-<version>-portable.zip   always (executable, README, LICENSE, THIRD-PARTY-NOTICES.txt)
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
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# Optional certificate-store signing. Set GLASS_MIXER_SIGN_CERT_SHA1 to the
# thumbprint of a code-signing certificate with its private key installed.
function Sign-File([string]$Path) {
    $thumbprint = $env:GLASS_MIXER_SIGN_CERT_SHA1
    if (-not $thumbprint) { return }
    $thumbprint = $thumbprint -replace '\s', ''
    if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') { throw 'GLASS_MIXER_SIGN_CERT_SHA1 must be a 40-digit SHA-1 thumbprint.' }
    $tool = $env:GLASS_MIXER_SIGNTOOL
    if (-not $tool) { $tool = (Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source) }
    if (-not $tool -or -not (Test-Path -LiteralPath $tool)) { throw 'Signing was requested but signtool.exe was not found; set GLASS_MIXER_SIGNTOOL.' }
    $arguments = @('sign', '/sha1', $thumbprint, '/fd', 'SHA256')
    if ($env:GLASS_MIXER_SIGN_TIMESTAMP_URL) { $arguments += @('/tr', $env:GLASS_MIXER_SIGN_TIMESTAMP_URL, '/td', 'SHA256') }
    $arguments += $Path
    & $tool @arguments
    if ($LASTEXITCODE -ne 0) { throw "Signing failed: $Path" }
    & $tool verify /pa $Path
    if ($LASTEXITCODE -ne 0) { throw "Signature verification failed: $Path" }
}
Sign-File (Join-Path $root 'target\release\glass-mixer.exe')

# License texts of every crate compiled into the executable ship with both packages.
$notices = Join-Path $root 'target\release\THIRD-PARTY-NOTICES.txt'
& (Join-Path $PSScriptRoot 'third-party-notices.ps1') -OutFile $notices

# The build script writes the generated icon to OUT_DIR; copy it where the installer expects it.
$ico = Get-ChildItem target\release\build -Recurse -Filter app.ico | Sort-Object LastWriteTime | Select-Object -Last 1
Copy-Item $ico.FullName target\release\app.ico -Force

New-Item -ItemType Directory -Force dist | Out-Null
$distRoot = [IO.Path]::GetFullPath((Join-Path $root 'dist')).TrimEnd('\') + '\'
$staging = Join-Path $distRoot "glass-mixer-$version"
if (-not ([IO.Path]::GetFullPath($staging)).StartsWith($distRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid staging path.' }
Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item target\release\glass-mixer.exe, README.md, LICENSE, $notices $staging
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
    Sign-File (Join-Path $root "dist\glass-mixer-$version-setup.exe")
    Write-Host "Installer: dist\glass-mixer-$version-setup.exe"
} else {
    Write-Warning "Inno Setup 6 not found; skipping installer. Install it with: winget install JRSoftware.InnoSetup"
}
