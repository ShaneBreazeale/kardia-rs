use crate::ecg_analysis::{self, AnalysisQuality, EcgMeasurements};
use anyhow::{anyhow, bail, Context, Result};
use kardia_ble::{
    decode_m2_notification, uuid_matches, RawCaptureFile,
    KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID, M2_SAMPLE_RATE_HZ,
};
use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, LineCapStyle, LineJoinStyle, Mm, PdfDocument,
    PdfLayerReference, Point, Rgb,
};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::Path;

const PAGE_WIDTH_MM: f64 = 297.0;
const PAGE_HEIGHT_MM: f64 = 210.0;
const TRACE_LEFT_MM: f64 = 30.0;
const TRACE_TOP_MM: f64 = 35.0;
const TRACE_WIDTH_MM: f64 = 250.0;
const TRACE_BOTTOM_MM: f64 = 187.0;
const LEAD_NAMES: [&str; 6] = ["I", "II", "III", "aVR", "aVL", "aVF"];
const PROVISIONAL_SCALE_NOTE: &str =
    "PROVISIONAL AMPLITUDE: inferred 14-bit left-aligned scale; * axes use unverified device-native polarity";

#[derive(Debug, Clone, Copy)]
pub struct ReportOptions {
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub speed_mm_s: f64,
    pub gain_mm_mv: f64,
    pub mv_per_count: f64,
    pub invert: bool,
}

#[derive(Debug)]
struct ReportData {
    leads_mv: [Vec<f64>; 6],
    start_seconds: f64,
    duration_seconds: f64,
    packet_count: usize,
    capture_samples: usize,
    speed_mm_s: f64,
    gain_mm_mv: f64,
    mv_per_count: f64,
    inverted: bool,
    measurements: EcgMeasurements,
}

#[derive(Debug, Clone, Copy)]
struct PointMm {
    x: f64,
    y: f64,
}

pub fn render(
    input: &Path,
    out: &Path,
    options: ReportOptions,
    source_label: Option<&str>,
) -> Result<()> {
    validate_options(options)?;
    let report = decode_report_data(input, options)?;
    let source_label = match source_label {
        Some(label) if label.trim().is_empty() => bail!("--source-label must not be empty"),
        Some(label) => Cow::Borrowed(label),
        None => Cow::Owned(input.display().to_string()),
    };

    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }

    match out
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("svg") => {
            std::fs::write(out, render_svg(&report, source_label.as_ref()))
                .with_context(|| format!("write {}", out.display()))?;
        }
        Some("pdf") => {
            std::fs::write(out, render_pdf(&report, source_label.as_ref())?)
                .with_context(|| format!("write {}", out.display()))?;
        }
        _ => bail!("{} must have a .svg or .pdf extension", out.display()),
    }

    println!(
        "rendered {:.3}s from {:.3}s as a six-lead limb ECG to {}",
        report.duration_seconds,
        report.start_seconds,
        out.display()
    );
    println!(
        "paper: {:.1} mm/s, {:.1} mm/mV; display baseline: per-lead median; signal filter: none",
        report.speed_mm_s, report.gain_mm_mv
    );
    println!(
        "warning: amplitude uses provisional {:.12} mV/raw-count scale; device-native polarity is {}",
        report.mv_per_count,
        if report.inverted {
            "manually inverted"
        } else {
            "unverified"
        }
    );
    println!(
        "experimental measurements: {}",
        console_measurements(&report.measurements)
    );
    Ok(())
}

fn validate_options(options: ReportOptions) -> Result<()> {
    let positive_finite = [
        ("seconds", options.duration_seconds),
        ("speed-mm-s", options.speed_mm_s),
        ("gain-mm-mv", options.gain_mm_mv),
        ("mv-per-count", options.mv_per_count),
    ];
    for (name, value) in positive_finite {
        if !value.is_finite() || value <= 0.0 {
            bail!("--{name} must be a positive finite number");
        }
    }
    if !options.start_seconds.is_finite() || options.start_seconds < 0.0 {
        bail!("--start-seconds must be a non-negative finite number");
    }

    let requested_width_mm = options.duration_seconds * options.speed_mm_s;
    if requested_width_mm > TRACE_WIDTH_MM + f64::EPSILON {
        bail!(
            "requested trace is {requested_width_mm:.1} mm wide; this A4 layout permits at most \
             {TRACE_WIDTH_MM:.0} mm (for example, 10s at 25 mm/s)"
        );
    }
    Ok(())
}

