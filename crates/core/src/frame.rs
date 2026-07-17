use crate::Size;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const PIPELINE_SIZE: Size = Size::new(1280, 720);
pub const PIPELINE_FPS: u32 = 30;
pub const FRAME_STALE_AFTER: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub struct FrameMetadata {
    pub sequence: u64,
    pub timestamp_100ns: i64,
    pub received_at: Instant,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pixels: Arc<[u8]>,
    pub size: Size,
    pub stride: u32,
    pub sequence: u64,
    pub timestamp_100ns: i64,
    pub received_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    Empty,
    InvalidStride,
    InvalidLength,
    TooLarge,
}

impl Frame {
    pub fn new(
        pixels: Arc<[u8]>,
        size: Size,
        stride: u32,
        sequence: u64,
        timestamp_100ns: i64,
        received_at: Instant,
    ) -> Result<Self, FrameError> {
        if size.width == 0 || size.height == 0 {
            return Err(FrameError::Empty);
        }
        let row_bytes = size.width.checked_mul(4).ok_or(FrameError::TooLarge)?;
        if stride != row_bytes {
            return Err(FrameError::InvalidStride);
        }
        let expected = usize::try_from(stride)
            .ok()
            .and_then(|value| value.checked_mul(size.height as usize))
            .ok_or(FrameError::TooLarge)?;
        if pixels.len() != expected {
            return Err(FrameError::InvalidLength);
        }
        Ok(Self {
            pixels,
            size,
            stride,
            sequence,
            timestamp_100ns,
            received_at,
        })
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    pub fn pixels_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }
    pub fn is_fresh_at(&self, now: Instant, maximum_age: Duration) -> bool {
        now.checked_duration_since(self.received_at)
            .is_some_and(|age| age <= maximum_age)
    }

    pub fn placeholder(
        size: Size,
        color_bgra: u32,
        sequence: u64,
        timestamp_100ns: i64,
        now: Instant,
    ) -> Self {
        let stride = size.width * 4;
        let mut pixels = vec![0; stride as usize * size.height as usize];
        let pixel = color_bgra.to_le_bytes();
        for output in pixels.chunks_exact_mut(4) {
            output.copy_from_slice(&pixel);
        }
        Self::new(pixels.into(), size, stride, sequence, timestamp_100ns, now)
            .expect("placeholder dimensions are valid")
    }

    pub fn aspect_fit(&self, output: Size, now: Instant) -> Self {
        let mut pixels = vec![0; output.width as usize * output.height as usize * 4];
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 0xff;
        }
        let scale = f64::min(
            output.width as f64 / self.size.width as f64,
            output.height as f64 / self.size.height as f64,
        );
        let width = (self.size.width as f64 * scale).round().max(1.0) as u32;
        let height = (self.size.height as f64 * scale).round().max(1.0) as u32;
        let x_offset = (output.width - width) / 2;
        let y_offset = (output.height - height) / 2;
        for y in 0..height {
            let source_y =
                ((u64::from(y) * u64::from(self.size.height)) / u64::from(height)) as u32;
            for x in 0..width {
                let source_x =
                    ((u64::from(x) * u64::from(self.size.width)) / u64::from(width)) as u32;
                let source = source_y as usize * self.stride as usize + source_x as usize * 4;
                let destination = (y + y_offset) as usize * output.width as usize * 4
                    + (x + x_offset) as usize * 4;
                pixels[destination..destination + 4]
                    .copy_from_slice(&self.pixels[source..source + 4]);
            }
        }
        Self::new(
            pixels.into(),
            output,
            output.width * 4,
            self.sequence,
            self.timestamp_100ns,
            now,
        )
        .expect("aspect-fit output is valid")
    }

    pub fn blend(
        camera: Option<&Self>,
        screen: Option<&Self>,
        screen_mix: f64,
        color_bgra: u32,
        output: Size,
        metadata: FrameMetadata,
    ) -> Self {
        if camera.is_none() && screen.is_none() {
            return Self::placeholder(
                output,
                color_bgra,
                metadata.sequence,
                metadata.timestamp_100ns,
                metadata.received_at,
            );
        }
        let camera = camera
            .map(|frame| frame.aspect_fit(output, metadata.received_at))
            .unwrap_or_else(|| {
                Self::placeholder(
                    output,
                    color_bgra,
                    metadata.sequence,
                    metadata.timestamp_100ns,
                    metadata.received_at,
                )
            });
        let screen = screen
            .map(|frame| frame.aspect_fit(output, metadata.received_at))
            .unwrap_or_else(|| {
                Self::placeholder(
                    output,
                    color_bgra,
                    metadata.sequence,
                    metadata.timestamp_100ns,
                    metadata.received_at,
                )
            });
        let mix = screen_mix.clamp(0.0, 1.0);
        let mut pixels = vec![0; camera.pixels.len()];
        for ((output, left), right) in pixels
            .iter_mut()
            .zip(camera.pixels.iter())
            .zip(screen.pixels.iter())
        {
            *output = ((*left as f64 * (1.0 - mix)) + (*right as f64 * mix)).round() as u8;
        }
        Self::new(
            pixels.into(),
            output,
            output.width * 4,
            metadata.sequence,
            metadata.timestamp_100ns,
            metadata.received_at,
        )
        .expect("blend output is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_frames() {
        let now = Instant::now();
        assert_eq!(
            Frame::new(vec![0; 16].into(), Size::new(2, 2), 4, 1, 0, now).unwrap_err(),
            FrameError::InvalidStride
        );
        assert_eq!(
            Frame::new(vec![0; 4].into(), Size::new(2, 2), 8, 1, 0, now).unwrap_err(),
            FrameError::InvalidLength
        );
    }

    #[test]
    fn aspect_fit_letterboxes_and_blends() {
        let now = Instant::now();
        let source = Frame::placeholder(Size::new(4, 2), 0xff20_4080, 1, 0, now);
        let fitted = source.aspect_fit(Size::new(4, 4), now);
        assert_eq!(&fitted.pixels()[..4], &[0, 0, 0, 0xff]);
        assert_eq!(&fitted.pixels()[16..20], &[0x80, 0x40, 0x20, 0xff]);
        let black = Frame::placeholder(Size::new(2, 2), 0xff00_0000, 1, 0, now);
        let white = Frame::placeholder(Size::new(2, 2), 0xffff_ffff, 2, 0, now);
        let blended = Frame::blend(
            Some(&black),
            Some(&white),
            0.5,
            0,
            Size::new(2, 2),
            FrameMetadata {
                sequence: 3,
                timestamp_100ns: 0,
                received_at: now,
            },
        );
        assert_eq!(&blended.pixels()[..3], &[128, 128, 128]);
    }
}
