#!/usr/bin/env python3
"""Prepare PTB-XL's 100 Hz six-limb-lead tensors for repeatable training."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import numpy as np
import wfdb

from ecg_ml import (
    LABELS,
    LEADS,
    SAMPLE_RATE_HZ,
    SAMPLES,
    classify_metadata,
    load_metadata,
    normalize_record,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, default=Path("data/ptb-xl"))
    parser.add_argument("--out", type=Path, default=Path("data/prepared"))
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    metadata = load_metadata(args.dataset / "ptbxl_database.csv")
    selection_path = args.dataset / "selection.json"
    selection_summary = None
    if selection_path.exists():
        selection = json.loads(selection_path.read_text())
        selected_ids = set(selection["ecg_ids"])
        metadata = [row for row in metadata if int(row["ecg_id"]) in selected_ids]
        selection_summary = {
            key: value for key, value in selection.items() if key != "ecg_ids"
        }
        print(f"using deterministic selection of {len(metadata)} records")
    args.out.mkdir(parents=True, exist_ok=True)

    signals = np.lib.format.open_memmap(
        args.out / "signals.npy",
        mode="w+",
        dtype=np.float32,
        shape=(len(metadata), len(LEADS), SAMPLES),
    )
    labels = np.empty(len(metadata), dtype=np.uint8)
    folds = np.empty(len(metadata), dtype=np.uint8)
    record_ids = np.empty(len(metadata), dtype=np.uint32)

    for index, row in enumerate(metadata):
        record_path = args.dataset / row["filename_lr"]
        physical, fields = wfdb.rdsamp(str(record_path))
        if int(fields["fs"]) != SAMPLE_RATE_HZ:
            raise ValueError(f"{record_path}: expected {SAMPLE_RATE_HZ} Hz")
        names = {
            name.casefold(): signal_index
            for signal_index, name in enumerate(fields["sig_name"])
        }
        try:
            lead_indices = [names[lead.casefold()] for lead in LEADS]
        except KeyError as error:
            raise ValueError(f"{record_path}: missing required limb lead") from error

        selected = physical[:, lead_indices].T
        if selected.shape[1] != SAMPLES:
            raise ValueError(f"{record_path}: expected {SAMPLES} samples, got {selected.shape[1]}")
        signals[index] = normalize_record(selected)
        labels[index] = classify_metadata(row)
        folds[index] = int(row["strat_fold"])
        record_ids[index] = int(row["ecg_id"])

        if (index + 1) % 1_000 == 0 or index + 1 == len(metadata):
            print(f"prepared {index + 1}/{len(metadata)}")

    signals.flush()
    np.save(args.out / "labels.npy", labels)
    np.save(args.out / "folds.npy", folds)
    np.save(args.out / "record_ids.npy", record_ids)

    counts = Counter(int(label) for label in labels)
    summary = {
        "records": len(metadata),
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "samples": SAMPLES,
        "leads": LEADS,
        "labels": {LABELS[index]: counts[index] for index in range(len(LABELS))},
        "split": "PTB-XL strat_fold 1-8 train, 9 validation, 10 test",
        "normalization": "per-lead median; global I/II p95 scale >=0.05 mV; clip +/-6",
        "selection": selection_summary,
    }
    (args.out / "dataset.json").write_text(
        json.dumps(summary, indent=2) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