fn decode_report_data(input: &Path, options: ReportOptions) -> Result<ReportData> {
    let capture = RawCaptureFile::read(input)?;
    ensure_confirmed_m2(&capture, input)?;

    let mut raw_leads: [Vec<f64>; 6] = std::array::from_fn(|_| Vec::new());
    let mut packet_count = 0usize;

    for record in capture.records.iter().filter(|record| {
        uuid_matches(
            &record.characteristic_uuid,
            KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID,
        )
    }) {
        let frame = decode_m2_notification(&record.payload).with_context(|| {
            format!(
                "{}:{}: decode M2 packet",
                input.display(),
                record.line_number
            )
        })?;
        packet_count += 1;

        for sample in &frame.samples {
            let lead_i = f64::from(sample.channel_1);
            let lead_ii = f64::from(sample.channel_2);
            let values = [
                lead_i,
                lead_ii,
                lead_ii - lead_i,
                -(lead_i + lead_ii) / 2.0,
                lead_i - lead_ii / 2.0,
                lead_ii - lead_i / 2.0,
            ];
            for (destination, value) in raw_leads.iter_mut().zip(values) {
                destination.push(value);
            }
        }
    }

    if packet_count == 0 {
        bail!("{} contains no M2 ECG notifications", input.display());
    }

    let capture_samples = raw_leads[0].len();
    let start_sample = seconds_to_sample(options.start_seconds)?;
    if start_sample >= capture_samples {
        bail!(
            "--start-seconds {:.3} is beyond the {:.3}s capture",
            options.start_seconds,
            capture_samples as f64 / f64::from(M2_SAMPLE_RATE_HZ)
        );
    }
    let requested_samples = seconds_to_sample(options.duration_seconds)?;
    let end_sample = start_sample
        .saturating_add(requested_samples)
        .min(capture_samples);
    if end_sample <= start_sample {
        return Err(anyhow!("selected report interval contains no samples"));
    }

    let polarity = if options.invert { -1.0 } else { 1.0 };
    let leads_mv = std::array::from_fn(|lead_index| {
        let segment = &raw_leads[lead_index][start_sample..end_sample];
        let baseline = median(segment);
        segment
            .iter()
            .map(|value| (value - baseline) * options.mv_per_count * polarity)
            .collect()
    });
    let measurements = ecg_analysis::analyze(&leads_mv, M2_SAMPLE_RATE_HZ as usize);

    Ok(ReportData {
        leads_mv,
        start_seconds: start_sample as f64 / f64::from(M2_SAMPLE_RATE_HZ),
        duration_seconds: (end_sample - start_sample) as f64 / f64::from(M2_SAMPLE_RATE_HZ),
        packet_count,
        capture_samples,
        speed_mm_s: options.speed_mm_s,
        gain_mm_mv: options.gain_mm_mv,
        mv_per_count: options.mv_per_count,
        inverted: options.invert,
        measurements,
    })
}

fn ensure_confirmed_m2(capture: &RawCaptureFile, input: &Path) -> Result<()> {
    if let Some(mode) = capture.metadata.requested_mode.as_deref() {
        if mode != "M2" {
            bail!(
                "{} requested mode {mode}; refusing to render it as a confirmed M2 recording",
                input.display()
            );
        }
    }
    Ok(())
}

