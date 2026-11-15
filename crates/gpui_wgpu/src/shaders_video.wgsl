// Video quads: a crop of an NV12 picture (luma texture + interleaved chroma texture, both
// persistent and updated in place), drawn with a rounded-corner mask, opacity, and a transform
// about the quad's centre (scale, rotation about Y through a perspective camera, translation).
//
// A standalone module: it shares only the layout of the globals uniform with shaders.wgsl.

struct VideoGlobals {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
}

struct VideoInstance {
    // Untransformed quad in device pixels: origin.xy, size.zw.
    quad: vec4<f32>,
    // Crop in normalized texture coordinates: origin.xy, size.zw.
    crop: vec4<f32>,
    // Content mask in device pixels: origin.xy, size.zw.
    clip: vec4<f32>,
    // scale, rotate_y (radians), perspective (device px, 0 = none), corner radius (device px).
    transform: vec4<f32>,
    // translate.xy (device px), opacity, unused.
    extra: vec4<f32>,
    // Rows of the YCbCr -> RGB matrix: rgb = M * (y, cb, cr, 1).
    rgb_r: vec4<f32>,
    rgb_g: vec4<f32>,
    rgb_b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> video_globals: VideoGlobals;
@group(1) @binding(0) var<storage, read> b_videos: array<VideoInstance>;
@group(1) @binding(1) var t_video_y: texture_2d<f32>;
@group(1) @binding(2) var t_video_uv: texture_2d<f32>;
@group(1) @binding(3) var s_video: sampler;

struct VideoVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) instance_id: u32,
}

@vertex
fn vs_video(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> VideoVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let v = b_videos[instance_id];
    let size = v.quad.zw;
    let center = v.quad.xy + size * 0.5;
    let local = (unit_vertex - vec2<f32>(0.5)) * size;

    let scaled = local * v.transform.x;
    let angle = v.transform.y;
    let x = scaled.x * cos(angle);
    // Positive z comes toward the viewer; a positive angle turns the right edge away (CSS rotateY).
    let z = -scaled.x * sin(angle);
    let p = v.transform.z;
    var w = 1.0;
    if (p > 0.0) {
        w = max((p - z) / p, 0.05);
    }
    let screen = center + vec2<f32>(x, scaled.y) / w + v.extra.xy;
    let ndc = screen / video_globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out = VideoVarying();
    // Multiplying by w makes the rasterizer interpolate `local` and `uv` perspective-correctly.
    out.position = vec4<f32>(ndc * w, 0.0, w);
    out.local = local;
    out.uv = v.crop.xy + unit_vertex * v.crop.zw;
    out.instance_id = instance_id;
    return out;
}

fn video_rounded_sdf(local: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let corner_center_to_point = abs(local) - half_size + vec2<f32>(r);
    return length(max(vec2<f32>(0.0), corner_center_to_point))
        + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))
        - r;
}

@fragment
fn fs_video(input: VideoVarying) -> @location(0) vec4<f32> {
    let v = b_videos[input.instance_id];

    // Everything that needs derivatives runs before any branch.
    let y = textureSample(t_video_y, s_video, input.uv).r;
    let cb_cr = textureSample(t_video_uv, s_video, input.uv).rg;
    let distance = video_rounded_sdf(input.local, v.quad.zw * 0.5, v.transform.w);
    // Distance is in quad pixels; under scale and tilt one screen pixel covers fwidth of it.
    let aa = max(fwidth(distance), 1e-4);
    let coverage = saturate(0.5 - distance / aa);

    let p = input.position.xy;
    let clip_min = v.clip.xy;
    let clip_max = v.clip.xy + v.clip.zw;
    if (any(p < clip_min) || any(p > clip_max)) {
        return vec4<f32>(0.0);
    }

    let ycc = vec4<f32>(y, cb_cr, 1.0);
    let rgb = saturate(vec3<f32>(dot(v.rgb_r, ycc), dot(v.rgb_g, ycc), dot(v.rgb_b, ycc)));
    let alpha = coverage * v.extra.z;
    let multiplier = select(1.0, alpha, video_globals.premultiplied_alpha != 0u);
    return vec4<f32>(rgb * multiplier, alpha);
}
