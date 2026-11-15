//! The SVG renderer for a gpui built without the `svg` feature. It keeps the types the rest of
//! gpui names, so the window and image code compile unchanged. SVG images fail to load with an
//! error, and SVG icons paint nothing. No SVG parser, rasteriser or bundled font is linked.

use crate::{AssetSource, DevicePixels, RenderImage, Result, SharedString, Size};
use std::sync::Arc;

/// The factor SVG pixmaps are rendered at, kept so callers compile.
pub const SMOOTH_SVG_SCALE_FACTOR: f32 = 2.;

/// The parameters of one SVG icon render.
#[derive(Clone, PartialEq, Hash, Eq)]
#[expect(missing_docs)]
pub struct RenderSvgParams {
    pub path: SharedString,
    pub size: Size<DevicePixels>,
}

/// Stand-in for the SVG renderer. It has no state.
#[derive(Clone)]
pub struct SvgRenderer {
    _asset_source: Arc<dyn AssetSource>,
}

/// The size in which to render the SVG.
pub enum SvgSize {
    /// An absolute size in device pixels.
    Size(Size<DevicePixels>),
    /// A scaling factor to apply to the size provided by the SVG.
    ScaleFactor(f32),
}

impl SvgRenderer {
    /// Creates the stand-in renderer.
    pub fn new(asset_source: Arc<dyn AssetSource>) -> Self {
        Self {
            _asset_source: asset_source,
        }
    }

    /// SVG images are not supported in this build.
    pub fn render_single_frame(
        &self,
        _bytes: &[u8],
        _scale_factor: f32,
    ) -> Result<Arc<RenderImage>> {
        anyhow::bail!("SVG images are not supported in this build (gpui without the svg feature)")
    }

    /// SVG icons paint nothing in this build.
    pub(crate) fn render_alpha_mask(
        &self,
        _params: &RenderSvgParams,
        _bytes: Option<&[u8]>,
    ) -> Result<Option<(Size<DevicePixels>, Vec<u8>)>> {
        Ok(None)
    }
}