fn seconds_to_sample(seconds: f64) -> Result<usize> {
    let samples = (seconds * f64::from(M2_SAMPLE_RATE_HZ)).round();
    if !samples.is_finite() || samples < 0.0 || samples > usize::MAX as f64 {
        bail!("time selection cannot be represented as a sample index");
    }
    Ok(samples as usize)
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

fn lead_baseline_y(lead_index: usize) -> f64 {
    let spacing = (TRACE_BOTTOM_MM - TRACE_TOP_MM) / 6.0;
    TRACE_TOP_MM + spacing * (lead_index as f64 + 0.5)
}

fn grid_lines() -> impl Iterator<Item = (bool, PointMm, PointMm)> {
    let x_start = 10;
    let x_end = 285;
    let y_start = TRACE_TOP_MM.floor() as i32;
    let y_end = TRACE_BOTTOM_MM.ceil() as i32;
    let vertical = (x_start..=x_end).map(move |x| {
        (
            x % 5 == 0,
            PointMm {
                x: f64::from(x),
                y: f64::from(y_start),
            },
            PointMm {
                x: f64::from(x),
                y: f64::from(y_end),
            },
        )
    });
    let horizontal = (y_start..=y_end).map(move |y| {
        (
            y % 5 == 0,
            PointMm {
                x: f64::from(x_start),
                y: f64::from(y),
            },
            PointMm {
                x: f64::from(x_end),
                y: f64::from(y),
            },
        )
    });
    vertical.chain(horizontal)
}

fn calibration_points(baseline_y: f64, gain_mm_mv: f64, speed_mm_s: f64) -> Vec<PointMm> {
    let pulse_x = 23.0;
    let baseline_width = 2.0;
    let pulse_width = 0.2 * speed_mm_s;
    vec![
        PointMm {
            x: pulse_x - baseline_width,
            y: baseline_y,
        },
        PointMm {
            x: pulse_x,
            y: baseline_y,
        },
        PointMm {
            x: pulse_x,
            y: baseline_y - gain_mm_mv,
        },
        PointMm {
            x: pulse_x + pulse_width,
            y: baseline_y - gain_mm_mv,
        },
        PointMm {
            x: pulse_x + pulse_width,
            y: baseline_y,
        },
        PointMm {
            x: pulse_x + pulse_width + baseline_width,
            y: baseline_y,
        },
    ]
}

fn trace_segments(report: &ReportData, lead_index: usize) -> Vec<Vec<PointMm>> {
    let baseline_y = lead_baseline_y(lead_index);
    vec![report.leads_mv[lead_index]
        .iter()
        .enumerate()
        .map(|(index, value_mv)| PointMm {
            x: TRACE_LEFT_MM + index as f64 * report.speed_mm_s / f64::from(M2_SAMPLE_RATE_HZ),
            y: baseline_y - value_mv * report.gain_mm_mv,
        })
        .collect()]
}

fn render_svg(report: &ReportData, source_label: &str) -> String {
    let mut svg = String::new();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{PAGE_WIDTH_MM}mm" height="{PAGE_HEIGHT_MM}mm" viewBox="0 0 {PAGE_WIDTH_MM} {PAGE_HEIGHT_MM}">"#
    )
    .unwrap();
    writeln!(svg, r##"<rect width="297" height="210" fill="#fffdfd"/>"##).unwrap();
    writeln!(
        svg,
        r#"<g fill="none" shape-rendering="geometricPrecision">"#
    )
    .unwrap();
    for (major, start, end) in grid_lines() {
        let (color, width) = if major {
            ("#e9a1b1", 0.18)
        } else {
            ("#f5d8df", 0.08)
        };
        writeln!(
            svg,
            r#"<path d="M{:.3} {:.3}L{:.3} {:.3}" stroke="{color}" stroke-width="{width}"/>"#,
            start.x, start.y, end.x, end.y
        )
        .unwrap();
    }

    for lead_index in 0..LEAD_NAMES.len() {
        let baseline_y = lead_baseline_y(lead_index);
        let calibration = calibration_points(baseline_y, report.gain_mm_mv, report.speed_mm_s);
        write_svg_path(&mut svg, &calibration, "#111827", 0.28);
        for segment in trace_segments(report, lead_index) {
            write_svg_path(&mut svg, &segment, "#111827", 0.28);
        }
    }
    writeln!(svg, "</g>").unwrap();
    writeln!(
        svg,
        r##"<rect x="174" y="4" width="113" height="27" rx="1" fill="#ffffff" stroke="#667085" stroke-width="0.25"/>"##
    )
    .unwrap();

    writeln!(
        svg,
        r##"<g font-family="Helvetica,Arial,sans-serif" fill="#111827">"##
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="10" y="10" font-size="5.2" font-weight="700">Six-lead limb ECG</text>"#
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="10" y="17" font-size="3.2">Source: {}</text>"#,
        escape_xml(source_label)
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="10" y="22" font-size="3.2">Start: {:.3}s  Duration: {:.3}s  Sample rate: {} Hz  Packets: {}  Capture samples: {}</text>"#,
        report.start_seconds,
        report.duration_seconds,
        M2_SAMPLE_RATE_HZ,
        report.packet_count,
        report.capture_samples,
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="10" y="27" font-size="3.2">{:.1} mm/s  {:.1} mm/mV  Baseline: per-lead median  Filter: none</text>"#,
        report.speed_mm_s, report.gain_mm_mv
    )
    .unwrap();
    writeln!(
        svg,
        r##"<text x="177" y="9" font-size="3.0" font-weight="700" fill="#b42318">AUTOMATED MEASUREMENTS - EXPERIMENTAL</text>"##
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="284" y="9" font-size="2.5" text-anchor="end">technical quality: {}</text>"#,
        quality_label(report.measurements.quality)
    )
    .unwrap();
    let rows = measurement_rows(&report.measurements);
    for (index, row) in rows[..3].iter().enumerate() {
        writeln!(
            svg,
            r#"<text x="177" y="{}" font-size="3.0" font-family="Courier New,Courier,monospace">{}</text>"#,
            15 + index * 6,
            escape_xml(row)
        )
        .unwrap();
    }
    for (index, row) in rows[3..].iter().enumerate() {
        writeln!(
            svg,
            r#"<text x="231" y="{}" font-size="3.0" font-family="Courier New,Courier,monospace">{}</text>"#,
            15 + index * 6,
            escape_xml(row)
        )
        .unwrap();
    }
    for (lead_index, name) in LEAD_NAMES.iter().enumerate() {
        writeln!(
            svg,
            r#"<text x="11.5" y="{:.3}" font-size="4.1" font-weight="700">{name}</text>"#,
            lead_baseline_y(lead_index) - 2.0
        )
        .unwrap();
    }
    writeln!(
        svg,
        r##"<text x="10" y="196" font-size="3.3" font-weight="700" fill="#b42318">{PROVISIONAL_SCALE_NOTE}</text>"##
    )
    .unwrap();
    writeln!(
        svg,
        r#"<text x="10" y="202" font-size="3.0">Six limb leads only - V1 through V6 were not recorded. Automated measurements are experimental; not for diagnosis or treatment.</text>"#
    )
    .unwrap();
    writeln!(svg, "</g></svg>").unwrap();
    svg
}

fn write_svg_path(output: &mut String, points: &[PointMm], color: &str, width: f64) {
    if points.len() < 2 {
        return;
    }
    write!(output, r#"<path d="M{:.3} {:.3}"#, points[0].x, points[0].y).unwrap();
    for point in &points[1..] {
        write!(output, "L{:.3} {:.3}", point.x, point.y).unwrap();
    }
    writeln!(
        output,
        r#"" stroke="{color}" stroke-width="{width}" stroke-linejoin="round" stroke-linecap="round"/>"#
    )
    .unwrap();
}

fn render_pdf(report: &ReportData, source_label: &str) -> Result<Vec<u8>> {
    let (document, page, layer) = PdfDocument::new(
        "Six-lead limb ECG",
        Mm(PAGE_WIDTH_MM as f32),
        Mm(PAGE_HEIGHT_MM as f32),
        "ECG",
    );
    let layer = document.get_page(page).get_layer(layer);

    for major in [false, true] {
        let (color, thickness_pt) = if major {
            (rgb(0.914, 0.631, 0.694), 0.42)
        } else {
            (rgb(0.961, 0.847, 0.875), 0.18)
        };
        layer.set_outline_color(color);
        layer.set_outline_thickness(thickness_pt);
        for (is_major, start, end) in grid_lines().filter(|(is_major, _, _)| *is_major == major) {
            let _ = is_major;
            layer.add_line(pdf_line(&[start, end]));
        }
    }

    layer.set_outline_color(rgb(0.067, 0.094, 0.153));
    layer.set_outline_thickness(0.8);
    layer.set_line_join_style(LineJoinStyle::Round);
    layer.set_line_cap_style(LineCapStyle::Round);
    for lead_index in 0..LEAD_NAMES.len() {
        layer.add_line(pdf_line(&calibration_points(
            lead_baseline_y(lead_index),
            report.gain_mm_mv,
            report.speed_mm_s,
        )));
        for segment in trace_segments(report, lead_index) {
            layer.add_line(pdf_line(&segment));
        }
    }
    layer.set_outline_color(rgb(0.40, 0.44, 0.52));
    layer.set_outline_thickness(0.7);
    let panel = [
        PointMm { x: 174.0, y: 4.0 },
        PointMm { x: 287.0, y: 4.0 },
        PointMm { x: 287.0, y: 31.0 },
        PointMm { x: 174.0, y: 31.0 },
        PointMm { x: 174.0, y: 4.0 },
    ];
    layer.add_line(pdf_line(&panel));

    let regular_font = document
        .add_builtin_font(BuiltinFont::Helvetica)
        .context("add PDF Helvetica font")?;
    let bold_font = document
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .context("add PDF Helvetica Bold font")?;
    let mono_font = document
        .add_builtin_font(BuiltinFont::Courier)
        .context("add PDF Courier font")?;
    add_pdf_text(
        &layer,
        &bold_font,
        10.0,
        10.0,
        15.0,
        "Six-lead limb ECG",
        rgb(0.067, 0.094, 0.153),
    );
    add_pdf_text(
        &layer,
        &regular_font,
        10.0,
        17.0,
        8.5,
        &format!("Source: {source_label}"),
        rgb(0.067, 0.094, 0.153),
    );
    add_pdf_text(
        &layer,
        &regular_font,
        10.0,
        22.0,
        8.5,
        &format!(
            "Start: {:.3}s  Duration: {:.3}s  Sample rate: {} Hz  Packets: {}  Capture samples: {}",
            report.start_seconds,
            report.duration_seconds,
            M2_SAMPLE_RATE_HZ,
            report.packet_count,
            report.capture_samples
        ),
        rgb(0.067, 0.094, 0.153),
    );
    add_pdf_text(
        &layer,
        &regular_font,
        10.0,
        27.0,
        8.5,
        &format!(
            "{:.1} mm/s  {:.1} mm/mV  Baseline: per-lead median  Filter: none",
            report.speed_mm_s, report.gain_mm_mv
        ),
        rgb(0.067, 0.094, 0.153),
    );
    for (lead_index, name) in LEAD_NAMES.iter().enumerate() {
        add_pdf_text(
            &layer,
            &bold_font,
            11.5,
            lead_baseline_y(lead_index) - 2.0,
            11.0,
            name,
            rgb(0.067, 0.094, 0.153),
        );
    }
    add_pdf_text(
        &layer,
        &bold_font,
        10.0,
        196.0,
        9.0,
        PROVISIONAL_SCALE_NOTE,
        rgb(0.706, 0.137, 0.094),
    );
    add_pdf_text(
        &layer,
        &bold_font,
        177.0,
        9.0,
        8.0,
        "AUTOMATED MEASUREMENTS - EXPERIMENTAL",
        rgb(0.706, 0.137, 0.094),
    );
    add_pdf_text(
        &layer,
        &regular_font,
        258.0,
        9.0,
        6.5,
        &format!(
            "technical quality: {}",
            quality_label(report.measurements.quality)
        ),
        rgb(0.067, 0.094, 0.153),
    );
    let rows = measurement_rows(&report.measurements);
    for (index, row) in rows[..3].iter().enumerate() {
        add_pdf_text(
            &layer,
            &mono_font,
            177.0,
            15.0 + index as f64 * 6.0,
            8.0,
            row,
            rgb(0.067, 0.094, 0.153),
        );
    }
    for (index, row) in rows[3..].iter().enumerate() {
        add_pdf_text(
            &layer,
            &mono_font,
            231.0,
            15.0 + index as f64 * 6.0,
            8.0,
            row,
            rgb(0.067, 0.094, 0.153),
        );
    }
    add_pdf_text(
        &layer,
        &regular_font,
        10.0,
        202.0,
        8.0,
        "Six limb leads only - V1 through V6 were not recorded. Automated measurements are experimental; not for diagnosis or treatment.",
        rgb(0.067, 0.094, 0.153),
    );

    document
        .save_to_bytes()
        .map_err(|error| anyhow!("serialize PDF: {error}"))
}

fn pdf_line(points: &[PointMm]) -> Line {
    Line {
        points: points
            .iter()
            .map(|point| {
                (
                    Point::new(Mm(point.x as f32), Mm((PAGE_HEIGHT_MM - point.y) as f32)),
                    false,
                )
            })
            .collect(),
        is_closed: false,
    }
}

fn add_pdf_text(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    x_mm: f64,
    y_mm: f64,
    size_pt: f32,
    text: &str,
    color: Color,
) {
    layer.set_fill_color(color);
    layer.use_text(
        text,
        size_pt,
        Mm(x_mm as f32),
        Mm((PAGE_HEIGHT_MM - y_mm) as f32),
        font,
    );
}

fn rgb(red: f32, green: f32, blue: f32) -> Color {
    Color::Rgb(Rgb {
        r: red,
        g: green,
        b: blue,
        icc_profile: None,
    })
}

fn quality_label(quality: AnalysisQuality) -> &'static str {
    match quality {
        AnalysisQuality::Good => "GOOD",
        AnalysisQuality::Marginal => "MARGINAL",
        AnalysisQuality::Insufficient => "INSUFFICIENT",
    }
}

fn measurement_rows(measurements: &EcgMeasurements) -> [String; 6] {
    [
        format!(
            "Vent rate {:>4} BPM",
            optional_u16(measurements.ventricular_rate_bpm)
        ),
        format!(
            "PR int    {:>4} ms",
            optional_u16(measurements.pr_interval_ms)
        ),
        format!(
            "QRS dur   {:>4} ms",
            optional_u16(measurements.qrs_duration_ms)
        ),
        format!(
            "QT/QTcB {:>3}/{:>3} ms",
            optional_u16(measurements.qt_interval_ms),
            optional_u16(measurements.qtc_bazett_ms)
        ),
        format!(
            "QTcF       {:>4} ms",
            optional_u16(measurements.qtc_fridericia_ms)
        ),
        format!(
            "P-QRS-T* {:>3} {:>3} {:>3} deg",
            optional_i16(measurements.p_axis_deg),
            optional_i16(measurements.qrs_axis_deg),
            optional_i16(measurements.t_axis_deg)
        ),
    ]
}

fn optional_u16(value: Option<u16>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_owned())
}

