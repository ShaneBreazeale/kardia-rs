use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub const KARDIA_6L_SIX_LEAD_SERVICE_UUID: &str = "AC060001-328C-A28F-9846-5A8AA212661B";
pub const KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID: &str = "AC060002-328C-A28F-9846-5A8AA212661B";
pub const KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID: &str = "AC060003-328C-A28F-9846-5A8AA212661B";
pub const KARDIA_6L_LIVE_READ_CHARACTERISTIC_005_UUID: &str =
    "AC060005-328C-A28F-9846-5A8AA212661B";
pub const KARDIA_6L_LIVE_READ_CHARACTERISTIC_006_UUID: &str =
    "AC060006-328C-A28F-9846-5A8AA212661B";

pub const BATTERY_SERVICE_UUID: &str = "0000180F-0000-1000-8000-00805F9B34FB";
pub const BATTERY_LEVEL_CHARACTERISTIC_UUID: &str = "00002A19-0000-1000-8000-00805F9B34FB";
pub const DEVICE_INFO_SERVICE_UUID: &str = "0000180A-0000-1000-8000-00805F9B34FB";
pub const DEVICE_INFO_SERIAL_NUMBER_CHARACTERISTIC_UUID: &str =
    "00002A25-0000-1000-8000-00805F9B34FB";
pub const DEVICE_INFO_FIRMWARE_REVISION_CHARACTERISTIC_UUID: &str =
    "00002A26-0000-1000-8000-00805F9B34FB";
pub const DEVICE_INFO_HARDWARE_REVISION_CHARACTERISTIC_UUID: &str =
    "00002A27-0000-1000-8000-00805F9B34FB";
pub const CLIENT_CHARACTERISTIC_CONFIG_DESCRIPTOR_UUID: &str =
    "00002902-0000-1000-8000-00805F9B34FB";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcgMode {
    SingleLead300Hz,
    DualLead300Hz,
    SingleLead600Hz,
    DualLead600Hz,
}

impl EcgMode {
    pub const fn setting(self) -> &'static str {
        match self {
            Self::SingleLead300Hz => "M1",
            Self::DualLead300Hz => "M2",
            Self::SingleLead600Hz => "M3",
            Self::DualLead600Hz => "M4",
        }
    }

    /// Lead count implied by the APK mode name, not independently observed.
    pub const fn nominal_lead_count(self) -> u8 {
        match self {
            Self::SingleLead300Hz | Self::SingleLead600Hz => 1,
            Self::DualLead300Hz | Self::DualLead600Hz => 2,
        }
    }

    /// Sample rate implied by the APK mode name, not necessarily the rate
    /// exposed by the device's notification transport.
    pub const fn nominal_sample_rate_hz(self) -> u16 {
        match self {
            Self::SingleLead300Hz | Self::DualLead300Hz => 300,
            Self::SingleLead600Hz | Self::DualLead600Hz => 600,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kardia6lGatt {
    pub service_uuid: &'static str,
    pub command_characteristic_uuid: &'static str,
    pub ecg_characteristic_uuid: &'static str,
}

impl Kardia6lGatt {
    pub const SIX_LEAD: Self = Self {
        service_uuid: KARDIA_6L_SIX_LEAD_SERVICE_UUID,
        command_characteristic_uuid: KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID,
        ecg_characteristic_uuid: KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID,
    };
}

pub fn unlock_token(device_name: &str) -> String {
    let digest = Sha256::digest(format!("Triangle{device_name}").as_bytes());
    let mut token = String::with_capacity(17);
    token.push('K');
    for byte in digest.iter().take(8) {
        write!(&mut token, "{byte:02x}").expect("writing to String cannot fail");
    }
    token
}

pub fn command_for_mode(device_name: &str, mode: EcgMode) -> String {
    format!("{} {}", mode.setting(), unlock_token(device_name))
}

pub fn uuid_matches(lhs: &str, rhs: &str) -> bool {
    lhs.eq_ignore_ascii_case(rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_apk_observed_unlock_command() {
        assert_eq!(unlock_token("Kardia6L"), "Kd8a179a137775575");
        assert_eq!(
            command_for_mode("Kardia6L", EcgMode::DualLead300Hz),
            "M2 Kd8a179a137775575"
        );
    }

    #[test]
    fn reports_mode_metadata() {
        assert_eq!(EcgMode::DualLead300Hz.setting(), "M2");
        assert_eq!(EcgMode::DualLead300Hz.nominal_lead_count(), 2);
        assert_eq!(EcgMode::DualLead300Hz.nominal_sample_rate_hz(), 300);
        assert_eq!(EcgMode::DualLead600Hz.setting(), "M4");
        assert_eq!(EcgMode::DualLead600Hz.nominal_sample_rate_hz(), 600);
    }
}
