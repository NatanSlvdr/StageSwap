use crate::Size;
use crate::{Frame, FrameError};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MAGIC: u32 = 0x4153_4346;
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 40;
pub const MAX_FRAME_BYTES: u32 = 1280 * 720 * 4;
pub const FRAME_EXPIRY: Duration = Duration::from_secs(2);

pub fn is_expected_pipe_disconnect(error: u32) -> bool {
    // ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED, and
    // ERROR_OPERATION_ABORTED are normal when a camera consumer closes or the
    // publisher is shutting down.
    matches!(error, 109 | 232 | 233 | 995)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    pub sequence: u64,
    pub size: Size,
    pub stride: u32,
    pub timestamp_100ns: i64,
    pub frame_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderError {
    InvalidLength,
    InvalidMagic,
    InvalidVersion,
    InvalidSequence,
    InvalidDimensions,
    InvalidStride,
    InvalidFrameBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    InvalidHeader(HeaderError),
    InvalidFrame(FrameError),
    InvalidLength,
    InvalidSequence,
}

#[derive(Clone, Debug, Default)]
pub struct SharedFrameCache {
    last_sequence: u64,
    latest: Option<Arc<Frame>>,
}

impl SharedFrameCache {
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn ingest(
        &mut self,
        header: FrameHeader,
        pixels: Arc<[u8]>,
        received_at: Instant,
    ) -> Result<(), CacheError> {
        header.validate().map_err(CacheError::InvalidHeader)?;
        if header.sequence <= self.last_sequence {
            return Err(CacheError::InvalidSequence);
        }
        self.last_sequence = header.sequence;
        if header.frame_bytes == 0 {
            if !pixels.is_empty() {
                return Err(CacheError::InvalidLength);
            }
            self.latest = None;
            return Ok(());
        }
        if pixels.len() != header.frame_bytes as usize {
            return Err(CacheError::InvalidLength);
        }
        let frame = Frame::new(
            pixels,
            header.size,
            header.stride,
            header.sequence,
            header.timestamp_100ns,
            received_at,
        )
        .map_err(CacheError::InvalidFrame)?;
        self.latest = Some(Arc::new(frame));
        Ok(())
    }

    pub fn latest(&self, now: Instant) -> Option<Arc<Frame>> {
        self.latest
            .as_ref()
            .filter(|frame| frame.is_fresh_at(now, FRAME_EXPIRY))
            .cloned()
    }

    pub fn invalidate(&mut self) {
        self.latest = None;
    }
}

impl FrameHeader {
    pub fn invalidation(sequence: u64) -> Result<Self, HeaderError> {
        if sequence == 0 {
            return Err(HeaderError::InvalidSequence);
        }
        Ok(Self {
            sequence,
            size: Size::default(),
            stride: 0,
            timestamp_100ns: 0,
            frame_bytes: 0,
        })
    }

    pub fn validate(&self) -> Result<(), HeaderError> {
        if self.sequence == 0 {
            return Err(HeaderError::InvalidSequence);
        }
        if self.frame_bytes == 0 {
            return if self.size == Size::default() && self.stride == 0 {
                Ok(())
            } else {
                Err(HeaderError::InvalidFrameBytes)
            };
        }
        if self.size.width == 0 || self.size.height == 0 {
            return Err(HeaderError::InvalidDimensions);
        }
        let stride = self
            .size
            .width
            .checked_mul(4)
            .ok_or(HeaderError::InvalidStride)?;
        if self.stride != stride {
            return Err(HeaderError::InvalidStride);
        }
        let bytes = stride
            .checked_mul(self.size.height)
            .ok_or(HeaderError::InvalidFrameBytes)?;
        if bytes != self.frame_bytes || bytes > MAX_FRAME_BYTES {
            return Err(HeaderError::InvalidFrameBytes);
        }
        Ok(())
    }

    pub fn encode(self) -> Result<[u8; HEADER_LEN], HeaderError> {
        self.validate()?;
        let mut output = [0; HEADER_LEN];
        output[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        output[4..8].copy_from_slice(&VERSION.to_le_bytes());
        output[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        output[16..20].copy_from_slice(&self.size.width.to_le_bytes());
        output[20..24].copy_from_slice(&self.size.height.to_le_bytes());
        output[24..28].copy_from_slice(&self.stride.to_le_bytes());
        output[28..36].copy_from_slice(&self.timestamp_100ns.to_le_bytes());
        output[36..40].copy_from_slice(&self.frame_bytes.to_le_bytes());
        Ok(output)
    }

    pub fn decode(bytes: &[u8], previous_sequence: Option<u64>) -> Result<Self, HeaderError> {
        if bytes.len() != HEADER_LEN {
            return Err(HeaderError::InvalidLength);
        }
        let u32_at =
            |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"));
        let u64_at =
            |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("eight bytes"));
        if u32_at(0) != MAGIC {
            return Err(HeaderError::InvalidMagic);
        }
        if u32_at(4) != VERSION {
            return Err(HeaderError::InvalidVersion);
        }
        let header = Self {
            sequence: u64_at(8),
            size: Size::new(u32_at(16), u32_at(20)),
            stride: u32_at(24),
            timestamp_100ns: i64::from_le_bytes(bytes[28..36].try_into().expect("eight bytes")),
            frame_bytes: u32_at(36),
        };
        header.validate()?;
        if previous_sequence.is_some_and(|previous| header.sequence <= previous) {
            return Err(HeaderError::InvalidSequence);
        }
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_round_trips_explicit_forty_byte_header() {
        let header = FrameHeader {
            sequence: 9,
            size: Size::new(1280, 720),
            stride: 5120,
            timestamp_100ns: 33,
            frame_bytes: MAX_FRAME_BYTES,
        };
        let bytes = header.encode().unwrap();
        assert_eq!(bytes.len(), 40);
        assert_eq!(FrameHeader::decode(&bytes, Some(8)).unwrap(), header);
        assert_eq!(
            FrameHeader::decode(&bytes, Some(9)),
            Err(HeaderError::InvalidSequence)
        );
    }
    #[test]
    fn contract_rejects_oversized_and_malformed_frames() {
        let invalid = FrameHeader {
            sequence: 1,
            size: Size::new(1920, 1080),
            stride: 7680,
            timestamp_100ns: 0,
            frame_bytes: 1920 * 1080 * 4,
        };
        assert_eq!(invalid.validate(), Err(HeaderError::InvalidFrameBytes));
    }

    #[test]
    fn contract_expected_windows_pipe_disconnects_are_not_transport_failures() {
        for error in [109, 232, 233, 995] {
            assert!(is_expected_pipe_disconnect(error));
        }
        assert!(!is_expected_pipe_disconnect(5));
        assert!(!is_expected_pipe_disconnect(87));
    }

    #[test]
    fn contract_cache_enforces_sequence_invalidation_and_two_second_expiry() {
        let now = Instant::now();
        let header = FrameHeader {
            sequence: 1,
            size: Size::new(2, 2),
            stride: 8,
            timestamp_100ns: 10,
            frame_bytes: 16,
        };
        let mut cache = SharedFrameCache::default();
        cache.ingest(header, vec![0; 16].into(), now).unwrap();
        assert!(cache.latest(now + Duration::from_secs(2)).is_some());
        assert!(cache.latest(now + Duration::from_millis(2001)).is_none());
        assert_eq!(
            cache.ingest(header, vec![0; 16].into(), now),
            Err(CacheError::InvalidSequence)
        );
        cache
            .ingest(
                FrameHeader::invalidation(2).unwrap(),
                Vec::<u8>::new().into(),
                now,
            )
            .unwrap();
        assert!(cache.latest(now).is_none());
    }
}
