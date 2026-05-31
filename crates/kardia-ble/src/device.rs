use crate::kardia6l::{uuid_matches, KARDIA_6L_SIX_LEAD_SERVICE_UUID};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFingerprint {
    pub name: Option<String>,
    pub address: Option<String>,
    pub advertised_services: Vec<String>,
    pub manufacturer_data: Vec<(u16, Vec<u8>)>,
}

impl DeviceFingerprint {
    pub fn looks_like_kardia_6l(&self) -> bool {
        let name_matches = self
            .name
            .as_deref()
            .map(|name| {
                let normalized = name.to_ascii_lowercase();
                normalized.contains("kardia") || normalized.contains("alivecor")
            })
            .unwrap_or(false);

        name_matches
            || self
                .advertised_services
                .iter()
                .any(|service| uuid_matches(service, KARDIA_6L_SIX_LEAD_SERVICE_UUID))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_kardia_6l_service_uuid() {
        let fingerprint = DeviceFingerprint {
            name: None,
            address: None,
            advertised_services: vec!["ac060001-328c-a28f-9846-5a8aa212661b".to_owned()],
            manufacturer_data: Vec::new(),
        };

        assert!(fingerprint.looks_like_kardia_6l());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCandidate {
    pub id: String,
    pub fingerprint: DeviceFingerprint,
    pub rssi: Option<i16>,
}
