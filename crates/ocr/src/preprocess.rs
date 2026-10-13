use image::{DynamicImage, GrayImage, ImageBuffer, ImageError, ImageReader, Limits, Luma};
use std::io::Cursor;
use std::path::Path;
use thiserror::Error;

/// Decode limits: a small compressed file can declare enormous dimensions and
/// exhaust memory when decoded (a decompression bomb). 100 MP is far beyond
/// any phone camera or scanner page.
const MAX_PIXELS: u64 = 100_000_000;
const MAX_DIMENSION: u32 = 20_000;
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PreprocessError {
    #[error("Failed to load image: {0}")]
    Load(#[from] image::ImageError),
    #[error("Image is too large ({width}x{height} pixels, max {MAX_PIXELS})")]
    TooLarge { width: u32, height: u32 },
    #[error("Failed to encode processed image: {0}")]
    Encode(String),
}

/// Load an image file, apply normalization, and return PNG bytes ready for OCR.
pub fn prepare_for_ocr(path: &Path) -> Result<Vec<u8>, PreprocessError> {
    let data = std::fs::read(path).map_err(ImageError::IoError)?;
    prepare_for_ocr_from_bytes(&data)
}

/// Process raw image bytes (JPEG / PNG / WEBP / …) and return normalized PNG bytes.
pub fn prepare_for_ocr_from_bytes(data: &[u8]) -> Result<Vec<u8>, PreprocessError> {
    encode_as_png(normalize(decode_limited(data)?))
}

/// Decode an image whose format is detected from its content, enforcing the
/// size limits above. Only the header is parsed before the pixel-count check.
fn decode_limited(data: &[u8]) -> Result<DynamicImage, PreprocessError> {
    let reader = || {
        ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(ImageError::IoError)
    };

    let (width, height) = reader()?.into_dimensions()?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(PreprocessError::TooLarge { width, height });
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = reader()?;
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// Grayscale + contrast stretch.
fn normalize(img: DynamicImage) -> DynamicImage {
    // Down-scale if the image is very large (Tesseract works best at 300 DPI / ~2000 px).
    let img = if img.width() > 2800 || img.height() > 2800 {
        img.resize(2800, 2800, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let gray: GrayImage = img.to_luma8();

    // Compute min and max pixel values for contrast stretching.
    let (min_px, max_px) = gray
        .pixels()
        .fold((255u8, 0u8), |(mn, mx), p| (mn.min(p[0]), mx.max(p[0])));

    if max_px == min_px {
        // Uniform image — return grayscale as-is.
        return DynamicImage::ImageLuma8(gray);
    }

    let range = (max_px - min_px) as u32;
    let stretched: GrayImage = ImageBuffer::from_fn(gray.width(), gray.height(), |x, y| {
        let p = gray.get_pixel(x, y)[0];
        let v = ((p - min_px) as u32 * 255 / range) as u8;
        Luma([v])
    });

    DynamicImage::ImageLuma8(stretched)
}

fn encode_as_png(img: DynamicImage) -> Result<Vec<u8>, PreprocessError> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| PreprocessError::Encode(e.to_string()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, ImageBuffer, Luma};

    fn solid_gray(width: u32, height: u32, value: u8) -> DynamicImage {
        let img: GrayImage = ImageBuffer::from_fn(width, height, |_, _| Luma([value]));
        DynamicImage::ImageLuma8(img)
    }

    fn gradient_gray(width: u32, height: u32) -> DynamicImage {
        let img: GrayImage =
            ImageBuffer::from_fn(width, height, |x, _| Luma([(x * 255 / width) as u8]));
        DynamicImage::ImageLuma8(img)
    }

    #[test]
    fn normalize_uniform_image_returns_same() {
        // A completely uniform gray image shouldn't panic or crash.
        let img = solid_gray(10, 10, 128);
        let result = normalize(img);
        assert_eq!(result.width(), 10);
        assert_eq!(result.height(), 10);
    }

    #[test]
    fn normalize_gradient_stretches_to_full_range() {
        let img = gradient_gray(256, 1);
        let result = normalize(img);
        let gray = result.to_luma8();
        // After stretching a 0..255 gradient the extremes should be 0 and 255.
        let min = gray.pixels().map(|p| p[0]).min().unwrap();
        let max = gray.pixels().map(|p| p[0]).max().unwrap();
        assert_eq!(min, 0);
        assert_eq!(max, 255);
    }

    #[test]
    fn prepare_from_bytes_produces_png_header() {
        // Create a tiny PNG in memory and round-trip it.
        let img = solid_gray(4, 4, 100);
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let result = prepare_for_ocr_from_bytes(&png_bytes).unwrap();
        // PNG magic bytes: 0x89 0x50 0x4E 0x47
        assert_eq!(&result[..4], b"\x89PNG");
    }

    #[test]
    fn rejects_non_image_bytes() {
        let err = prepare_for_ocr_from_bytes(b"%PDF-1.7\n%binary").unwrap_err();
        assert!(matches!(err, PreprocessError::Load(_)), "{err}");
    }

    #[test]
    fn rejects_decompression_bomb_before_decoding() {
        // A 22-byte PGM header claiming 400 megapixels.
        let err = prepare_for_ocr_from_bytes(b"P5\n20000 20000\n255\n\0\0\0").unwrap_err();
        assert!(
            matches!(
                err,
                PreprocessError::TooLarge {
                    width: 20000,
                    height: 20000
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_oversized_dimension() {
        let err = prepare_for_ocr_from_bytes(b"P5\n30000 1\n255\n\0\0\0").unwrap_err();
        assert!(matches!(err, PreprocessError::Load(_)), "{err}");
    }

    #[test]
    fn large_image_is_resized() {
        // A 3000×3000 image should be scaled down.
        let img: GrayImage = ImageBuffer::from_fn(3000, 3000, |_, _| Luma([200u8]));
        let result = normalize(DynamicImage::ImageLuma8(img));
        assert!(result.width() <= 2800 && result.height() <= 2800);
    }
}
