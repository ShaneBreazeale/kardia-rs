# Kardia 6L Protocol Evidence

## Target

Device: AliveCor KardiaMobile 6L

Objective: identify the BLE services, characteristics, packet structure, sample rate, sample encoding, calibration, and lead ordering needed to reconstruct the ECG stream.

## Evidence Log

Add one section per observation. Include:

- date and host platform,
- tool or code revision used,
- device firmware/app state if known,
- sanitized advertisement capabilities,
- GATT service/characteristic dump,
- raw notification bytes with timestamps,
- interpretation and confidence.

Before publishing, remove local paths, peripheral IDs, serial numbers,
device-name suffixes, device-specific command values, and personal ECG
payloads. See [protocol-provenance.md](protocol-provenance.md) for the complete
public-evidence policy.

### 2026-05-31: Initial Interoperability Scope

The initial compatibility hypotheses below were validated against a normally
bonded device. Each later section distinguishes observed behavior from
inference and standard Bluetooth or limb-lead identities.

Initial compatibility hypotheses:

- six-lead service: `AC060001-328C-A28F-9846-5A8AA212661B`
- command characteristic: `AC060002-328C-A28F-9846-5A8AA212661B`
- ECG notification characteristic: `AC060003-328C-A28F-9846-5A8AA212661B`
- command CCCD: `00002902-0000-1000-8000-00805F9B34FB`
- nominal modes M1 through M4;
- compatibility command value generated locally after normal OS bonding.

The independently reproduced stream startup sequence is:

1. Scan without Android filters.
2. Keep advertisements that contain the Kardia 6L six-lead service UUID.
3. Connect with LE transport and bond if needed.
4. Discover services and require ECG, command, battery, serial, firmware, and hardware characteristics.
5. Enable indications on the command characteristic.
6. Write the locally generated compatibility command.
7. Enable notifications on the ECG characteristic.
8. Persist raw ECG notifications with receive timestamps.

### 2026-05-31: Live GATT Dump

Host: macOS via `btleplug`/CoreBluetooth

Command:

```sh
cargo run -p kardia-cli -- gatt-dump --rescan 30
```

Finding: a KardiaMobile 6L connected and exposed the expected 6L service and
command/ECG characteristics.

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
- serial number `00002a25...`: redacted
- firmware revision `00002a26...`: `3.0.1`
- hardware revision `00002a27...`: `19SC03.03`
- manufacturer name `00002a29...`: `AliveCor`
- battery level `00002a19...`: byte/string value `d`
- `ac060005-328c-a28f-9846-5a8aa212661b`: `0xf609`
- `ac060006-328c-a28f-9846-5a8aa212661b`: `0xc70b`

Stream attempts:

- Android-like sequence connected and discovered services, then timed out enabling command indications on `ac060002...`.
- Fallback sequence skipped command indications and timed out writing a
  redacted M2 compatibility command.
- Listen-only sequence skipped the unlock write and timed out enabling ECG notifications on `ac060003...`.

Interpretation: basic BLE connect/discover/read is working. The current blocker is CoreBluetooth completion of CCCD writes and command writes. Next probes should focus on macOS pairing/security, CoreBluetooth CCCD behavior, and whether writes complete when initiated from a bonded Android session first.

### 2026-05-31: Blocker Root Cause — Insufficient Authentication

Host: macOS via `bleak`/CoreBluetooth (reference stack, `scripts/bleak_probe.py`)

Purpose: isolate whether the CCCD-write/write hang is a `btleplug` bug or a device
requirement, by replaying the same sequence on Apple's own CoreBluetooth via bleak.

Result (KardiaMobile 6L device awake):

- reads OK: model `AC-019`, `ac060005` = `0xfc08` (changed from earlier `0xf609`; likely live), `ac060006` = `0xc70b` (stable)
- subscribe battery `00002a19` (unencrypted control): **OK**
- subscribe ECG `ac060003` (notify-only): **HANG** (no CoreBluetooth callback in 15s)
- enable command indications `ac060002`: **HANG**
- write the redacted M2 compatibility command to `ac060002`: **FAIL —
  `GATT Protocol Error 5: Insufficient Authentication`**

Conclusion (high confidence):

