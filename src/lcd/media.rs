//! User images and GIFs for the "Image / GIF" and "Animation" modes. Port of `loadImage`,
//! `loadLcdImages`, `decodePalettedFrames` and `common.ResizeImage`/`ResizeGifImage`.
//!
//! Files are read from `%APPDATA%\OpenLinkHub\lcd`. Only names that appear in that folder's
//! listing are ever opened, so a setting can't point outside it.

use super::render::{Canvas, Overlay, QUALITY_ANIMATION, QUALITY_DEFAULT};
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::AnimationDecoder;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

const EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "bmp", "webp", "gif"];
/// Refuse anything larger; upstream's upload limit was 5 MB.
const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
/// Upstream background for the Animation mode (`database/lcd/images/concentric.gif`).
static CONCENTRIC_GIF: &[u8] = include_bytes!("../../assets/lcd/concentric.gif");

fn concentric() -> Arc<[u8]> {
    static GIF: OnceLock<Arc<[u8]>> = OnceLock::new();
    Arc::clone(GIF.get_or_init(|| Arc::from(CONCENTRIC_GIF)))
}

/// One encoded frame and how long it stays on screen (0 = as fast as possible).
#[derive(Debug, Clone)]
pub struct LcdFrame {
    pub jpeg: Vec<u8>,
    pub delay_ms: u32,
}

pub fn dir() -> PathBuf {
    let dir = crate::config::path("lcd");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Unable to create {}: {e}", dir.display());
    }
    dir
}

fn supported(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

fn is_gif(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("gif"))
}

/// Image file names in the LCD folder, sorted case-insensitively.
pub fn list() -> Vec<String> {
    let dir = dir();
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| supported(n))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// The listed file called `name`, or (upstream style) whose name without extension is `name`.
pub fn resolve(name: &str) -> Option<(String, PathBuf)> {
    if name.is_empty() {
        return None;
    }
    let names = list();
    let found = names
        .iter()
        .find(|n| n.eq_ignore_ascii_case(name))
        .or_else(|| names.iter().find(|n| n.rsplit_once('.').is_some_and(|(stem, _)| stem.eq_ignore_ascii_case(name))))?;
    Some((found.clone(), dir().join(found)))
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: &PathBuf) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(format!("file is larger than {} MB", MAX_FILE_SIZE / 1024 / 1024));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

/// Decode an image file into the JPEG frames sent to the panel (Go: loadImage). Stills are
/// stretched to 480×480 with Catmull-Rom; GIF frames are composited and scaled bilinearly.
pub fn decode_image(bytes: &[u8]) -> Result<Vec<LcdFrame>, String> {
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    if reader.format() == Some(image::ImageFormat::Gif) {
        let mut frames = Vec::new();
        gif_frames(bytes, |canvas, delay_ms| {
            frames.push(LcdFrame { jpeg: canvas.jpeg(QUALITY_DEFAULT), delay_ms });
            true
        })?;
        return Ok(frames);
    }
    let img = reader.decode().map_err(|e| e.to_string())?;
    Ok(vec![LcdFrame { jpeg: Canvas::from_image(&img, FilterType::CatmullRom).jpeg(QUALITY_DEFAULT), delay_ms: 0 }])
}

/// Stream the frames of a GIF as 480×480 canvases. `f` returns false to stop early.
/// Returns the number of frames produced.
pub fn gif_frames(bytes: &[u8], mut f: impl FnMut(Canvas, u32) -> bool) -> Result<usize, String> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut n = 0;
    for frame in decoder.into_frames() {
        let frame = match frame {
            Ok(fr) => fr,
            Err(e) if n > 0 => {
                log::warn!("GIF truncated after {n} frames: {e}");
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        let (num, den) = frame.delay().numer_denom_ms();
        let delay_ms = num.checked_div(den).unwrap_or(0);
        let buffer = frame.into_buffer();
        let canvas = if buffer.dimensions() == (super::render::WIDTH, super::render::HEIGHT) {
            Canvas::from_rgba(buffer.as_raw())
        } else {
            let scaled = image::imageops::resize(&buffer, super::render::WIDTH, super::render::HEIGHT, FilterType::Triangle);
            Canvas::from_rgba(scaled.as_raw())
        };
        n += 1;
        if !f(canvas, delay_ms) {
            break;
        }
    }
    if n == 0 {
        return Err("GIF has no frames".into());
    }
    Ok(n)
}

/// Encoded frames of the selected image, decoded on first use and kept until the selection
/// or the file changes. Only one image is held at a time.
#[derive(Default)]
pub struct ImageCache {
    current: Option<(String, Option<SystemTime>, Arc<Vec<LcdFrame>>)>,
}

impl ImageCache {
    pub fn get(&mut self, name: &str) -> Result<Arc<Vec<LcdFrame>>, String> {
        let (file, path) = resolve(name).ok_or_else(|| format!("image \"{name}\" not found in {}", dir().display()))?;
        let stamp = modified(&path);
        if let Some((cached, cached_stamp, frames)) = &self.current {
            if *cached == file && *cached_stamp == stamp {
                return Ok(Arc::clone(frames));
            }
        }
        self.current = None;
        let frames = decode_image(&read(&path)?).map_err(|e| format!("{file}: {e}"))?;
        if frames.is_empty() || frames.iter().any(|f| f.jpeg.is_empty()) {
            return Err(format!("{file}: unable to encode"));
        }
        let frames = Arc::new(frames);
        self.current = Some((file, stamp, Arc::clone(&frames)));
        Ok(frames)
    }
}

/// GIF bytes behind the Animation mode: the selected image if it is a GIF, else upstream's
/// default `concentric` background. Frames are decoded on the fly each pass, so only the
/// compressed file is held.
#[derive(Default)]
pub struct AnimationCache {
    current: Option<(String, Option<SystemTime>, Arc<[u8]>)>,
}

impl AnimationCache {
    pub fn get(&mut self, name: &str) -> Arc<[u8]> {
        let Some((file, path)) = resolve(name).filter(|(file, _)| is_gif(file)) else {
            self.current = None;
            return concentric();
        };
        let stamp = modified(&path);
        if let Some((cached, cached_stamp, bytes)) = &self.current {
            if *cached == file && *cached_stamp == stamp {
                return Arc::clone(bytes);
            }
        }
        match read(&path) {
            Ok(bytes) => {
                let bytes: Arc<[u8]> = Arc::from(bytes);
                self.current = Some((file, stamp, Arc::clone(&bytes)));
                bytes
            }
            Err(e) => {
                log::warn!("Unable to read LCD animation {file}: {e}");
                self.current = None;
                concentric()
            }
        }
    }
}

/// Stream animation frames: GIF background + sensor text, JPEG quality 90.
/// Falls back to the static data-screen background if the GIF can't be decoded.
pub fn animation_frames(gif: &[u8], overlay: &Overlay, mut f: impl FnMut(LcdFrame) -> bool) {
    let result = gif_frames(gif, |mut canvas, delay_ms| {
        overlay.apply(&mut canvas);
        f(LcdFrame { jpeg: canvas.jpeg(QUALITY_ANIMATION), delay_ms })
    });
    if let Err(e) = result {
        log::warn!("Unable to decode LCD animation: {e}");
        let mut canvas = Canvas::background();
        overlay.apply(&mut canvas);
        f(LcdFrame { jpeg: canvas.jpeg(QUALITY_ANIMATION), delay_ms: 0 });
    }
}
