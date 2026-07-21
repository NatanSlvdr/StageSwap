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

#[derive(Clone, Debug)]
struct ScalePlan {
    source: Size,
    source_stride: u32,
    x_offset: u32,
    y_offset: u32,
    source_x: Vec<usize>,
    source_rows: Vec<usize>,
}

impl ScalePlan {
    fn new(source: &Frame, output: Size) -> Self {
        let scale = f64::min(
            output.width as f64 / source.size.width as f64,
            output.height as f64 / source.size.height as f64,
        );
        let width = (source.size.width as f64 * scale).round().max(1.0) as u32;
        let height = (source.size.height as f64 * scale).round().max(1.0) as u32;
        let source_x = (0..width)
            .map(|x| {
                ((u64::from(x) * u64::from(source.size.width)) / u64::from(width)) as usize * 4
            })
            .collect();
        let source_rows = (0..height)
            .map(|y| {
                ((u64::from(y) * u64::from(source.size.height)) / u64::from(height)) as usize
                    * source.stride as usize
            })
            .collect();
        Self {
            source: source.size,
            source_stride: source.stride,
            x_offset: (output.width - width) / 2,
            y_offset: (output.height - height) / 2,
            source_x,
            source_rows,
        }
    }

    fn matches(&self, source: &Frame) -> bool {
        self.source == source.size && self.source_stride == source.stride
    }
}

#[derive(Clone, Debug, Default)]
struct FittedCache {
    source: Option<Arc<Frame>>,
    pixels: Option<Arc<[u8]>>,
    plan: Option<ScalePlan>,
}

impl FittedCache {
    fn fit(&mut self, source: &Arc<Frame>, output: Size) -> Arc<[u8]> {
        if source.size == output && source.stride == output.width * 4 {
            return source.pixels_arc();
        }
        if self
            .source
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, source))
            && let Some(pixels) = &self.pixels
        {
            return Arc::clone(pixels);
        }
        if self.plan.as_ref().is_none_or(|plan| !plan.matches(source)) {
            self.plan = Some(ScalePlan::new(source, output));
        }
        let plan = self.plan.as_ref().expect("scale plan was initialized");
        let mut pixels = vec![0; output.width as usize * output.height as usize * 4];
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 0xff;
        }
        for (y, source_row) in plan.source_rows.iter().copied().enumerate() {
            let destination_row = (y + plan.y_offset as usize) * output.width as usize * 4;
            for (x, source_x) in plan.source_x.iter().copied().enumerate() {
                let source_offset = source_row + source_x;
                let destination = destination_row + (x + plan.x_offset as usize) * 4;
                pixels[destination..destination + 4]
                    .copy_from_slice(&source.pixels[source_offset..source_offset + 4]);
            }
        }
        let pixels: Arc<[u8]> = pixels.into();
        self.source = Some(Arc::clone(source));
        self.pixels = Some(Arc::clone(&pixels));
        pixels
    }
}

#[derive(Clone, Debug)]
pub struct FrameCompositor {
    output: Size,
    camera: FittedCache,
    screen: FittedCache,
    placeholder_color: Option<u32>,
    placeholder_pixels: Arc<[u8]>,
}

impl FrameCompositor {
    pub fn new(output: Size) -> Self {
        assert!(
            output.width > 0 && output.height > 0,
            "output size must be non-empty"
        );
        Self {
            output,
            camera: FittedCache::default(),
            screen: FittedCache::default(),
            placeholder_color: None,
            placeholder_pixels: Arc::from([]),
        }
    }