- The hang is **not** a `btleplug` bug. bleak/CoreBluetooth reproduces it exactly.
- Every Kardia vendor characteristic (`ac06000x`) requires an **encrypted/bonded** link. The unencrypted battery characteristic subscribes fine; the vendor characteristics return ATT error 5 (insufficient authentication) on write and stall on CCCD writes while CoreBluetooth tries and fails to elevate security.
- macOS CoreBluetooth only forms a bond when the peripheral issues a security request and a bond can complete. The stall means no bond is completing — most likely the device already holds a bond with the phone (BLE devices typically keep a single bond).

Independent device tests confirmed the boundary: the operating system must
establish a normal encrypted bond before vendor-characteristic access. This
project does not use a static pairing key, bypass pairing, extract bond
material, or impersonate an account. The compatibility command is sent only
after bonding.

Next action: clear the existing bond (close the AliveCor app, forget the device on the phone, pull the Kardia battery ~15s to reset stored pairing), wake the device, and rerun the probe while watching for the macOS pairing dialog. Retry if the first attempt hangs — bonding is unreliable by design. Once a bond exists it is system-wide, so the Rust/`btleplug` capture path should then complete the same writes.

### 2026-07-23: Capture Reliability Audit

The raw capture path now creates its `btleplug` notification receiver before
enabling command indications, writing the unlock command, or enabling ECG
notifications. `btleplug` uses a broadcast channel for these events, so the
previous ordering could lose an immediate command acknowledgement or the
first ECG packets before a receiver existed.

Capture files are now opened as soon as a peripheral is selected and contain
timestamped setup-stage records. Failed authentication, CCCD, and unlock
attempts therefore leave a replayable diagnostic artifact. The peripheral is
also disconnected on every exit path so a failed attempt does not consume the
device's next short advertising window.

### 2026-07-23: Fresh Bond and First Live M2 Capture

Host: macOS 15.7.4 via `btleplug` 0.11.8/CoreBluetooth

Procedure:

1. Closed the AliveCor app and cleared the phone-side relationship.
2. Power-cycled the Kardia and woke it for a fresh host connection.
3. Ran `capture-raw` first so GATT inspection did not consume the short
   advertising window.

Command:

```sh
cargo run -p kardia-cli -- capture-raw --rescan 60 --seconds 30 \
    --out captures/kardia-live-2026-07-23.csv
```

Result:

- Device: KardiaMobile 6L, firmware previously observed as 3.0.1.
- Command indication subscription: success.
- Compatibility command write: redacted M2 value, success.
- Command indication received: `0x01`.
- ECG notification subscription: success.
- Capture: 1,000 ECG packets / 36,000 bytes in 30 seconds.
- Every ECG payload was 36 bytes.
- Observed packet cadence: 33.337 packets/s over the capture.

Confirmed `M2` frame interpretation:

- nine sample pairs per notification;
- each channel value is a little-endian signed 16-bit integer;
- values are interleaved `channel 1, channel 2`;
- 33 1/3 packets/s × 9 sample pairs/packet = 300 samples/s/channel.

Little-endian interleaving is strongly supported by continuity: its median
absolute within-packet and packet-boundary deltas were both 104 raw counts.
Big-endian candidates had median deltas around 18,432 counts, and splitting
each packet into channel blocks produced much worse within-packet continuity.

The decoder preserves the two raw channels without scaling or bit masking.
Channel identity was later confirmed by controlled electrode-removal tests.
Signal polarity and ADC-to-voltage calibration remain unknown.

### 2026-07-23: Labeled Live Monitor Verification

Added a throttled `--live` observer to `capture-raw`. It renders `Ch1/I*`,
`Ch2/II*`, `III*`, `aVR*`, `aVL*`, and `aVF*` from decoded M2 frames while the
capture loop continues to persist every raw notification first. The asterisks
make the unverified channel-to-lead mapping visible in the UI.

Command:

```sh
cargo run -p kardia-cli -- capture-raw --live --rescan 60 --seconds 15 \
    --out captures/kardia-live-labeled-2026-07-23.csv
```

Result:

- saved bond reused successfully with no new pairing failure;
- unlock response `0x01`;
- 499 ECG packets / 17,964 bytes;
- all 499 payloads were 36 bytes;
- 33.329 packets/s over the captured span;
- 4,491 decoded channel pairs, representing 14.970 seconds at 300 Hz;
- labeled six-lead values updated in place without interrupting raw capture.

### 2026-07-23: Controlled Electrode Mapping

Two complementary contact-removal captures were recorded with the device arrow
pointing away from the torso, both top electrodes under the corresponding
fingers, and the bottom electrode on the bare left knee.

