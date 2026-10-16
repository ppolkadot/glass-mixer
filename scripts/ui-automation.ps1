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
$tabNames = @($elements | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and @('Mixer', 'Devices', 'Presets', 'Settings') -contains $_.Current.Name } | ForEach-Object { $_.Current.Name })
if (($tabNames -join ',') -ne 'Mixer,Devices,Presets,Settings') { throw "Unexpected view selector order: $($tabNames -join ', ')" }
$settingsRoutes = @($elements | Where-Object { $_.Current.Name -eq 'Settings' }).Count
if ($settingsRoutes -ne 1) { throw "Expected one 'Settings' control, found $settingsRoutes (duplicate navigation)." }
$interactive = 'ControlType.Button', 'ControlType.CheckBox', 'ControlType.RadioButton', 'ControlType.ComboBox', 'ControlType.Edit', 'ControlType.Spinner', 'ControlType.Slider', 'ControlType.Hyperlink'
$unnamed = @($elements | Where-Object { $interactive -contains $_.Current.ControlType.ProgrammaticName -and [string]::IsNullOrWhiteSpace($_.Current.Name) })
if ($unnamed.Count -gt 0) {
    $kinds = ($unnamed | ForEach-Object { $_.Current.ControlType.ProgrammaticName.Replace('ControlType.', '') }) -join ', '
    throw "$($unnamed.Count) interactive control(s) have no accessible name: $kinds"
}
Write-Host 'UI Automation: view selectors are in order, Settings has one route, and every interactive control has a name.'
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
