use crate::Size;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrayImage {
    pub size: Size,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageError {
    InvalidLayout,
    InvalidDimensions,
}

impl GrayImage {
    pub fn new(size: Size, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let expected = usize::try_from(size.width)
            .ok()
            .and_then(|width| width.checked_mul(size.height as usize))
            .ok_or(ImageError::InvalidDimensions)?;
        if size.width == 0 || size.height == 0 || pixels.len() != expected {
            return Err(ImageError::InvalidDimensions);
        }
        Ok(Self { size, pixels })
    }

    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[y as usize * self.size.width as usize + x as usize]
    }
}

pub fn bgra_to_gray(bgra: &[u8], size: Size, row_pitch: usize) -> Result<GrayImage, ImageError> {
    let row_bytes = size
        .width
        .checked_mul(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(ImageError::InvalidLayout)?;
    let required = row_pitch
        .checked_mul(size.height as usize)
        .ok_or(ImageError::InvalidLayout)?;
    if size.width == 0 || size.height == 0 || row_pitch < row_bytes || bgra.len() < required {
        return Err(ImageError::InvalidLayout);
    }
    let mut pixels = vec![0; size.width as usize * size.height as usize];
    for y in 0..size.height as usize {
        for x in 0..size.width as usize {
            let offset = y * row_pitch + x * 4;
            let b = f64::from(bgra[offset]);
            let g = f64::from(bgra[offset + 1]);
            let r = f64::from(bgra[offset + 2]);
            pixels[y * size.width as usize + x] = (0.0722 * b + 0.7152 * g + 0.2126 * r)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    GrayImage::new(size, pixels)
}

pub fn resize_bgra_to_gray(
    bgra: &[u8],
    size: Size,
    row_pitch: usize,
    target: Size,
) -> Result<GrayImage, ImageError> {
    let row_bytes = size
        .width
        .checked_mul(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(ImageError::InvalidLayout)?;
    let required = row_pitch
        .checked_mul(size.height as usize)
        .ok_or(ImageError::InvalidLayout)?;
    if size.width == 0
        || size.height == 0
        || target.width == 0
        || target.height == 0
        || row_pitch < row_bytes
        || bgra.len() < required
    {
        return Err(ImageError::InvalidLayout);
    }

    let gray_at = |x: u32, y: u32| {
        let offset = y as usize * row_pitch + x as usize * 4;
        let b = f64::from(bgra[offset]);
        let g = f64::from(bgra[offset + 1]);
        let r = f64::from(bgra[offset + 2]);
        (0.0722 * b + 0.7152 * g + 0.2126 * r)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let mut output = vec![0; target.width as usize * target.height as usize];
    let scale_x = size.width as f64 / target.width as f64;
    let scale_y = size.height as f64 / target.height as f64;
    for y in 0..target.height {
        let source_y = (f64::from(y) + 0.5) * scale_y - 0.5;
        let y0 = source_y.floor().clamp(0.0, f64::from(size.height - 1)) as u32;
        let y1 = (y0 + 1).min(size.height - 1);
        let fraction_y = (source_y - source_y.floor()).clamp(0.0, 1.0);
        for x in 0..target.width {
            let source_x = (f64::from(x) + 0.5) * scale_x - 0.5;
            let x0 = source_x.floor().clamp(0.0, f64::from(size.width - 1)) as u32;
            let x1 = (x0 + 1).min(size.width - 1);
            let fraction_x = (source_x - source_x.floor()).clamp(0.0, 1.0);
            let top = f64::from(gray_at(x0, y0)) * (1.0 - fraction_x)
                + f64::from(gray_at(x1, y0)) * fraction_x;
            let bottom = f64::from(gray_at(x0, y1)) * (1.0 - fraction_x)
                + f64::from(gray_at(x1, y1)) * fraction_x;
            output[y as usize * target.width as usize + x as usize] =
                (top * (1.0 - fraction_y) + bottom * fraction_y)
                    .round()
                    .clamp(0.0, 255.0) as u8;
        }
    }
    GrayImage::new(target, output)
}

pub fn resize_bilinear(source: &GrayImage, target: Size) -> Result<GrayImage, ImageError> {
    if target.width == 0 || target.height == 0 {
        return Err(ImageError::InvalidDimensions);
    }
    let mut output = vec![0; target.width as usize * target.height as usize];
    let scale_x = source.size.width as f64 / target.width as f64;
    let scale_y = source.size.height as f64 / target.height as f64;
    for y in 0..target.height {
        let source_y = (f64::from(y) + 0.5) * scale_y - 0.5;
        let y0 = source_y
            .floor()
            .clamp(0.0, f64::from(source.size.height - 1)) as u32;
        let y1 = (y0 + 1).min(source.size.height - 1);
        let fraction_y = (source_y - source_y.floor()).clamp(0.0, 1.0);
        for x in 0..target.width {
            let source_x = (f64::from(x) + 0.5) * scale_x - 0.5;
            let x0 = source_x
                .floor()
                .clamp(0.0, f64::from(source.size.width - 1)) as u32;
            let x1 = (x0 + 1).min(source.size.width - 1);
            let fraction_x = (source_x - source_x.floor()).clamp(0.0, 1.0);
            let top = f64::from(source.at(x0, y0)) * (1.0 - fraction_x)
                + f64::from(source.at(x1, y0)) * fraction_x;
            let bottom = f64::from(source.at(x0, y1)) * (1.0 - fraction_x)
                + f64::from(source.at(x1, y1)) * fraction_x;
            output[y as usize * target.width as usize + x as usize] =
                (top * (1.0 - fraction_y) + bottom * fraction_y)
                    .round()
                    .clamp(0.0, 255.0) as u8;
        }
    }
    GrayImage::new(target, output)
}

pub fn image_similarity(reference: &GrayImage, candidate: &GrayImage) -> f64 {
    if reference.size != candidate.size || reference.pixels.len() != candidate.pixels.len() {
        return 0.0;
    }
    let border_x = if reference.size.width >= 80 {
        reference.size.width / 80
    } else {
        0
    };
    let border_y = if reference.size.height >= 45 {
        reference.size.height / 45
    } else {
        0
    };
    let x_end = reference.size.width - border_x;
    let y_end = reference.size.height - border_y;
    let mut sum_reference = 0.0;
    let mut sum_candidate = 0.0;
    let mut absolute_error = 0.0;
    let mut count = 0_u64;
    for y in border_y..y_end {
        for x in border_x..x_end {
            let left = f64::from(reference.at(x, y));
            let right = f64::from(candidate.at(x, y));
            sum_reference += left;
            sum_candidate += right;
            absolute_error += (left - right).abs();
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let divisor = count as f64;
    let mean_reference = sum_reference / divisor;
    let mean_candidate = sum_candidate / divisor;
    let mut variance_reference = 0.0;
    let mut variance_candidate = 0.0;
    let mut covariance = 0.0;
    for y in border_y..y_end {
        for x in border_x..x_end {
            let left = f64::from(reference.at(x, y)) - mean_reference;
            let right = f64::from(candidate.at(x, y)) - mean_candidate;
            variance_reference += left * left;
            variance_candidate += right * right;
            covariance += left * right;
        }
    }
    variance_reference /= divisor;
    variance_candidate /= divisor;
    covariance /= divisor;
    const C1: f64 = 6.5025;
    const C2: f64 = 58.5225;
    let ssim = ((2.0 * mean_reference * mean_candidate + C1) * (2.0 * covariance + C2))
        / ((mean_reference * mean_reference + mean_candidate * mean_candidate + C1)
            * (variance_reference + variance_candidate + C2));
    let pixel_score = 1.0 - absolute_error / (divisor * 255.0);
    (0.8 * ssim.clamp(0.0, 1.0) + 0.2 * pixel_score).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_converts_resizes_and_compares() {
        let bgra = [0, 0, 255, 255, 255, 255, 255, 255];
        let image = bgra_to_gray(&bgra, Size::new(2, 1), 8).unwrap();
        assert_eq!(image.pixels, [54, 255]);
        let resized = resize_bilinear(&image, Size::new(4, 2)).unwrap();
        let direct = resize_bgra_to_gray(&bgra, Size::new(2, 1), 8, Size::new(4, 2)).unwrap();
        assert_eq!(resized.size, Size::new(4, 2));
        assert_eq!(direct, resized);
        assert!((image_similarity(&image, &image) - 1.0).abs() < f64::EPSILON);
    }
}
