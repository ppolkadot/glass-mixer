# Development and packaging scripts

Run these from Windows PowerShell in the repository root.

| Script | Purpose |
| --- | --- |
| `lifecycle.ps1` | Launches a debug build with a temporary, isolated settings profile; checks its window, single-instance guard, close-to-tray, and clean timed shutdown. |
| `ui-automation.ps1` | Reads the visible window's UI Automation tree and checks the four view selectors (in the order Mixer, Devices, Presets, Settings), that Settings has a single route, that Settings toggles expose descriptions through `FullDescription` or `HelpText`, and that every interactive control has an accessible name. It then activates the window and checks that real `Tab` traversal focuses and scrolls an off-screen control into view. `-ExerciseAudio` optionally invokes System sounds mute and volume controls, restoring their original values. |
| `capture-window.ps1` | Saves the selected Glass Mixer window to `target/window-capture.png` with `PrintWindow`. |
| `verify-ui.ps1` | Launches an isolated debug instance, runs the read-only UI Automation check, and captures its window. |
| `hotkey-smoke.ps1` | Starts an isolated debug instance with Ctrl+Alt+F23 bound to show/hide, sends the chord, and checks both hidden and visible states. `-Minimized` also tests opening from a hidden startup. |
| `package.ps1` | Builds a release portable zip and, when Inno Setup is installed, an installer. Both include `LICENSE` and `THIRD-PARTY-NOTICES.txt`. |
| `third-party-notices.ps1` | Writes `THIRD-PARTY-NOTICES.txt` (`-OutFile`) from `cargo metadata`: every crate compiled into the Windows executable, with its license and the license files it ships. `package.ps1` runs it; CI checks that it succeeds. |

The lifecycle, UI verification, and hotkey smoke scripts refresh the debug build before launching it. For direct UI Automation and capture, build and launch the app first:

```powershell
cargo build
powershell -ExecutionPolicy Bypass -File scripts\lifecycle.ps1
powershell -ExecutionPolicy Bypass -File scripts\ui-automation.ps1 -ProcessId 1234
powershell -ExecutionPolicy Bypass -File scripts\capture-window.ps1 -ProcessId 1234
```

The lifecycle script removes only its own temporary profile. In debug builds,
`GLASS_MIXER_DEV_PROFILE` also gives the app separate settings and a separate
single-instance mutex; `GLASS_MIXER_DEV_EXIT_AFTER_MS` makes it exit cleanly
after the given time. `GLASS_MIXER_DEV_RENDERER=wgpu` selects the opaque
fallback path for a debug-profile test, and `--renderer=wgpu` forces the
opaque fallback on the command line in both release and debug builds.
`lifecycle.ps1` and `verify-ui.ps1` also accept `-Renderer wgpu`. Release
builds ignore the `GLASS_MIXER_DEV_*` environment variables.

Packaging skips signing when `GLASS_MIXER_SIGN_CERT_SHA1` is unset. To sign,
install a code-signing certificate with its private key in the current user's
certificate store, then set the 40-digit thumbprint in that environment variable.
Set `GLASS_MIXER_SIGNTOOL` if `signtool.exe` is not on `PATH`. Optionally set
`GLASS_MIXER_SIGN_TIMESTAMP_URL` to an RFC 3161 timestamp service. The script
signs and verifies both the release exe and, if built, the installer before
reporting success. Keep the certificate and private key out of this repository.
