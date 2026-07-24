#!/usr/bin/env python3
"""Train, calibrate, evaluate, and export the research three-way classifier."""

from __future__ import annotations

import argparse
import hashlib
import json
import random
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch
from sklearn.metrics import (
    balanced_accuracy_score,
    confusion_matrix,
    precision_recall_fscore_support,
    roc_auc_score,
)
from torch import nn
from torch.utils.data import DataLoader, WeightedRandomSampler

from ecg_ml import (
    LABELS,
    LEADS,
    SAMPLE_RATE_HZ,
    SAMPLES,
    LimbRhythmNetV2,
    PreparedDataset,
    ProbabilityModel,
    prescribed_split,
)

DEFAULT_MODEL_ID = "limb6-rhythm-v0.2.0"
TARGET_PRECISIONS = np.array([0.85, 0.80, 0.85], dtype=np.float64)
MINIMUM_THRESHOLD = 0.50
MINIMUM_MARGIN = 0.15
MINIMUM_ACCEPTED_PER_CLASS = 30


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=Path("data/prepared-v0.2"))
    parser.add_argument("--out", type=Path, default=Path("../models"))
    parser.add_argument("--checkpoints", type=Path, default=Path("checkpoints"))
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--epochs", type=int, default=20)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--seed", type=int, default=20260723)
    parser.add_argument("--workers", type=int, default=0)
    return parser.parse_args()


def seed_everything(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)


def choose_device() -> torch.device:
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def infer(
    model: nn.Module, loader: DataLoader, device: torch.device
) -> tuple[np.ndarray, np.ndarray]:
    model.eval()
    logits: list[np.ndarray] = []
    labels: list[np.ndarray] = []
    with torch.no_grad():
        for inputs, target in loader:
            logits.append(model(inputs.to(device)).cpu().numpy())
            labels.append(target.numpy())
    return np.concatenate(logits), np.concatenate(labels)


