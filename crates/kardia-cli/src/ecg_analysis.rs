//! Experimental automated measurements for a simultaneous six-limb-lead ECG.
//!
//! This module deliberately reports measurements, not diagnoses. It detects
//! beats from the two independent channels, constructs a representative median
//! complex, and measures global fiducial points across those channels.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisQuality {
    Good,
    Marginal,
    Insufficient,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EcgMeasurements {
    pub quality: AnalysisQuality,
    pub detected_beats: usize,
    pub median_beats: usize,
    pub ventricular_rate_bpm: Option<u16>,
    pub average_rr_ms: Option<u16>,
    pub pr_interval_ms: Option<u16>,
    pub qrs_duration_ms: Option<u16>,
    pub qt_interval_ms: Option<u16>,
    pub qtc_bazett_ms: Option<u16>,
    pub qtc_fridericia_ms: Option<u16>,
    pub p_axis_deg: Option<i16>,
    pub qrs_axis_deg: Option<i16>,
    pub t_axis_deg: Option<i16>,
}

impl EcgMeasurements {
    fn insufficient(detected_beats: usize) -> Self {
        Self {
            quality: AnalysisQuality::Insufficient,
            detected_beats,
            median_beats: 0,
            ventricular_rate_bpm: None,
            average_rr_ms: None,
            pr_interval_ms: None,
            qrs_duration_ms: None,
            qt_interval_ms: None,
            qtc_bazett_ms: None,
            qtc_fridericia_ms: None,
            p_axis_deg: None,
            qrs_axis_deg: None,
            t_axis_deg: None,
        }
    }
}

impl Default for EcgMeasurements {
    fn default() -> Self {
        Self::insufficient(0)
    }
}

