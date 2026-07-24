use crate::{
    decode_m2_notification, uuid_matches, KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID,
    KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID, M2_SAMPLES_PER_PACKET,
};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawCaptureMetadata {
    pub format_version: Option<u8>,
    pub requested_mode: Option<String>,
    pub nominal_sample_rate_hz: Option<u16>,
    pub nominal_lead_count: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCaptureRecord {
    pub line_number: usize,
    pub received_unix_micros: u128,
    pub characteristic_uuid: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawCaptureFile {
    pub metadata: RawCaptureMetadata,
    pub records: Vec<RawCaptureRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawCaptureInspection {
    pub total_records: usize,
    pub ecg_packets: usize,
    pub ecg_bytes: usize,
    pub ecg_payload_lengths: BTreeMap<usize, usize>,
    pub command_indications: Vec<Vec<u8>>,
    pub first_ecg_unix_micros: Option<u128>,
    pub last_ecg_unix_micros: Option<u128>,
    pub observed_packet_rate_hz: Option<f64>,
    pub m2_compatible_packets: usize,
    pub implied_m2_sample_rate_hz: Option<f64>,
}

impl RawCaptureFile {
    pub fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::from_reader(BufReader::new(file)).with_context(|| format!("parse {}", path.display()))
    }

    pub fn from_reader(reader: impl BufRead) -> Result<Self> {
        let mut capture = Self::default();

        for (line_index, line_result) in reader.lines().enumerate() {
            let line_number = line_index + 1;
            let line = line_result.with_context(|| format!("read line {line_number}"))?;
            if line.is_empty() {
                continue;
            }
            if line.starts_with("# kardia_raw_capture_v") {
                capture.metadata = parse_header(&line)
                    .with_context(|| format!("line {line_number}: parse capture header"))?;
                continue;
            }
            if line.starts_with('#') {
                continue;
            }

            let mut fields = line.splitn(3, ',');
            let received_unix_micros = required_field(&mut fields, "timestamp", line_number)?
                .parse::<u128>()
                .with_context(|| format!("line {line_number}: parse timestamp"))?;
            let characteristic_uuid =
                required_field(&mut fields, "characteristic UUID", line_number)?.to_owned();
            let payload_hex = required_field(&mut fields, "payload", line_number)?;
            let payload = decode_hex(payload_hex)
                .with_context(|| format!("line {line_number}: decode payload"))?;

            capture.records.push(RawCaptureRecord {
                line_number,
                received_unix_micros,
                characteristic_uuid,
                payload,
            });
        }

        Ok(capture)
    }

    pub fn inspect_kardia_6l(&self) -> RawCaptureInspection {
        let mut inspection = RawCaptureInspection {
            total_records: self.records.len(),
            ecg_packets: 0,
            ecg_bytes: 0,
            ecg_payload_lengths: BTreeMap::new(),
            command_indications: Vec::new(),
            first_ecg_unix_micros: None,
            last_ecg_unix_micros: None,
            observed_packet_rate_hz: None,
            m2_compatible_packets: 0,
            implied_m2_sample_rate_hz: None,
        };

        for record in &self.records {
            if uuid_matches(
                &record.characteristic_uuid,
                KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID,
            ) {
                inspection.ecg_packets += 1;
                inspection.ecg_bytes += record.payload.len();
                *inspection
                    .ecg_payload_lengths
                    .entry(record.payload.len())
                    .or_default() += 1;
                inspection
                    .first_ecg_unix_micros
                    .get_or_insert(record.received_unix_micros);
                inspection.last_ecg_unix_micros = Some(record.received_unix_micros);
                if decode_m2_notification(&record.payload).is_ok() {
                    inspection.m2_compatible_packets += 1;
                }
            } else if uuid_matches(
                &record.characteristic_uuid,
                KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID,
            ) {
                inspection.command_indications.push(record.payload.clone());
            }
        }

        inspection.observed_packet_rate_hz = observed_packet_rate(
            inspection.ecg_packets,
            inspection.first_ecg_unix_micros,
            inspection.last_ecg_unix_micros,
        );
        if inspection.ecg_packets > 0 && inspection.m2_compatible_packets == inspection.ecg_packets
        {
            inspection.implied_m2_sample_rate_hz = inspection
                .observed_packet_rate_hz
                .map(|rate| rate * M2_SAMPLES_PER_PACKET as f64);
        }

        inspection
    }
}

fn required_field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    name: &str,
    line_number: usize,
) -> Result<&'a str> {
    fields
        .next()
        .ok_or_else(|| anyhow!("line {line_number}: missing {name}"))
}