def fit_vector_scaling(
    logits: np.ndarray, labels: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    """Calibrate each class while regularizing toward the unscaled logits."""
    logits_tensor = torch.tensor(logits, dtype=torch.float32)
    labels_tensor = torch.tensor(labels, dtype=torch.long)
    log_scales = torch.zeros(len(LABELS), requires_grad=True)
    biases = torch.zeros(len(LABELS), requires_grad=True)
    optimizer = torch.optim.LBFGS(
        [log_scales, biases], lr=0.05, max_iter=150, line_search_fn="strong_wolfe"
    )

    def closure() -> torch.Tensor:
        optimizer.zero_grad()
        scales = log_scales.exp().clamp(0.05, 20.0)
        calibrated = logits_tensor / scales + biases
        regularization = 1e-3 * (log_scales.square().mean() + biases.square().mean())
        loss = nn.functional.cross_entropy(calibrated, labels_tensor) + regularization
        loss.backward()
        return loss

    optimizer.step(closure)
    scales = log_scales.detach().exp().clamp(0.05, 20.0).numpy()
    calibrated_biases = biases.detach().numpy()
    calibrated_biases -= calibrated_biases.mean()
    return scales, calibrated_biases


def calibrated_softmax(
    logits: np.ndarray, scales: np.ndarray, biases: np.ndarray
) -> np.ndarray:
    calibrated = logits / scales + biases
    calibrated -= calibrated.max(axis=1, keepdims=True)
    exponent = np.exp(calibrated)
    return exponent / exponent.sum(axis=1, keepdims=True)


def thresholds_for_precision(labels: np.ndarray, probabilities: np.ndarray) -> np.ndarray:
    """Select the lowest useful validation threshold meeting each precision target."""
    thresholds = np.ones(len(LABELS), dtype=np.float64)
    order = np.argsort(probabilities, axis=1)
    margin = (
        probabilities[np.arange(len(probabilities)), order[:, -1]]
        - probabilities[np.arange(len(probabilities)), order[:, -2]]
    )
    winners = order[:, -1]

    for class_index in range(len(LABELS)):
        for threshold in np.linspace(MINIMUM_THRESHOLD, 0.995, 496):
            selected = (
                (winners == class_index)
                & (probabilities[:, class_index] >= threshold)
                & (margin >= MINIMUM_MARGIN)
            )
            if selected.sum() < MINIMUM_ACCEPTED_PER_CLASS:
                continue
            precision = float((labels[selected] == class_index).mean())
            if precision >= TARGET_PRECISIONS[class_index]:
                thresholds[class_index] = float(threshold)
                break
    return thresholds


def metrics(
    labels: np.ndarray, probabilities: np.ndarray, thresholds: np.ndarray
) -> dict[str, object]:
    predictions = probabilities.argmax(axis=1)
    precision, recall, f1, support = precision_recall_fscore_support(
        labels,
        predictions,
        labels=np.arange(len(LABELS)),
        zero_division=0,
    )
    sorted_probabilities = np.sort(probabilities, axis=1)
    confidence = sorted_probabilities[:, -1]
    margin = sorted_probabilities[:, -1] - sorted_probabilities[:, -2]
    accepted = (
        (confidence >= thresholds[predictions])
        & (margin >= MINIMUM_MARGIN)
    )
    accepted_accuracy = (
        float((predictions[accepted] == labels[accepted]).mean())
        if accepted.any()
        else None
    )
    accepted_per_class = {}
    for class_index, label in enumerate(LABELS):
        selected = accepted & (predictions == class_index)
        selected_count = int(selected.sum())
        accepted_per_class[label] = {
            "threshold": float(thresholds[class_index]),
            "target_validation_precision": float(TARGET_PRECISIONS[class_index]),
            "accepted_records": selected_count,
            "accepted_precision": (
                float((labels[selected] == class_index).mean())
                if selected_count
                else None
            ),
        }
    return {
        "records": int(labels.size),
        "balanced_accuracy": float(balanced_accuracy_score(labels, predictions)),
        "macro_auroc_ovr": float(
            roc_auc_score(
                labels,
                probabilities,
                labels=np.arange(len(LABELS)),
                multi_class="ovr",
                average="macro",
            )
        ),
        "confusion_matrix": confusion_matrix(
            labels, predictions, labels=np.arange(len(LABELS))
        ).tolist(),
        "per_class": {
            label: {
                "precision": float(precision[index]),
                "recall": float(recall[index]),
                "f1": float(f1[index]),
                "support": int(support[index]),
            }
            for index, label in enumerate(LABELS)
        },
        "abstention": {
            "coverage": float(accepted.mean()),
            "accepted_accuracy": accepted_accuracy,
            "accepted_records": int(accepted.sum()),
            "per_class": accepted_per_class,
        },
    }


def main() -> None:
    args = parse_args()
    seed_everything(args.seed)
    device = choose_device()
    args.out.mkdir(parents=True, exist_ok=True)
    args.checkpoints.mkdir(parents=True, exist_ok=True)

    dataset_summary = json.loads((args.data / "dataset.json").read_text())
    labels = np.load(args.data / "labels.npy")
    folds = np.load(args.data / "folds.npy")
    split = prescribed_split(folds)

    train_dataset = PreparedDataset(
        args.data,
        split.train,
        augment=True,
        augmentation_version=2,
    )
    validation_dataset = PreparedDataset(args.data, split.validation)
    test_dataset = PreparedDataset(args.data, split.test)

    train_labels = labels[split.train]
    counts = np.bincount(train_labels, minlength=len(LABELS))
    class_weights = counts.sum() / np.maximum(counts, 1)
    sample_weights = class_weights[train_labels]
    sampler = WeightedRandomSampler(
        torch.as_tensor(sample_weights, dtype=torch.double),
        num_samples=len(sample_weights),
        replacement=True,
    )
    train_loader = DataLoader(
        train_dataset,
        batch_size=args.batch_size,
        sampler=sampler,
        num_workers=args.workers,
    )
    validation_loader = DataLoader(
        validation_dataset,
        batch_size=args.batch_size,
        shuffle=False,
        num_workers=args.workers,
    )
    test_loader = DataLoader(
        test_dataset,
        batch_size=args.batch_size,
        shuffle=False,
        num_workers=args.workers,
    )

    model = LimbRhythmNetV2().to(device)
    optimizer = torch.optim.AdamW(model.parameters(), lr=7e-4, weight_decay=2e-4)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
        optimizer, mode="max", factor=0.5, patience=2
    )
    criterion = nn.CrossEntropyLoss(label_smoothing=0.03)
    best_macro_f1 = -1.0
    patience = 0
    checkpoint = args.checkpoints / f"{args.model_id}.pt"

    print(f"device={device} train={len(split.train)} validation={len(split.validation)}")
    for epoch in range(1, args.epochs + 1):
        model.train()
        total_loss = 0.0
        total_records = 0
        for inputs, target in train_loader:
            inputs = inputs.to(device)
            target = target.to(device)
            optimizer.zero_grad()
            logits = model(inputs)
            loss = criterion(logits, target)
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), max_norm=5.0)
            optimizer.step()
            total_loss += float(loss.detach()) * len(target)
            total_records += len(target)

        validation_logits, validation_labels = infer(model, validation_loader, device)
        validation_predictions = validation_logits.argmax(axis=1)
        _, _, validation_f1, _ = precision_recall_fscore_support(
            validation_labels,
            validation_predictions,
            labels=np.arange(len(LABELS)),
            zero_division=0,
        )
        macro_f1 = float(validation_f1.mean())
        scheduler.step(macro_f1)
        print(
            f"epoch={epoch:02d} loss={total_loss / total_records:.5f} "
            f"val_macro_f1={macro_f1:.5f}"
        )

        if macro_f1 > best_macro_f1 + 1e-4:
            best_macro_f1 = macro_f1
            patience = 0
            torch.save(model.state_dict(), checkpoint)
        else:
            patience += 1
            if patience >= 6:
                print("early stopping")
                break

    model.load_state_dict(torch.load(checkpoint, map_location=device, weights_only=True))
    validation_logits, validation_labels = infer(model, validation_loader, device)
    scales, biases = fit_vector_scaling(validation_logits, validation_labels)
    validation_probabilities = calibrated_softmax(validation_logits, scales, biases)
    thresholds = thresholds_for_precision(validation_labels, validation_probabilities)

    test_logits, test_labels = infer(model, test_loader, device)
    test_probabilities = calibrated_softmax(test_logits, scales, biases)
    test_metrics = metrics(test_labels, test_probabilities, thresholds)

    export_model = ProbabilityModel(model.cpu().eval(), scales, biases).eval()
    model_path = args.out / f"{args.model_id}.onnx"
    example = torch.zeros((1, len(LEADS), SAMPLES), dtype=torch.float32)
    torch.onnx.export(
        export_model,
        example,
        model_path,
        input_names=["ecg"],
        output_names=["probabilities"],
        opset_version=17,
        dynamo=False,
    )

    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    test_input = test_dataset[0][0].numpy()[None, :, :]
    onnx_output = session.run(None, {"ecg": test_input})[0]
    with torch.no_grad():
        torch_output = export_model(torch.from_numpy(test_input)).numpy()
    maximum_export_error = float(np.max(np.abs(onnx_output - torch_output)))
    if maximum_export_error > 1e-5:
        raise RuntimeError(f"ONNX parity error {maximum_export_error}")

    digest = hashlib.sha256(model_path.read_bytes()).hexdigest()
    manifest = {
        "schema_version": 1,
        "model_id": args.model_id,
        "model_file": model_path.name,
        "sha256": digest,
        "research_only": True,
        "input": {
            "sample_rate_hz": SAMPLE_RATE_HZ,
            "samples": SAMPLES,
            "leads": LEADS,
            "normalization": "per-lead median; global I/II p95 scale >=0.05 mV; clip +/-6",
        },
        "labels": LABELS,
        "decision": {
            "class_thresholds": {
                LABELS[index]: float(thresholds[index])
                for index in range(len(LABELS))
            },
            "minimum_margin": MINIMUM_MARGIN,
            "requires_measurement_quality": "GOOD",
            "low_confidence_action": "abstain",
            "threshold_selection": {
                "split": "validation fold 9",
                "target_precision": {
                    LABELS[index]: float(TARGET_PRECISIONS[index])
                    for index in range(len(LABELS))
                },
                "minimum_accepted_per_class": MINIMUM_ACCEPTED_PER_CLASS,
            },
        },
        "training": {
            "dataset": "PTB-XL 1.0.3 records100",
            "prepared_dataset": dataset_summary,
            "architecture": "temporal-statistics residual CNN v2",
            "augmentation": (
                "gain, time shift, global polarity, baseline wander, and white noise"
            ),
            "label_policy": {
                "sinus-rhythm-like": "SR present, AFIB absent, no listed signal contamination",
                "af-like": "AFIB present, SR absent, no listed signal contamination",
                "other/noisy": "all other rhythms or listed signal contamination",
            },
            "split": "strat_fold 1-8 train, 9 validation, 10 held-out test",
            "calibration": {
                "method": "regularized per-class vector scaling",
                "scales": [float(value) for value in scales],
                "biases": [float(value) for value in biases],
            },
            "seed": args.seed,
        },
        "held_out_test": test_metrics,
        "onnx_maximum_absolute_error": maximum_export_error,
        "limitations": [
            "Not a medical device and not for diagnosis or treatment.",
            "Trained on PTB-XL, not on Kardia recordings.",
            "Only two leads are independent; III/aVR/aVL/aVF are derived from I/II.",
            "No chest leads are present; unsupported conditions must not be inferred.",
            "A classified output means model similarity, not clinical confirmation.",
            "Class thresholds are selected on one validation fold and require external validation.",
        ],
    }
    manifest_path = args.out / f"{args.model_id}.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