    fn placeholder(&mut self, color_bgra: u32) -> Arc<[u8]> {
        if self.placeholder_color != Some(color_bgra) {
            let mut pixels = vec![0; self.output.width as usize * self.output.height as usize * 4];
            let color = color_bgra.to_le_bytes();
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.copy_from_slice(&color);
            }
            self.placeholder_pixels = pixels.into();
            self.placeholder_color = Some(color_bgra);
        }
        Arc::clone(&self.placeholder_pixels)
    }

    pub fn compose(
        &mut self,
        camera: Option<&Arc<Frame>>,
        screen: Option<&Arc<Frame>>,
        screen_mix: f64,
        color_bgra: u32,
        metadata: FrameMetadata,
    ) -> Frame {
        let mix = screen_mix.clamp(0.0, 1.0);
        let pixels = if mix <= 0.0 {
            camera
                .map(|frame| self.camera.fit(frame, self.output))
                .unwrap_or_else(|| self.placeholder(color_bgra))
        } else if mix >= 1.0 {
            screen
                .map(|frame| self.screen.fit(frame, self.output))
                .unwrap_or_else(|| self.placeholder(color_bgra))
        } else {
            let camera = camera
                .map(|frame| self.camera.fit(frame, self.output))
                .unwrap_or_else(|| self.placeholder(color_bgra));
            let screen = screen
                .map(|frame| self.screen.fit(frame, self.output))
                .unwrap_or_else(|| self.placeholder(color_bgra));
            let weight = (mix * 256.0).round() as u16;
            let inverse = 256 - weight;
            let mut pixels = vec![0; camera.len()];
            for ((output, left), right) in pixels.iter_mut().zip(camera.iter()).zip(screen.iter()) {
                *output =
                    ((u16::from(*left) * inverse + u16::from(*right) * weight + 128) >> 8) as u8;
            }
            pixels.into()
        };
        Frame::new(
            pixels,
            self.output,
            self.output.width * 4,
            metadata.sequence,
            metadata.timestamp_100ns,
            metadata.received_at,
        )
        .expect("compositor output is valid")
    }
}

impl Default for FrameCompositor {
    fn default() -> Self {
        Self::new(PIPELINE_SIZE)
    }
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

    #[test]
    fn compositor_reuses_full_size_sources_and_skips_unselected_input() {
        let now = Instant::now();
        let camera = Arc::new(Frame::placeholder(Size::new(2, 2), 0xff20_4080, 1, 0, now));
        let screen = Arc::new(Frame::placeholder(Size::new(4, 4), 0xffff_ffff, 2, 0, now));
        let camera_pixels = camera.pixels_arc();
        let mut compositor = FrameCompositor::new(Size::new(2, 2));
        let output = compositor.compose(
            Some(&camera),
            Some(&screen),
            0.0,
            0,
            FrameMetadata {
                sequence: 3,
                timestamp_100ns: 10,
                received_at: now,
            },
        );
        assert!(Arc::ptr_eq(&output.pixels_arc(), &camera_pixels));
        assert_eq!(output.sequence, 3);
    }

    #[test]
    fn compositor_caches_letterboxing_and_blends_with_integer_weights() {
        let now = Instant::now();
        let camera = Arc::new(Frame::placeholder(Size::new(4, 2), 0xff00_0000, 1, 0, now));
        let screen = Arc::new(Frame::placeholder(Size::new(4, 4), 0xffff_ffff, 2, 0, now));
        let mut compositor = FrameCompositor::new(Size::new(4, 4));
        let metadata = FrameMetadata {
            sequence: 3,
            timestamp_100ns: 0,
            received_at: now,
        };
        let first = compositor.compose(Some(&camera), None, 0.0, 0, metadata);
        let second = compositor.compose(Some(&camera), None, 0.0, 0, metadata);
        assert!(Arc::ptr_eq(&first.pixels_arc(), &second.pixels_arc()));
        assert_eq!(&first.pixels()[..4], &[0, 0, 0, 0xff]);
        let blended = compositor.compose(Some(&camera), Some(&screen), 0.5, 0, metadata);
        assert_eq!(&blended.pixels()[16..19], &[128, 128, 128]);

        let replacement = Arc::new(Frame::placeholder(
            Size::new(4, 2),
            0xffff_ffff,
            camera.sequence,
            0,
            now,
        ));
        let replaced = compositor.compose(Some(&replacement), None, 0.0, 0, metadata);
        assert_eq!(&replaced.pixels()[16..20], &[255, 255, 255, 255]);

        let placeholder = compositor.compose(None, Some(&screen), 0.0, 0xff03_0201, metadata);
        assert_eq!(&placeholder.pixels()[..4], &[1, 2, 3, 255]);
    }
}
