# kardia-rs

`kardia-rs` is a Rust reverse-engineering workbench for capturing, inspecting,
and decoding the Bluetooth Low Energy ECG stream from an AliveCor
KardiaMobile 6L.

The verified path connects to a bonded Kardia 6L, records every raw
notification, displays a live six-limb-lead view, inspects the observed packet
transport, and exports confirmed M2 recordings to analysis-friendly CSV.

> [!WARNING]
> This is independent research software, not a medical device. It is not
> affiliated with or endorsed by AliveCor, and its output must not be used for
> diagnosis or treatment. Signal polarity and physical-unit calibration remain
> unverified.

## Contents

- [Status](#status)
- [Requirements](#requirements)
- [Installation](#installation)
- [Quickstart](#quickstart)
- [Pairing and troubleshooting](#pairing-and-troubleshooting)
- [Capture format](#capture-format)
- [M2 and M4 findings](#m2-and-m4-findings)
- [Workspace](#workspace)
- [Development](#development)
- [Contributing](#contributing)
- [Privacy](#privacy)
- [License](#license)

## Status

Tested with a `Kardia6L F241` running firmware 3.0.1 on macOS 15.7.4:

| Capability | Status |
| --- | --- |
| BLE scan, connection, GATT discovery, and characteristic reads | Confirmed |
| Encrypted/bonded vendor characteristic access | Confirmed |
| Unlock/mode command generation | Confirmed against APK and device |
| M2 dual-channel stream | 36-byte packets, 9 interleaved `i16` pairs, 300 Hz/channel |
| Channel identity | Channel 1 = lead I; channel 2 = lead II |
| Live I/II/III/aVR/aVL/aVF display | Working in uncalibrated raw counts |
| Raw capture inspection and M2 CSV export | Working |
| M4 dual-lead 600 Hz request | Command sent, but observed transport remains M2-compatible at 300 Hz |
| Signal polarity and volts-per-count | Unknown |
| Linux and Windows BLE capture | Architecturally supported, not yet device-tested |

The detailed evidence log is in
[docs/re/kardia-6l.md](docs/re/kardia-6l.md). APK-derived constants and native
decoder boundaries are documented in
[docs/re/apk-inspection.md](docs/re/apk-inspection.md).

## Features

- Discovers Kardia-like BLE advertisements and inspects GATT services.
- Reproduces the vendor unlock command:
  `<mode> K<sha256("Triangle" + device_name)[..16]>`.
- Journals connection, bonding, subscription, unlock, and collection stages.
- Preserves raw command indications and ECG notifications before decoding.
- Displays a throttled six-lead view while lossless M2 capture continues.
- Distinguishes requested/nominal mode metadata from observed packet behavior.
- Inspects packet sizes, cadence, command indications, and M2 compatibility.
- Exports M2 samples with both raw channels and all six derived limb leads.
- Keeps captures out of Git by default because they may contain health data and
  device identifiers.

## Requirements

- Rust 1.82 or newer
- A KardiaMobile 6L
- Bluetooth Low Energy support
- An encrypted/bonded host-device relationship

Only macOS/CoreBluetooth has completed the full live-capture path so far.

## Installation

```sh
git clone https://github.com/ShaneBreazeale/kardia-rs.git
cd kardia-rs
cargo build --release
```

During development, replace `target/release/kardia` with
`cargo run -p kardia-cli --` in the examples below.

## Quickstart

Close the AliveCor mobile app before connecting so it does not claim the
device. Wake the Kardia, then scan:

```sh
cargo run -p kardia-cli -- scan --seconds 10
```

Capture 30 seconds in confirmed M2 mode and show live labeled output:

```sh
cargo run -p kardia-cli -- capture-raw \
  --mode m2 \
  --live \
  --rescan 20 \
  --seconds 30 \
  --out captures/kardia-m2.csv
```

The capture command persists every raw packet first, updates the live view at a
lower terminal rate, and prints an observed-transport inspection when it
finishes.

Inspect any saved capture again:

```sh
cargo run -p kardia-cli -- inspect-raw captures/kardia-m2.csv
```

Export a confirmed M2 capture:

```sh
cargo run -p kardia-cli -- export-six-lead-m2 \
  captures/kardia-m2.csv \
  --out captures/kardia-six-lead.csv
```

The exported columns include the raw channel pair, leads I and II, derived
III/aVR/aVL/aVF, an exact 300 Hz sample clock, and the original packet receive
timestamp. Values remain device-native counts, not millivolts.

## Pairing and Troubleshooting

The `ac06000x` vendor characteristics require an encrypted BLE link. Without a
valid bond, writes can return ATT error 5 (*Insufficient Authentication*) and
vendor notification subscriptions can stall.

If capture does not reach the unlock write:

1. Close the AliveCor app.
2. Forget or remove the Kardia relationship from the phone.
3. Remove the Kardia battery for about 15 seconds, then reinstall it.
4. Wake the device and rerun `capture-raw`.
5. Accept the macOS pairing dialog when it appears.

Run the lower-level probes when isolating a failure:

Inspect every service and characteristic:

```sh
cargo run -p kardia-cli -- gatt-dump --rescan 20
```

Read every readable characteristic:

```sh
cargo run -p kardia-cli -- read-probe --rescan 90 --verbose
```

Compare a standard battery subscription with the vendor ECG subscription:

```sh
cargo run -p kardia-cli -- notify-probe --rescan 90 --verbose
```

The device advertises for a short window. Wake it immediately before scanning,
and prefer `capture-raw` first when collecting evidence.

## Capture Format

Raw captures are append-only CSV-like text:

```text
 # kardia_raw_capture_v2 ... requested_mode=M2 nominal_sample_rate_hz=300 ...
 # stage unix_micros=... name="connect_and_discover" status="ok" detail=""
 # unix_micros,characteristic_uuid,payload_hex
1784854547341388,ac060002-328c-a28f-9846-5a8aa212661b,01
```

Header mode/rate fields describe what was requested from the device. They are
not treated as observed signal facts. `inspect-raw` derives packet cadence,
payload sizes, command indications, and compatible sample rates from the
records themselves. The parser remains backward-compatible with v1 captures.

## M2 and M4 Findings

Confirmed M2 notifications contain nine little-endian signed 16-bit pairs:

```text
[channel_1_sample_0, channel_2_sample_0, ... channel_1_sample_8, channel_2_sample_8]
```

At 33 1/3 packets/s, that is 300 samples/s/channel. Controlled contact-removal
tests identify channel 1 as lead I and channel 2 as lead II. Standard limb-lead
relationships derive the remaining four leads.

M4 is present in the APK as “dual lead, 600 Hz.” On the tested device it
returned indication `0x03`, but still produced 36-byte packets at 33.336
packets/s that decode cleanly as the same 300 Hz M2-compatible transport. The
software therefore records M4 raw data but does not label or export it as 600
Hz.

## Workspace

| Path | Responsibility |
| --- | --- |
| `crates/kardia-core` | Protocol-neutral samples, timestamps, recording metadata, and limb-lead derivation |
| `crates/kardia-ble` | Device identification, mode commands, live BLE capture, raw capture parsing, and packet decoding |
| `crates/kardia-cli` | Scan/probe/capture/inspect/export workflows and live terminal view |
| `docs/re` | Evidence log, APK findings, hypotheses, and unresolved questions |
| `scripts/bleak_probe.py` | Independent Python/CoreBluetooth probe for comparing `bleak` with `btleplug` |

The BLE layer owns device evidence and lossless capture. The core crate remains
independent of AliveCor-specific framing so future UI, replay, and model code
can consume stable ECG types.

## Cross-checking with Bleak

The Python probe runs the same scan/read/subscribe/unlock sequence through
Apple CoreBluetooth via
[`bleak`](https://github.com/hbldh/bleak):

```sh
uv venv .venv
uv pip install --python .venv/bin/python bleak
.venv/bin/python scripts/bleak_probe.py --stream --scan-secs 120
```

If a step works in `bleak` but not `btleplug`, investigate the Rust BLE layer.
If both fail at the same operation, investigate bonding or device behavior.

## Development

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The workspace currently has three crates and requires no external services for
unit tests. Live BLE verification requires the physical device.

## Contributing

Reverse-engineering changes should:

- preserve raw bytes and receive timestamps before interpretation;
- distinguish requested configuration from observed behavior;
- cite the device, firmware, host, command, and capture statistics;
- add parser/decoder tests with synthetic or deliberately anonymized fixtures;
- avoid committing personal ECG captures; and
- update [docs/re/kardia-6l.md](docs/re/kardia-6l.md) when evidence changes an
  assumption.

Please run the complete development command set before opening a pull request.

## Privacy

ECG captures are health-related data and may also contain device identifiers.
The repository ignores `captures/` by default. Do not publish a capture unless
it has been intentionally reviewed, minimized, and anonymized.

## License

Licensed under either the Apache License, Version 2.0 or the MIT License, at
your option, as declared by the workspace manifests.