pub fn analyze(leads_mv: &[Vec<f64>; 6], sample_rate_hz: usize) -> EcgMeasurements {
    if sample_rate_hz == 0
        || leads_mv[0].len() != leads_mv[1].len()
        || leads_mv[0].len() < sample_rate_hz * 3
    {
        return EcgMeasurements::insufficient(0);
    }

    let lead_i = &leads_mv[0];
    let lead_ii = &leads_mv[1];
    let detection_i = detection_signal(lead_i, sample_rate_hz);
    let detection_ii = detection_signal(lead_ii, sample_rate_hz);
    let energy = qrs_energy(&detection_i, &detection_ii, sample_rate_hz);
    let energy_threshold = percentile(&energy, 0.90);
    let refractory = millis_to_samples(250, sample_rate_hz);
    let refine_radius = millis_to_samples(80, sample_rate_hz);

    let candidates = local_maxima_with_refractory(&energy, energy_threshold, refractory);
    let mut peaks = candidates
        .into_iter()
        .map(|peak| refine_peak(peak, refine_radius, &detection_i, &detection_ii))
        .collect::<Vec<_>>();
    peaks.sort_unstable();
    peaks.dedup();

    if peaks.len() < 4 {
        return EcgMeasurements::insufficient(peaks.len());
    }

    let average_rr_seconds = (peaks[peaks.len() - 1] - peaks[0]) as f64
        / (peaks.len() - 1) as f64
        / sample_rate_hz as f64;
    if !(0.25..=3.0).contains(&average_rr_seconds) {
        return EcgMeasurements::insufficient(peaks.len());
    }

    let ventricular_rate_bpm = round_u16(60.0 / average_rr_seconds);
    let average_rr_ms = round_u16(average_rr_seconds * 1_000.0);
    let rr_intervals = peaks
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / sample_rate_hz as f64)
        .collect::<Vec<_>>();
    let rr_mean = mean(&rr_intervals);
    let rr_cv = standard_deviation(&rr_intervals, rr_mean) / rr_mean.max(f64::EPSILON);

    let pre_samples = millis_to_samples(350, sample_rate_hz);
    let post_samples = millis_to_samples(600, sample_rate_hz);
    let median_peaks = peaks
        .iter()
        .copied()
        .filter(|peak| *peak >= pre_samples && peak + post_samples < lead_i.len())
        .collect::<Vec<_>>();
    if median_peaks.len() < 3 {
        let mut result = EcgMeasurements::insufficient(peaks.len());
        result.ventricular_rate_bpm = Some(ventricular_rate_bpm);
        result.average_rr_ms = Some(average_rr_ms);
        return result;
    }

    let median_i = median_complex(lead_i, &median_peaks, pre_samples, post_samples);
    let median_ii = median_complex(lead_ii, &median_peaks, pre_samples, post_samples);
    let smooth_window = millis_to_samples(17, sample_rate_hz).max(3) | 1;
    let smooth_i = centered_moving_average(&median_i, smooth_window);
    let smooth_ii = centered_moving_average(&median_ii, smooth_window);

    let baseline_i = edge_baseline(&smooth_i, sample_rate_hz);
    let baseline_ii = edge_baseline(&smooth_ii, sample_rate_hz);
    let vector_magnitude = smooth_i
        .iter()
        .zip(&smooth_ii)
        .map(|(i, ii)| ((i - baseline_i).powi(2) + (ii - baseline_ii).powi(2)).sqrt())
        .collect::<Vec<_>>();
    let slope = vector_slope(&smooth_i, &smooth_ii);
    let (slope_noise, amplitude_noise) =
        measurement_noise(&slope, &vector_magnitude, sample_rate_hz);

    let center = pre_samples;
    let qrs_search_start = center.saturating_sub(millis_to_samples(150, sample_rate_hz));
    let qrs_search_end = (center + millis_to_samples(170, sample_rate_hz)).min(slope.len() - 1);
    let qrs_max_slope = slope[qrs_search_start..=qrs_search_end]
        .iter()
        .copied()
        .fold(0.0, f64::max);
    let qrs_threshold = (slope_noise * 5.0).max(qrs_max_slope * 0.06);
    let quiet_run = millis_to_samples(17, sample_rate_hz).max(3);
    let qrs_onset = find_qrs_onset(&slope, center, qrs_threshold, quiet_run, sample_rate_hz);
    let qrs_offset = find_qrs_offset(&slope, center, qrs_threshold, quiet_run, sample_rate_hz);

    let qrs_snr = qrs_max_slope / slope_noise.max(f64::EPSILON);
    let qrs_duration_ms = interval_ms(qrs_onset, qrs_offset, sample_rate_hz)
        .filter(|value| (35..=260).contains(value));

    let p = delineate_p_wave(
        &vector_magnitude,
        qrs_onset,
        amplitude_noise,
        sample_rate_hz,
    );
    let t = delineate_t_wave(
        &vector_magnitude,
        qrs_offset,
        center,
        amplitude_noise,
        sample_rate_hz,
    );

    let p_snr = p
        .as_ref()
        .map(|wave| wave.peak_amplitude / amplitude_noise.max(f64::EPSILON))
        .unwrap_or(0.0);
    let t_snr = t
        .as_ref()
        .map(|wave| wave.peak_amplitude / amplitude_noise.max(f64::EPSILON))
        .unwrap_or(0.0);

    let pr_interval_ms = p
        .as_ref()
        .and_then(|wave| interval_ms(wave.onset, qrs_onset, sample_rate_hz))
        .filter(|value| (60..=420).contains(value))
        .filter(|_| p_snr >= 3.0 && rr_cv <= 0.20);
    let qt_interval_ms = t
        .as_ref()
        .and_then(|wave| interval_ms(qrs_onset, wave.offset, sample_rate_hz))
        .filter(|value| (180..=720).contains(value))
        .filter(|_| t_snr >= 3.0);
    let qtc_bazett_ms = qt_interval_ms.map(|qt| corrected_qt_bazett(qt, average_rr_seconds));
    let qtc_fridericia_ms =
        qt_interval_ms.map(|qt| corrected_qt_fridericia(qt, average_rr_seconds));

    let p_axis_deg = p.as_ref().filter(|_| p_snr >= 4.0).and_then(|wave| {
        frontal_axis(
            &smooth_i,
            &smooth_ii,
            baseline_i,
            baseline_ii,
            wave.onset,
            wave.offset,
        )
    });
    let qrs_axis_deg = qrs_duration_ms.filter(|_| qrs_snr >= 6.0).and_then(|_| {
        frontal_axis(
            &smooth_i,
            &smooth_ii,
            baseline_i,
            baseline_ii,
            qrs_onset,
            qrs_offset,
        )
    });
    let t_axis_deg = t.as_ref().filter(|_| t_snr >= 4.0).and_then(|wave| {
        frontal_axis(
            &smooth_i,
            &smooth_ii,
            baseline_i,
            baseline_ii,
            wave.onset,
            wave.offset,
        )
    });

    let quality = if qrs_snr >= 10.0
        && rr_cv <= 0.15
        && qrs_duration_ms.is_some()
        && qt_interval_ms.is_some()
    {
        AnalysisQuality::Good
    } else if qrs_snr >= 6.0 && qrs_duration_ms.is_some() {
        AnalysisQuality::Marginal
    } else {
        AnalysisQuality::Insufficient
    };

    EcgMeasurements {
        quality,
        detected_beats: peaks.len(),
        median_beats: median_peaks.len(),
        ventricular_rate_bpm: Some(ventricular_rate_bpm),
        average_rr_ms: Some(average_rr_ms),
        pr_interval_ms,
        qrs_duration_ms,
        qt_interval_ms,
        qtc_bazett_ms,
        qtc_fridericia_ms,
        p_axis_deg,
        qrs_axis_deg,
        t_axis_deg,
    }
}

