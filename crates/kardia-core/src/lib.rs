//! Protocol-neutral ECG types used by the BLE, storage, UI, and ML layers.

pub mod lead;
pub mod recording;
pub mod sample;

pub use lead::{DerivedSixLeadSample, LeadSample, LimbLeadSample};
pub use recording::{Recording, RecordingMetadata};
pub use sample::{SampleRateHz, TimestampMicros};
