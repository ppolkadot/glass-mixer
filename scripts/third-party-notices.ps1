<#
.SYNOPSIS
  Writes THIRD-PARTY-NOTICES.txt for the crates compiled into glass-mixer.exe.

.DESCRIPTION
  Lists every crate in the Windows build's normal (non-build, non-dev) dependency
  graph with its version, license expression and repository, followed by the
  license and notice files each crate ships (LICENSE*, COPYING*, NOTICE*,
  COPYRIGHT*, plus the font licenses in a crate's fonts folder). Identical texts
  are printed once and referenced by number. Run from the repository root:

    powershell -ExecutionPolicy Bypass -File scripts\third-party-notices.ps1 -OutFile target\THIRD-PARTY-NOTICES.txt
#>
param([Parameter(Mandatory = $true)][string]$OutFile)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
$json = & $cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$meta = ($json -join "`n") | ConvertFrom-Json

$packages = @{}
foreach ($package in $meta.packages) { $packages[$package.id] = $package }
$nodes = @{}
foreach ($node in $meta.resolve.nodes) { $nodes[$node.id] = $node }

# Walk normal dependencies only: build scripts and dev-dependencies are not shipped.
$shipped = New-Object 'System.Collections.Generic.HashSet[string]'
$queue = New-Object 'System.Collections.Generic.Queue[string]'
$queue.Enqueue($meta.resolve.root)
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    foreach ($dep in $nodes[$id].deps) {
        $normal = @($dep.dep_kinds | Where-Object { $null -eq $_.kind }).Count -gt 0
        if ($normal -and $shipped.Add($dep.pkg)) { $queue.Enqueue($dep.pkg) }
    }
}
if ($shipped.Count -eq 0) { throw 'No shipped dependencies were found.' }

$noticePattern = '^(licen[cs]e|copying|notice|copyright|unlicense)'
$texts = New-Object System.Collections.Generic.List[string]
$textUsers = New-Object System.Collections.Generic.List[object]
$entries = foreach ($id in $shipped) {
    $package = $packages[$id]
    $dir = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -match $noticePattern })
    $fonts = Join-Path $dir 'fonts'
    if (Test-Path -LiteralPath $fonts) { $files += @(Get-ChildItem -LiteralPath $fonts -File -Filter '*.txt') }
    if ($package.license_file) {
        $declared = Join-Path $dir $package.license_file
        if ((Test-Path -LiteralPath $declared) -and -not ($files | Where-Object { $_.FullName -eq (Resolve-Path -LiteralPath $declared).Path })) {
            $files += Get-Item -LiteralPath $declared
        }
    }
    $refs = foreach ($file in ($files | Sort-Object Name)) {
        $text = ([IO.File]::ReadAllText($file.FullName)).Trim()
        if (-not $text) { continue }
        $index = $texts.IndexOf($text)
        if ($index -lt 0) {
            $texts.Add($text)
            $textUsers.Add((New-Object System.Collections.Generic.List[string]))
            $index = $texts.Count - 1
        }
        $textUsers[$index].Add("$($package.name) $($package.version) ($($file.Name))")
        "[$($index + 1)]"
    }
    [pscustomobject]@{
        Name = $package.name
        Version = $package.version
        License = if ($package.license) { $package.license } else { 'see license file' }
        Repository = $package.repository
        Refs = (@($refs) | Select-Object -Unique) -join ' '
    }
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('THIRD-PARTY SOFTWARE NOTICES FOR GLASS MIXER')
$lines.Add('')
$lines.Add('Glass Mixer is distributed under the MIT License (see LICENSE). The executable')
$lines.Add('includes the following open-source components, each under its own license.')
$lines.Add('License and notice texts follow the list; [n] refers to text n below.')
$lines.Add('')
foreach ($entry in ($entries | Sort-Object Name, Version)) {
    $refs = if ($entry.Refs) { $entry.Refs } else { '(the crate ships no license file; standard texts: https://spdx.org/licenses/)' }
    $lines.Add("$($entry.Name) $($entry.Version)  -  $($entry.License)  $refs")
    if ($entry.Repository) { $lines.Add("    $($entry.Repository)") }
}
for ($i = 0; $i -lt $texts.Count; $i++) {
    $lines.Add('')
    $lines.Add(('=' * 78))
    $lines.Add("[$($i + 1)] Used by: $($textUsers[$i] -join ', ')")
    $lines.Add(('=' * 78))
    $lines.Add($texts[$i])
}

$target = [IO.Path]::GetFullPath($OutFile)
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
[IO.File]::WriteAllLines($target, $lines, (New-Object Text.UTF8Encoding($false)))
Write-Host "Third-party notices: $target ($($shipped.Count) crates, $($texts.Count) distinct license texts)"
