# Kardia 6L Reverse-Engineering Notes

## Target

Device: AliveCor KardiaMobile 6L

Objective: identify the BLE services, characteristics, packet structure, sample rate, sample encoding, calibration, and lead ordering needed to reconstruct the ECG stream.

## Evidence Log

Add one section per observation. Include:

- date and host platform,
- tool or code revision used,
- device firmware/app state if known,
- raw advertisement data,
- GATT service/characteristic dump,
- raw notification bytes with timestamps,
- interpretation and confidence.

### 2026-05-31: Downloaded APK Check

Artifact: `/Users/shane/Downloads/alivecor-inc-kardia.apk`

Finding: not useful for Kardia BLE reverse engineering. It decodes as Aptoide (`cm.aptoide.pt`), has no BLE permissions, and contains no AliveCor/Kardia references in manifest/resources. See `docs/re/apk-inspection.md`.

### 2026-05-31: AliveECG APK 5.29.1 BLE Constants

Artifact: `/Users/shane/Downloads/com.alivecor.aliveecg_5.29.1-41a0fe800-591_minAPI23(arm64-v8a,armeabi-v7a,x86,x86_64)(nodpi)_apkmirror.com.apk`

Finding: useful Kardia 6L BLE metadata was recovered from the AliveCor app. See `docs/re/apk-inspection.md` for the full notes.

High-confidence constants:

- six-lead service: `AC060001-328C-A28F-9846-5A8AA212661B`
- command characteristic: `AC060002-328C-A28F-9846-5A8AA212661B`
- ECG notification characteristic: `AC060003-328C-A28F-9846-5A8AA212661B`
- command CCCD: `00002902-0000-1000-8000-00805F9B34FB`

Observed stream startup sequence:

1. Scan without Android filters.
2. Keep advertisements that contain the Kardia 6L six-lead service UUID.
3. Connect with LE transport and bond if needed.
4. Discover services and require ECG, command, battery, serial, firmware, and hardware characteristics.
5. Enable indications on the command characteristic.
6. Write the unlock/mode command.
7. Enable notifications on the ECG characteristic.
8. Persist raw ECG notifications with receive timestamps.

Command format:

- mode strings: `M1` single-lead 300 Hz, `M2` dual-lead 300 Hz, `M3` single-lead 600 Hz, `M4` dual-lead 600 Hz
- unlock token: `K` plus the first 16 lowercase hex chars of `sha256("Triangle" + bluetooth device name)`
- command payload example for device name `Kardia6L`, dual-lead 300 Hz: `M2 Kd8a179a137775575`

Java callback metadata:

- ECG notifications are forwarded as two leads at 300 Hz in the app path inspected.
- Packet decoding appears to happen in `libuniversal_monitor_jni.so`, not Java.

### 2026-05-31: Live Kardia6L F241 GATT Dump

Host: macOS via `btleplug`/CoreBluetooth

Command:

```sh
cargo run -p kardia-cli -- gatt-dump --rescan 30
```

Finding: the device advertised as `Kardia6L F241`, connected, and exposed the expected 6L service and command/ECG characteristics.

Services:

- device information: `0000180a-0000-1000-8000-00805f9b34fb`
  - model number: `00002a24-0000-1000-8000-00805f9b34fb`, read
  - serial number: `00002a25-0000-1000-8000-00805f9b34fb`, read
  - firmware revision: `00002a26-0000-1000-8000-00805f9b34fb`, read
  - hardware revision: `00002a27-0000-1000-8000-00805f9b34fb`, read
  - manufacturer name: `00002a29-0000-1000-8000-00805f9b34fb`, read
- battery: `0000180f-0000-1000-8000-00805f9b34fb`
  - battery level: `00002a19-0000-1000-8000-00805f9b34fb`, read + notify
- DFU-like service: `0000fe59-0000-1000-8000-00805f9b34fb`
  - `8ec90003-f315-4f60-9fb8-838830daea50`, write + indicate
- Kardia 6L six-lead ECG: `ac060001-328c-a28f-9846-5a8aa212661b`
  - command: `ac060002-328c-a28f-9846-5a8aa212661b`, write + indicate
  - ECG stream: `ac060003-328c-a28f-9846-5a8aa212661b`, notify
  - unknown/read: `ac060005-328c-a28f-9846-5a8aa212661b`, read
  - unknown/read: `ac060006-328c-a28f-9846-5a8aa212661b`, read

Follow-up attempt:

- `cargo run -p kardia-cli -- capture-raw --rescan 30 --seconds 15 --out captures/kardia-raw.csv`
- Result: no Kardia 6L candidate observed in that later 30s scan. The next capture attempt should run while the device is actively advertising and should use `capture-raw` first, before `gatt-dump`, to avoid the short advertising window.

