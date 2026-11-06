<#
.SYNOPSIS
  Writes the release notes for a version: its CHANGELOG section, plus a line about
  whether the downloads are signed.

.DESCRIPTION
  Takes the text under "## <version> (date)" in CHANGELOG.md, up to the next "## "
  heading, and adds the checksums and signing status. Fails if the version has no
  section. Works in Windows PowerShell 5.1 and PowerShell 7.

  Run from the repository root:
    powershell -ExecutionPolicy Bypass -File scripts\release-notes.ps1 -Version 0.5.0 -OutFile notes.md [-Signed]
#>
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$OutFile,
    # Say that the executable and installer carry a valid signature.
    [switch]$Signed
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$changelog = Get-Content -LiteralPath (Join-Path $root 'CHANGELOG.md') -Raw -Encoding UTF8
$pattern = '(?ms)^## ' + [regex]::Escape($Version) + ' \([^\r\n]*\)[ \t]*\r?\n(.*?)(?=^## |\z)'
if ($changelog -notmatch $pattern) { throw "CHANGELOG.md has no '## $Version (date)' section." }
$section = $Matches[1].Trim()
if (-not $section) { throw "The CHANGELOG section for $Version is empty." }

$policy = 'https://github.com/ppolkadot/glass-mixer/blob/main/CODE_SIGNING_POLICY.md'
$signing = if ($Signed) {
    "The executable and the installer are digitally signed. See the [code signing policy]($policy)."
} else {
    "**Signing status:** these downloads are **not code-signed** yet, so Windows SmartScreen may warn the first time you run them. Compare the SHA-256 checksums in ``SHA256SUMS.txt`` before running. See the [code signing policy]($policy)."
}
$text = @(
    "# Glass Mixer $Version",
    '',
    $section,
    '',
    '## Downloads',
    '',
    "- ``glass-mixer-$Version-setup.exe``: installer for the current user, no administrator rights needed.",
    "- ``glass-mixer-$Version-portable.zip``: portable copy, no installation.",
    "- ``glass-mixer-$Version-SHA256SUMS.txt``: SHA-256 checksums of both.",
    '',
    $signing,
    ''
) -join "`n"
[IO.File]::WriteAllText($OutFile, $text, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Wrote release notes for $Version ($($section.Length) characters of changes) to $OutFile"
exit 0
