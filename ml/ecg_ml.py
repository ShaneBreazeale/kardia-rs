"""Shared data and model definitions for the research ECG classifier."""

from __future__ import annotations

import ast
import csv
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import torch
from torch import nn

LABELS = ("sinus-rhythm-like", "af-like", "other/noisy")
LEADS = ("I", "II", "III", "aVR", "aVL", "aVF")
SAMPLE_RATE_HZ = 100
SAMPLES = 1_000
NOISE_FIELDS = (
    "baseline_drift",
    "static_noise",
    "burst_noise",
    "electrodes_problems",
)


def has_value(value: str | None) -> bool:
    return bool(value and value.strip() and value.strip().lower() != "nan")


def classify_metadata(row: dict[str, str]) -> int:
    """Map PTB-XL metadata to the three explicitly documented research labels."""
    codes = ast.literal_eval(row["scp_codes"])
    noisy = any(has_value(row.get(field)) for field in NOISE_FIELDS)
    has_af = "AFIB" in codes
    has_sinus = "SR" in codes

    if noisy:
        return 2
    if has_af and not has_sinus:
        return 1
    if has_sinus and not has_af:
        return 0
    return 2


def load_metadata(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as source:
        return list(csv.DictReader(source))


def normalize_record(signal: np.ndarray) -> np.ndarray:
    """Match the normalization implemented by the Rust inference path."""
    signal = np.nan_to_num(signal, nan=0.0, posinf=0.0, neginf=0.0)
    signal = signal - np.median(signal, axis=1, keepdims=True)
    scale = float(np.percentile(np.abs(signal[:2]), 95.0))
    scale = max(scale, 0.05)
    return np.clip(signal / scale, -6.0, 6.0).astype(np.float32)


def derive_limb_leads(lead_i: np.ndarray, lead_ii: np.ndarray) -> np.ndarray:
    """Construct the six frontal-plane leads from the two independent leads."""
    return np.stack(
        (
            lead_i,
            lead_ii,
            lead_ii - lead_i,
            -(lead_i + lead_ii) / 2.0,
            lead_i - lead_ii / 2.0,
            lead_ii - lead_i / 2.0,
        )
    )


class PreparedDataset(torch.utils.data.Dataset):
    def __init__(
        self,
        root: Path,
        indices: np.ndarray,
        augment: bool = False,
        augmentation_version: int = 1,
    ):
        self.signals = np.load(root / "signals.npy", mmap_mode="r")
        self.labels = np.load(root / "labels.npy", mmap_mode="r")
        self.indices = indices.astype(np.int64, copy=False)
        self.augment = augment
        self.augmentation_version = augmentation_version

    def __len__(self) -> int:
        return int(self.indices.size)

    def __getitem__(self, index: int) -> tuple[torch.Tensor, torch.Tensor]:
        record_index = int(self.indices[index])
        signal = np.array(self.signals[record_index], dtype=np.float32, copy=True)
        if self.augment:
            signal = (
                augment_v2(signal)
                if self.augmentation_version >= 2
                else augment_v1(signal)
            )
        label = int(self.labels[record_index])
        return torch.from_numpy(signal), torch.tensor(label, dtype=torch.long)


def augment_v1(signal: np.ndarray) -> np.ndarray:
    gain = np.random.uniform(0.85, 1.15)
    shift = np.random.randint(-30, 31)
    signal = np.roll(signal * gain, shift, axis=1)
    signal += np.random.normal(0.0, 0.008, signal.shape).astype(np.float32)
    return signal


def augment_v2(signal: np.ndarray) -> np.ndarray:
    """Add morphology-preserving acquisition variation for device robustness."""
    global_gain = np.random.uniform(0.70, 1.30)
    lead_gain = np.random.uniform(0.92, 1.08, size=(len(LEADS), 1))
    signal *= (global_gain * lead_gain).astype(np.float32)
    signal = np.roll(signal, np.random.randint(-100, 101), axis=1)

    if np.random.random() < 0.25:
        signal *= -1.0

    time = np.arange(SAMPLES, dtype=np.float32) / SAMPLE_RATE_HZ
    frequency = np.random.uniform(0.05, 0.45)
    phase = np.random.uniform(0.0, 2.0 * np.pi)
    wander = np.sin(2.0 * np.pi * frequency * time + phase)
    wander_amplitude = np.random.uniform(0.0, 0.08, size=(len(LEADS), 1))
    signal += (wander_amplitude * wander).astype(np.float32)

    noise_sigma = np.exp(np.random.uniform(np.log(0.002), np.log(0.030)))
    signal += np.random.normal(0.0, noise_sigma, signal.shape).astype(np.float32)
    return np.clip(signal, -6.0, 6.0)


class ResidualBlock(nn.Module):
    def __init__(
        self,
        input_channels: int,
        output_channels: int,
        stride: int,
        dilation: int = 1,
    ):
        super().__init__()
        padding = 4 * dilation
        self.body = nn.Sequential(
            nn.Conv1d(
                input_channels,
                output_channels,
                kernel_size=9,
                stride=stride,
                padding=padding,
                dilation=dilation,
                bias=False,
            ),
            nn.BatchNorm1d(output_channels),
            nn.ReLU(),
            nn.Conv1d(
                output_channels,
                output_channels,
                kernel_size=9,
                padding=padding,
                dilation=dilation,
                bias=False,
            ),
            nn.BatchNorm1d(output_channels),
        )
        self.skip = (
            nn.Identity()
            if input_channels == output_channels and stride == 1
            else nn.Sequential(
                nn.Conv1d(
                    input_channels,
                    output_channels,
                    kernel_size=1,
                    stride=stride,
                    bias=False,
                ),
                nn.BatchNorm1d(output_channels),
            )
        )
        self.activation = nn.ReLU()

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.activation(self.body(inputs) + self.skip(inputs))


class LimbRhythmNet(nn.Module):
    """The v0.1 average-pooling architecture retained for reproducibility."""

    def __init__(self) -> None:
        super().__init__()
        self.features = nn.Sequential(
            nn.Conv1d(6, 24, kernel_size=15, stride=2, padding=7, bias=False),
            nn.BatchNorm1d(24),
            nn.ReLU(),
            ResidualBlock(24, 48, stride=2),
            ResidualBlock(48, 96, stride=2),
            ResidualBlock(96, 128, stride=2),
            ResidualBlock(128, 128, stride=1),
            nn.AdaptiveAvgPool1d(1),
        )
        self.classifier = nn.Sequential(nn.Flatten(), nn.Dropout(0.20), nn.Linear(128, 3))

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.classifier(self.features(inputs))


class TemporalStatisticsPool(nn.Module):
    """Preserve average morphology, variability, and strong transient features."""

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        mean = inputs.mean(dim=2)
        variance = (inputs.square().mean(dim=2) - mean.square()).clamp_min(1e-6)
        maximum = inputs.amax(dim=2)
        return torch.cat((mean, variance.sqrt(), maximum), dim=1)


class LimbRhythmNetV2(nn.Module):
    """A wider-receptive-field model that retains ten-second rhythm dynamics."""

    def __init__(self) -> None:
        super().__init__()
        self.features = nn.Sequential(
            nn.Conv1d(6, 32, kernel_size=15, stride=2, padding=7, bias=False),
            nn.BatchNorm1d(32),
            nn.ReLU(),
            ResidualBlock(32, 64, stride=2),
            ResidualBlock(64, 96, stride=2),
            ResidualBlock(96, 128, stride=2),
            ResidualBlock(128, 128, stride=1, dilation=2),
            ResidualBlock(128, 128, stride=1, dilation=4),
        )
        self.pool = TemporalStatisticsPool()
        self.classifier = nn.Sequential(
            nn.Linear(128 * 3, 128),
            nn.ReLU(),
            nn.Dropout(0.25),
            nn.Linear(128, 3),
        )

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.classifier(self.pool(self.features(inputs)))


class ProbabilityModel(nn.Module):
    def __init__(
        self,
        model: nn.Module,
        scales: np.ndarray | list[float],
        biases: np.ndarray | list[float],
    ):
        super().__init__()
        self.model = model
        self.register_buffer(
            "scales",
            torch.as_tensor(scales, dtype=torch.float32).clamp(0.05, 20.0),
        )
        self.register_buffer("biases", torch.as_tensor(biases, dtype=torch.float32))

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        calibrated = self.model(inputs) / self.scales + self.biases
        return torch.softmax(calibrated, dim=1)


@dataclass(frozen=True)
class Split:
    train: np.ndarray
    validation: np.ndarray
    test: np.ndarray


def prescribed_split(folds: np.ndarray) -> Split:
    return Split(
        train=np.flatnonzero(folds <= 8),
        validation=np.flatnonzero(folds == 9),
        test=np.flatnonzero(folds == 10),
    )
