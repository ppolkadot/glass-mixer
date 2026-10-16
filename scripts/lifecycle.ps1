param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\debug\glass-mixer.exe'),
    [int]$ExitAfterMs = 12000,
    [ValidateSet('glow', 'wgpu')][string]$Renderer = 'glow'
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe)) { throw "Build the debug executable first: cargo build" }
$profile = Join-Path $env:TEMP ("glass-mixer-lifecycle-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profile | Out-Null
$previousProfile = $env:GLASS_MIXER_DEV_PROFILE
$previousExit = $env:GLASS_MIXER_DEV_EXIT_AFTER_MS
$previousRenderer = $env:GLASS_MIXER_DEV_RENDERER
$env:GLASS_MIXER_DEV_PROFILE = $profile
$env:GLASS_MIXER_DEV_EXIT_AFTER_MS = [string]$ExitAfterMs
$env:GLASS_MIXER_DEV_RENDERER = $Renderer
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class GlassWindow {
    public delegate bool Visit(IntPtr hwnd, IntPtr value);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit callback, IntPtr value);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
}
'@
function Find-Window([int]$ProcessId) {
    $found = [IntPtr]::Zero
    $visit = [GlassWindow+Visit]{ param($h, $unused)
        [uint32]$owner = 0
        [void][GlassWindow]::GetWindowThreadProcessId($h, [ref]$owner)
        $name = New-Object System.Text.StringBuilder 80
        [void][GlassWindow]::GetWindowText($h, $name, 80)
        if ($owner -eq $ProcessId -and $name.ToString() -eq 'Glass Mixer') { $script:foundWindow = $h; return $false }
        return $true
    }
    $script:foundWindow = [IntPtr]::Zero
    [void][GlassWindow]::EnumWindows($visit, [IntPtr]::Zero)
    return $script:foundWindow
}
$app = $null
$minimized = $null
try {
    $startup = [Diagnostics.Stopwatch]::StartNew()
    $app = Start-Process -FilePath $Exe -PassThru
    $deadline = (Get-Date).AddSeconds(8)
    do {
        Start-Sleep -Milliseconds 100
        $hwnd = Find-Window $app.Id
    } while ($hwnd -eq [IntPtr]::Zero -and -not $app.HasExited -and (Get-Date) -lt $deadline)
    if ($hwnd -eq [IntPtr]::Zero) { throw 'The debug window did not appear.' }
    $startup.Stop()
    Write-Host "Launched PID $($app.Id), window $hwnd, first window after $($startup.ElapsedMilliseconds) ms"
    $second = Start-Process -FilePath $Exe -PassThru
    if (-not $second.WaitForExit(5000)) { $second.Kill(); throw 'Second instance did not exit.' }
    if ($second.ExitCode -ne 0) { throw "Second instance exited with $($second.ExitCode)" }
    [void][GlassWindow]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    $app.Refresh()
    if ($app.HasExited -or [GlassWindow]::IsWindowVisible($hwnd)) { throw 'Close-to-tray did not leave a hidden running process.' }
    Write-Host 'Close-to-tray and second-instance checks passed.'
    $app.Refresh()
    $cpuBefore = $app.CPU
    Start-Sleep -Seconds 1
    $app.Refresh()
    Write-Host "Hidden CPU over 1 s: $([math]::Round(1000 * ($app.CPU - $cpuBefore))) ms; private memory: $([math]::Round($app.PrivateMemorySize64 / 1MB, 1)) MB"
    $app.WaitForExit($ExitAfterMs + 5000) | Out-Null
    $app.Refresh()
    if (-not $app.HasExited -or $app.ExitCode -ne 0) { throw 'The debug timeout did not shut down cleanly.' }
    Write-Host 'Clean shutdown passed.'
    $settingsPath = Join-Path $profile 'settings.json'
    if (-not (Test-Path -LiteralPath $settingsPath)) { throw 'Settings were not saved.' }
    $saved = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
    if (-not $saved.window) { throw 'Window geometry was not saved.' }
    Write-Host 'Settings and window geometry saved.'

    $minimized = Start-Process -FilePath $Exe -ArgumentList '--minimized' -PassThru
    $deadline = (Get-Date).AddSeconds(6)
    do {
        Start-Sleep -Milliseconds 100
        $hiddenWindow = Find-Window $minimized.Id
    } while ($hiddenWindow -eq [IntPtr]::Zero -and -not $minimized.HasExited -and (Get-Date) -lt $deadline)
    if ($hiddenWindow -eq [IntPtr]::Zero) { throw 'The minimized instance created no window.' }
    Start-Sleep -Seconds 1
    if ([GlassWindow]::IsWindowVisible($hiddenWindow)) { throw '--minimized did not hide the window.' }
    Write-Host '--minimized started hidden.'
    $minimized.WaitForExit($ExitAfterMs + 5000) | Out-Null
    $minimized.Refresh()
    if (-not $minimized.HasExited -or $minimized.ExitCode -ne 0) { throw 'The minimized instance did not exit cleanly.' }
} finally {
    if ($app -and -not $app.HasExited) { $app.Kill() }
    if ($minimized -and -not $minimized.HasExited) { $minimized.Kill() }
    $env:GLASS_MIXER_DEV_PROFILE = $previousProfile
    $env:GLASS_MIXER_DEV_EXIT_AFTER_MS = $previousExit
    $env:GLASS_MIXER_DEV_RENDERER = $previousRenderer
    $resolved = [IO.Path]::GetFullPath($profile)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot + 'glass-mixer-lifecycle-', [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $resolved)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