#[derive(Debug, Clone, Copy)]
struct WaveBoundary {
    onset: usize,
    offset: usize,
    peak_amplitude: f64,
}

fn detection_signal(input: &[f64], sample_rate_hz: usize) -> Vec<f64> {
    let baseline_window = millis_to_samples(200, sample_rate_hz).max(3);
    let smoothing_window = millis_to_samples(30, sample_rate_hz).max(2);
    let baseline = trailing_moving_average(input, baseline_window);
    let high_pass = input
        .iter()
        .zip(baseline)
        .map(|(sample, baseline)| sample - baseline)
        .collect::<Vec<_>>();
    trailing_moving_average(&high_pass, smoothing_window)
}

fn qrs_energy(lead_i: &[f64], lead_ii: &[f64], sample_rate_hz: usize) -> Vec<f64> {
    let mut energy = vec![0.0; lead_i.len()];
    for index in 1..lead_i.len() {
        let derivative_i = lead_i[index] - lead_i[index - 1];
        let derivative_ii = lead_ii[index] - lead_ii[index - 1];
        energy[index] = derivative_i.powi(2) + derivative_ii.powi(2);
    }
    trailing_moving_average(&energy, millis_to_samples(80, sample_rate_hz).max(2))
}

fn local_maxima_with_refractory(
    signal: &[f64],
    threshold: f64,
    refractory_samples: usize,
) -> Vec<usize> {
    let mut peaks = Vec::new();
    for index in 1..signal.len().saturating_sub(1) {
        if signal[index] < threshold
            || signal[index] < signal[index - 1]
            || signal[index] <= signal[index + 1]
        {
            continue;
        }
        if let Some(last) = peaks.last_mut() {
            if index - *last < refractory_samples {
                if signal[index] > signal[*last] {
                    *last = index;
                }
                continue;
            }
        }
        peaks.push(index);
    }
    peaks
}

fn refine_peak(peak: usize, radius: usize, lead_i: &[f64], lead_ii: &[f64]) -> usize {
    let start = peak.saturating_sub(radius);
    let end = (peak + radius + 1).min(lead_i.len());
    (start..end)
        .max_by(|left, right| {
            vector_power(lead_i, lead_ii, *left).total_cmp(&vector_power(lead_i, lead_ii, *right))
        })
        .unwrap_or(peak)
}

fn vector_power(lead_i: &[f64], lead_ii: &[f64], index: usize) -> f64 {
    lead_i[index].powi(2) + lead_ii[index].powi(2)
}

fn median_complex(
    signal: &[f64],
    peaks: &[usize],
    pre_samples: usize,
    post_samples: usize,
) -> Vec<f64> {
    (0..=pre_samples + post_samples)
        .map(|offset| {
            let samples = peaks
                .iter()
                .map(|peak| signal[peak + offset - pre_samples])
                .collect::<Vec<_>>();
            median(&samples)
        })
        .collect()
}

fn edge_baseline(signal: &[f64], sample_rate_hz: usize) -> f64 {
    let leading = millis_to_samples(83, sample_rate_hz).max(1);
    let trailing = millis_to_samples(67, sample_rate_hz).max(1);
    let mut edges = signal[..leading.min(signal.len())].to_vec();
    edges.extend_from_slice(&signal[signal.len().saturating_sub(trailing)..]);
    median(&edges)
}

