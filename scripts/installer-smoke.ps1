<#
.SYNOPSIS
  Installs, upgrades and uninstalls a TEST VARIANT of the installer in an isolated
  folder, and proves an existing Glass Mixer installation is untouched.

.DESCRIPTION
  The variant is generated from installer\glass-mixer.iss with a different AppId,
  name, shortcuts and registry values, so it can sit beside a real installation. It
  packages the debug build: a debug build honours GLASS_MIXER_DEV_PROFILE, which gives
  the running copy its own settings and single-instance guard, so it can run while a
  real Glass Mixer does. The script refuses to continue if any identifier of the real
  installation survives in the variant.

  Scenarios:
    1. Fresh install (files, uninstall entry, launch-at-login entry, shortcut).
    2. Upgrade over a RUNNING copy (the installer must close it; reports whether it
       exited cleanly or was terminated).
    3. Uninstall of a running copy (it must exit cleanly through --exit, and the files,
       registry entries and shortcut must be gone, with no process left behind).
  Before and after, the real installation's files, registry entries, shortcuts and
  processes are compared.

  Needs Inno Setup 6. Run from the repository root:
    powershell -ExecutionPolicy Bypass -File scripts\installer-smoke.ps1
#>
param(
    # Keep the temporary folder (installers, logs) for inspection.
    [switch]$KeepFiles
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path" }
$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 was not found. Install it with: winget install JRSoftware.InnoSetup' }

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$numeric = ((($version -split '[-+]')[0]) + '.0')

$realAppId = 'B6C1A2E4-6F5D-4C1B-9B7E-3A1D2F7C9E10'
$smokeAppId = 'B6C1A2E4-6F5D-4C1B-9B7E-3A1D2F7C5A5E'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$approvedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'

$work = Join-Path $env:TEMP ('gm-smoke-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
$installDir = Join-Path $work 'app'
$profileDir = Join-Path $work 'profile'
$outDir = Join-Path $work 'out'
$generated = Join-Path $root 'installer\smoke.generated.iss'
New-Item -ItemType Directory -Force $work, $profileDir, $outDir | Out-Null

$results = New-Object System.Collections.Generic.List[object]
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Name = $name; Ok = $ok; Detail = $detail })
    $mark = if ($ok) { 'PASS' } else { 'FAIL' }
    $line = "  $mark  $name"
    if ($detail) { $line += " ($detail)" }
    Write-Host $line -ForegroundColor $(if ($ok) { 'Green' } else { 'Red' })
}

