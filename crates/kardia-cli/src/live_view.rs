use kardia_ble::{M2Frame, M2_SAMPLES_PER_PACKET, M2_SAMPLE_RATE_HZ};
use kardia_core::{LimbLeadSample, TimestampMicros};

pub fn format_m2(frame_count: u64, frame: &M2Frame) -> String {
    let raw = frame
        .samples
        .last()
        .expect("an M2 frame always contains samples");
    let elapsed_seconds =
        frame_count as f64 * M2_SAMPLES_PER_PACKET as f64 / M2_SAMPLE_RATE_HZ as f64;
    let six = LimbLeadSample::new(
        TimestampMicros::ZERO,
        i32::from(raw.channel_1),
        i32::from(raw.channel_2),
    )
    .derive_six_lead();
    format!(
        "{elapsed_seconds:6.2}s | Ch1/I {:+7} | Ch2/II {:+7} | III {:+7} | aVR {:+7} | aVL {:+7} | aVF {:+7}",
        six.lead_i, six.lead_ii, six.lead_iii, six.avr, six.avl, six.avf
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kardia_ble::decode_m2_notification;

    #[test]
    fn formats_all_six_live_lead_labels_and_values() {
        let mut payload = Vec::new();
        for _ in 0..M2_SAMPLES_PER_PACKET {
            payload.extend_from_slice(&100i16.to_le_bytes());
            payload.extend_from_slice(&300i16.to_le_bytes());
        }
        let frame = decode_m2_notification(&payload).expect("valid M2 frame");

        let line = format_m2(10, &frame);

        assert!(line.contains("Ch1/I    +100"));
        assert!(line.contains("Ch2/II    +300"));
        assert!(line.contains("III    +200"));
        assert!(line.contains("aVR    -200"));
        assert!(line.contains("aVL     -50"));
        assert!(line.contains("aVF    +250"));
    }
}