fn vector_slope(lead_i: &[f64], lead_ii: &[f64]) -> Vec<f64> {
    let mut slope = vec![0.0; lead_i.len()];
    for index in 1..lead_i.len() {
        slope[index] = ((lead_i[index] - lead_i[index - 1]).powi(2)
            + (lead_ii[index] - lead_ii[index - 1]).powi(2))
        .sqrt();
    }
    slope
}

fn measurement_noise(slope: &[f64], magnitude: &[f64], sample_rate_hz: usize) -> (f64, f64) {
    let leading = millis_to_samples(100, sample_rate_hz).max(1);
    let trailing = millis_to_samples(67, sample_rate_hz).max(1);
    let mut slope_edges = slope[..leading.min(slope.len())].to_vec();
    slope_edges.extend_from_slice(&slope[slope.len().saturating_sub(trailing)..]);
    let mut magnitude_edges = magnitude[..leading.min(magnitude.len())].to_vec();
    magnitude_edges.extend_from_slice(&magnitude[magnitude.len().saturating_sub(trailing)..]);
    (
        median(&slope_edges).max(f64::EPSILON),
        median(&magnitude_edges).max(f64::EPSILON),
    )
}

fn find_qrs_onset(
    slope: &[f64],
    center: usize,
    threshold: f64,
    quiet_run: usize,
    sample_rate_hz: usize,
) -> usize {
    let near = center.saturating_sub(millis_to_samples(50, sample_rate_hz));
    let far = center.saturating_sub(millis_to_samples(200, sample_rate_hz));
    for index in (far.max(quiet_run)..=near).rev() {
        if slope[index + 1 - quiet_run..=index]
            .iter()
            .all(|value| *value < threshold)
        {
            return index + 1;
        }
    }
    far
}

fn find_qrs_offset(
    slope: &[f64],
    center: usize,
    threshold: f64,
    quiet_run: usize,
    sample_rate_hz: usize,
) -> usize {
    let start = center + millis_to_samples(33, sample_rate_hz);
    let end = (center + millis_to_samples(250, sample_rate_hz))
        .min(slope.len().saturating_sub(quiet_run + 1));
    for index in start..=end {
        if slope[index..index + quiet_run]
            .iter()
            .all(|value| *value < threshold)
        {
            return index;
        }
    }
    end
}

fn delineate_p_wave(
    magnitude: &[f64],
    qrs_onset: usize,
    amplitude_noise: f64,
    sample_rate_hz: usize,
) -> Option<WaveBoundary> {
    let start = qrs_onset.saturating_sub(millis_to_samples(300, sample_rate_hz));
    let end = qrs_onset.saturating_sub(millis_to_samples(40, sample_rate_hz));
    if start >= end {
        return None;
    }
    let peak = max_index(magnitude, start, end)?;
    let peak_amplitude = magnitude[peak];
    let threshold = (amplitude_noise * 2.5).max(peak_amplitude * 0.12);
    let onset = search_below_backward(magnitude, peak, start, threshold);
    let offset = search_below_forward(magnitude, peak, end, threshold);
    (onset < offset).then_some(WaveBoundary {
        onset,
        offset,
        peak_amplitude,
    })
}

fn delineate_t_wave(
    magnitude: &[f64],
    qrs_offset: usize,
    center: usize,
    amplitude_noise: f64,
    sample_rate_hz: usize,
) -> Option<WaveBoundary> {
    let start = qrs_offset + millis_to_samples(60, sample_rate_hz);
    let end = (center + millis_to_samples(550, sample_rate_hz)).min(
        magnitude
            .len()
            .saturating_sub(millis_to_samples(50, sample_rate_hz)),
    );
    if start >= end {
        return None;
    }
    let peak = max_index(magnitude, start, end)?;
    let peak_amplitude = magnitude[peak];
    let threshold = (amplitude_noise * 3.0).max(peak_amplitude * 0.10);
    let onset = search_below_backward(magnitude, peak, start, threshold);
    let quiet_run = millis_to_samples(27, sample_rate_hz).max(3);
    let offset = (peak + 1..end)
        .find(|index| {
            *index + quiet_run < magnitude.len()
                && magnitude[*index..*index + quiet_run]
                    .iter()
                    .all(|value| *value < threshold)
        })
        .unwrap_or(end);
    (onset < offset).then_some(WaveBoundary {
        onset,
        offset,
        peak_amplitude,
    })
}