fn parse_header(line: &str) -> Result<RawCaptureMetadata> {
    let version_text = line
        .strip_prefix("# kardia_raw_capture_v")
        .ok_or_else(|| anyhow!("invalid capture header prefix"))?;
    let version_digits: String = version_text
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let format_version = version_digits
        .parse::<u8>()
        .context("parse capture format version")?;

    let requested_mode = header_value(line, "requested_mode")
        .or_else(|| header_value(line, "mode"))
        .filter(|value| !value.eq_ignore_ascii_case("none"))
        .map(str::to_owned);
    let nominal_sample_rate_hz =
        parse_optional_header_number::<u16>(line, "nominal_sample_rate_hz")?
            .or(parse_optional_header_number::<u16>(line, "sample_rate_hz")?);
    let nominal_lead_count = parse_optional_header_number::<u8>(line, "nominal_leads")?
        .or(parse_optional_header_number::<u8>(line, "leads")?);

    Ok(RawCaptureMetadata {
        format_version: Some(format_version),
        requested_mode,
        nominal_sample_rate_hz,
        nominal_lead_count,
    })
}

fn header_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split_ascii_whitespace()
        .find_map(|field| field.strip_prefix(&prefix))
}

fn parse_optional_header_number<T>(line: &str, key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    header_value(line, key)
        .map(|value| {
            value
                .parse::<T>()
                .with_context(|| format!("parse header field {key}"))
        })
        .transpose()
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0 {
        return Err(anyhow!("hex payload has odd length {}", value.len()));
    }
    (0..value.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&value[offset..offset + 2], 16)
                .with_context(|| format!("invalid hex at byte {}", offset / 2))
        })
        .collect()
}

fn observed_packet_rate(
    packet_count: usize,
    first_unix_micros: Option<u128>,
    last_unix_micros: Option<u128>,
) -> Option<f64> {
    if packet_count < 2 {
        return None;
    }
    let elapsed_micros = last_unix_micros?.checked_sub(first_unix_micros?)?;
    if elapsed_micros == 0 {
        return None;
    }
    Some((packet_count - 1) as f64 * 1_000_000.0 / elapsed_micros as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn m2_payload_hex() -> String {
        "00000000".repeat(M2_SAMPLES_PER_PACKET)
    }

    #[test]
    fn parses_v2_metadata_and_inspects_m2_transport() {
        let input = format!(
            "# kardia_raw_capture_v2 requested_mode=M2 nominal_sample_rate_hz=300 nominal_leads=2\n\
             1000000,{KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID},01\n\
             1000000,{KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID},{}\n\
             1030000,{KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID},{}\n",
            m2_payload_hex(),
            m2_payload_hex()
        );

        let capture = RawCaptureFile::from_reader(Cursor::new(input)).expect("valid capture");
        assert_eq!(capture.metadata.format_version, Some(2));
        assert_eq!(capture.metadata.requested_mode.as_deref(), Some("M2"));
        assert_eq!(capture.metadata.nominal_sample_rate_hz, Some(300));
        assert_eq!(capture.metadata.nominal_lead_count, Some(2));

        let inspection = capture.inspect_kardia_6l();
        assert_eq!(inspection.total_records, 3);
        assert_eq!(inspection.ecg_packets, 2);
        assert_eq!(inspection.ecg_bytes, 72);
        assert_eq!(inspection.ecg_payload_lengths, BTreeMap::from([(36, 2)]));
        assert_eq!(inspection.command_indications, vec![vec![0x01]]);
        assert_eq!(inspection.m2_compatible_packets, 2);
        assert!((inspection.observed_packet_rate_hz.unwrap() - 33.333_333).abs() < 0.001);
        assert!((inspection.implied_m2_sample_rate_hz.unwrap() - 300.0).abs() < 0.001);
    }

    #[test]
    fn reads_legacy_v1_requested_metadata_names() {
        let input = "# kardia_raw_capture_v1 mode=M4 sample_rate_hz=600 leads=2\n";
        let capture = RawCaptureFile::from_reader(Cursor::new(input)).expect("valid capture");

        assert_eq!(capture.metadata.format_version, Some(1));
        assert_eq!(capture.metadata.requested_mode.as_deref(), Some("M4"));
        assert_eq!(capture.metadata.nominal_sample_rate_hz, Some(600));
        assert_eq!(capture.metadata.nominal_lead_count, Some(2));
    }

    #[test]
    fn treats_listen_only_mode_as_unrequested() {
        let input = "# kardia_raw_capture_v2 requested_mode=none\n";
        let capture = RawCaptureFile::from_reader(Cursor::new(input)).expect("valid capture");

        assert_eq!(capture.metadata.format_version, Some(2));
        assert_eq!(capture.metadata.requested_mode, None);
        assert_eq!(capture.metadata.nominal_sample_rate_hz, None);
        assert_eq!(capture.metadata.nominal_lead_count, None);
    }

    #[test]
    fn rejects_malformed_payload_hex_with_line_number() {
        let input = format!("1000000,{KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID},123\n");
        let error = RawCaptureFile::from_reader(Cursor::new(input)).unwrap_err();

        assert!(format!("{error:#}").contains("line 1"));
        assert!(format!("{error:#}").contains("odd length"));
    }
}
