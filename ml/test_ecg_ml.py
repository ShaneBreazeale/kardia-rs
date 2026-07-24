"""Regression tests for the research ECG training pipeline."""

from __future__ import annotations

import unittest

import numpy as np
import torch

from ecg_ml import (
    LimbRhythmNetV2,
    ProbabilityModel,
    derive_limb_leads,
)
from train_classifier import thresholds_for_precision


class EcgMlTests(unittest.TestCase):
    def test_derived_limb_leads_match_frontal_plane_identities(self) -> None:
        lead_i = np.array([1.0, 2.0], dtype=np.float32)
        lead_ii = np.array([3.0, 5.0], dtype=np.float32)
        leads = derive_limb_leads(lead_i, lead_ii)

        np.testing.assert_allclose(leads[2], lead_ii - lead_i)
        np.testing.assert_allclose(leads[3], -(lead_i + lead_ii) / 2.0)
        np.testing.assert_allclose(leads[4], lead_i - lead_ii / 2.0)
        np.testing.assert_allclose(leads[5], lead_ii - lead_i / 2.0)

    def test_threshold_search_can_select_below_point_nine(self) -> None:
        labels = np.array([0] * 35 + [1] * 5)
        probabilities = np.tile([0.80, 0.10, 0.10], (40, 1))

        thresholds = thresholds_for_precision(labels, probabilities)

        self.assertLess(thresholds[0], 0.90)
        self.assertEqual(thresholds[1], 1.0)
        self.assertEqual(thresholds[2], 1.0)

    def test_export_wrapper_returns_probabilities(self) -> None:
        model = LimbRhythmNetV2().eval()
        wrapper = ProbabilityModel(model, np.ones(3), np.zeros(3)).eval()

        with torch.no_grad():
            probabilities = wrapper(torch.zeros((1, 6, 1_000)))

        self.assertEqual(tuple(probabilities.shape), (1, 3))
        self.assertTrue(
            torch.allclose(probabilities.sum(dim=1), torch.ones(1), atol=1e-6)
        )


if __name__ == "__main__":
    unittest.main()
