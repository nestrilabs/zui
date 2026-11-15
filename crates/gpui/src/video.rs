//! Video frames drawn as transformed, rounded, cropped quads.
//!
//! A [`VideoSource`] holds the newest NV12 picture a producer (a decoder thread, for example)
//! has handed it. A renderer that supports video keeps one persistent pair of textures per
//! source (luma, and interleaved chroma at half size) and uploads into them in place whenever
//! the source's generation moves on; it never allocates a texture or atlas tile per frame. Any
//! number of [`Window::paint_video`](crate::Window::paint_video) calls in a frame may draw
//! different crops of the same source.
//!
//! Support: the Linux/FreeBSD wgpu renderer draws video. Windows (DirectX 11) and macOS (Metal)
//! accept the calls and draw nothing yet.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::{Bounds, Pixels, Point, Radians};

/// The YCbCr to RGB matrix a picture was encoded with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoColorMatrix {
    /// ITU-R BT.709 (HD).
    #[default]
    Bt709,
    /// ITU-R BT.601 (SD).
    Bt601,
}

/// One NV12 picture in CPU memory.
///
/// `y` holds `height` rows of `y_stride` bytes, the first `width` of each being luma. `uv` holds
/// `height / 2` rows of `uv_stride` bytes, the first `width` of each (`width / 2` Cb, Cr pairs)
/// being chroma. Width and height must be even.
#[derive(Clone)]
pub struct VideoFrame {
    /// Width in luma pixels.
    pub width: u32,
    /// Height in luma pixels.
    pub height: u32,
    /// Bytes per luma row.
    pub y_stride: u32,
    /// Bytes per chroma row.
    pub uv_stride: u32,
    /// The luma plane.
    pub y: Arc<[u8]>,
    /// The interleaved Cb, Cr plane at half resolution in both directions.
    pub uv: Arc<[u8]>,
    /// The matrix the picture was encoded with.
    pub matrix: VideoColorMatrix,
    /// Full range (0..255) when true, limited (16..235 luma, 16..240 chroma) when false.
    pub full_range: bool,
}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("matrix", &self.matrix)
            .field("full_range", &self.full_range)
            .finish()
    }
}

/// Identifies a [`VideoSource`] to the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoSourceId(pub u64);

static NEXT_VIDEO_SOURCE: AtomicU64 = AtomicU64::new(1);

struct SourceState {
    generation: u64,
    frame: Option<VideoFrame>,
}

/// The newest picture of one video stream, shared between its producer and the renderer.
///
/// [`submit`](Self::submit) replaces the held picture; pictures the renderer has not uploaded
/// before the next one arrives are dropped (counted in [`VideoSource::superseded`]).
pub struct VideoSource {
    id: VideoSourceId,
    state: Mutex<SourceState>,
    superseded: AtomicU64,
    uploaded_generation: AtomicU64,
}

impl fmt::Debug for VideoSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoSource").field("id", &self.id).finish()
    }
}

impl VideoSource {
    /// A source with no picture yet.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: VideoSourceId(NEXT_VIDEO_SOURCE.fetch_add(1, Ordering::Relaxed)),
            state: Mutex::new(SourceState {
                generation: 0,
                frame: None,
            }),
            superseded: AtomicU64::new(0),
            uploaded_generation: AtomicU64::new(0),
        })
    }

    /// This source's id.
    pub fn id(&self) -> VideoSourceId {
        self.id
    }

    /// Hands the source a new picture. Returns its generation (1 for the first).
    pub fn submit(&self, frame: VideoFrame) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.generation > self.uploaded_generation.load(Ordering::Relaxed)
            && state.frame.is_some()
        {
            self.superseded.fetch_add(1, Ordering::Relaxed);
        }
        state.generation += 1;
        state.frame = Some(frame);
        state.generation
    }

    /// The generation of the newest picture (0 before the first).
    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .generation
    }

    /// The size of the newest picture, in luma pixels.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .frame
            .as_ref()
            .map(|f| (f.width, f.height))
    }

    /// For renderers: the newest picture and its generation, when it is newer than `seen`.
    /// The planes are shared, not copied.
    pub fn frame_newer_than(&self, seen: u64) -> Option<(u64, VideoFrame)> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.generation > seen {
            state.frame.clone().map(|f| (state.generation, f))
        } else {
            None
        }
    }

    /// For renderers: records that the picture of `generation` is on the GPU.
    pub fn mark_uploaded(&self, generation: u64) {
        self.uploaded_generation
            .fetch_max(generation, Ordering::Relaxed);
    }

    /// The newest generation a renderer has uploaded.
    pub fn uploaded_generation(&self) -> u64 {
        self.uploaded_generation.load(Ordering::Relaxed)
    }

    /// Pictures replaced by a newer one before any renderer uploaded them.
    pub fn superseded(&self) -> u64 {
        self.superseded.load(Ordering::Relaxed)
    }
}

/// A transform applied to a video quad about its centre, in this order: scale, rotation about the
/// vertical (Y) axis seen through a perspective camera, then translation. The result matches the
/// CSS `transform: translate(tx, ty) perspective(p) rotateY(a) scale(s)` with the transform origin
/// at the centre of the quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoTransform {
    /// Uniform scale.
    pub scale: f32,
    /// Translation after the rotation.
    pub translate: Point<Pixels>,
    /// Rotation about the vertical axis through the centre. Positive turns the right edge away
    /// from the viewer.
    pub rotate_y: Radians,
    /// Distance from the viewer to the quad's plane (CSS `perspective`). Zero or less disables
    /// the perspective (an orthographic rotation).
    pub perspective: Pixels,
}

