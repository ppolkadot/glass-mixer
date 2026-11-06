<#
.SYNOPSIS
  Builds the release binary and packages it.

.DESCRIPTION
  Produces, in .\dist:
    glass-mixer-<version>-portable.zip   always (executable, README, LICENSE, THIRD-PARTY-NOTICES.txt, fonts\LICENSE.txt)
    glass-mixer-<version>-setup.exe      when Inno Setup 6 (ISCC.exe) is installed
    glass-mixer-<version>-SHA256SUMS.txt checksums of the above, in sha256sum format

  The version is checked first (scripts\check-version.ps1) and the packages are
  verified last (scripts\verify-package.ps1).

  Run from the repository root:  powershell -ExecutionPolicy Bypass -File scripts\package.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$numeric = ((($version -split '[-+]')[0]) + '.0')
Write-Host "Glass Mixer $version"
$global:LASTEXITCODE = 0
& (Join-Path $PSScriptRoot 'check-version.ps1')
if ($LASTEXITCODE -ne 0) { throw 'The version check failed.' }

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
}
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$global:LASTEXITCODE = 0
& (Join-Path $PSScriptRoot 'check-version.ps1') -Exe (Join-Path $root 'target\release\glass-mixer.exe')
if ($LASTEXITCODE -ne 0) { throw 'The built executable has the wrong version.' }

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
# The Polaris font is embedded in the exe; its license must travel with it, unmodified, as LICENSE.txt.
New-Item -ItemType Directory -Force (Join-Path $staging 'fonts') | Out-Null
Copy-Item assets\fonts\LICENSE.txt (Join-Path $staging 'fonts')
$zip = Join-Path dist "glass-mixer-$version-portable.zip"
Remove-Item $zip -Force -ErrorAction SilentlyContinue
# Not Compress-Archive: Windows PowerShell 5.1 writes backslashes in entry names, which other tools mishandle.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open((Join-Path $root $zip), 'Create')
try {
    foreach ($file in Get-ChildItem -LiteralPath $staging -Recurse -File) {
        $entry = $file.FullName.Substring($staging.Length + 1).Replace('\', '/')
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName, $entry, 'Optimal')
    }
} finally {
    $archive.Dispose()
}
Remove-Item $staging -Recurse -Force
Write-Host "Portable zip: $zip"

$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    & $iscc "/DAppVersion=$version" "/DAppVersionNumeric=$numeric" installer\glass-mixer.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
    Sign-File (Join-Path $root "dist\glass-mixer-$version-setup.exe")
    Write-Host "Installer: dist\glass-mixer-$version-setup.exe"
} else {
    Write-Warning "Inno Setup 6 not found; skipping installer. Install it with: winget install JRSoftware.InnoSetup"
}

# A hashtable, so the switches bind by name (an array would pass them as positional values).
$verify = @{ WriteChecksums = $true }
if ($env:GLASS_MIXER_SIGN_CERT_SHA1) { $verify.RequireSigned = $true }
$global:LASTEXITCODE = 0
& (Join-Path $PSScriptRoot 'verify-package.ps1') @verify
if ($LASTEXITCODE -ne 0) { throw 'Package verification failed.' }
