# APK Inspection Notes

## `/Users/shane/Downloads/alivecor-inc-kardia.apk`

Inspected: 2026-05-31

Result: this file does not appear to be the AliveCor Kardia app.

Evidence:

- SHA-256: `188bbdb899be2da401ac0471ba4d10693612fa1067f2ea0987cc6e1afcfd4374`
- Decoded manifest package: `cm.aptoide.pt`
- App name resource: `Aptoide`
- App version from `apktool.yml`: `9.22.5.3`
- Manifest contains no Android Bluetooth or BLE permissions.
- Manifest application/activity classes are Aptoide classes such as `cm.aptoide.pt.NotificationApplicationView` and `cm.aptoide.pt.view.MainActivity`.
- Targeted searches over decoded resources and manifest found no `AliveCor` or `Kardia` references.

Conclusion: treat this APK as an Aptoide app or wrapper mislabeled as `alivecor-inc-kardia.apk`. It should not drive the Kardia 6L BLE protocol model.

Next useful APK evidence:

- A real Kardia app APK should have an AliveCor package name, likely `com.alivecor.*`.
- The manifest should request some combination of Bluetooth permissions such as `BLUETOOTH`, `BLUETOOTH_ADMIN`, `BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT`, or location permissions for BLE scanning.
- High-signal strings/classes to search once the correct APK is available: `BluetoothGatt`, `BluetoothLeScanner`, `ScanFilter`, `connectGatt`, `setCharacteristicNotification`, `writeCharacteristic`, `UUID`, `Kardia`, `AliveCor`, `ECG`, and `recording`.

## `/Users/shane/Downloads/com.alivecor.aliveecg_5.29.1-41a0fe800-591_minAPI23(arm64-v8a,armeabi-v7a,x86,x86_64)(nodpi)_apkmirror.com.apk`

Inspected: 2026-05-31

Result: this is the AliveCor ECG app and contains high-value Kardia BLE protocol evidence.

Artifact identity:

- SHA-256: `caef219206d7178ccdfbf608162bc12df1494353c952957d4e2b0a0c5a0b94b1`
- Manifest package: `com.alivecor.aliveecg`
- Version: `5.29.1-41a0fe800`, version code `591`
- SDK range: min API 23, target API 33
- Decode tools: `apktool` completed; `jadx` completed with 15 decompiler errors but produced useful Java sources.

Manifest BLE permissions:

- `android.permission.BLUETOOTH_CONNECT`
- `android.permission.BLUETOOTH_SCAN` with `neverForLocation`
- legacy `BLUETOOTH` and `BLUETOOTH_ADMIN` with max SDK 30
- fine/coarse location permissions for older BLE scanning flows

Relevant classes:

- `com.alivecor.universal_monitor.bluetooth.ble.BLEDeviceConstants`
- `com.alivecor.universal_monitor.bluetooth.ble.EcgBleManager`
- `com.alivecor.universal_monitor.bluetooth.ble.BLEECGMode`
- `com.alivecor.universal_monitor.bluetooth.BluetoothDeviceController`
- `com.alivecor.universal_monitor.devices.BLEDevice`
- `com.alivecor.universal_monitor.devices.Kardia6LDevice`
- `com.alivecor.universal_monitor.devices.KardiaCardDevice`

Kardia 6L GATT constants:

- six-lead service: `AC060001-328C-A28F-9846-5A8AA212661B`
- six-lead command characteristic: `AC060002-328C-A28F-9846-5A8AA212661B`
- six-lead ECG characteristic: `AC060003-328C-A28F-9846-5A8AA212661B`
- single-lead service: `AC010001-F0A3-A691-444E-2A8AC9345D06`
- single-lead characteristic: `AC010002-F0A3-A691-444E-2A8AC9345D06`

Standard services used by the app:

- battery service: `0000180F-0000-1000-8000-00805F9B34FB`
- battery level characteristic: `00002A19-0000-1000-8000-00805F9B34FB`
- device info service: `0000180A-0000-1000-8000-00805F9B34FB`
- serial number characteristic: `00002A25-0000-1000-8000-00805F9B34FB`
- firmware revision characteristic: `00002A26-0000-1000-8000-00805F9B34FB`
- hardware revision characteristic: `00002A27-0000-1000-8000-00805F9B34FB`
- CCCD: `00002902-0000-1000-8000-00805F9B34FB`

Scan behavior:

- The app starts BLE scanning with no Android `ScanFilter`.
- It uses low-latency scan mode and zero report delay.
- It filters discovered devices in the scan callback by advertised service UUID.
- It accepts advertisements containing the Kardia 6L six-lead service or Kardia Card single-lead service.

Connection and stream startup:

- Connects with LE transport when available.
- Bonds before service discovery when needed.
- Requires the ECG service, command characteristic, ECG characteristic, battery characteristic, serial, firmware, and hardware characteristics before it marks the device ready to stream.
- Enables indications on the command characteristic.
- Writes an unlock/mode command to the command characteristic.
- Enables notifications on the ECG characteristic.
- Reads serial, firmware, hardware, and battery, then enables battery notifications.

Mode and unlock command:

- `M1`: single lead, 300 Hz
- `M2`: dual lead, 300 Hz
- `M3`: single lead, 600 Hz
- `M4`: dual lead, 600 Hz
- command payload: `<mode> K<first 16 lowercase hex chars of sha256("Triangle" + bluetooth device name)>`
- example for device name `Kardia6L` and dual-lead 300 Hz: `M2 Kd8a179a137775575`

ECG data path:

- ECG characteristic notifications are forwarded as raw bytes to `onEcgPacketReceived(bluetoothDevice, 2, 300, payload)`.
- Java does not expose the packet decoder. It passes raw notification bytes into native `BLEDevice.receiveData(byte[])`.
- `libuniversal_monitor_jni.so` contains likely decoder symbols including `ac::BLEDevice::ReceiveData`, `ac::ECGFrame`, `ac::DerivedLeads`, and lead strings for I, II, III, aVR, aVL, and aVF.

Bonding / pairing (no extractable key):

Inspected 2026-05-31 via `strings` over `classes*.dex`. The link-layer authentication
gate on the Kardia vendor characteristics (ATT error 5, see `kardia-6l.md`) is standard
Android bonding, not an app-held secret.

- The app calls bare `createBond()` (no-argument), i.e. it lets the OS run the BLE pairing handshake. There is **no** `setPin`/`setPasskey` with a literal value and no BLE out-of-band key in the dex. (The only `OOB`/`out of band` strings belong to the Google measurement SDK and an account email-confirmation flow, unrelated to BLE pairing.)
- Therefore the bond LTK is generated live during pairing and stored by the OS bluetooth stack. **It cannot be lifted from the APK.** The only APK-held secret is the `K...` unlock token (`sha256("Triangle" + name)[:16]`), which is an application-layer command written to `ac060002` *after* an encrypted link exists.
- The app ships remote-config flags showing bonding is treated as unreliable and is retried: `bleCallCreateBondFlag`, `bleAutoSetPinFlag`, `bleCallCreateBondLimit`, `bleCallCreateBondLimitRetry`, `bleCallCreateBondRetryTimeout`, plus a logged `createBond exception.` path. This matches the CoreBluetooth hang we observe: bonding the Kardia is flaky, and the official app loops `createBond()` to get through it.

Implication for this project: the blocker is real OS-level bonding, not a missing key. Establish a fresh bond from the host (clear any phone bond first), retrying as needed; once bonded the existing `K...` command and notification flow should work.
