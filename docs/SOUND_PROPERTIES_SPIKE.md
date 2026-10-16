# Sound properties spike — Windows 11 build 26200

Run: `cargo run --example sound_properties_probe`, 2026-10-15. Read-only:
no stream initialization, property/registry writes, format changes or elevation.

## Ran and passed

All three active outputs and three inputs returned `PKEY_AudioEngine_DeviceFormat`.
Outputs: 24-bit/48000 Hz, 24-bit/192000 Hz and 16-bit/48000 Hz.
Inputs: mono 16-bit at 16000, 48000 and 44100 Hz. Internal mix formats used
32-bit float. These are distinct values and the UI labels them separately.

`IAudioClient::IsFormatSupported` was queried for stereo 16-bit PCM at
44100, 48000 and 96000 Hz in shared and exclusive modes. Results differed
by device and mode. `S_FALSE` supplies a closest format, not exact support.
These queries describe stream support, not a complete list of formats safe
to set as the device default. Format selection therefore opens Windows Settings.

All outputs activated `ISpatialAudioClient` but returned zero dynamic
objects. This cannot identify the selected spatial provider or prove Off.
The UI reports availability without inventing a provider list.

## Ran and failed / unavailable

No device exposed a usable `PKEY_AudioEndpoint_Disable_SysFx`. That legacy
flag cannot stand in for all driver/vendor enhancements. The expected mono
registry value was absent (error 2); absence is not Off. These are explicit
unavailable results, not application crashes.

## Not run and implementation decision

No format, effects, spatial or mono setter was invoked. Unused undocumented
policy-vtable setter slots remain untouched; layout, permissions and live
application behavior are unverified. Dolby/DTS providers, Windows 10,
cross-build compatibility and mono live-apply notifications were not tested.

Expandable sound-properties sections provide reliable readouts plus Windows
Sound and Accessibility Audio shortcuts, and warn of possible interruptions.
Property callbacks refresh device readouts without polling while hidden.
Mono stays unavailable in-app; no stale state or registry polling is introduced.

## API evidence

- [Endpoint properties](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audio-endpoint-properties)
- [GetMixFormat](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getmixformat)
- [IsFormatSupported](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported)

Signatures checked against installed windows-rs 0.62.2. Unit tests cover PCM,
extensible valid bits, truncated data and invalid channel counts.
