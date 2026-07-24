use crate::ecg_analysis::AnalysisQuality;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use tract_onnx::prelude::*;

const SOURCE_SAMPLE_RATE_HZ: usize = 300;
const EXPECTED_LABELS: [&str; 3] = ["sinus-rhythm-like", "af-like", "other/noisy"];
const EXPECTED_LEADS: [&str; 6] = ["I", "II", "III", "aVR", "aVL", "aVF"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchClass {
    SinusRhythmLike,
    AfLike,
    OtherOrNoisy,
}

impl ResearchClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::SinusRhythmLike => "sinus-rhythm-like",
            Self::AfLike => "AF-like",
            Self::OtherOrNoisy => "other/noisy",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelDecision {
    Classified {
        class: ResearchClass,
        confidence: f32,
    },
    Abstained {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelAssessment {
    pub model_id: String,
    pub probabilities: [f32; 3],
    pub decision: ModelDecision,
}

impl ModelAssessment {
    pub fn summary(&self) -> String {
        match &self.decision {
            ModelDecision::Classified { class, confidence } => {
                format!("{} ({:.0}%)", class.label(), confidence * 100.0)
            }
            ModelDecision::Abstained { reason } => format!("ABSTAIN: {reason}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    model_id: String,
    model_file: String,
    sha256: String,
    research_only: bool,
    input: InputManifest,
    labels: Vec<String>,
    decision: DecisionManifest,
}

#[derive(Debug, Deserialize)]
struct InputManifest {
    sample_rate_hz: usize,
    samples: usize,
    leads: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DecisionManifest {
    class_thresholds: ClassThresholds,
    minimum_margin: f32,
    requires_measurement_quality: String,
    low_confidence_action: String,
}

#[derive(Debug, Deserialize)]
struct ClassThresholds {
    #[serde(rename = "sinus-rhythm-like")]
    sinus_rhythm_like: f32,
    #[serde(rename = "af-like")]
    af_like: f32,
    #[serde(rename = "other/noisy")]
    other_or_noisy: f32,
}

pub fn classify(
    leads_mv: &[Vec<f64>; 6],
    measurement_quality: AnalysisQuality,
    manifest_path: &Path,
) -> Result<ModelAssessment> {
    let manifest = load_manifest(manifest_path)?;
    let mut assessment = ModelAssessment {
        model_id: manifest.model_id.clone(),
        probabilities: [0.0; 3],
        decision: ModelDecision::Abstained {
            reason: "not evaluated".to_owned(),
        },
    };

    if manifest.decision.requires_measurement_quality == "GOOD"
        && measurement_quality != AnalysisQuality::Good
    {
        assessment.decision = ModelDecision::Abstained {
            reason: "technical quality is not GOOD".to_owned(),
        };
        return Ok(assessment);
    }

    let input = preprocess(leads_mv, manifest.input.samples)?;
    let model_path = resolve_model_path(manifest_path, &manifest.model_file);
    verify_model_digest(&model_path, &manifest.sha256)?;

    let model = tract_onnx::onnx()
        .model_for_path(&model_path)
        .with_context(|| format!("load ONNX model {}", model_path.display()))?
        .with_input_fact(
            0,
            f32::fact([
                1,
                EXPECTED_LEADS.len() as i64,
                manifest.input.samples as i64,
            ])
            .into(),
        )?
        .into_optimized()
        .context("optimize ONNX model")?
        .into_runnable()
        .context("make ONNX model runnable")?;
    let tensor = Tensor::from_shape(&[1, EXPECTED_LEADS.len(), manifest.input.samples], &input)
        .context("construct ECG model tensor")?;
    let outputs = model.run(tvec!(tensor.into())).context("run ECG model")?;
    let probabilities = outputs[0]
        .as_slice::<f32>()
        .context("read ECG model probabilities")?;
    if probabilities.len() != EXPECTED_LABELS.len() {
        bail!(
            "model returned {} probabilities; expected {}",
            probabilities.len(),
            EXPECTED_LABELS.len()
        );
    }
    assessment.probabilities.copy_from_slice(probabilities);
    validate_probabilities(&assessment.probabilities)?;
    assessment.decision = decide(&manifest.decision, assessment.probabilities);
    Ok(assessment)
}

fn load_manifest(path: &Path) -> Result<Manifest> {
    let bytes =
        fs::read(path).with_context(|| format!("read model manifest {}", path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse model manifest {}", path.display()))?;
    if manifest.schema_version != 1 {
        bail!(
            "{} uses unsupported model-manifest schema {}",
            path.display(),
            manifest.schema_version
        );
    }
    if !manifest.research_only {
        bail!("{} must declare research_only=true", path.display());
    }
    if manifest.input.sample_rate_hz != 100 {
        bail!("model input sample rate must be 100 Hz");
    }
    if manifest.input.samples != 1_000 {
        bail!("model input must contain exactly 1000 samples");
    }
    if manifest.labels != EXPECTED_LABELS {
        bail!("model labels must be {:?}", EXPECTED_LABELS);
    }
    if manifest.input.leads != EXPECTED_LEADS {
        bail!("model leads must be {:?}", EXPECTED_LEADS);
    }
    if manifest.decision.low_confidence_action != "abstain" {
        bail!("model low_confidence_action must be abstain");
    }
    let thresholds = thresholds(&manifest.decision);
    if thresholds
        .iter()
        .any(|value| !value.is_finite() || !(0.5..=1.0).contains(value))
    {
        bail!("model class thresholds must be finite values from 0.5 to 1.0");
    }
    if !manifest.decision.minimum_margin.is_finite()
        || !(0.0..=1.0).contains(&manifest.decision.minimum_margin)
    {
        bail!("model minimum margin must be a finite value from 0.0 to 1.0");
    }
    Ok(manifest)
}

fn resolve_model_path(manifest_path: &Path, model_file: &str) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(model_file)
}

fn verify_model_digest(model_path: &Path, expected: &str) -> Result<()> {
    let bytes = fs::read(model_path)
        .with_context(|| format!("read ONNX model {}", model_path.display()))?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.to_ascii_lowercase() {
        bail!(
            "{} SHA-256 mismatch: expected {}, got {}",
            model_path.display(),
            expected,
            actual
        );
    }
    Ok(())
}

fn preprocess(leads_mv: &[Vec<f64>; 6], output_samples: usize) -> Result<Vec<f32>> {
    let minimum_samples = SOURCE_SAMPLE_RATE_HZ * 9;
    let available = leads_mv.iter().map(Vec::len).min().unwrap_or(0);
    if available < minimum_samples {
        bail!("classifier requires at least 9 seconds of simultaneous signal");
    }

    let target_source_samples = output_samples * (SOURCE_SAMPLE_RATE_HZ / 100);
    let mut downsampled = vec![vec![0.0_f32; output_samples]; EXPECTED_LEADS.len()];
    for (lead_index, lead) in leads_mv.iter().enumerate() {
        for (output_index, destination) in downsampled[lead_index].iter_mut().enumerate() {
            let source_start = output_index * 3;
            let mut sum = 0.0;
            for offset in 0..3 {
                let index = (source_start + offset).min(available - 1);
                sum += lead[index];
            }
            *destination = (sum / 3.0) as f32;
        }
    }
    debug_assert_eq!(target_source_samples, 3_000);

    for lead in &mut downsampled {
        let baseline = median_f32(lead);
        for value in lead {
            *value -= baseline;
        }
    }
    let mut independent_magnitudes: Vec<f32> = downsampled[..2]
        .iter()
        .flat_map(|lead| lead.iter().map(|value| value.abs()))
        .collect();
    independent_magnitudes.sort_by(f32::total_cmp);
    let scale = linear_percentile(&independent_magnitudes, 0.95).max(0.05);

    let mut flattened = Vec::with_capacity(EXPECTED_LEADS.len() * output_samples);
    for lead in downsampled {
        flattened.extend(
            lead.into_iter()
                .map(|value| (value / scale).clamp(-6.0, 6.0)),
        );
    }
    Ok(flattened)
}

fn linear_percentile(sorted: &[f32], quantile: f32) -> f32 {
    let rank = (sorted.len() - 1) as f32 * quantile;
    let lower_index = rank.floor() as usize;
    let upper_index = rank.ceil() as usize;
    let weight = rank - lower_index as f32;
    sorted[lower_index] * (1.0 - weight) + sorted[upper_index] * weight
}

fn median_f32(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn validate_probabilities(probabilities: &[f32; 3]) -> Result<()> {
    if probabilities
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        bail!("model returned an invalid probability");
    }
    let sum: f32 = probabilities.iter().sum();
    if (sum - 1.0).abs() > 1e-3 {
        bail!("model probabilities sum to {sum:.6}, expected 1.0");
    }
    Ok(())
}

fn decide(decision: &DecisionManifest, probabilities: [f32; 3]) -> ModelDecision {
    let mut order = [0usize, 1, 2];
    order.sort_by(|left, right| {
        probabilities[*right]
            .partial_cmp(&probabilities[*left])
            .unwrap_or(Ordering::Equal)
    });
    let winner = order[0];
    let confidence = probabilities[winner];
    let margin = confidence - probabilities[order[1]];
    let required = thresholds(decision)[winner];
    if confidence < required {
        return ModelDecision::Abstained {
            reason: format!(
                "top confidence {:.0}% is below threshold",
                confidence * 100.0
            ),
        };
    }
    if margin < decision.minimum_margin {
        return ModelDecision::Abstained {
            reason: format!("top-class margin {:.0}% is too small", margin * 100.0),
        };
    }
    ModelDecision::Classified {
        class: class_for_index(winner),
        confidence,
    }
}

fn thresholds(decision: &DecisionManifest) -> [f32; 3] {
    [
        decision.class_thresholds.sinus_rhythm_like,
        decision.class_thresholds.af_like,
        decision.class_thresholds.other_or_noisy,
    ]
}

fn class_for_index(index: usize) -> ResearchClass {
    match index {
        0 => ResearchClass::SinusRhythmLike,
        1 => ResearchClass::AfLike,
        2 => ResearchClass::OtherOrNoisy,
        _ => unreachable!("validated model output has exactly three classes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision() -> DecisionManifest {
        DecisionManifest {
            class_thresholds: ClassThresholds {
                sinus_rhythm_like: 0.90,
                af_like: 0.90,
                other_or_noisy: 0.85,
            },
            minimum_margin: 0.20,
            requires_measurement_quality: "GOOD".to_owned(),
            low_confidence_action: "abstain".to_owned(),
        }
    }

    #[test]
    fn conservative_decision_accepts_only_confident_separated_output() {
        assert!(matches!(
            decide(&decision(), [0.93, 0.03, 0.04]),
            ModelDecision::Classified {
                class: ResearchClass::SinusRhythmLike,
                ..
            }
        ));
        assert!(matches!(
            decide(&decision(), [0.51, 0.01, 0.48]),
            ModelDecision::Abstained { .. }
        ));
        assert!(matches!(
            decide(&decision(), [0.08, 0.84, 0.08]),
            ModelDecision::Abstained { .. }
        ));
    }

    #[test]
    fn preprocessing_matches_model_shape_and_normalizes_scale() {
        let leads: [Vec<f64>; 6] = std::array::from_fn(|lead| {
            (0..3_000)
                .map(|sample| lead as f64 + (sample as f64 / 30.0).sin())
                .collect()
        });
        let input = preprocess(&leads, 1_000).expect("preprocess");
        assert_eq!(input.len(), 6_000);
        assert!(input.iter().all(|value| value.is_finite()));
        assert!(input.iter().all(|value| (-6.0..=6.0).contains(value)));
    }

    #[test]
    fn preprocessing_rejects_short_reports() {
        let leads: [Vec<f64>; 6] = std::array::from_fn(|_| vec![0.0; 2_699]);
        assert!(preprocess(&leads, 1_000).is_err());
    }

    #[test]
    fn percentile_matches_numpy_linear_interpolation() {
        let sorted = [0.0, 10.0, 20.0, 30.0, 40.0];
        assert!((linear_percentile(&sorted, 0.95) - 38.0).abs() < f32::EPSILON);
    }
}
