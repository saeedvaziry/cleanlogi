# Protocol notes

Both MX Master 3 and MX Master 4 use the direct Bluetooth HID++ 2.0 interface on
vendor usage page `0xFF43` (usage `0x0202`) over BLE, device index `0xFF`, long
reports only. On connect (and whenever the device reports a reconnection through
`0x1D4B`) CleanLogi pushes the config and diverts the configured controls; on quit
it hands them back to the device.

| Feature | Use |
|---|---|
| `0x1000` BatteryLevelStatus | MX Master 3: fn0 status / event0 `[level %, next level %, charging status]` |
| `0x1004` UnifiedBattery | MX Master 4: fn1 status / event0 `[soc %, level flags, charging status, external power]` |
| `0x1814` ChangeHost | Easy-Switch |
| `0x19B0` HapticFeedback* | fn1 get / fn2 set `[flags, level]`, fn4 play `[waveform, report]` |
| `0x19C0` ForceSensingButton* | fn1 caps `[?, ?, default, max, min]`, fn3 set `[button, force u16be, u16be]` |
| `0x1B04` ReprogControlsV4 | divert buttons; raw XY for the gesture button |
| `0x2110` SmartShift | MX Master 3: fn1 `[mode 1=free/2=ratchet, threshold, 0]`; third byte leaves the persistent default unchanged |
| `0x2111` SmartShiftEnhanced | MX Master 4: fn2 `[mode 1=free/2=ratchet, threshold, torque]` |
| `0x2121` HiResWheel | wheel kept undiverted so macOS scrolls natively |
| `0x2150` Thumbwheel | diverted for thumbwheel actions |
| `0x2201` AdjustableDpi | fn1 supported DPI list/range, fn3 `[sensor, dpi u16be]` |

Feature IDs are discovered at runtime, not selected by product name. The newer
battery and SmartShift variants take precedence when both are present. Both
battery formats put the percentage in byte 0 and charging status in byte 2
(1 = recharging, 2 = slow recharge). DPI is rounded down to a supported value,
clamped at the ends of the device's list; unsupported presets cannot be selected
from the menu or DPI cycle. The saved preference is kept when switching mice.

The MX Master 3 has no `0x19B0`, `0x19C0` or haptic panel control (`0x01A0`). Its
other configurable button IDs match the MX Master 4. Only controls reported by
the mouse are configured and restored. Successful control and DPI capability
reads are cached for the connection, so restoring controls never depends on a
fresh enumeration. USB receiver routing is not implemented.

Public references: [SmartShift 0x2110](https://lekensteyn.nl/files/logitech/x2110_smartshift.html),
[AdjustableDpi 0x2201](https://lekensteyn.nl/files/logitech/x2201_adjustabledpi.html),
and an [MX Master 3 feature dump](https://github.com/pwr-Solaar/Solaar/issues/563).
Protocol tests use simulated replies; physical Bluetooth behavior still needs
verification on an MX Master 3.

\* Undocumented. Function numbers and payloads were recovered from the Logi
Options+ agent (`devio::Feature19B0HapticFeedback`,
`devio::Feature19C0ForceSensingButton`); SmartShift sensitivity uses the same
mapping as `devio_feature_xsmartshift::get_sensitivity`.
