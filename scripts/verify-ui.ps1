param([ValidateSet('glow', 'wgpu')][string]$Renderer = 'glow')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\debug\glass-mixer.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Build the debug executable first: cargo build' }
$profile = Join-Path $root ('target\phase4-ui-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profile | Out-Null
$oldProfile = $env:GLASS_MIXER_DEV_PROFILE
$oldExit = $env:GLASS_MIXER_DEV_EXIT_AFTER_MS
$oldRenderer = $env:GLASS_MIXER_DEV_RENDERER
$env:GLASS_MIXER_DEV_PROFILE = $profile
$env:GLASS_MIXER_DEV_EXIT_AFTER_MS = '20000'
$env:GLASS_MIXER_DEV_RENDERER = $Renderer
$app = $null
try {
    $app = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 3
    & (Join-Path $PSScriptRoot 'ui-automation.ps1') -ProcessId $app.Id
    & (Join-Path $PSScriptRoot 'capture-window.ps1') -ProcessId $app.Id -Output (Join-Path $root 'target\phase4-ui.png')
    $app.WaitForExit(23000) | Out-Null
    $app.Refresh()
    if (-not $app.HasExited -or $app.ExitCode -ne 0) { throw 'The debug app did not exit cleanly.' }
} finally {
    if ($app -and -not $app.HasExited) { $app.Kill() }
    $env:GLASS_MIXER_DEV_PROFILE = $oldProfile
    $env:GLASS_MIXER_DEV_EXIT_AFTER_MS = $oldExit
    $env:GLASS_MIXER_DEV_RENDERER = $oldRenderer
    $resolved = [IO.Path]::GetFullPath($profile)
    $targetRoot = [IO.Path]::GetFullPath((Join-Path $root 'target')).TrimEnd('\') + '\'
    if ($resolved.StartsWith($targetRoot + 'phase4-ui-', [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $resolved)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
