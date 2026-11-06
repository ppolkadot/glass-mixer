<#
.SYNOPSIS
  Verifies the packages in dist\ and optionally writes their SHA-256 checksums.

.DESCRIPTION
  Checks the portable ZIP (always) and the installer (when it was built):
    - the ZIP holds exactly glass-mixer.exe, README.md, LICENSE, THIRD-PARTY-NOTICES.txt
      and fonts\LICENSE.txt, with no extra files
    - the executable is a 64-bit Windows program of a sane size whose file and product
      version match Cargo.toml, and it embeds the Polaris font's required metadata line
    - LICENSE, README.md and fonts\LICENSE.txt are byte-for-byte what the repository has
      (the font license must be redistributed unmodified)
    - THIRD-PARTY-NOTICES.txt is present and lists the compiled-in components
    - the installer's file version matches
    - Authenticode status of the executable and installer (-RequireSigned makes anything
      other than a valid signature an error; unsigned builds are otherwise reported, not hidden)
    - with -WriteChecksums, writes glass-mixer-<version>-SHA256SUMS.txt (sha256sum format)

  Exits with 1 and lists every problem. Works in Windows PowerShell 5.1 and PowerShell 7.

  Run from the repository root:  powershell -ExecutionPolicy Bypass -File scripts\verify-package.ps1
#>
param(
    [string]$Version,
    [string]$Dist,
    [switch]$WriteChecksums,
    [switch]$RequireSigned,
    [switch]$RequireInstaller
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Version) {
    $cargo = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw -Encoding UTF8
    if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'Cargo.toml has no package version.' }
    $Version = $Matches[1]
}
if (-not $Dist) { $Dist = Join-Path $root 'dist' }
$numeric = ((($Version -split '[-+]')[0]) + '.0')
$problems = New-Object System.Collections.Generic.List[string]
$notes = New-Object System.Collections.Generic.List[string]

function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

