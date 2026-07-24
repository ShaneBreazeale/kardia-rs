use thiserror::Error;

pub const M2_PACKET_BYTES: usize = 36;
pub const M2_SAMPLES_PER_PACKET: usize = 9;
pub const M2_SAMPLE_RATE_HZ: u16 = 300;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("M2 packet has {actual} bytes; expected {expected}")]
    InvalidM2Length { expected: usize, actual: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDualLeadSample {
    pub channel_1: i16,
    pub channel_2: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M2Frame {
    pub samples: [RawDualLeadSample; M2_SAMPLES_PER_PACKET],
}

/// Decode one dual-lead, 300 Hz (`M2`) Kardia 6L notification.
///
/// A live capture from firmware 3.0.1 established that every notification is
/// 36 bytes at 33 1/3 packets per second. Each frame contains nine pairs of
/// little-endian signed 16-bit values, interleaved as channel 1 then channel 2.
/// Controlled electrode-removal tests identify channel 1 as lead I and channel
/// 2 as lead II. Values stay in device-native units; polarity, scaling, and any
/// low-bit status encoding remain unconfirmed.
pub fn decode_m2_notification(payload: &[u8]) -> Result<M2Frame, DecodeError> {
    if payload.len() != M2_PACKET_BYTES {
        return Err(DecodeError::InvalidM2Length {
            expected: M2_PACKET_BYTES,
            actual: payload.len(),
        });
    }

    let mut samples = [RawDualLeadSample {
        channel_1: 0,
        channel_2: 0,
    }; M2_SAMPLES_PER_PACKET];
    for (sample, bytes) in samples.iter_mut().zip(payload.chunks_exact(4)) {
        sample.channel_1 = i16::from_le_bytes([bytes[0], bytes[1]]);
        sample.channel_2 = i16::from_le_bytes([bytes[2], bytes[3]]);
    }

    Ok(M2Frame { samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_nine_little_endian_interleaved_m2_samples() {
        let expected = [
            (-32_768, 32_767),
            (-2_000, 2_000),
            (-3, 3),
            (-2, 2),
            (-1, 1),
            (0, 0),
            (1, -1),
            (2, -2),
            (3, -3),
        ];
        let mut payload = Vec::with_capacity(M2_PACKET_BYTES);
        for (lead_i, lead_ii) in expected {
            payload.extend_from_slice(&i16::to_le_bytes(lead_i));
            payload.extend_from_slice(&i16::to_le_bytes(lead_ii));
        }

        let frame = decode_m2_notification(&payload).expect("valid M2 packet");
        let actual: Vec<_> = frame
            .samples
            .iter()
            .map(|sample| (sample.channel_1, sample.channel_2))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_m2_packets_with_unexpected_length() {
        assert_eq!(
            decode_m2_notification(&[0; M2_PACKET_BYTES - 1]),
            Err(DecodeError::InvalidM2Length {
                expected: M2_PACKET_BYTES,
                actual: M2_PACKET_BYTES - 1,
            })
        );
    }
}