fn max_index(signal: &[f64], start: usize, end: usize) -> Option<usize> {
    (start..end).max_by(|left, right| signal[*left].total_cmp(&signal[*right]))
}

fn search_below_backward(signal: &[f64], from: usize, limit: usize, threshold: f64) -> usize {
    (limit..from)
        .rev()
        .find(|index| signal[*index] < threshold)
        .unwrap_or(limit)
}

fn search_below_forward(signal: &[f64], from: usize, limit: usize, threshold: f64) -> usize {
    (from + 1..limit)
        .find(|index| signal[*index] < threshold)
        .unwrap_or(limit)
}

fn frontal_axis(
    lead_i: &[f64],
    lead_ii: &[f64],
    baseline_i: f64,
    baseline_ii: f64,
    onset: usize,
    offset: usize,
) -> Option<i16> {
    if onset >= offset || offset > lead_i.len() || offset > lead_ii.len() {
        return None;
    }
    let area_i = lead_i[onset..offset]
        .iter()
        .map(|value| value - baseline_i)
        .sum::<f64>();
    let area_ii = lead_ii[onset..offset]
        .iter()
        .map(|value| value - baseline_ii)
        .sum::<f64>();
    let y = (2.0 * area_ii - area_i) / 3.0_f64.sqrt();
    let magnitude = area_i.hypot(y);
    if magnitude <= f64::EPSILON {
        return None;
    }
    Some(normalize_axis(y.atan2(area_i).to_degrees().round() as i16))
}

fn normalize_axis(axis: i16) -> i16 {
    if axis > 180 {
        axis - 360
    } else if axis <= -180 {
        axis + 360
    } else {
        axis
    }
}

fn corrected_qt_bazett(qt_ms: u16, average_rr_seconds: f64) -> u16 {
    round_u16(f64::from(qt_ms) / average_rr_seconds.sqrt())
}

fn corrected_qt_fridericia(qt_ms: u16, average_rr_seconds: f64) -> u16 {
    round_u16(f64::from(qt_ms) / average_rr_seconds.cbrt())
}

fn interval_ms(start: usize, end: usize, sample_rate_hz: usize) -> Option<u16> {
    (end > start).then(|| round_u16((end - start) as f64 * 1_000.0 / sample_rate_hz as f64))
}

fn millis_to_samples(milliseconds: usize, sample_rate_hz: usize) -> usize {
    (milliseconds * sample_rate_hz + 500) / 1_000
}

fn trailing_moving_average(input: &[f64], window: usize) -> Vec<f64> {
    let mut output = Vec::with_capacity(input.len());
    let mut sum = 0.0;
    for (index, value) in input.iter().copied().enumerate() {
        sum += value;
        if index >= window {
            sum -= input[index - window];
        }
        output.push(sum / (index + 1).min(window) as f64);
    }
    output
}