In the controlled leg-off capture, removing the bottom/left-leg electrode produced
two repeatable channel-2 rail episodes near +/-25,100 raw counts. During the
cleaner 17.233-18.697 second episode, channel 2 was beyond 20,000 counts for
79.5% of samples while channel 1 never crossed that threshold. Both channels
returned to normal after contact was restored.

In the controlled left-fingers-off capture, device rocking initially caused
additional bottom-contact artifacts. A clean 28.32-28.52 second interval then
isolated the intended condition: channel 1 was beyond 20,000 counts for 100% of
samples while channel 2 never crossed the threshold. Both channels returned to
normal after the left fingers were restored.

The complementary dependencies identify channel 1 as lead I (LA-RA) and channel
2 as lead II (LL-RA). The live display now removes the provisional asterisks.
This experiment confirms channel identity, not signal polarity or volts per raw
count; those remain open calibration tasks.

A final 10-second `--live` verification rendered all six confirmed
labels without asterisks. It captured 332 packets / 11,952 bytes, representing
2,988 channel pairs or 9.960 seconds at 300 Hz.

### 2026-07-23: M4 Dual-Lead 600 Hz Probe

The device accepted a redacted M4 compatibility command and returned command
indication `0x03`. A 15-second raw capture then produced 499 ECG notifications
/ 17,964 bytes. Every notification was 36 bytes, and the 14.938794-second
first-to-last packet span gave a cadence of 33.336 packets/s.

The payload remained consistent with the confirmed M2 layout:

- nine little-endian signed 16-bit channel pairs per packet;
- 33.336 packets/s x 9 pairs = 300.024 sample pairs/s;
- median absolute adjacent deltas of 108 and 92 raw counts;
- 100% of values in both channels divisible by four.

Reinterpreting each packet as 18 interleaved signed 8-bit pairs would nominally
produce 600 pairs/s, but the resulting channels alternate 16-bit low and high
bytes and do not form two plausible signals. The capture therefore contains no
evidence of an exposed 600 Hz dual-lead stream.

The meaning of indication `0x03` is not yet confirmed. It may echo the
zero-based M4 mode index, report an unsupported mode/fallback, or select a
higher internal ADC rate that is still decimated to the same 300 Hz transport.
Do not decode or label M4 captures as 600 Hz without further evidence.

Raw capture format v2 now names these header fields `requested_mode`,
`nominal_sample_rate_hz`, and `nominal_leads` so nominal compatibility metadata
cannot be mistaken for observation. The shared capture parser remains
compatible with v1.
The result above can be reproduced without an ad hoc analysis script:

```sh
cargo run -p kardia-cli -- inspect-raw \
    captures/local-capture.csv
```

The inspector reports requested metadata separately from payload length
distribution, packet cadence, command indication bytes, M2 packet
compatibility, and the sample rate implied by that observed transport. The M2
exporter also refuses captures whose header requested another mode.

### 2026-07-23: Standard-Grid Six-Lead Report

The CLI now renders confirmed M2 captures as simultaneous I, II, III, aVR, aVL,
and aVF strips on a dimensioned A4 landscape ECG grid. PDF and SVG use the same
geometry: 25 mm/s, 10 mm/mV, 1 mm minor grid lines, 5 mm major grid lines, and
a 1 mV by 200 ms calibration pulse. The renderer subtracts each selected
lead's median only to position its baseline and applies no waveform filter.
M4-tagged captures are rejected.

The initial voltage scale is an explicit, provisional inference. AliveCor's
published 6L specifications describe a 10 mV peak-to-peak input range, 14-bit
resolution, and 300 samples/s. In every inspected M2 and M4-compatible capture,
all signed 16-bit values are divisible by four. A 14-bit sample left-aligned in
the stored signed 16-bit field would therefore imply:

```text
mV = raw_i16 * (10 / 65536)
1 stored count = 0.000152587890625 mV
1 effective four-count ADC step = 0.0006103515625 mV
```

This inference produces plausible report amplitudes but is not electrical
calibration evidence. Reports visibly label amplitude and polarity as
provisional. Validation requires a known isolated ECG simulator signal or an
equivalent traceable reference; do not connect a mains-referenced bench
generator while electrodes are attached to a person.

### 2026-07-23: Experimental Automated Measurements

The report now includes a generic automated-measurements panel. It contains no
diagnostic interpretation and labels every result experimental. The analysis
pipeline is separate from the displayed waveform:

1. suppress baseline and smooth copies of the two independent I/II channels;
2. detect QRS candidates from combined derivative energy with a 250 ms
   refractory period;
3. refine and align QRS locations;
4. form representative I/II complexes from the sample-by-sample median of
   qualifying beats;
5. estimate global P onset, QRS onset/offset, and T offset from simultaneous
   vector magnitude and slope; and
6. withhold fields whose signal-to-noise, rhythm consistency, interval, or
   boundary checks fail.

Ventricular rate uses the number of first-to-last QRS intervals divided by
their elapsed time. QTcB is `QT / sqrt(RR)` and QTcF is `QT / cbrt(RR)`, with
QT in milliseconds and average RR in seconds. Frontal axes integrate I and II
over each detected wave and recover the orthogonal component with
`y = (2*II - I) / sqrt(3)` before `atan2(y, I)`.

On the 9.960-second confirmed-label capture, the first implementation found 12
QRS complexes and used 10 complete beats for the median complex. It reported
69 bpm, PR 143 ms, QRS 100 ms, QT/QTcB 377/405 ms, QTcF 395 ms, and
device-native P/QRS/T axes 61/62/48 degrees. These values are useful regression
evidence only; they have not been compared with manual annotations or a
validated analysis system.

### 2026-07-23: Optional ML Rhythm-Similarity Model

The report path can optionally load a versioned JSON manifest plus ONNX model.
The first bundle, `limb6-rhythm-v0.1.0`, is a three-way research classifier:
`sinus-rhythm-like`, `af-like`, and `other/noisy`.

The model is not part of packet decoding and never changes the rendered
waveform. Its analysis copy is reduced from 300 to 100 Hz, median-centered,
normalized by a shared I/II amplitude statistic, and clipped to the training
range. The runtime validates the model SHA-256, manifest schema, lead/label
order, tensor shape, research-only declaration, and decision thresholds before
inference.

The model must abstain unless deterministic technical quality is `GOOD`, the
winning class passes its threshold, and the top-two probability margin is at
least 0.20. Its report panel always identifies the result as research-only and
the Kardia domain as unvalidated.

End-to-end device smoke testing was performed locally with a private, untracked
capture. Its waveform, measurements, probability output, and generated reports
are intentionally not included in the repository. The smoke test confirmed
preprocessing, local ONNX execution, conservative abstention, and PDF/SVG
integration; it is not evidence of Kardia-domain model accuracy.
Training/evaluation provenance and held-out PTB-XL metrics are recorded in the
[`limb6-rhythm-v0.1.0` model card](../../models/limb6-rhythm-v0.1.0.md).

Version `limb6-rhythm-v0.2.0` expands training to the complete PTB-XL training
fold, derives the four dependent limb leads from I/II during preparation,
retains temporal variability during pooling, adds stronger acquisition
augmentation, and calibrates each class separately. It also fixes the v0.1
threshold search, which could fall back to 0.99 when a useful lower threshold
was excluded before the search began.

On held-out fold 10, v0.2 increased accepted-output coverage from 5.8% to
48.3%, while accepted accuracy decreased from 92.2% to 83.5%. Macro AUROC
increased from 0.832 to 0.839. This is a research tradeoff, not a claim of
clinical improvement. Full per-class results are in the
[`limb6-rhythm-v0.2.0` model card](../../models/limb6-rhythm-v0.2.0.md).

## Remaining Assumptions

- Raw captures and CSV exports should stay unscaled until calibration is
  confirmed. Reports may use the specification-derived provisional scale only
  when it is visibly disclosed.
- BLE packet timing may matter; preserve receive timestamps even if packet payloads contain sequence numbers.
- Automated fiducial points and measurement confidence thresholds remain
  experimental until validated on annotated public ECG databases.
- ML rhythm-similarity thresholds remain experimental and Kardia-domain
  performance is unvalidated.

## Open Questions

- Does the device also advertise the older 6L single-lead service in current firmware?
- Do command indications `0x00` through `0x03` echo the selected mode index?
- Does M4 fall back to M2, or does it sample internally at 600 Hz and export a
  decimated 300 Hz stream?
- What do `ac060005` and `ac060006` contain?
- Does an isolated 1 mV simulator input confirm the inferred
  `10 mV / 65536` stored-count scale and device-native polarity?
- How do automated interval and axis errors compare against an annotated
  reference set after resampling representative records to 300 Hz?
- How does the selective classifier perform on an independently reviewed,
  patient-level Kardia test set acquired with this electrode geometry?
