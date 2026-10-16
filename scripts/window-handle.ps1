# Resolve the visible or hidden eframe window by PID and title. A tray window
# can be returned by Process.MainWindowHandle, so scripts must not use it.
if (-not ('GlassWindowLookup' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class GlassWindowLookup {
    public delegate bool Visit(IntPtr hwnd, IntPtr value);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit callback, IntPtr value);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
'@
}
function Get-GlassWindowHandle([int]$ProcessId, [string]$ExpectedTitle = 'Glass Mixer') {
    $script:glassFound = [IntPtr]::Zero
    $visit = [GlassWindowLookup+Visit]{ param($h, $unused)
        [uint32]$owner = 0
        [void][GlassWindowLookup]::GetWindowThreadProcessId($h, [ref]$owner)
        $windowText = New-Object System.Text.StringBuilder 80
        [void][GlassWindowLookup]::GetWindowText($h, $windowText, 80)
        if ($owner -eq $ProcessId -and $windowText.ToString() -eq $ExpectedTitle) { $script:glassFound = $h; return $false }
        return $true
    }
    [void][GlassWindowLookup]::EnumWindows($visit, [IntPtr]::Zero)
    return $script:glassFound
}