fn centered_moving_average(input: &[f64], window: usize) -> Vec<f64> {
    let radius = window / 2;
    let mut prefix = Vec::with_capacity(input.len() + 1);
    prefix.push(0.0);
    for value in input {
        prefix.push(prefix.last().copied().unwrap_or_default() + value);
    }
    (0..input.len())
        .map(|index| {
            let start = index.saturating_sub(radius);
            let end = (index + radius + 1).min(input.len());
            (prefix[end] - prefix[start]) / (end - start) as f64
        })
        .collect()
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn standard_deviation(values: &[f64], mean: f64) -> f64 {
    (values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len().max(1) as f64)
        .sqrt()
}

fn round_u16(value: f64) -> u16 {
    value.round().clamp(0.0, f64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontal_axis_uses_the_einthoven_coordinate_transform() {
        let lead_i = vec![1.0; 10];
        let lead_ii = vec![0.5; 10];
        assert_eq!(frontal_axis(&lead_i, &lead_ii, 0.0, 0.0, 0, 10), Some(0));

        let lead_i = vec![0.5; 10];
        let lead_ii = vec![1.0; 10];
        assert_eq!(frontal_axis(&lead_i, &lead_ii, 0.0, 0.0, 0, 10), Some(60));
    }

    #[test]
    fn calculates_bazett_and_fridericia_qt_corrections() {
        assert_eq!(corrected_qt_bazett(400, 1.0), 400);
        assert_eq!(corrected_qt_fridericia(400, 1.0), 400);
        assert_eq!(corrected_qt_bazett(400, 0.64), 500);
        assert_eq!(corrected_qt_fridericia(400, 0.64), 464);
    }

    #[test]
    fn withholds_measurements_when_no_qrs_is_detectable() {
        let leads = std::array::from_fn(|_| vec![0.0; 3_000]);
        let result = analyze(&leads, 300);
        assert_eq!(result.quality, AnalysisQuality::Insufficient);
        assert_eq!(result.ventricular_rate_bpm, None);
        assert_eq!(result.qt_interval_ms, None);
    }

    #[test]
    fn detects_measurements_in_a_synthetic_regular_ecg() {
        let sample_rate = 300usize;
        let sample_count = sample_rate * 10;
        let mut lead_i = vec![0.0; sample_count];
        let mut lead_ii = vec![0.0; sample_count];

        for beat_second in 1..10 {
            let r = beat_second * sample_rate;
            add_wave(
                &mut lead_i,
                r as isize - 60,
                9.0,
                0.10,
                45.0_f64.to_radians().cos(),
            );
            add_wave(
                &mut lead_ii,
                r as isize - 60,
                9.0,
                0.10,
                (45.0_f64 - 60.0).to_radians().cos(),
            );
            for (offset, width, amplitude) in [(-9, 3.0, -0.15), (0, 4.0, 1.0), (10, 4.0, -0.30)] {
                add_wave(
                    &mut lead_i,
                    r as isize + offset,
                    width,
                    amplitude,
                    60.0_f64.to_radians().cos(),
                );
                add_wave(
                    &mut lead_ii,
                    r as isize + offset,
                    width,
                    amplitude,
                    0.0_f64.to_radians().cos(),
                );
            }
            add_wave(
                &mut lead_i,
                r as isize + 84,
                18.0,
                0.35,
                30.0_f64.to_radians().cos(),
            );
            add_wave(
                &mut lead_ii,
                r as isize + 84,
                18.0,
                0.35,
                (30.0_f64 - 60.0).to_radians().cos(),
            );
        }

        for index in 0..sample_count {
            let noise = ((index * 17 % 23) as f64 - 11.0) * 0.0005;
            lead_i[index] += noise;
            lead_ii[index] -= noise * 0.7;
        }
        let leads = derive_six(lead_i, lead_ii);
        let result = analyze(&leads, sample_rate);

        assert_eq!(result.quality, AnalysisQuality::Good);
        assert!(matches!(result.ventricular_rate_bpm, Some(59..=61)));
        assert!(matches!(result.pr_interval_ms, Some(100..=260)));
        assert!(matches!(result.qrs_duration_ms, Some(35..=160)));
        assert!(matches!(result.qt_interval_ms, Some(250..=550)));
        assert!(matches!(result.qrs_axis_deg, Some(45..=75)));
    }

    fn add_wave(
        signal: &mut [f64],
        center: isize,
        sigma_samples: f64,
        amplitude: f64,
        projection: f64,
    ) {
        let radius = (sigma_samples * 4.0).ceil() as isize;
        for offset in -radius..=radius {
            let index = center + offset;
            if !(0..signal.len() as isize).contains(&index) {
                continue;
            }
            let gaussian = (-(offset as f64).powi(2) / (2.0 * sigma_samples.powi(2))).exp();
            signal[index as usize] += amplitude * projection * gaussian;
        }
    }

    fn derive_six(lead_i: Vec<f64>, lead_ii: Vec<f64>) -> [Vec<f64>; 6] {
        let mut leads: [Vec<f64>; 6] = std::array::from_fn(|_| Vec::with_capacity(lead_i.len()));
        for (i, ii) in lead_i.into_iter().zip(lead_ii) {
            let values = [i, ii, ii - i, -(i + ii) / 2.0, i - ii / 2.0, ii - i / 2.0];
            for (lead, value) in leads.iter_mut().zip(values) {
                lead.push(value);
            }
        }
        leads
    }
}
