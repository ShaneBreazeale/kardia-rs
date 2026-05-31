use std::fmt;

/// Monotonic timestamp in microseconds from the start of a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimestampMicros(pub u64);

impl TimestampMicros {
    pub const ZERO: Self = Self(0);

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// ECG sample rate in hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleRateHz(u16);

impl SampleRateHz {
    pub const fn new(hz: u16) -> Self {
        Self(hz)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Display for SampleRateHz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}