impl Default for VideoTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            translate: Point::default(),
            rotate_y: Radians(0.0),
            perspective: Pixels(0.0),
        }
    }
}

impl VideoTransform {
    /// Where the local point `local` (relative to the centre, before any transform) lands, relative
    /// to the centre, and its perspective divisor `w`.
    pub fn project(&self, local: (f32, f32)) -> ((f32, f32), f32) {
        let (sx, sy) = (local.0 * self.scale, local.1 * self.scale);
        let (sin, cos) = self.rotate_y.0.sin_cos();
        let x = sx * cos;
        // Positive z comes toward the viewer; a positive angle turns the right edge away.
        let z = -sx * sin;
        let p = f32::from(self.perspective);
        let w = if p > 0.0 {
            ((p - z) / p).max(0.05)
        } else {
            1.0
        };
        (
            (
                x / w + f32::from(self.translate.x),
                sy / w + f32::from(self.translate.y),
            ),
            w,
        )
    }
}

/// How one [`Window::paint_video`](crate::Window::paint_video) call draws its source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoParams {
    /// The part of the picture to draw, in picture pixels. `None` draws the whole picture.
    pub crop: Option<Bounds<f32>>,
    /// The corner radius of the quad, before the transform (so it scales and tilts with it).
    pub corner_radius: Pixels,
    /// Opacity, multiplied with the element opacity in effect.
    pub opacity: f32,
    /// The transform about the quad's centre.
    pub transform: VideoTransform,
}

impl Default for VideoParams {
    fn default() -> Self {
        Self {
            crop: None,
            corner_radius: Pixels(0.0),
            opacity: 1.0,
            transform: VideoTransform::default(),
        }
    }
}

/// A video quad as it reaches the renderer, in device pixels.
#[derive(Clone, Debug)]
pub struct VideoQuad {
    /// The source whose textures are sampled.
    pub source: Arc<VideoSource>,
    /// The quad before its transform, in device pixels.
    pub quad_bounds: crate::Bounds<crate::ScaledPixels>,
    /// The crop in picture pixels; `None` is the whole picture.
    pub crop: Option<Bounds<f32>>,
    /// Corner radius in device pixels, before the transform.
    pub corner_radius: f32,
    /// Final opacity.
    pub opacity: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Translation in device pixels.
    pub translate: (f32, f32),
    /// Rotation about Y, radians.
    pub rotate_y: f32,
    /// Perspective distance in device pixels (0 = none).
    pub perspective: f32,
}

/// One rendered frame's video timings, recorded by a renderer that draws video.
#[derive(Clone, Copy, Debug, Default)]
pub struct VideoFrameTiming {
    /// When the renderer started the frame.
    pub at: Option<Instant>,
    /// CPU time spent writing new pictures into the textures, in microseconds.
    pub upload_us: u32,
    /// Bytes uploaded this frame.
    pub upload_bytes: u32,
    /// CPU time of the whole draw call (acquire, upload, encode, submit, present), in microseconds.
    pub draw_us: u32,
    /// CPU time spent waiting for the swapchain image, in microseconds.
    pub acquire_us: u32,
    /// Video quads drawn this frame.
    pub quads: u32,
}

static TIMINGS: Mutex<Vec<VideoFrameTiming>> = Mutex::new(Vec::new());

/// Upper bound on held timings, so an app that never drains them does not grow without end.
const MAX_TIMINGS: usize = 1 << 16;

/// For renderers: records one frame's video timings.
pub fn record_video_frame_timing(timing: VideoFrameTiming) {
    let mut timings = TIMINGS.lock().unwrap_or_else(|p| p.into_inner());
    if timings.len() < MAX_TIMINGS {
        timings.push(timing);
    }
}

/// Takes the video frame timings recorded since the last call.
pub fn take_video_frame_timings() -> Vec<VideoFrameTiming> {
    std::mem::take(&mut *TIMINGS.lock().unwrap_or_else(|p| p.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::px;

    #[test]
    fn identity_projection_is_the_point() {
        let t = VideoTransform::default();
        assert_eq!(t.project((10.0, -4.0)), ((10.0, -4.0), 1.0));
    }

    #[test]
    fn a_turned_quad_shrinks_its_far_edge() {
        let t = VideoTransform {
            rotate_y: Radians(24f32.to_radians()),
            perspective: px(640.0),
            ..Default::default()
        };
        let ((_, near_y), _) = t.project((-100.0, 50.0));
        let ((_, far_y), _) = t.project((100.0, 50.0));
        assert!(far_y < 50.0 && near_y > 50.0, "{far_y} {near_y}");
    }

    #[test]
    fn superseded_pictures_are_counted() {
        let source = VideoSource::new();
        let frame = VideoFrame {
            width: 2,
            height: 2,
            y_stride: 2,
            uv_stride: 2,
            y: Arc::from(vec![0u8; 4]),
            uv: Arc::from(vec![128u8; 2]),
            matrix: VideoColorMatrix::Bt709,
            full_range: false,
        };
        source.submit(frame.clone());
        source.submit(frame.clone());
        assert_eq!(source.superseded(), 1);
        let (generation, _) = source.frame_newer_than(0).unwrap();
        source.mark_uploaded(generation);
        source.submit(frame);
        assert_eq!(source.superseded(), 1);
    }
}
