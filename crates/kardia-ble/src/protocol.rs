use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("packet format is not known yet")]
    UnknownFormat,
}

/// Placeholder for the Kardia 6L stream decoder.
///
/// The first reverse-engineering milestone should feed this function with raw
/// notification payload fixtures and document each accepted packet shape.
pub fn decode_notification(_payload: &[u8]) -> Result<(), DecodeError> {
    Err(DecodeError::UnknownFormat)
}
