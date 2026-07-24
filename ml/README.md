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

The first model uses PTB-XL 1.0.3 `records100`. Place `ptbxl_database.csv` and
`scp_statements.csv` under `ml/data/ptb-xl/`, then run the downloader. It keeps
every record in folds 9 and 10, every AF-like training record, and a
fixed-seed sample from the two common training classes. PTB-XL's `strat_fold`
values remain authoritative: folds 1-8 train, fold 9 validates/calibrates, and
fold 10 remains held out.

The label policy is implemented in `ecg_ml.py` and recorded in every exported
model manifest. Records carrying the PTB-XL baseline-drift, static-noise,
burst-noise, or electrode-problem flags are assigned to `other/noisy`.

## Prepare and Train

```sh
cd ml
uv sync
uv run python download_ptbxl.py
uv run python prepare_ptbxl.py
uv run python train_classifier.py
```

Preparation writes memory-mapped tensors under ignored `ml/data/prepared/`.
Training writes ignored checkpoints under `ml/checkpoints/`, then exports a
small ONNX model and JSON manifest into `models/`.

The held-out metrics in the manifest describe PTB-XL performance only. They do
not establish performance on the Kardia device. Device-domain validation
requires independently reviewed Kardia recordings before any broader claim.
