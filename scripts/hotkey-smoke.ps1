$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\glass-mixer.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Build the debug executable first: cargo build' }
$profile = Join-Path $root ('target\phase4-hotkey-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profile | Out-Null
[IO.File]::WriteAllText(
    (Join-Path $profile 'settings.json'),
    '{"version":4,"hotkeys":[{"action":"ToggleWindow","chord":"Ctrl+Alt+F23"}]}',
    (New-Object System.Text.UTF8Encoding($false))
)
$oldProfile = $env:GLASS_MIXER_DEV_PROFILE
$oldExit = $env:GLASS_MIXER_DEV_EXIT_AFTER_MS
$env:GLASS_MIXER_DEV_PROFILE = $profile
$env:GLASS_MIXER_DEV_EXIT_AFTER_MS = '12000'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GlassHotkeyTest {
    public delegate bool Visit(IntPtr hwnd, IntPtr value);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit callback, IntPtr value);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hwnd, System.Text.StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    public static void Send() {
        keybd_event(0x11, 0, 0, UIntPtr.Zero); // Ctrl
        keybd_event(0x12, 0, 0, UIntPtr.Zero); // Alt
        keybd_event(0x86, 0, 0, UIntPtr.Zero); // F23
        keybd_event(0x86, 0, 2, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        keybd_event(0x11, 0, 2, UIntPtr.Zero);
    }
}
'@
function Find-MixerWindow([int]$ProcessId) {
    $script:foundWindow = [IntPtr]::Zero
    $visit = [GlassHotkeyTest+Visit]{ param($h, $unused)
        [uint32]$owner = 0
        [void][GlassHotkeyTest]::GetWindowThreadProcessId($h, [ref]$owner)
        $title = New-Object System.Text.StringBuilder 80
        [void][GlassHotkeyTest]::GetWindowText($h, $title, 80)
        if ($owner -eq $ProcessId -and $title.ToString() -eq 'Glass Mixer') { $script:foundWindow = $h; return $false }
        return $true
    }
    [void][GlassHotkeyTest]::EnumWindows($visit, [IntPtr]::Zero)
    return $script:foundWindow
}
$app = $null
try {
    $app = Start-Process -FilePath $exe -PassThru
    $deadline = (Get-Date).AddSeconds(6)
    do {
        Start-Sleep -Milliseconds 100
        $app.Refresh()
        $hwnd = Find-MixerWindow $app.Id
    } while ($hwnd -eq [IntPtr]::Zero -and -not $app.HasExited -and (Get-Date) -lt $deadline)
    if ($hwnd -eq [IntPtr]::Zero) { throw 'The mixer window did not appear.' }
    Write-Host "Observed window $hwnd"
    Start-Sleep -Milliseconds 500
    [GlassHotkeyTest]::Send()
    Start-Sleep -Milliseconds 600
    if ([GlassHotkeyTest]::IsWindowVisible($hwnd)) { throw 'Global shortcut did not hide the window.' }
    [GlassHotkeyTest]::Send()
    Start-Sleep -Milliseconds 600
    if (-not [GlassHotkeyTest]::IsWindowVisible($hwnd)) { throw 'Global shortcut did not reopen the hidden window.' }
    Write-Host 'Global show/hide shortcut worked while the mixer was hidden.'
    $app.WaitForExit(15000) | Out-Null
    $app.Refresh()
    if (-not $app.HasExited -or $app.ExitCode -ne 0) { throw 'The debug app did not exit cleanly.' }
} catch {
    $log = Join-Path $profile 'logs\glass-mixer.log'
    if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 30 }
    throw
} finally {
    if ($app -and -not $app.HasExited) { $app.Kill() }
    $env:GLASS_MIXER_DEV_PROFILE = $oldProfile
    $env:GLASS_MIXER_DEV_EXIT_AFTER_MS = $oldExit
    $resolved = [IO.Path]::GetFullPath($profile)
    $targetRoot = [IO.Path]::GetFullPath((Join-Path $root 'target')).TrimEnd('\') + '\'
    if ($resolved.StartsWith($targetRoot + 'phase4-hotkey-', [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $resolved)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