fn optional_i16(value: Option<i16>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_owned())
}

fn console_measurements(measurements: &EcgMeasurements) -> String {
    format!(
        "quality={} beats={} median_beats={} HR={} bpm PR={} ms QRS={} ms QT/QTcB={} / {} ms QTcF={} ms P-QRS-T={}/{}/{} deg",
        quality_label(measurements.quality),
        measurements.detected_beats,
        measurements.median_beats,
        optional_u16(measurements.ventricular_rate_bpm),
        optional_u16(measurements.pr_interval_ms),
        optional_u16(measurements.qrs_duration_ms),
        optional_u16(measurements.qt_interval_ms),
        optional_u16(measurements.qtc_bazett_ms),
        optional_u16(measurements.qtc_fridericia_ms),
        optional_i16(measurements.p_axis_deg),
        optional_i16(measurements.qrs_axis_deg),
        optional_i16(measurements.t_axis_deg),
    )
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn standard_ten_second_trace_fits_a4_layout() {
        let options = ReportOptions {
            start_seconds: 0.0,
            duration_seconds: 10.0,
            speed_mm_s: 25.0,
            gain_mm_mv: 10.0,
            mv_per_count: 10.0 / 65_536.0,
            invert: false,
        };
        validate_options(options).expect("standard report options");
    }

    #[test]
    fn rejects_trace_wider_than_plot() {
        let options = ReportOptions {
            start_seconds: 0.0,
            duration_seconds: 11.0,
            speed_mm_s: 25.0,
            gain_mm_mv: 10.0,
            mv_per_count: 10.0 / 65_536.0,
            invert: false,
        };
        assert!(validate_options(options).is_err());
    }

    #[test]
    fn median_removes_only_constant_display_offset() {
        let values = [100.0, 102.0, 104.0, 106.0];
        assert_eq!(median(&values), 103.0);
        let centered = values.map(|value| value - median(&values));
        assert_eq!(centered, [-3.0, -1.0, 1.0, 3.0]);
    }

    #[test]
    fn report_svg_has_physical_page_and_safety_labels() {
        let report = ReportData {
            leads_mv: std::array::from_fn(|_| vec![0.0; 300]),
            start_seconds: 0.0,
            duration_seconds: 1.0,
            packet_count: 34,
            capture_samples: 300,
            speed_mm_s: 25.0,
            gain_mm_mv: 10.0,
            mv_per_count: 10.0 / 65_536.0,
            inverted: false,
            measurements: EcgMeasurements::default(),
        };
        let svg = render_svg(&report, "capture.csv");
        assert!(svg.contains(r#"width="297mm" height="210mm""#));
        assert!(svg.contains("Six-lead limb ECG"));
        assert!(svg.contains("V1 through V6 were not recorded"));
        assert!(svg.contains(PROVISIONAL_SCALE_NOTE));
        assert!(svg.contains("AUTOMATED MEASUREMENTS - EXPERIMENTAL"));
    }

    #[test]
    fn m4_capture_is_not_rendered_as_confirmed_m2() {
        let input = format!(
            "# kardia_raw_capture_v2 requested_mode=M4 nominal_sample_rate_hz=600 nominal_leads=2\n\
             1000000,{KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID},{}\n",
            "00000000".repeat(9)
        );
        let capture = RawCaptureFile::from_reader(Cursor::new(input)).expect("synthetic capture");
        let error = ensure_confirmed_m2(&capture, Path::new("m4.csv")).unwrap_err();
        assert!(error.to_string().contains("refusing to render"));
    }
}
