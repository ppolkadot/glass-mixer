param(
    [int]$ProcessId,
    [switch]$ExerciseAudio
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
. (Join-Path $PSScriptRoot 'window-handle.ps1')
if (-not $ProcessId) {
    $candidates = @(Get-Process glass-mixer -ErrorAction SilentlyContinue |
        Where-Object { [IntPtr]::Zero -ne (Get-GlassWindowHandle $_.Id) })
    if ($candidates.Count -ne 1) { throw 'Pass -ProcessId for the visible Glass Mixer window.' }
    $ProcessId = $candidates[0].Id
}
Get-Process -Id $ProcessId -ErrorAction Stop | Out-Null
$hwnd = Get-GlassWindowHandle $ProcessId
if ($hwnd -eq [IntPtr]::Zero) { throw 'The mixer window was not found.' }
$root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
$trueCondition = [System.Windows.Automation.Condition]::TrueCondition
$elements = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $trueCondition))
$names = @($elements | ForEach-Object { $_.Current.Name } | Where-Object { $_ })
foreach ($tab in @('Mixer', 'Devices', 'Settings', 'Presets')) {
    if ($names -notcontains $tab) { throw "UI Automation did not expose the $tab view." }
}
Write-Host "UI Automation: four view selectors found in PID $ProcessId."
if (-not $ExerciseAudio) { return }

# Audio mutations are opt-in. Every successful change is reversed in finally.
$slider = $elements | Where-Object { $_.Current.Name -like 'System sounds volume*' } | Select-Object -First 1
$mute = $elements | Where-Object { $_.Current.Name -like 'Mute System sounds*' } | Select-Object -First 1
if (-not $slider -or -not $mute) {
    Write-Warning 'System sounds controls are unavailable; audio exercise skipped.'
    return
}
$range = $null
$invoke = $null
try { $range = $slider.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern) } catch {}
try { $invoke = $mute.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern) } catch {}
if (-not $range -or -not $invoke) {
    Write-Warning 'System sounds controls do not expose the expected patterns; audio exercise skipped.'
    return
}
$original = $range.Current.Value
$muted = $false
try {
    $invoke.Invoke()
    $muted = $true
    $target = [Math]::Min($range.Current.Maximum, $original + 1)
    if ($target -eq $original) { $target = [Math]::Max($range.Current.Minimum, $original - 1) }
    $range.SetValue($target)
    Write-Host "Audio controls accepted mute and range actions (original $original, trial $target)."
} finally {
    try { $range.SetValue($original) } finally {
        if ($muted) { $invoke.Invoke() }
    }
}
Write-Host 'Audio control values restored.'
