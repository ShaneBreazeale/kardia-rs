# kardia-rs

Rust tooling for reverse engineering and studying the AliveCor KardiaMobile 6L BLE ECG stream.

The goal is a cross-platform ECG workbench for:

- recording raw BLE captures from a Kardia 6L device,
- documenting and replaying decoded ECG frames,
- deriving six-lead views from the device stream,
- exporting research-friendly recordings, and
- applying ML models to live or recorded ECG data.

This repository follows the same broad shape as `openstetho`: a small protocol/data core, a CLI that can make reproducible captures, a future live workbench UI, and a separate ML pipeline. ECG-specific protocol evidence stays isolated from UI, storage, and model experiments.

## Workspace

- `crates/kardia-core`: ECG sample types, lead derivation, recording metadata, and protocol-neutral data structures.
- `crates/kardia-ble`: BLE scanning/capture boundaries and Kardia-specific reverse-engineering notes in code.
- `crates/kardia-cli`: command-line workbench for capture, decode, replay, and export workflows.
- `docs/re`: reverse-engineering notes, packet captures, and protocol hypotheses.
- `scripts/bleak_probe.py`: Python/bleak diagnostic that replays the BLE startup sequence on Apple's reference CoreBluetooth stack, to separate library bugs from device behavior.
- planned `kardia-ui`: live ECG viewer/workbench for capture review, annotations, and model outputs.
- planned `model/`: training, validation, export, and regression scripts for ECG models.

## Current Status

Early reverse-engineering workbench. Confirmed so far against a live `Kardia6L F241` (firmware 3.0.1) on macOS:

- Scan, connect, GATT discovery, and characteristic reads all work.
- APK-derived service UUIDs, ECG modes, and the control-point unlock command (`<mode> K<sha256("Triangle"+name)[..16]>`) are modeled in `kardia-ble`.

**Known blocker — link-layer bonding.** The Kardia vendor characteristics (`ac06000x`) require an encrypted/bonded BLE link. Writing the unlock command returns ATT error 5 (*Insufficient Authentication*) and enabling notifications hangs until a bond exists. This was reproduced identically through Python/bleak, so it is a device security requirement, **not** a `btleplug` bug. There is no key to extract from the APK: the app calls bare `createBond()` and the bond LTK is negotiated live and stored by the OS. See `docs/re/kardia-6l.md` and `docs/re/apk-inspection.md`. To stream, establish a fresh OS-level bond (clear any phone bond first, then pair from the host, retrying as bonding is unreliable by design).

Packet decoding is intentionally still gated on replayable raw fixtures, which require an unblocked stream first.

## Development

```sh
cargo check
cargo test

# Find nearby Kardia 6L candidates.
cargo run -p kardia-cli -- scan --seconds 10

# Show all observed BLE peripherals when debugging advertisement visibility.
cargo run -p kardia-cli -- scan --seconds 10 --all

# Scan and immediately connect to the first Kardia candidate for GATT inspection.
cargo run -p kardia-cli -- gatt-dump --rescan 20

# Read every readable characteristic on the first Kardia candidate.
cargo run -p kardia-cli -- read-probe --rescan 90 --verbose

# Isolate the CCCD-write blocker: subscribe to the unencrypted battery notify
# (control) vs the ECG notify (target). Battery OK + ECG hang => bonding gate.
cargo run -p kardia-cli -- notify-probe --rescan 90 --verbose

# Attempt a raw notification capture in dual-lead 300 Hz mode against
# the first Kardia candidate found in the same scan.
cargo run -p kardia-cli -- capture-raw --rescan 20 \
    --seconds 30 --out captures/kardia-raw.csv
```

### Cross-checking BLE behavior with bleak

`scripts/bleak_probe.py` runs the same scan/read/subscribe/unlock sequence through
Apple's CoreBluetooth via [`bleak`](https://github.com/hbldh/bleak). Use it to decide
whether a stall is a library issue or a device requirement: if a step works in bleak
but hangs in `btleplug`, suspect the library; if it hangs in both, it is the device.

```sh
uv venv .venv
uv pip install --python .venv/bin/python bleak
.venv/bin/python scripts/bleak_probe.py --stream --scan-secs 120
```

## Reverse-Engineering Rules

- Keep raw captures and decoded interpretations separate.
- Preserve timestamps and connection metadata for every capture.
- Document every protocol assumption with the capture or observation that supports it.
- Prefer replayable fixtures over one-off live debugging.
