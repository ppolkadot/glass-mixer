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
function Assert-NamedInteractiveControls([object[]]$viewElements, [string]$viewLabel) {
    $unnamed = @($viewElements | Where-Object { $interactive -contains $_.Current.ControlType.ProgrammaticName -and [string]::IsNullOrWhiteSpace($_.Current.Name) })
    if ($unnamed.Count -gt 0) {
        $kinds = ($unnamed | ForEach-Object { $_.Current.ControlType.ProgrammaticName.Replace('ControlType.', '') }) -join ', '
        throw "${viewLabel}: $($unnamed.Count) interactive control(s) have no accessible name: $kinds"
    }
}
function Invoke-UiaButton([System.Windows.Automation.AutomationElement]$button, [string]$label) {
    if (-not $button) { throw "UI Automation could not find the '$label' button." }
    try {
        $invoke = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        if ($invoke) { $invoke.Invoke(); return }
    } catch {}
    try {
        $toggle = $button.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
        if ($toggle) { $toggle.Toggle(); return }
    } catch {}
    $patterns = ($button.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) -join ', '
    throw "The '$label' button did not expose InvokePattern or TogglePattern (supported: $patterns)."
}
Assert-NamedInteractiveControls $elements 'Initial view'
$settingsButton = $elements | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and $_.Current.Name -eq 'Settings' } | Select-Object -First 1
if (-not ('GlassUiaKey' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GlassUiaKey {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X; public int Y; }
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);
    public static bool SendTab(IntPtr hWnd) {
        if (!PostMessageW(hWnd, 0x0100, new IntPtr(0x09), new IntPtr(0x000F0001))) return false;
        if (!PostMessageW(hWnd, 0x0101, new IntPtr(0x09), unchecked((IntPtr)0xC00F0001))) return false;
        return true;
    }
    public static bool GetClientScreenBounds(IntPtr hWnd, out int left, out int top, out int right, out int bottom) {
        left = top = right = bottom = 0;
        RECT rc;
        if (!GetClientRect(hWnd, out rc)) return false;
        POINT tl = new POINT { X = rc.Left, Y = rc.Top };
        POINT br = new POINT { X = rc.Right, Y = rc.Bottom };
        if (!ClientToScreen(hWnd, ref tl) || !ClientToScreen(hWnd, ref br)) return false;
        left = tl.X; top = tl.Y; right = br.X; bottom = br.Y;
        return (right > left) && (bottom > top);
    }
}
'@
}
function Get-UiaDescription([System.Windows.Automation.AutomationElement]$element) {
    if (-not [string]::IsNullOrWhiteSpace($element.Current.HelpText)) {
        return [string]$element.Current.HelpText
    }
    $hnodeField = [System.Windows.Automation.AutomationElement].GetField('_hnode', [System.Reflection.BindingFlags]'Instance,NonPublic')
    $uiaCoreApi = [System.Windows.Automation.AutomationElement].Assembly.GetType('MS.Internal.Automation.UiaCoreApi')
    $getProp = $uiaCoreApi.GetMethod('RawUiaGetPropertyValue', [System.Reflection.BindingFlags]'Static,NonPublic,Public')
    if ($hnodeField -and $getProp) {
        $hnode = $hnodeField.GetValue($element)
        $args = [object[]]@($hnode, 30159, $null)
        $hr = [int]$getProp.Invoke($null, $args)
        if ($hr -eq 0 -and ($args[2] -is [string]) -and -not [string]::IsNullOrWhiteSpace([string]$args[2])) {
            return [string]$args[2]
        }
    }
    return ''
}
Invoke-UiaButton $settingsButton 'Settings'
Start-Sleep -Milliseconds 350
$settingsElements = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $trueCondition))
Assert-NamedInteractiveControls $settingsElements 'Settings view'
foreach ($toggleName in @('Glass effect', 'Level meters')) {
    $toggle = $settingsElements | Where-Object { $_.Current.Name -eq $toggleName } | Select-Object -First 1
    if (-not $toggle) { throw "Settings view did not expose the '$toggleName' toggle." }
    $desc = Get-UiaDescription $toggle
    if ([string]::IsNullOrWhiteSpace($desc)) {
        throw "Settings toggle '$toggleName' did not expose a UI Automation description (FullDescription/HelpText)."
    }
}
[int]$clientLeft = 0; [int]$clientTop = 0; [int]$clientRight = 0; [int]$clientBottom = 0
if (-not [GlassUiaKey]::GetClientScreenBounds($hwnd, [ref]$clientLeft, [ref]$clientTop, [ref]$clientRight, [ref]$clientBottom)) {
    throw "Could not query client viewport bounds for HWND $hwnd."
}
$openFolder = $settingsElements | Where-Object { $_.Current.Name -eq 'Open settings folder' } | Select-Object -First 1
if (-not $openFolder) { throw "Settings view did not expose 'Open settings folder'." }
# Ensure focus starts at the top of Settings and confirm the target starts below the client viewport.
$backToMixer = $settingsElements | Where-Object { $_.Current.Name -eq 'Back to mixer (Esc)' } | Select-Object -First 1
if ($backToMixer -and $openFolder.Current.BoundingRectangle.Bottom -le $clientBottom) {
    try { $backToMixer.SetFocus() } catch {}
    if (-not [GlassUiaKey]::SendTab($hwnd)) {
        throw "PostMessageW(VK_TAB) failed while resetting Settings scroll position."
    }
    Start-Sleep -Milliseconds 150
}
$initialRect = $openFolder.Current.BoundingRectangle
if ($initialRect.IsEmpty -or $initialRect.Bottom -le $clientBottom) {
    throw "Settings 'Open settings folder' did not start below the client viewport before Tab traversal (initial=$initialRect, clientBottom=$clientBottom)."
}
if ($openFolder.Current.HasKeyboardFocus) {
    throw "Settings 'Open settings folder' already had keyboard focus before Tab traversal."
}
$scrolledIntoView = $false
for ($step = 0; $step -lt 45; $step++) {
    if (-not [GlassUiaKey]::SendTab($hwnd)) {
        $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "PostMessageW(VK_TAB) failed at step $step (Win32 error $err)."
    }
    for ($poll = 0; $poll -lt 4; $poll++) {
        Start-Sleep -Milliseconds 30
        $rect = $openFolder.Current.BoundingRectangle
        $focused = $openFolder.Current.HasKeyboardFocus
        if ($focused -and -not $rect.IsEmpty -and $rect.Top -ge ($clientTop - 2) -and $rect.Bottom -le ($clientBottom + 2) -and $rect.Bottom -lt ($initialRect.Bottom - 20)) {
            $scrolledIntoView = $true
            break
        }
        if (-not $focused) { break }
    }
    if ($scrolledIntoView) { break }
}
if (-not $scrolledIntoView) {
    $finalRect = $openFolder.Current.BoundingRectangle
    $finalFocus = $openFolder.Current.HasKeyboardFocus
    throw "Settings live Tab traversal did not focus and scroll 'Open settings folder' into the client viewport (focused=$finalFocus, initial=$initialRect, final=$finalRect, client=$clientTop..$clientBottom)."
}
$mixerButton = $settingsElements | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and $_.Current.Name -eq 'Mixer' } | Select-Object -First 1
Invoke-UiaButton $mixerButton 'Mixer'
Start-Sleep -Milliseconds 250
$elements = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $trueCondition))
Write-Host 'UI Automation: view selectors are in order, Settings has one route, toggle descriptions are exposed via UIA FullDescription, live Tab traversal focuses and scrolls off-screen controls into the client viewport, and every interactive control has a name.'
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
