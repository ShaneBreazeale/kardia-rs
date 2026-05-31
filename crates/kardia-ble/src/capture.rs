use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacteristicId {
    pub service_uuid: String,
    pub characteristic_uuid: String,
}

impl CharacteristicId {
    pub fn new(service_uuid: impl Into<String>, characteristic_uuid: impl Into<String>) -> Self {
        Self {
            service_uuid: service_uuid.into(),
            characteristic_uuid: characteristic_uuid.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNotification {
    pub characteristic: CharacteristicId,
    pub received_at: SystemTime,
    pub payload: Vec<u8>,
}

impl RawNotification {
    pub fn new(
        characteristic: CharacteristicId,
        received_at: SystemTime,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            characteristic,
            received_at,
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BleCaptureEvent {
    Connected { device_id: String },
    Disconnected { device_id: String },
    Notification(RawNotification),
}
