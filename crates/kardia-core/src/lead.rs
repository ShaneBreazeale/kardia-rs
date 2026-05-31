use crate::sample::TimestampMicros;

/// One signed ECG lead value in device-native units.
///
/// Calibration is deliberately not baked into this type. During reverse
/// engineering we need to keep raw units intact until scale and offset are
/// supported by captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeadSample {
    pub timestamp: TimestampMicros,
    pub value: i32,
}

impl LeadSample {
    pub fn new(timestamp: TimestampMicros, value: i32) -> Self {
        Self { timestamp, value }
    }
}

/// The two independent limb leads expected from a six-lead Kardia-style device.
///
/// Standard limb-lead derivation can reconstruct III, aVR, aVL, and aVF from I
/// and II.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimbLeadSample {
    pub timestamp: TimestampMicros,
    pub lead_i: i32,
    pub lead_ii: i32,
}

impl LimbLeadSample {
    pub fn new(timestamp: TimestampMicros, lead_i: i32, lead_ii: i32) -> Self {
        Self {
            timestamp,
            lead_i,
            lead_ii,
        }
    }

    pub fn derive_six_lead(self) -> DerivedSixLeadSample {
        let lead_iii = self.lead_ii - self.lead_i;

        DerivedSixLeadSample {
            timestamp: self.timestamp,
            lead_i: self.lead_i,
            lead_ii: self.lead_ii,
            lead_iii,
            avr: -((self.lead_i + self.lead_ii) / 2),
            avl: self.lead_i - (self.lead_ii / 2),
            avf: self.lead_ii - (self.lead_i / 2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedSixLeadSample {
    pub timestamp: TimestampMicros,
    pub lead_i: i32,
    pub lead_ii: i32,
    pub lead_iii: i32,
    pub avr: i32,
    pub avl: i32,
    pub avf: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_standard_limb_leads_from_i_and_ii() {
        let sample = LimbLeadSample::new(TimestampMicros(42), 100, 300).derive_six_lead();

        assert_eq!(sample.timestamp, TimestampMicros(42));
        assert_eq!(sample.lead_i, 100);
        assert_eq!(sample.lead_ii, 300);
        assert_eq!(sample.lead_iii, 200);
        assert_eq!(sample.avr, -200);
        assert_eq!(sample.avl, -50);
        assert_eq!(sample.avf, 250);
    }
}