### 2026-05-31: Live Read Probe and Stream Blocker

Host: macOS via `btleplug`/CoreBluetooth

Read probe command:

```sh
cargo run -p kardia-cli -- read-probe --rescan 90 --verbose
```

Result: connection, service discovery, and normal characteristic reads work.

Read values:

- model number `00002a24...`: `AC-019`
- serial number `00002a25...`: `2025081219902`
- firmware revision `00002a26...`: `3.0.1`
- hardware revision `00002a27...`: `19SC03.03`
- manufacturer name `00002a29...`: `AliveCor`
- battery level `00002a19...`: byte/string value `d`
- `ac060005-328c-a28f-9846-5a8aa212661b`: `0xf609`
- `ac060006-328c-a28f-9846-5a8aa212661b`: `0xc70b`

Stream attempts:

- Android-like sequence connected and discovered services, then timed out enabling command indications on `ac060002...`.
- Fallback sequence skipped command indications and timed out writing unlock command `M2 K934de27399369534`.
- Listen-only sequence skipped the unlock write and timed out enabling ECG notifications on `ac060003...`.

Interpretation: basic BLE connect/discover/read is working. The current blocker is CoreBluetooth completion of CCCD writes and command writes. Next probes should focus on macOS pairing/security, CoreBluetooth CCCD behavior, and whether writes complete when initiated from a bonded Android session first.

### 2026-05-31: Blocker Root Cause — Insufficient Authentication

Host: macOS via `bleak`/CoreBluetooth (reference stack, `scripts/bleak_probe.py`)

Purpose: isolate whether the CCCD-write/write hang is a `btleplug` bug or a device
requirement, by replaying the same sequence on Apple's own CoreBluetooth via bleak.

Result (device `Kardia6L F241`, awake):

- reads OK: model `AC-019`, `ac060005` = `0xfc08` (changed from earlier `0xf609`; likely live), `ac060006` = `0xc70b` (stable)
- subscribe battery `00002a19` (unencrypted control): **OK**
- subscribe ECG `ac060003` (notify-only): **HANG** (no CoreBluetooth callback in 15s)
- enable command indications `ac060002`: **HANG**
- write unlock `M2 K934de27399369534` to `ac060002`: **FAIL — `GATT Protocol Error 5: Insufficient Authentication`**

Conclusion (high confidence):

- The hang is **not** a `btleplug` bug. bleak/CoreBluetooth reproduces it exactly.
- Every Kardia vendor characteristic (`ac06000x`) requires an **encrypted/bonded** link. The unencrypted battery characteristic subscribes fine; the vendor characteristics return ATT error 5 (insufficient authentication) on write and stall on CCCD writes while CoreBluetooth tries and fails to elevate security.
- macOS CoreBluetooth only forms a bond when the peripheral issues a security request and a bond can complete. The stall means no bond is completing — most likely the device already holds a bond with the phone (BLE devices typically keep a single bond).

No key can be lifted from the APK to bypass this. The AliveCor app calls bare `createBond()` and lets the OS run BLE pairing; there is no static PIN/passkey/OOB key in the dex, so the bond LTK is negotiated live and stored by the OS, not the app. The only APK-held secret is the `K...` unlock token, which is an application-layer command written *after* the link is encrypted. The app also ships bonding-retry flags (`bleCallCreateBondLimitRetry`, `bleCallCreateBondRetryTimeout`, a logged `createBond exception.`), evidence that bonding the Kardia is flaky even for the official app. See `apk-inspection.md` ("Bonding / pairing").

Next action: clear the existing bond (close the AliveCor app, forget the device on the phone, pull the Kardia battery ~15s to reset stored pairing), wake the device, and rerun the probe while watching for the macOS pairing dialog. Retry if the first attempt hangs — bonding is unreliable by design. Once a bond exists it is system-wide, so the Rust/`btleplug` capture path should then complete the same writes.

## Working Assumptions

- A six-lead limb ECG can be represented by independent leads I and II, with III, aVR, aVL, and aVF derived.
- Raw units should stay unscaled until calibration is confirmed.
- BLE packet timing may matter; preserve receive timestamps even if packet payloads contain sequence numbers.

## Open Questions

- Does the device also advertise the older 6L single-lead service in current firmware?
- Does the stream expose one lead pair per packet, interleaved channels, or compressed batches?
- Are samples signed 16-bit, 24-bit, delta encoded, encrypted, or framed with checksums?
- Does 600 Hz mode work on the 6L hardware and app version under test?
- What do `ac060005` and `ac060006` contain?
