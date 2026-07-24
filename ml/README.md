# kardia-ml

This directory contains the reproducible, research-only training pipeline for
the optional three-way six-limb-lead waveform classifier:

- `sinus-rhythm-like`
- `af-like`
- `other/noisy`

These are model-similarity labels, not diagnoses. The classifier must abstain
when signal quality, confidence, or separation between the top classes is
insufficient.

## Dataset

The models use PTB-XL 1.0.3 `records100`. Place `ptbxl_database.csv` and
`scp_statements.csv` under `ml/data/ptb-xl/`, then run the downloader. PTB-XL's
`strat_fold` values remain authoritative: folds 1-8 train, fold 9
validates/calibrates, and fold 10 remains held out.

The label policy is implemented in `ecg_ml.py` and recorded in every exported
model manifest. Records carrying the PTB-XL baseline-drift, static-noise,
burst-noise, or electrode-problem flags are assigned to `other/noisy`.

## Prepare and Train

```sh
cd ml
uv sync
uv run python download_ptbxl.py --all-records
uv run python prepare_ptbxl.py
uv run python train_classifier.py
```

Preparation writes memory-mapped tensors under ignored
`ml/data/prepared-v0.2/`. It uses PTB-XL I and II as the two independent
channels and derives III/aVR/aVL/aVF algebraically, matching the device runtime.
Training writes ignored checkpoints under `ml/checkpoints/`, then exports a
versioned ONNX model and JSON manifest into `models/`.

Version 0.2 adds the full training fold, stronger acquisition augmentation,
temporal statistics pooling, per-class probability calibration, and corrected
precision/coverage threshold selection. Its model card records the resulting
tradeoff: substantially greater accepted-output coverage with lower selective
accuracy than the deliberately restrictive v0.1 model.

Run the lightweight pipeline regression tests with:

```sh
uv run python -m unittest -v test_ecg_ml.py
```

The held-out metrics in the manifest describe PTB-XL performance only. They do
not establish performance on the Kardia device. Device-domain validation
requires independently reviewed Kardia recordings before any broader claim.
