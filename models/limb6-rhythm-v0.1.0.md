# limb6-rhythm-v0.1.0 Model Card

`limb6-rhythm-v0.1.0` is a research-only three-way waveform-similarity model
for 10-second recordings containing limb leads I, II, III, aVR, aVL, and aVF.
It is not a medical device and must not be used for diagnosis or treatment.

## Intended Output

The model produces three probabilities in this fixed order:

1. `sinus-rhythm-like`
2. `af-like`
3. `other/noisy`

These labels describe similarity to the training categories, not a clinical
interpretation. “Sinus-rhythm-like” does not mean “normal ECG.” A recording is
reported only when deterministic signal quality is `GOOD`, the winning
probability passes its class threshold, and the top-two probability margin is
at least 0.20. Otherwise, the model abstains.

## Model and Input

- Architecture: six-channel one-dimensional residual CNN
- Parameters: 731,267
- Input: `[1, 6, 1000]` float32
- Sample rate: 100 Hz
- Duration: 10 seconds
- Leads: I, II, III, aVR, aVL, aVF
- Export: ONNX opset 17 with calibrated softmax output
- Calibration temperature: 1.291029
- ONNX/PyTorch maximum absolute parity error: `1.49e-7`
- ONNX SHA-256:
  `3d97de838a6c295768b07bfe7d1872bcc68c2472ca6ba3f773d45656c1f5743e`

Each lead is median-centered. A shared scale is the 95th percentile absolute
amplitude across the two independent I/II channels, floored at 0.05 mV.
Values are divided by that scale and clipped to ±6. The Rust implementation
applies the same normalization after reducing Kardia input from 300 to 100 Hz.

## Training Data and Labels

The model uses a deterministic 11,240-record subset of
[PTB-XL 1.0.3](https://physionet.org/content/ptb-xl/1.0.3/), distributed under
the dataset's CC BY 4.0 license. All records are 10-second `records100`
waveforms. PTB-XL's patient-aware recommended folds remain authoritative:

- folds 1-8: 6,859 training records;
- fold 9: 2,183 validation/calibration records; and
- fold 10: 2,198 held-out test records.

The training subset contains all 859 available AF-like training records and a
fixed-seed sample of 3,000 records from each common training class. Every fold
9 and fold 10 record is retained.

Label rules:

- `sinus-rhythm-like`: `SR` present, `AFIB` absent, and no listed signal
  contamination;
- `af-like`: `AFIB` present, `SR` absent, and no listed signal contamination;
- `other/noisy`: all other rhythms, ambiguous records, or PTB-XL
  baseline-drift, static-noise, burst-noise, or electrode-problem flags.

## Held-Out Results

Unthresholded three-way test performance:

| Metric | Result |
| --- | ---: |
| Balanced accuracy | 0.700 |
| Macro one-vs-rest AUROC | 0.832 |
| Sinus-like precision / recall / F1 | 0.753 / 0.819 / 0.784 |
| AF-like precision / recall / F1 | 0.587 / 0.771 / 0.667 |
| Other/noisy precision / recall / F1 | 0.624 / 0.509 / 0.561 |

The conservative accepted-output thresholds are:

| Class | Minimum probability |
| --- | ---: |
| Sinus-rhythm-like | 0.990 |
| AF-like | 0.990 |
| Other/noisy | 0.868 |

With those thresholds and the 0.20 margin requirement, the model accepted 128
of 2,198 held-out records (5.8% coverage) and 92.2% of accepted outputs matched
the dataset label. This selective accuracy is not a clinical performance
claim.

## Public Sample Smoke Test

The repository's anonymized 9.96-second public sample produced:

```text
SIN=38.4%  AF=0.1%  OTHER=61.5%
ABSTAIN: top confidence below threshold
```

This verifies tensor preprocessing, ONNX execution, manifest enforcement,
abstention, and report rendering. The rendered output is the
[public sample report](../docs/assets/kardia-six-lead-sample.svg). It does not
validate model accuracy on the Kardia device.

## Limitations

- Training data came from clinical 12-lead equipment, with only the six limb
  leads selected. No Kardia recordings were used for training.
- Kardia III/aVR/aVL/aVF are deterministic derivations of I and II, so only
  two channels contain independent information.
- Kardia amplitude calibration and device-native polarity remain provisional.
- The model cannot assess findings that require V1-V6.
- PTB-XL labels, hardware, acquisition conditions, and population differ from
  the intended Kardia research workflow.
- The heterogeneous `other/noisy` category is not a specific medical finding.
- External, clinician-reviewed Kardia-domain evaluation is required before
  making any diagnostic, screening, or generalization claim.

The machine-readable thresholds, split details, metrics, limitations, and
model digest are in
[`limb6-rhythm-v0.1.0.json`](limb6-rhythm-v0.1.0.json).