function Check-Signature([string]$Path, [string]$What) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -eq 'Valid') {
        $notes.Add("$What is signed by $($signature.SignerCertificate.Subject).")
    } elseif ($RequireSigned) {
        $problems.Add("$What has no valid signature (status: $($signature.Status)).")
    } else {
        $notes.Add("$What is NOT signed (status: $($signature.Status)); Windows SmartScreen may warn on first run.")
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem

$zip = Join-Path $Dist "glass-mixer-$Version-portable.zip"
if (-not (Test-Path -LiteralPath $zip)) {
    $problems.Add("The portable ZIP is missing: $zip")
} else {
    $extract = Join-Path ([IO.Path]::GetTempPath()) ("glass-mixer-verify-" + [guid]::NewGuid().ToString('N'))
    try {
        [IO.Compression.ZipFile]::ExtractToDirectory($zip, $extract)
        $expected = @('glass-mixer.exe', 'README.md', 'LICENSE', 'THIRD-PARTY-NOTICES.txt', 'fonts/LICENSE.txt')
        $actual = @(Get-ChildItem -LiteralPath $extract -Recurse -File | ForEach-Object {
            $_.FullName.Substring($extract.Length + 1).Replace('\', '/')
        })
        foreach ($name in $expected) { if ($actual -notcontains $name) { $problems.Add("The ZIP is missing $name.") } }
        foreach ($name in $actual) { if ($expected -notcontains $name) { $problems.Add("The ZIP has an unexpected file: $name.") } }

        $exe = Join-Path $extract 'glass-mixer.exe'
        if (Test-Path -LiteralPath $exe) {
            $info = (Get-Item -LiteralPath $exe).VersionInfo
            if ($info.FileVersion.Trim() -ne $Version) { $problems.Add("glass-mixer.exe has FileVersion $($info.FileVersion), expected $Version.") }
            if ($info.ProductVersion.Trim() -ne $Version) { $problems.Add("glass-mixer.exe has ProductVersion $($info.ProductVersion), expected $Version.") }
            $bytes = [IO.File]::ReadAllBytes($exe)
            $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
            $machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
            if ($bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A -or $machine -ne 0x8664) { $problems.Add('glass-mixer.exe is not a 64-bit Windows executable.') }
            $megabytes = [math]::Round($bytes.Length / 1MB, 1)
            if ($bytes.Length -lt 10MB -or $bytes.Length -gt 40MB) { $problems.Add("glass-mixer.exe is $megabytes MB, outside the expected 10 to 40 MB (a debug build?).") }
            $text = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
            if (-not $text.Contains('Salted Flutters Polaris - Created by ppoolar')) { $problems.Add('glass-mixer.exe does not embed the Polaris font (its required metadata line is missing).') }
            $notes.Add("glass-mixer.exe: $megabytes MB, version $($info.FileVersion).")
            Check-Signature $exe 'glass-mixer.exe'
        }

        foreach ($pair in @(@('LICENSE', 'LICENSE'), @('README.md', 'README.md'), @('fonts\LICENSE.txt', 'assets\fonts\LICENSE.txt'))) {
            $packed = Join-Path $extract $pair[0]
            $source = Join-Path $root $pair[1]
            if ((Test-Path -LiteralPath $packed) -and ((Hash $packed) -ne (Hash $source))) {
                $problems.Add("$($pair[0]) in the ZIP differs from $($pair[1]) in the repository.")
            }
        }
        $fontLicense = Join-Path $extract 'fonts\LICENSE.txt'
        if (Test-Path -LiteralPath $fontLicense) {
            $head = (Get-Content -LiteralPath $fontLicense -TotalCount 1 -Encoding UTF8)
            if ($head -ne 'SALTED FLUTTERS POLARIS FONT LICENSE') { $problems.Add('fonts\LICENSE.txt does not start with the Polaris license heading.') }
        }
        $notices = Join-Path $extract 'THIRD-PARTY-NOTICES.txt'
        if (Test-Path -LiteralPath $notices) {
            $size = (Get-Item -LiteralPath $notices).Length
            $noticeText = Get-Content -LiteralPath $notices -Raw -Encoding UTF8
            if ($size -lt 10KB -or -not $noticeText.Contains('egui')) { $problems.Add('THIRD-PARTY-NOTICES.txt looks empty or incomplete.') }
        }
    } finally {
        Remove-Item -LiteralPath $extract -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$setup = Join-Path $Dist "glass-mixer-$Version-setup.exe"
$haveSetup = Test-Path -LiteralPath $setup
if ($haveSetup) {
    $info = (Get-Item -LiteralPath $setup).VersionInfo
    # Inno pads the version strings of Setup.exe with spaces.
    if ($info.FileVersion.Trim() -ne $numeric) { $problems.Add("The installer has FileVersion $($info.FileVersion), expected $numeric.") }
    $megabytes = [math]::Round((Get-Item -LiteralPath $setup).Length / 1MB, 1)
    if ($megabytes -lt 1 -or $megabytes -gt 80) { $problems.Add("The installer is $megabytes MB, outside the expected 1 to 80 MB.") }
    $notes.Add("Installer: $megabytes MB, version $($info.FileVersion).")
    Check-Signature $setup 'The installer'
} elseif ($RequireInstaller) {
    $problems.Add("The installer is missing: $setup")
} else {
    $notes.Add('No installer was built (Inno Setup 6 is not installed).')
}

$sums = Join-Path $Dist "glass-mixer-$Version-SHA256SUMS.txt"
$artifacts = @($zip) + $(if ($haveSetup) { @($setup) } else { @() })
if ($problems.Count -eq 0) {
    if ($WriteChecksums) {
        $lines = $artifacts | ForEach-Object { '{0} *{1}' -f (Hash $_), (Split-Path -Leaf $_) }
        [IO.File]::WriteAllText($sums, (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
        $notes.Add("Wrote $sums")
    } elseif (Test-Path -LiteralPath $sums) {
        foreach ($line in (Get-Content -LiteralPath $sums)) {
            if ($line -match '^([0-9a-f]{64}) \*(.+)$') {
                $file = Join-Path $Dist $Matches[2]
                if (-not (Test-Path -LiteralPath $file)) { $problems.Add("$($Matches[2]) is listed in the checksums but missing.") }
                elseif ((Hash $file) -ne $Matches[1]) { $problems.Add("$($Matches[2]) does not match its checksum.") }
            }
        }
    }
}

if ($problems.Count -gt 0) {
    Write-Host "Package verification FAILED for ${Version}:" -ForegroundColor Red
    $problems | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
Write-Host "Package verification passed for $Version."
$notes | ForEach-Object { Write-Host "  $_" }
exit 0
