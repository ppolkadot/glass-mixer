param(
    [int]$ProcessId,
    [string]$Output = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\window-capture.png'),
    [string]$WindowTitle = 'Glass Mixer'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
. (Join-Path $PSScriptRoot 'window-handle.ps1')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GlassCapture {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
}
'@
if (-not $ProcessId) {
    $candidates = @(Get-Process glass-mixer -ErrorAction SilentlyContinue |
        Where-Object { [IntPtr]::Zero -ne (Get-GlassWindowHandle $_.Id $WindowTitle) })
    if ($candidates.Count -ne 1) { throw 'Pass -ProcessId for the visible Glass Mixer window.' }
    $ProcessId = $candidates[0].Id
}
Get-Process -Id $ProcessId -ErrorAction Stop | Out-Null
$hwnd = Get-GlassWindowHandle $ProcessId $WindowTitle
if ($hwnd -eq [IntPtr]::Zero) { throw 'The mixer window was not found.' }
$rect = New-Object GlassCapture+Rect
if (-not [GlassCapture]::GetWindowRect($hwnd, [ref]$rect)) { throw 'GetWindowRect failed.' }
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0 -or $width -gt 8000 -or $height -gt 8000) { throw 'Invalid window size.' }
$bitmap = New-Object System.Drawing.Bitmap($width, $height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $hdc = $graphics.GetHdc()
    try {
        if (-not [GlassCapture]::PrintWindow($hwnd, $hdc, 2)) { throw 'PrintWindow failed.' }
    } finally { $graphics.ReleaseHdc($hdc) }
    $path = [IO.Path]::GetFullPath($Output)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Host "Captured $width x $height window: $path"
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
