# Public Protocol Provenance

## Purpose

This repository documents only the behavior necessary for an independently
written client to interoperate with a lawfully owned KardiaMobile 6L. Published
evidence is limited to reproducible device observations, standard protocol
identities, and clearly labeled inferences. Device credentials, unique
identifiers, and personal ECG captures are not published.

## Public Evidence Classes

Public protocol claims must be supported by one or more of these evidence
classes:

1. **Observed** — captured directly from a lawfully owned device using normal
   operating-system Bluetooth APIs.
2. **Controlled experiment** — reproduced by changing one contact or command
   condition while holding the remaining setup constant.
3. **Protocol identity** — derived from published Bluetooth specifications or
   standard limb-lead equations.
4. **Compatibility behavior** — required by the independently written client
   and confirmed against the bonded device without reproducing vendor source
   code.

Each claim should distinguish an observation from an inference and identify
unresolved calibration or device-version assumptions.

## Authentication Boundary

The vendor characteristics require a normally encrypted and bonded Bluetooth
link. This project relies on the operating system's standard pairing process;
it does not bypass pairing, extract a bond key, impersonate another account, or
access another person's device.

A compatibility command is sent only after the operating system establishes
the encrypted link. Public evidence records may state whether a redacted
command succeeded, failed, or produced a response, but must not include a
device-specific command value.

## Reproducible Device Observations

The public evidence log may include sanitized results from these project
commands:

```sh
cargo run -p kardia-cli -- gatt-dump --rescan 30
cargo run -p kardia-cli -- notify-probe --rescan 90
cargo run -p kardia-cli -- capture-raw --mode m2 --seconds 15 \
  --out captures/local-capture.csv
cargo run -p kardia-cli -- inspect-raw captures/local-capture.csv
```

Raw captures remain ignored. Tests use synthetic inputs or the single
explicitly authorized anonymized sample.

## Publication Checklist

Before committing protocol evidence:

- replace device names with `KardiaMobile 6L device`;
- remove suffixes, serials, peripheral IDs, command values, and local paths;
- report only aggregate packet counts, lengths, cadence, and decoded layout;
- do not paste raw ECG samples or Bluetooth payloads from a person;
- label hypotheses and nominal mode metadata separately from observations;
- retain the independent-research and non-diagnostic warnings.

## Legal and Trademark Notice

This is independent interoperability research, not legal advice and not a
medical device. It is not affiliated with or endorsed by AliveCor. Product
names are used only to identify compatibility targets; associated trademarks
belong to their respective owners.
