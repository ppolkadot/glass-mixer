<#
.SYNOPSIS
  Checks that every place that carries the version agrees with Cargo.toml.

.DESCRIPTION
  Cargo.toml is the source of truth. This compares it with:
    - the glass-mixer entry in Cargo.lock
    - assets\app.manifest (assemblyIdentity, four-part)
    - the defaults in installer\glass-mixer.iss (AppVersion and AppVersionNumeric)
    - a "## <version> (" heading in CHANGELOG.md, with no "## Unreleased" section left
    - docs\releases\<version>.md, when not -AllowUnreleased
    - optionally a release tag (-Tag v0.5.0)
    - optionally a built executable's file and product version (-Exe path)

  Exits with 1 and lists every disagreement. Works in Windows PowerShell 5.1 and PowerShell 7.

  Run from the repository root:  powershell -ExecutionPolicy Bypass -File scripts\check-version.ps1
#>
param(
    [string]$Tag,
    [string]$Exe,
    # A branch in development may still have an Unreleased section.
    [switch]$AllowUnreleased
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$problems = New-Object System.Collections.Generic.List[string]

function Read-Repo([string]$relative) {
    Get-Content -LiteralPath (Join-Path $root $relative) -Raw -Encoding UTF8
}

$cargo = Read-Repo 'Cargo.toml'
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'Cargo.toml has no package version.' }
$version = $Matches[1]
$numeric = ((($version -split '[-+]')[0]) + '.0')

$lock = Read-Repo 'Cargo.lock'
if ($lock -match '(?s)\[\[package\]\]\s*name = "glass-mixer"\s*version = "([^"]+)"') {
    if ($Matches[1] -ne $version) { $problems.Add("Cargo.lock has glass-mixer $($Matches[1]), Cargo.toml has $version.") }
} else {
    $problems.Add('Cargo.lock has no glass-mixer package entry.')
}

$manifest = Read-Repo 'assets\app.manifest'
if ($manifest -match 'name="GlassMixer"\s+version="([^"]+)"') {
    if ($Matches[1] -ne $numeric) { $problems.Add("assets\app.manifest has $($Matches[1]), expected $numeric.") }
} else {
    $problems.Add('assets\app.manifest has no GlassMixer assemblyIdentity.')
}

$iss = Read-Repo 'installer\glass-mixer.iss'
if ($iss -match '#define AppVersion "([^"]+)"') {
    if ($Matches[1] -ne $version) { $problems.Add("installer AppVersion default is $($Matches[1]), expected $version.") }
} else {
    $problems.Add('installer\glass-mixer.iss has no AppVersion default.')
}
if ($iss -match '#define AppVersionNumeric "([^"]+)"') {
    if ($Matches[1] -ne $numeric) { $problems.Add("installer AppVersionNumeric default is $($Matches[1]), expected $numeric.") }
} else {
    $problems.Add('installer\glass-mixer.iss has no AppVersionNumeric default.')
}

$changelog = Read-Repo 'CHANGELOG.md'
if ($changelog -notmatch ('(?m)^## ' + [regex]::Escape($version) + ' \(')) {
    $problems.Add("CHANGELOG.md has no '## $version (date)' heading.")
}
if (-not $AllowUnreleased) {
    if ($changelog -match '(?m)^## Unreleased') {
        $problems.Add("CHANGELOG.md still has an 'Unreleased' section; move it under '## $version'.")
    }
    $releaseRecord = Join-Path $root "docs\releases\$version.md"
    if (-not (Test-Path -LiteralPath $releaseRecord)) {
        $problems.Add("docs\releases\$version.md does not exist.")
    } elseif ([string]::IsNullOrWhiteSpace((Get-Content -LiteralPath $releaseRecord -Raw -Encoding UTF8))) {
        $problems.Add("docs\releases\$version.md is empty.")
    }
}

if ($Tag -and $Tag -ne "v$version") { $problems.Add("The tag is $Tag, but Cargo.toml is version $version (expected v$version).") }

if ($Exe) {
    if (-not (Test-Path -LiteralPath $Exe)) {
        $problems.Add("The executable does not exist: $Exe")
    } else {
        $info = (Get-Item -LiteralPath $Exe).VersionInfo
        if ($info.FileVersion -ne $version) { $problems.Add("$Exe has FileVersion $($info.FileVersion), expected $version.") }
        if ($info.ProductVersion -ne $version) { $problems.Add("$Exe has ProductVersion $($info.ProductVersion), expected $version.") }
    }
}

if ($problems.Count -gt 0) {
    Write-Host "Version check FAILED (Cargo.toml says $version):" -ForegroundColor Red
    $problems | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
$checked = 'Cargo.lock, app.manifest, installer, CHANGELOG'
if (-not $AllowUnreleased) { $checked += ', release record' }
if ($Tag) { $checked += ', tag' }
if ($Exe) { $checked += ', executable' }
Write-Host "Version $version is consistent ($checked)."
exit 0
