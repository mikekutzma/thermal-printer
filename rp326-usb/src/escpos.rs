/// ESC/POS command builder for the Rongta RP326.

use image::DynamicImage;

/// Maximum printable dots across the RP326's 72mm print head.
const PRINT_WIDTH: u32 = 576;

/// Dithering algorithm used when converting a colour/grayscale image to 1-bit.
pub enum DitherMode {
    /// Simple threshold: pixels below 128 become black, the rest white.
    /// Fast — O(n) with no neighbour writes.
    Threshold,
    /// Floyd-Steinberg error diffusion. Slower but produces much smoother
    /// gradients and detail.
    FloydSteinberg,
}

pub struct Packet(Vec<u8>);

impl Packet {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// ESC @ — reset printer to defaults
    pub fn initialize(mut self) -> Self {
        self.0.extend_from_slice(&[0x1B, 0x40]);
        self
    }

    /// Raw ASCII text. `\n` advances one line.
    pub fn text(mut self, s: &str) -> Self {
        self.0.extend_from_slice(s.as_bytes());
        self
    }

    /// ESC d n — feed n lines
    pub fn feed(mut self, lines: u8) -> Self {
        self.0.extend_from_slice(&[0x1B, 0x64, lines]);
        self
    }

    /// GS V A 0 — partial cut
    pub fn cut(mut self) -> Self {
        self.0.extend_from_slice(&[0x1D, 0x56, 0x41, 0x00]);
        self
    }

    /// Print a raster image using GS v 0.
    ///
    /// The image is converted to grayscale, scaled down to fit the print width
    /// if wider, then dithered to 1-bit using the chosen algorithm.
    pub fn image(mut self, img: DynamicImage, dither: DitherMode) -> Self {
        // Convert to grayscale first to reduce data before the resize.
        let img = img.to_luma8();

        // Scale down to print width if needed; never upscale.
        let img = if img.width() > PRINT_WIDTH {
            let h = (img.height() as f32 * PRINT_WIDTH as f32 / img.width() as f32) as u32;
            image::imageops::resize(&img, PRINT_WIDTH, h, image::imageops::FilterType::Triangle)
        } else {
            img
        };

        let (w, h) = (img.width(), img.height());
        let mut pixels: Vec<f32> = img.pixels().map(|p| p.0[0] as f32).collect();

        match dither {
            DitherMode::Threshold => {
                for p in pixels.iter_mut() {
                    *p = if *p < 128.0 { 0.0 } else { 255.0 };
                }
            }
            DitherMode::FloydSteinberg => {
                for y in 0..h {
                    for x in 0..w {
                        let i = (y * w + x) as usize;
                        let old = pixels[i];
                        let new = if old < 128.0 { 0.0 } else { 255.0 };
                        pixels[i] = new;
                        let err = old - new;

                        if x + 1 < w {
                            pixels[i + 1] = (pixels[i + 1] + err * 7.0 / 16.0).clamp(0.0, 255.0);
                        }
                        if y + 1 < h {
                            if x > 0 {
                                let j = ((y + 1) * w + x - 1) as usize;
                                pixels[j] = (pixels[j] + err * 3.0 / 16.0).clamp(0.0, 255.0);
                            }
                            let j = ((y + 1) * w + x) as usize;
                            pixels[j] = (pixels[j] + err * 5.0 / 16.0).clamp(0.0, 255.0);
                            if x + 1 < w {
                                let j = ((y + 1) * w + x + 1) as usize;
                                pixels[j] = (pixels[j] + err * 1.0 / 16.0).clamp(0.0, 255.0);
                            }
                        }
                    }
                }
            }
        }

        // Pack pixels into bytes: 8 per byte, MSB = leftmost, 1 = black.
        let width_bytes = (w + 7) / 8;
        let mut raster: Vec<u8> = Vec::with_capacity((width_bytes * h) as usize);
        for y in 0..h {
            for bx in 0..width_bytes {
                let mut byte = 0u8;
                for bit in 0..8 {
                    let x = bx * 8 + bit;
                    if x < w && pixels[(y * w + x) as usize] < 128.0 {
                        byte |= 0x80 >> bit;
                    }
                }
                raster.push(byte);
            }
        }

        // GS v 0 — print raster bit image
        let xl = (width_bytes & 0xFF) as u8;
        let xh = (width_bytes >> 8) as u8;
        let yl = (h & 0xFF) as u8;
        let yh = ((h >> 8) & 0xFF) as u8;
        self.0.extend_from_slice(&[0x1D, 0x76, 0x30, 0x00, xl, xh, yl, yh]);
        self.0.extend_from_slice(&raster);
        self
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}
