# Limb6 Rhythm Similarity v0.2.0

Research-only three-way rhythm-similarity model for simultaneous six-limb-lead
ECG reports. It is not a medical device and must not be used for diagnosis or
treatment.

## Intended Output

The model estimates similarity to:

- `sinus-rhythm-like`
- `af-like`
- `other/noisy`

A class name is displayed only when technical quality is `GOOD`, the winning
probability passes its class threshold, and the top-two probability margin is
at least 0.15. Otherwise the runtime reports `ABSTAIN` while retaining the
probabilities and deterministic measurements.

## Input and Preprocessing

- Ten seconds at 100 Hz: tensor shape `[1, 6, 1000]`.
- I and II are independent inputs.
- III, aVR, aVL, and aVF are algebraically derived from I and II in both the
  training tensors and device runtime.
- Each lead is median-centered.
- All leads share the 95th-percentile absolute I/II scale, floored at 0.05 mV.
- Normalized values are clipped to +/-6.

## Training

- Dataset: PTB-XL 1.0.3 `records100`.
- Selected records: all 21,799 records meeting the documented label policy.
- Train: folds 1-8, 17,418 records.
- Calibration and threshold selection: fold 9, 2,183 records.
- Held-out test: fold 10, 2,198 records.
- Architecture: 1.12-million-parameter residual 1D CNN with temporal mean,
  variability, and maximum pooling.
- Augmentation: gain, time shift, global polarity reversal, baseline wander,
  and white noise.
- Probability calibration: regularized per-class vector scaling.

The label policy is unchanged from v0.1.0: uncontaminated `SR` records are
sinus-rhythm-like, uncontaminated `AFIB` records are AF-like, and all remaining
records are other/noisy.

## Held-out Performance

| Metric | v0.1.0 | v0.2.0 |
| --- | ---: | ---: |
| Balanced accuracy | 0.700 | 0.680 |
| Macro one-vs-rest AUROC | 0.832 | 0.839 |
| Accepted coverage | 5.8% | 48.3% |
| Accepted accuracy | 92.2% | 83.5% |

| Class | Precision | Recall | F1 | Accepted precision | Accepted records |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sinus-rhythm-like | 0.736 | 0.905 | 0.812 | 0.823 | 775 |
| AF-like | 0.673 | 0.697 | 0.685 | 0.771 | 70 |
| Other/noisy | 0.698 | 0.437 | 0.537 | 0.898 | 216 |

These figures describe one held-out PTB-XL fold. They are not Kardia-domain or
clinical-performance claims. The increased read coverage deliberately trades
some selective accuracy compared with v0.1.0.

## Decision Thresholds

| Class | Threshold |
| --- | ---: |
| Sinus-rhythm-like | 0.768 |
| AF-like | 0.784 |
| Other/noisy | 0.750 |

The thresholds were selected on validation fold 9 for empirical precision
targets of 0.85, 0.80, and 0.85 respectively, with at least 30 accepted
validation records per class. Held-out precision was slightly lower for the
first two classes, which demonstrates why external validation remains
necessary.

## Public Sample Smoke Test

The repository's anonymized 9.96-second public sample produced:

```text
SIN=57.6%  AF=0.3%  OTHER=42.1%
ABSTAIN: top confidence below threshold
```

The deterministic analysis reported good technical quality, 69 bpm, 4.3% RR
coefficient of variation, PR 143 ms, QRS 100 ms, and QTcF 395 ms. These
measurements and the model output are experimental.

## Limitations

- Training data came from clinical 12-lead equipment; no Kardia recordings were
  used for training.
- Only two leads are independent, and no V1-V6 chest leads are present.
- Device polarity and amplitude calibration remain provisional.
- The broad other/noisy class combines many rhythms, morphologies, and signal
  problems.
- A classified output means model similarity, not clinical confirmation.
- The report must remain subject to qualified human review.
