#!/usr/bin/env python3
"""Download a deterministic PTB-XL subset with complete validation/test folds."""

from __future__ import annotations

import argparse
import json
import os
import random
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from ecg_ml import LABELS, classify_metadata, load_metadata

BASE_URL = "https://physionet-open.s3.amazonaws.com/ptb-xl/1.0.3/"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, default=Path("data/ptb-xl"))
    parser.add_argument("--per-common-train-class", type=int, default=3_000)
    parser.add_argument("--workers", type=int, default=48)
    parser.add_argument("--seed", type=int, default=20260723)
    return parser.parse_args()


def choose_records(
    metadata: list[dict[str, str]], per_common_class: int, seed: int
) -> list[dict[str, str]]:
    held_out = [row for row in metadata if int(row["strat_fold"]) >= 9]
    training = [row for row in metadata if int(row["strat_fold"]) <= 8]
    by_label = {
        label_index: [row for row in training if classify_metadata(row) == label_index]
        for label_index in range(len(LABELS))
    }
    generator = random.Random(seed)
    selected_training: list[dict[str, str]] = []
    for label_index, rows in by_label.items():
        limit = len(rows) if label_index == 1 else min(per_common_class, len(rows))
        selected_training.extend(generator.sample(rows, limit))
    return sorted(selected_training + held_out, key=lambda row: int(row["ecg_id"]))


def download_one(dataset: Path, relative_path: str) -> None:
    destination = dataset / relative_path
    if destination.exists() and destination.stat().st_size > 0:
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".part")
    url = BASE_URL + relative_path
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                temporary.write_bytes(response.read())
            os.replace(temporary, destination)
            return
        except Exception:
            temporary.unlink(missing_ok=True)
            if attempt == 3:
                raise
            time.sleep(2**attempt)


def main() -> None:
    args = parse_args()
    metadata = load_metadata(args.dataset / "ptbxl_database.csv")
    selected = choose_records(metadata, args.per_common_train_class, args.seed)
    counts = {label: 0 for label in LABELS}
    for row in selected:
        counts[LABELS[classify_metadata(row)]] += 1

    selection = {
        "dataset": "PTB-XL 1.0.3 records100",
        "seed": args.seed,
        "policy": (
            "all fold 9/10 records; all AF-like fold 1-8 records; fixed sample "
            f"of up to {args.per_common_train_class} records from each common training class"
        ),
        "records": len(selected),
        "labels": counts,
        "ecg_ids": [int(row["ecg_id"]) for row in selected],
    }
    (args.dataset / "selection.json").write_text(
        json.dumps(selection, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({key: value for key, value in selection.items() if key != "ecg_ids"}, indent=2))

    paths = [
        f"{row['filename_lr']}.{extension}"
        for row in selected
        for extension in ("hea", "dat")
    ]
    remaining = paths
    for round_number in range(1, 4):
        failures: list[str] = []
        completed = 0
        with ThreadPoolExecutor(max_workers=args.workers) as executor:
            futures = {
                executor.submit(download_one, args.dataset, path): path for path in remaining
            }
            for future in as_completed(futures):
                path = futures[future]
                try:
                    future.result()
                except Exception as error:
                    print(f"retry later: {path}: {error}")
                    failures.append(path)
                completed += 1
                if completed % 500 == 0 or completed == len(remaining):
                    print(
                        f"round {round_number}: checked {completed}/{len(remaining)} files"
                    )
        if not failures:
            return
        remaining = failures
    raise RuntimeError(f"failed to download {len(remaining)} files after three rounds")


if __name__ == "__main__":
    main()
