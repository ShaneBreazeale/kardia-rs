use crate::{lead::LimbLeadSample, sample::SampleRateHz};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingMetadata {
    pub device_name: Option<String>,
    pub device_id: Option<String>,
    pub sample_rate: Option<SampleRateHz>,
    pub notes: Option<String>,
}

impl RecordingMetadata {
    pub fn empty() -> Self {
        Self {
            device_name: None,
            device_id: None,
            sample_rate: None,
            notes: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub metadata: RecordingMetadata,
    pub samples: Vec<LimbLeadSample>,
}

impl Recording {
    pub fn new(metadata: RecordingMetadata) -> Self {
        Self {
            metadata,
            samples: Vec::new(),
        }
    }

    pub fn push(&mut self, sample: LimbLeadSample) {
        self.samples.push(sample);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}
