# Protocol notes

The mouse exposes HID++ 2.0 on vendor usage page `0xFF43` (usage `0x0202`)
over BLE, device index `0xFF`, long reports only. On connect (and whenever the
device reports a reconnection through `0x1D4B`) cleanlogi pushes the config and
diverts the configured controls; on quit it hands them back to the device.

| Feature | Use |
|---|---|
| `0x1004` UnifiedBattery | battery level and charge events |
| `0x1814` ChangeHost | Easy-Switch |
| `0x19B0` HapticFeedback* | fn1 get / fn2 set `[flags, level]`, fn4 play `[waveform, report]` |
| `0x19C0` ForceSensingButton* | fn1 caps `[?, ?, default, max, min]`, fn3 set `[button, force u16be, u16be]` |
| `0x1B04` ReprogControlsV4 | divert buttons; raw XY for the gesture button |
| `0x2111` SmartShiftEnhanced | fn2 `[mode 1=free/2=ratchet, threshold, torque]` |
| `0x2121` HiResWheel | wheel kept undiverted so macOS scrolls natively |
| `0x2150` Thumbwheel | diverted for thumbwheel actions |
| `0x2201` AdjustableDpi | pointer speed |

\* Undocumented. Function numbers and payloads were recovered from the Logi
Options+ agent (`devio::Feature19B0HapticFeedback`,
`devio::Feature19C0ForceSensingButton`); SmartShift sensitivity uses the same
mapping as `devio_feature_xsmartshift::get_sensitivity`.
