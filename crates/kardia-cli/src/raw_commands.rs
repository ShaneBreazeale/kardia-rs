use anyhow::{anyhow, Context, Result};
use kardia_ble::{
    decode_m2_notification, uuid_matches, RawCaptureFile,
    KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID, M2_SAMPLE_RATE_HZ,
};
use kardia_core::{LimbLeadSample, TimestampMicros};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub fn inspect_raw(input: &Path) -> Result<()> {
    let capture = RawCaptureFile::read(input)?;
    let observed = capture.inspect_kardia_6l();
    let metadata = &capture.metadata;

    println!("capture: {}", input.display());
    println!(
        "requested: mode={} nominal_rate={} nominal_leads={}",
        metadata.requested_mode.as_deref().unwrap_or("unknown"),
        metadata
            .nominal_sample_rate_hz
            .map(|value| format!("{value} Hz"))
            .unwrap_or_else(|| "unknown".to_owned()),
        metadata
            .nominal_lead_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned()),
    );

    let packet_lengths = observed
        .ecg_payload_lengths
        .iter()
        .map(|(bytes, count)| format!("{bytes}B x {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "observed: {} ECG packets / {} bytes; payloads: {}",
        observed.ecg_packets,
        observed.ecg_bytes,
        if packet_lengths.is_empty() {
            "none"
        } else {
            &packet_lengths
        }
    );
    if let Some(rate) = observed.observed_packet_rate_hz {
        println!("packet cadence: {rate:.3} packets/s");
    }

    println!(
        "M2 transport compatibility: {}/{} packets",
        observed.m2_compatible_packets, observed.ecg_packets
    );
    if let Some(rate) = observed.implied_m2_sample_rate_hz {
        println!("M2-compatible implied sample rate: {rate:.3} samples/s/channel");
    }

    if observed.command_indications.is_empty() {
        println!("command indications: none");
    } else {
        println!(
            "command indications: {}",
            observed
                .command_indications
                .iter()
                .map(|payload| encode_hex(payload))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if let (Some(nominal), Some(observed_rate)) = (
        metadata.nominal_sample_rate_hz,
        observed.implied_m2_sample_rate_hz,
    ) {
        if f64::from(nominal) > observed_rate * 1.5 {
            println!(
                "warning: requested nominal rate is {nominal} Hz, but the observed M2-compatible transport exposes only {observed_rate:.3} samples/s/channel"
            );
        }
    }

    Ok(())
}

pub fn export_six_lead_m2(input: &Path, out: &Path) -> Result<()> {
    let capture = RawCaptureFile::read(input)?;
    if let Some(mode) = capture.metadata.requested_mode.as_deref() {
        if mode != "M2" {
            return Err(anyhow!(
                "{} requested mode {mode}; refusing to label it as a confirmed M2 recording",
                input.display()
            ));
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let output_file = File::create(out).with_context(|| format!("create {}", out.display()))?;
    let mut writer = BufWriter::new(output_file);
    writeln!(
        writer,
        "sample_index,elapsed_micros,packet_received_unix_micros,sample_in_packet,raw_channel_1,raw_channel_2,lead_i,lead_ii,lead_iii,avr,avl,avf"
    )?;

    let mut packet_count = 0usize;
    let mut sample_index = 0u64;
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
        for (sample_in_packet, raw) in frame.samples.iter().enumerate() {
            // Generate an exact sample clock anchored at capture start. Packet
            // receive time remains separate so transport jitter stays visible.
            let elapsed_micros = sample_index * 1_000_000 / u64::from(M2_SAMPLE_RATE_HZ);
            let six = LimbLeadSample::new(
                TimestampMicros(elapsed_micros),
                i32::from(raw.channel_1),
                i32::from(raw.channel_2),
            )
            .derive_six_lead();
            writeln!(
                writer,
                "{sample_index},{elapsed_micros},{},{sample_in_packet},{},{},{},{},{},{},{},{}",
                record.received_unix_micros,
                raw.channel_1,
                raw.channel_2,
                six.lead_i,
                six.lead_ii,
                six.lead_iii,
                six.avr,
                six.avl,
                six.avf,
            )?;
            sample_index += 1;
        }
        packet_count += 1;
    }

    writer.flush()?;
    if packet_count == 0 {
        return Err(anyhow!(
            "{} contains no M2 ECG notifications",
            input.display()
        ));
    }
    println!(
        "decoded {packet_count} M2 packets into {sample_index} samples at {M2_SAMPLE_RATE_HZ} Hz and wrote {}",
        out.display()
    );
    println!(
        "note: channel identity is confirmed; polarity and physical-unit calibration remain unknown"
    );
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
