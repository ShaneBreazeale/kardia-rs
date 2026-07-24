//! BLE-facing types for discovering and capturing Kardia 6L data.
//!
//! This crate starts with protocol-neutral capture records. Platform BLE
//! implementations can be added behind these boundaries once we have device
//! observations to test against.

pub mod capture;
pub mod capture_file;
pub mod device;
pub mod kardia6l;
pub mod live;
pub mod protocol;

pub use capture::{BleCaptureEvent, CharacteristicId, RawNotification};
pub use capture_file::{
    RawCaptureFile, RawCaptureInspection, RawCaptureMetadata, RawCaptureRecord,
};
pub use device::{DeviceCandidate, DeviceFingerprint};
pub use kardia6l::{
    command_for_mode, unlock_token, uuid_matches, EcgMode, Kardia6lGatt,
    BATTERY_LEVEL_CHARACTERISTIC_UUID, BATTERY_SERVICE_UUID,
    CLIENT_CHARACTERISTIC_CONFIG_DESCRIPTOR_UUID,
    DEVICE_INFO_FIRMWARE_REVISION_CHARACTERISTIC_UUID,
    DEVICE_INFO_HARDWARE_REVISION_CHARACTERISTIC_UUID,
    DEVICE_INFO_SERIAL_NUMBER_CHARACTERISTIC_UUID, DEVICE_INFO_SERVICE_UUID,
    KARDIA_6L_LIVE_READ_CHARACTERISTIC_005_UUID, KARDIA_6L_LIVE_READ_CHARACTERISTIC_006_UUID,
    KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID, KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID,
    KARDIA_6L_SIX_LEAD_SERVICE_UUID,
};
pub use live::{
    capture_raw_with_m2_observer, scan, CaptureOptions, DiscoveredDevice, RawCaptureStats,
};
pub use protocol::{
    decode_m2_notification, DecodeError, M2Frame, RawDualLeadSample, M2_PACKET_BYTES,
    M2_SAMPLES_PER_PACKET, M2_SAMPLE_RATE_HZ,
};
