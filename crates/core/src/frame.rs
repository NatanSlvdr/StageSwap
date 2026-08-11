use crate::Size;
use image::imageops::FilterType;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const PIPELINE_SIZE: Size = Size::new(1280, 720);
pub const PIPELINE_FPS: u32 = 30;
pub const FRAME_STALE_AFTER: Duration = Duration::from_secs(1);
pub const CAPTURE_FRAME_POOL_CAPACITY: usize = 4;

#[derive(Debug)]
pub struct FrameBufferPool {
    frame_bytes: usize,
    capacity: usize,
    slots: Vec<Arc<[u8]>>,
    exhaustion_count: u64,
}

impl FrameBufferPool {
    pub fn new(frame_bytes: usize, capacity: usize) -> Self {
        assert!(frame_bytes > 0, "pooled frames must not be empty");
        assert!(capacity > 0, "frame pool capacity must not be zero");
        Self {
            frame_bytes,
            capacity,
            slots: Vec::with_capacity(capacity),
            exhaustion_count: 0,
        }
    }

    pub fn try_write<E>(
        &mut self,
        write: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<Option<Arc<[u8]>>, E> {
        self.try_write_sized(self.frame_bytes, write)
    }

    pub fn try_write_sized<E>(
        &mut self,
        frame_bytes: usize,
        write: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<Option<Arc<[u8]>>, E> {
        assert!(frame_bytes > 0, "pooled frames must not be empty");
        let Some(index) = self.acquire_slot(frame_bytes) else {
            return Ok(None);
        };
        let slot = &mut self.slots[index];
        let destination = Arc::get_mut(slot).expect("available pool slot is uniquely owned");
        write(destination)?;
        Ok(Some(Arc::clone(slot)))
    }

    /// Writes into a reusable slot when possible and falls back to a temporary
    /// allocation when every bounded slot is retained by a consumer. This keeps
    /// the normal path allocation-free without turning pool pressure into a
    /// dropped frame.
    pub fn write_with_fallback<E>(
        &mut self,
        write: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<Arc<[u8]>, E> {
        self.write_with_fallback_sized(self.frame_bytes, write)
    }

    pub fn write_with_fallback_sized<E>(
        &mut self,
        frame_bytes: usize,
        write: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<Arc<[u8]>, E> {
        assert!(frame_bytes > 0, "pooled frames must not be empty");
        if let Some(index) = self.acquire_slot(frame_bytes) {
            let slot = &mut self.slots[index];
            let destination = Arc::get_mut(slot).expect("available pool slot is uniquely owned");
            write(destination)?;
            return Ok(Arc::clone(slot));
        }
        let mut pixels = vec![0; frame_bytes];
        write(&mut pixels)?;
        Ok(pixels.into())
    }

    fn acquire_slot(&mut self, frame_bytes: usize) -> Option<usize> {
        let available = self
            .slots
            .iter()
            .position(|slot| Arc::strong_count(slot) == 1 && slot.len() == frame_bytes);
        let index = match available {
            Some(index) => index,
            None if self.slots.len() < self.capacity => {
                self.slots.push(vec![0; frame_bytes].into());
                self.slots.len() - 1
            }
            None if self.slots.iter().any(|slot| Arc::strong_count(slot) == 1) => {
                let index = self
                    .slots
                    .iter()
                    .position(|slot| Arc::strong_count(slot) == 1)
                    .expect("a uniquely owned slot was found");
                self.slots[index] = vec![0; frame_bytes].into();
                index
            }
            None => {
                self.exhaustion_count = self.exhaustion_count.saturating_add(1);
                return None;
            }
        };
        Some(index)
    }

    pub fn allocated_slots(&self) -> usize {
        self.slots.len()
    }

    pub fn exhaustion_count(&self) -> u64 {
        self.exhaustion_count
    }
}

pub fn aspect_fit_bgra_into(
    source: &[u8],
    source_size: Size,
    source_stride: u32,
    destination: &mut [u8],
    output: Size,
) -> Result<(), FrameError> {
    if source_size.width == 0 || source_size.height == 0 || output.width == 0 || output.height == 0
    {
        return Err(FrameError::Empty);
    }
    let source_row_bytes = source_size
        .width
        .checked_mul(4)
        .ok_or(FrameError::TooLarge)?;
    if source_stride < source_row_bytes {
        return Err(FrameError::InvalidStride);
    }
    let source_bytes = usize::try_from(source_stride)
        .ok()
        .and_then(|stride| stride.checked_mul(source_size.height as usize))
        .ok_or(FrameError::TooLarge)?;
    let output_stride = output.width.checked_mul(4).ok_or(FrameError::TooLarge)?;
    let output_bytes = usize::try_from(output_stride)
        .ok()
        .and_then(|stride| stride.checked_mul(output.height as usize))
        .ok_or(FrameError::TooLarge)?;
    if source.len() < source_bytes || destination.len() != output_bytes {
        return Err(FrameError::InvalidLength);
    }
    if source_size == output && source_stride == output_stride {
        destination.copy_from_slice(&source[..output_bytes]);
        return Ok(());
    }

    destination.fill(0);
    for alpha in destination.iter_mut().skip(3).step_by(4) {
        *alpha = 0xff;
    }
    let scale = f64::min(
        output.width as f64 / source_size.width as f64,
        output.height as f64 / source_size.height as f64,
    );
    let width = (source_size.width as f64 * scale).round().max(1.0) as u32;
    let height = (source_size.height as f64 * scale).round().max(1.0) as u32;
    let x_offset = (output.width - width) / 2;
    let y_offset = (output.height - height) / 2;
    let source_x_step = u64::from(source_size.width / width);
    let source_x_increment = u64::from(source_size.width % width);
    let source_y_step = u64::from(source_size.height / height);
    let source_y_increment = u64::from(source_size.height % height);
    let width = u64::from(width);
    let height = u64::from(height);
    let mut source_y = 0_u64;
    let mut source_y_remainder = 0_u64;
    for y in 0..height {
        let source_row = source_y as usize * source_stride as usize;
        let destination_row = (y as usize + y_offset as usize) * output_stride as usize;
        let mut source_x = 0_u64;
        let mut source_x_remainder = 0_u64;
        for x in 0..width {
            let source_offset = source_row + source_x as usize * 4;
            let destination_offset = destination_row + (x as usize + x_offset as usize) * 4;
            destination[destination_offset..destination_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
            source_x += source_x_step;
            source_x_remainder += source_x_increment;
            if source_x_remainder >= width {
                source_x += 1;
                source_x_remainder -= width;
            }
        }
        source_y += source_y_step;
        source_y_remainder += source_y_increment;
        if source_y_remainder >= height {
            source_y += 1;
            source_y_remainder -= height;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct FrameMetadata {
    pub sequence: u64,
    pub timestamp_100ns: i64,
    pub received_at: Instant,
}

/// Returns the canonical application-off image shared by the dashboard and
/// the out-of-process Media Foundation source.
pub fn off_frame(metadata: FrameMetadata) -> Frame {
    Frame::new(
        off_frame_pixels(),
        PIPELINE_SIZE,
        PIPELINE_SIZE.width * 4,
        metadata.sequence,
        metadata.timestamp_100ns,
        metadata.received_at,
    )
    .expect("off frame dimensions are valid")
}

pub fn off_frame_pixels() -> Arc<[u8]> {
    static PIXELS: OnceLock<Arc<[u8]>> = OnceLock::new();
    Arc::clone(PIXELS.get_or_init(render_off_frame))
}

fn render_off_frame() -> Arc<[u8]> {
    const ICON_SIZE: u32 = 256;
    let icon = image::load_from_memory(include_bytes!("../../app/assets/app-icon.png"))
        .expect("embedded StageSwap app icon is a valid image")
        .resize_exact(ICON_SIZE, ICON_SIZE, FilterType::Lanczos3)
        .to_rgba8();
    let mut frame = vec![0; PIPELINE_SIZE.width as usize * PIPELINE_SIZE.height as usize * 4];
    for alpha in frame.iter_mut().skip(3).step_by(4) {
        *alpha = 0xff;
    }
    let x_offset = (PIPELINE_SIZE.width - ICON_SIZE) / 2;
    let y_offset = (PIPELINE_SIZE.height - ICON_SIZE) / 2;
    for (x, y, pixel) in icon.enumerate_pixels() {
        let destination = (((y + y_offset) * PIPELINE_SIZE.width + x + x_offset) * 4) as usize;
        let alpha = u16::from(pixel[3]);
        frame[destination] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
        frame[destination + 1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        frame[destination + 2] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
    }
    frame.into()
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

#[derive(Debug)]
pub struct FrameCompositor {
    output: Size,
    camera: FittedCache,
    screen: FittedCache,
    placeholder_color: Option<u32>,
    placeholder_pixels: Arc<[u8]>,
    blend_pool: FrameBufferPool,
}

impl Clone for FrameCompositor {
    fn clone(&self) -> Self {
        Self {
            output: self.output,
            camera: self.camera.clone(),
            screen: self.screen.clone(),
            placeholder_color: self.placeholder_color,
            placeholder_pixels: Arc::clone(&self.placeholder_pixels),
            blend_pool: FrameBufferPool::new(
                self.output.width as usize * self.output.height as usize * 4,
                CAPTURE_FRAME_POOL_CAPACITY,
            ),
        }
    }
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
            blend_pool: FrameBufferPool::new(
                output.width as usize * output.height as usize * 4,
                CAPTURE_FRAME_POOL_CAPACITY,
            ),
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
            self.blend_pool
                .write_with_fallback(|pixels| {
                    for ((output, left), right) in
                        pixels.iter_mut().zip(camera.iter()).zip(screen.iter())
                    {
                        *output = ((u16::from(*left) * inverse + u16::from(*right) * weight + 128)
                            >> 8) as u8;
                    }
                    Ok::<(), Infallible>(())
                })
                .expect("an infallible blend cannot fail")
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
        aspect_fit_bgra_into(&self.pixels, self.size, self.stride, &mut pixels, output)
            .expect("validated frame and output dimensions are valid");
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
    fn contract_off_frame_is_deterministic_black_with_a_centered_stageswap_icon() {
        let now = Instant::now();
        let first = off_frame(FrameMetadata {
            sequence: 7,
            timestamp_100ns: 11,
            received_at: now,
        });
        let second = off_frame(FrameMetadata {
            sequence: 8,
            timestamp_100ns: 12,
            received_at: now,
        });

        assert_eq!(first.size, PIPELINE_SIZE);
        assert_eq!(first.stride, PIPELINE_SIZE.width * 4);
        assert!(Arc::ptr_eq(&first.pixels_arc(), &second.pixels_arc()));
        let pixel = |x: usize, y: usize| {
            let offset = (y * PIPELINE_SIZE.width as usize + x) * 4;
            &first.pixels()[offset..offset + 4]
        };
        for corner in [pixel(0, 0), pixel(1279, 0), pixel(0, 719), pixel(1279, 719)] {
            assert!(corner[..3].iter().all(|channel| *channel <= 8));
            assert_eq!(corner[3], 0xff);
        }
        const ICON_LEFT: usize = (1280 - 256) / 2;
        const ICON_TOP: usize = (720 - 256) / 2;
        let mut branded_pixels = 0;
        for y in 0..720 {
            for x in 0..1280 {
                let pixel = pixel(x, y);
                let inside_icon = (ICON_LEFT..ICON_LEFT + 256).contains(&x)
                    && (ICON_TOP..ICON_TOP + 256).contains(&y);
                if pixel[..3].iter().any(|channel| *channel > 8) {
                    assert!(
                        inside_icon,
                        "branded pixel escaped the centered icon at ({x}, {y})"
                    );
                    branded_pixels += 1;
                }
            }
        }
        assert!(
            branded_pixels > 30_000,
            "centered app icon has only {branded_pixels} visible pixels"
        );
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 0xff));
    }

    #[test]
    fn contract_rejects_invalid_frames() {
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
    fn contract_aspect_fit_letterboxes_and_blends() {
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
    fn contract_compositor_reuses_full_size_sources_and_skips_unselected_input() {
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
    fn contract_compositor_caches_letterboxing_and_blends_with_integer_weights() {
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

    #[test]
    fn contract_aspect_fit_sizes_and_incremental_mapping_are_stable() {
        for source_size in [
            Size::new(1280, 720),
            Size::new(1920, 1080),
            Size::new(3840, 2160),
            Size::new(1920, 1200),
            Size::new(3440, 1440),
        ] {
            let source_stride = source_size.width * 4;
            let source = vec![0x7b; source_stride as usize * source_size.height as usize];
            let mut output =
                vec![0xcc; PIPELINE_SIZE.width as usize * PIPELINE_SIZE.height as usize * 4];
            aspect_fit_bgra_into(
                &source,
                source_size,
                source_stride,
                &mut output,
                PIPELINE_SIZE,
            )
            .unwrap();
            assert_eq!(output.len(), 1280 * 720 * 4);
            assert!(
                output
                    .chunks_exact(4)
                    .all(|pixel| pixel[3] == 0x7b || pixel[3] == 0xff)
            );
            let pixel = |x: usize, y: usize| {
                let offset = (y * PIPELINE_SIZE.width as usize + x) * 4;
                &output[offset..offset + 4]
            };
            assert_eq!(pixel(640, 360), &[0x7b; 4]);
            if source_size == Size::new(1920, 1200) || source_size == Size::new(3440, 1440) {
                assert_eq!(pixel(0, 0), &[0, 0, 0, 0xff]);
            } else {
                assert_eq!(pixel(0, 0), &[0x7b; 4]);
            }
            if source_size == PIPELINE_SIZE {
                assert_eq!(output, source);
            }
        }

        let horizontal = (0_u8..5)
            .flat_map(|value| [value, value, value, 0xff])
            .collect::<Vec<_>>();
        let mut output = vec![0; 7 * 2 * 4];
        aspect_fit_bgra_into(
            &horizontal,
            Size::new(5, 1),
            20,
            &mut output,
            Size::new(7, 2),
        )
        .unwrap();
        let mapped = output[..7 * 4]
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect::<Vec<_>>();
        assert_eq!(mapped, [0, 0, 1, 2, 2, 3, 4]);
        assert!(
            output[7 * 4..]
                .chunks_exact(4)
                .all(|pixel| pixel == [0, 0, 0, 0xff])
        );

        let vertical = (0_u8..5)
            .flat_map(|value| [value, value, value, 0xff])
            .collect::<Vec<_>>();
        let mut output = vec![0; 2 * 7 * 4];
        aspect_fit_bgra_into(&vertical, Size::new(1, 5), 4, &mut output, Size::new(2, 7)).unwrap();
        let mapped = (0..7).map(|y| output[y * 2 * 4]).collect::<Vec<_>>();
        assert_eq!(mapped, [0, 0, 1, 2, 2, 3, 4]);
        assert!((0..7).all(|y| output[(y * 2 + 1) * 4..(y * 2 + 2) * 4] == [0, 0, 0, 0xff]));
    }

    #[test]
    fn contract_frame_buffer_pool_bounds_reuse_and_fallback() {
        let mut pool = FrameBufferPool::new(16, 2);
        let first = pool
            .try_write(|pixels| {
                pixels.fill(1);
                Ok::<(), ()>(())
            })
            .unwrap()
            .unwrap();
        let second = pool
            .try_write(|pixels| {
                pixels.fill(2);
                Ok::<(), ()>(())
            })
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(pool.allocated_slots(), 2);
        assert_eq!(
            pool.try_write(|_| Ok::<(), ()>(())),
            Ok(None),
            "both published slots are still owned"
        );
        assert_eq!(pool.exhaustion_count(), 1);
        assert!(first.iter().all(|byte| *byte == 1));
        drop(first);
        let reused = pool
            .try_write(|pixels| {
                pixels.fill(3);
                Ok::<(), ()>(())
            })
            .unwrap()
            .unwrap();
        assert!(reused.iter().all(|byte| *byte == 3));
        assert!(second.iter().all(|byte| *byte == 2));
        assert_eq!(pool.allocated_slots(), 2);
        {
            let mut pool = FrameBufferPool::new(8, 2);
            let first = pool
                .write_with_fallback(|pixels| {
                    pixels.fill(1);
                    Ok::<(), ()>(())
                })
                .unwrap();
            let second = pool
                .write_with_fallback(|pixels| {
                    pixels.fill(2);
                    Ok::<(), ()>(())
                })
                .unwrap();
            let fallback = pool
                .write_with_fallback(|pixels| {
                    pixels.fill(3);
                    Ok::<(), ()>(())
                })
                .unwrap();

            assert_eq!(pool.allocated_slots(), 2);
            assert_eq!(pool.exhaustion_count(), 1);
            assert!(fallback.iter().all(|byte| *byte == 3));
            assert!(!Arc::ptr_eq(&fallback, &first));
            assert!(!Arc::ptr_eq(&fallback, &second));

            let first_pointer = Arc::as_ptr(&first);
            drop(first);
            let reused = pool
                .write_with_fallback(|pixels| {
                    pixels.fill(4);
                    Ok::<(), ()>(())
                })
                .unwrap();
            assert_eq!(Arc::as_ptr(&reused), first_pointer);
            assert!(reused.iter().all(|byte| *byte == 4));
            assert!(second.iter().all(|byte| *byte == 2));
            assert!(fallback.iter().all(|byte| *byte == 3));
        }
    }
}