# Everything about an existing real installation that this test must not change.
function Get-RealState {
    $realDir = Join-Path $env:LOCALAPPDATA 'Programs\Glass Mixer'
    $exe = Join-Path $realDir 'glass-mixer.exe'
    $uninstall = Get-ItemProperty -LiteralPath "$uninstallRoot\{$realAppId}_is1" -ErrorAction SilentlyContinue
    $processes = @(Get-Process glass-mixer -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$realDir\*" } |
        ForEach-Object { '{0}:{1}' -f $_.Id, $_.StartTime.Ticks }) -join ','
    [pscustomobject]@{
        Autostart       = (Get-ItemProperty -LiteralPath $runKey -Name GlassMixer -ErrorAction SilentlyContinue).GlassMixer
        StartupApproved = ((Get-ItemProperty -LiteralPath $approvedKey -Name GlassMixer -ErrorAction SilentlyContinue).GlassMixer) -join ','
        UninstallKey    = [bool]$uninstall
        UninstallVer    = $uninstall.DisplayVersion
        ExeHash         = $(if (Test-Path -LiteralPath $exe) { (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash } else { '' })
        FileCount       = $(if (Test-Path -LiteralPath $realDir) { @(Get-ChildItem -LiteralPath $realDir -Recurse -File).Count } else { 0 })
        StartMenuLink   = Test-Path -LiteralPath (Join-Path $programs 'Glass Mixer.lnk')
        DesktopLink     = Test-Path -LiteralPath (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Glass Mixer.lnk')
        Processes       = $processes
    }
}

function Stop-SmokeProcesses {
    Get-Process glass-mixer -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$installDir\*" } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}

function Start-SmokeApp {
    $env:GLASS_MIXER_DEV_PROFILE = $profileDir
    $env:GLASS_MIXER_DEV_EXIT_AFTER_MS = '900000'
    try { Start-Process -FilePath (Join-Path $installDir 'glass-mixer.exe') -ArgumentList '--minimized' -PassThru }
    finally { Remove-Item Env:GLASS_MIXER_DEV_PROFILE, Env:GLASS_MIXER_DEV_EXIT_AFTER_MS -ErrorAction SilentlyContinue }
}

function Wait-Until([scriptblock]$condition, [int]$milliseconds = 15000) {
    $end = (Get-Date).AddMilliseconds($milliseconds)
    while ((Get-Date) -lt $end) { if (& $condition) { return $true }; Start-Sleep -Milliseconds 200 }
    return [bool](& $condition)
}

function Read-AppLog { $log = Join-Path $profileDir 'logs\glass-mixer.log'; if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Raw } else { '' } }

function Install-Variant([string]$setup, [string]$logName) {
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$installDir`"", '/TASKS="autostart"', "/LOG=`"$work\$logName`"")
    Start-Process -FilePath $setup -ArgumentList $arguments -Wait -PassThru
}

try {
    Write-Host "Glass Mixer $version installer smoke test (variant, isolated in $work)"
    $before = Get-RealState
    Write-Host ("  Real installation present: {0}; running copies: {1}" -f $before.UninstallKey, $(if ($before.Processes) { $before.Processes.Split(',').Count } else { 0 }))

    # No 2>&1: in Windows PowerShell 5.1 redirected native stderr becomes a terminating error.
    cargo build --locked --quiet
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $iss = [IO.File]::ReadAllText((Join-Path $root 'installer\glass-mixer.iss'))
    $variant = $iss
    $variant = $variant -replace 'AppId=\{\{[0-9A-Fa-f-]+\}', "AppId={{$smokeAppId}"
    $variant = $variant -replace '(?m)^AppName=Glass Mixer\r?$', 'AppName=Glass Mixer Smoke Test'
    $variant = $variant -replace '(?m)^DefaultDirName=.*\r?$', 'DefaultDirName={localappdata}\Programs\Glass Mixer Smoke Test'
    $variant = $variant -replace '(?m)^DefaultGroupName=Glass Mixer\r?$', 'DefaultGroupName=Glass Mixer Smoke Test'
    $variant = $variant -replace '(?m)^SetupIconFile=.*\r?\n', ''
    $variant = $variant -replace '(?m)^Compression=.*\r?$', 'Compression=lzma2/fast'
    $variant = $variant -replace '(?m)^Source: "\.\.\\target\\release\\THIRD-PARTY-NOTICES\.txt".*\r?\n', ''
    $variant = $variant.Replace('..\target\release\glass-mixer.exe', '..\target\debug\glass-mixer.exe')
    $variant = $variant.Replace('Name: "{userprograms}\Glass Mixer"', 'Name: "{userprograms}\Glass Mixer Smoke Test"')
    $variant = $variant.Replace('Name: "{userdesktop}\Glass Mixer"', 'Name: "{userdesktop}\Glass Mixer Smoke Test"')
    $variant = $variant.Replace('ValueName: "GlassMixer"', 'ValueName: "GlassMixerSmoke"')
    $variant = $variant.Replace("'GlassMixer'", "'GlassMixerSmoke'")
    $variant = $variant.Replace('{userappdata}\GlassMixer', '{userappdata}\GlassMixerSmoke')

    # Refuse to go on if anything of the real installation is still in the variant.
    $leaks = @()
    if ($variant -match [regex]::Escape($realAppId)) { $leaks += 'the real AppId' }
    if ($variant -match "'GlassMixer'" -or $variant -match 'ValueName: "GlassMixer"') { $leaks += 'the real autostart value name' }
    if ($variant -match '\\GlassMixer[''"]') { $leaks += 'the real settings folder' }
    if ($variant -match '(?m)^AppName=Glass Mixer\r?$' -or $variant -match '\\Glass Mixer"') { $leaks += 'the real application or shortcut name' }
    if ($variant -match 'target\\release\\glass-mixer\.exe') { $leaks += 'the release executable' }
    if ($leaks.Count -gt 0) { throw "The test variant still contains: $($leaks -join ', '). Not continuing." }

    $variantB = $variant.Replace('Source: "..\README.md"', 'Source: "..\CHANGELOG.md"; DestDir: "{app}"; DestName: "marker-b.txt"; Flags: ignoreversion' + "`r`nSource: `"..\README.md`"")
    foreach ($spec in @(@('1', $variant, "$version-smoke1"), @('2', $variantB, "$version-smoke2"))) {
        [IO.File]::WriteAllText($generated, $spec[1], (New-Object System.Text.UTF8Encoding($false)))
        & $iscc "/DAppVersion=$($spec[2])" "/DAppVersionNumeric=$numeric" "/O$outDir" "/Fsmoke-$($spec[0])" $generated | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "ISCC failed for variant $($spec[0])" }
    }
    Write-Host '  Variant installers built.'

    Write-Host 'Fresh install:'
    $install = Install-Variant (Join-Path $outDir 'smoke-1.exe') 'install-1.log'
    Check 'the installer exits with 0' ($install.ExitCode -eq 0) "exit $($install.ExitCode)"
    $files = @('glass-mixer.exe', 'README.md', 'LICENSE.txt', 'fonts\LICENSE.txt')
    $missing = @($files | Where-Object { -not (Test-Path -LiteralPath (Join-Path $installDir $_)) })
    Check 'the application files are installed' ($missing.Count -eq 0) $(if ($missing) { "missing: $($missing -join ', ')" } else { "$($files.Count) files" })
    $entry = Get-ItemProperty -LiteralPath "$uninstallRoot\{$smokeAppId}_is1" -ErrorAction SilentlyContinue
    Check 'the uninstall entry shows this version' ($entry -and $entry.DisplayVersion -eq "$version-smoke1") "$($entry.DisplayVersion)"
    $autostart = (Get-ItemProperty -LiteralPath $runKey -Name GlassMixerSmoke -ErrorAction SilentlyContinue).GlassMixerSmoke
    Check 'launch at login points at the installed copy' ($autostart -eq ('"{0}\glass-mixer.exe" --minimized' -f $installDir)) $autostart
    Check 'the Start menu shortcut exists' (Test-Path -LiteralPath (Join-Path $programs 'Glass Mixer Smoke Test.lnk'))
    Check 'no copy was started by a silent install' (@(Get-Process glass-mixer -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" }).Count -eq 0)

    Write-Host 'Upgrade while the application is running:'
    $app = Start-SmokeApp
    $up = Wait-Until { (Read-AppLog) -match 'audio engine initialized' }
    Check 'the installed copy runs (in its own profile, beside the real one)' ($up -and -not $app.HasExited)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $upgrade = Install-Variant (Join-Path $outDir 'smoke-2.exe') 'install-2.log'
    $timer.Stop()
    Check 'the upgrade installer exits with 0' ($upgrade.ExitCode -eq 0) ("exit {0}, {1:N1} s" -f $upgrade.ExitCode, $timer.Elapsed.TotalSeconds)
    $app.Refresh()
    Check 'the running copy was closed by the upgrade' ($app.HasExited)
    $log = Read-AppLog
    $graceful = $log -match 'shutdown complete'
    Check 'it exited cleanly (saved and removed its tray icon), not terminated' $graceful $(if ($graceful) { 'shutdown complete logged' } else { 'no clean shutdown in its log' })
    $entry = Get-ItemProperty -LiteralPath "$uninstallRoot\{$smokeAppId}_is1" -ErrorAction SilentlyContinue
    Check 'the uninstall entry shows the new version' ($entry -and $entry.DisplayVersion -eq "$version-smoke2") "$($entry.DisplayVersion)"
    $entries = @(Get-ChildItem -LiteralPath $uninstallRoot | Where-Object { $_.PSChildName -like "{$smokeAppId}*" })
    Check 'there is still exactly one uninstall entry' ($entries.Count -eq 1) "$($entries.Count)"
    Check 'the new files were laid down' (Test-Path -LiteralPath (Join-Path $installDir 'marker-b.txt'))
    Check 'launch at login still points at the installed copy' ((Get-ItemProperty -LiteralPath $runKey -Name GlassMixerSmoke -ErrorAction SilentlyContinue).GlassMixerSmoke -eq ('"{0}\glass-mixer.exe" --minimized' -f $installDir))
    Check 'no process was left behind' (@(Get-Process glass-mixer -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" }).Count -eq 0)

    Write-Host 'Uninstall while the application is running:'
    Remove-Item -LiteralPath (Join-Path $profileDir 'logs') -Recurse -Force -ErrorAction SilentlyContinue
    $app = Start-SmokeApp
    [void](Wait-Until { (Read-AppLog) -match 'audio engine initialized' })
    $timer.Restart()
    $uninstall = Start-Process -FilePath (Join-Path $installDir 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$work\uninstall.log`"" -Wait -PassThru
    $timer.Stop()
    Check 'the uninstaller exits with 0' ($uninstall.ExitCode -eq 0) ("exit {0}, {1:N1} s" -f $uninstall.ExitCode, $timer.Elapsed.TotalSeconds)
    $app.Refresh()
    Check 'the running copy was closed' ($app.HasExited)
    $log = Read-AppLog
    Check 'it was asked to exit and did so cleanly' (($log -match 'exit requested') -and ($log -match 'shutdown complete'))
    Check 'the install folder is gone' (-not (Test-Path -LiteralPath $installDir))
    Check 'the uninstall entry is gone' (-not (Test-Path -LiteralPath "$uninstallRoot\{$smokeAppId}_is1"))
    Check 'the launch-at-login entry is gone' ($null -eq (Get-ItemProperty -LiteralPath $runKey -Name GlassMixerSmoke -ErrorAction SilentlyContinue).GlassMixerSmoke)
    Check 'the Start menu shortcut is gone' (-not (Test-Path -LiteralPath (Join-Path $programs 'Glass Mixer Smoke Test.lnk')))
    Check 'no process was left behind' (@(Get-Process glass-mixer -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" }).Count -eq 0)

    Write-Host 'The real installation:'
    $after = Get-RealState
    foreach ($property in $before.PSObject.Properties.Name) {
        Check "unchanged: $property" ("$($before.$property)" -eq "$($after.$property)") $(if ("$($before.$property)" -ne "$($after.$property)") { "was '$($before.$property)', now '$($after.$property)'" } else { '' })
    }
} finally {
    Stop-SmokeProcesses
    # Safety net: remove only the variant's own leftovers if a step failed midway.
    Remove-Item -LiteralPath "$uninstallRoot\{$smokeAppId}_is1" -Force -ErrorAction SilentlyContinue
    Remove-ItemProperty -LiteralPath $runKey -Name GlassMixerSmoke -ErrorAction SilentlyContinue
    Remove-ItemProperty -LiteralPath $approvedKey -Name GlassMixerSmoke -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $programs 'Glass Mixer Smoke Test.lnk') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $generated -Force -ErrorAction SilentlyContinue
    if (-not $KeepFiles) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
    else { Write-Host "Kept: $work" }
}

$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ''
if ($failed.Count -gt 0) {
    Write-Host ("{0} of {1} checks FAILED." -f $failed.Count, $results.Count) -ForegroundColor Red
    exit 1
}
Write-Host ("All {0} checks passed." -f $results.Count) -ForegroundColor Green
