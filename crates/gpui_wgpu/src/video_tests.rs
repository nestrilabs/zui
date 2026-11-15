use super::*;
use wgpu::util::DeviceExt;

const SIZE: u32 = 256;

/// A 64x32 limited-range BT.709 picture: the left half white, the right half red, or all `fill`.
fn picture(fill: Option<(u8, u8, u8)>) -> gpui::VideoFrame {
    let (w, h) = (64u32, 32u32);
    let white = (235u8, 128u8, 128u8);
    let red = (63u8, 102u8, 240u8);
    let pick = |x: u32| fill.unwrap_or(if x < w / 2 { white } else { red });
    let mut y = vec![0u8; (w * h) as usize];
    let mut uv = vec![0u8; (w * h / 2) as usize];
    for row in 0..h {
        for x in 0..w {
            y[(row * w + x) as usize] = pick(x).0;
        }
    }
    for row in 0..h / 2 {
        for cx in 0..w / 2 {
            let (_, cb, cr) = pick(cx * 2);
            uv[(row * w + cx * 2) as usize] = cb;
            uv[(row * w + cx * 2 + 1) as usize] = cr;
        }
    }
    gpui::VideoFrame {
        width: w,
        height: h,
        y_stride: w,
        uv_stride: w,
        y: y.into(),
        uv: uv.into(),
        matrix: gpui::VideoColorMatrix::Bt709,
        full_range: false,
    }
}

fn instance(gpu: &GpuVideo, quad: [f32; 4], crop: [f32; 4], transform: [f32; 4]) -> VideoInstance {
    VideoInstance {
        quad,
        crop,
        clip: [0.0, 0.0, SIZE as f32, SIZE as f32],
        transform,
        extra: [0.0, 0.0, 1.0, 0.0],
        rgb_r: gpu.matrix[0],
        rgb_g: gpu.matrix[1],
        rgb_b: gpu.matrix[2],
    }
}

/// BGRA bytes at (x, y), as (r, g, b, a).
fn pixel(pixels: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let i = ((y * SIZE + x) * 4) as usize;
    (pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3])
}

fn near(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8), at: &str) {
    let d = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs();
    assert!(
        d(actual.0, expected.0) <= 4
            && d(actual.1, expected.1) <= 4
            && d(actual.2, expected.2) <= 4
            && d(actual.3, expected.3) <= 4,
        "{at}: {actual:?} != {expected:?}"
    );
}

// Uses a real adapter and the production pipeline and texture upload.
#[test]
#[ignore = "requires a Vulkan adapter (software Vulkan is sufficient)"]
fn video_quads_crop_round_tilt_and_update_in_place() {
    let instance_desc = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    };
    let wgpu_instance = wgpu::Instance::new(instance_desc);
    let adapter = pollster::block_on(wgpu_instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let layouts = WgpuRenderer::create_bind_group_layouts(&device);
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let pipelines = WgpuRenderer::create_pipelines(
        &device,
        &layouts,
        format,
        wgpu::CompositeAlphaMode::PostMultiplied,
        4,
        false,
    );
    let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&GlobalParams {
            viewport_size: [SIZE as f32, SIZE as f32],
            premultiplied_alpha: 0,
            pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let gamma = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layouts.globals,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gamma.as_entire_binding(),
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let mut gpu = GpuVideo::new(&device, 64, 32, 0);
    gpu.write(&queue, &picture(None));

    let render = |gpu: &GpuVideo, instances: &[VideoInstance]| -> Vec<u8> {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layouts.video,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&gpu.y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&gpu.uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.video);
            pass.set_bind_group(0, &globals_group, &[]);
            pass.set_bind_group(1, &group, &[]);
            pass.draw(0..4, 0..instances.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = readback.slice(..).get_mapped_range().to_vec();
        readback.unmap();
        pixels
    };

    let white = (255, 255, 255, 255);
    let red = (255, 0, 0, 255);
    // A: the whole picture, rounded 16. B: the right half (a different crop of the same
    // textures), tilted 24 degrees with perspective 640. One instanced draw.
    let a = instance(
        &gpu,
        [16.0, 16.0, 128.0, 64.0],
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, 0.0, 16.0],
    );
    let b = instance(
        &gpu,
        [16.0, 128.0, 128.0, 64.0],
        [0.5, 0.0, 0.5, 1.0],
        [1.0, 24f32.to_radians(), 640.0, 16.0],
    );
    let pixels = render(&gpu, &[a, b]);
    near(pixel(&pixels, 40, 48), white, "A left half");
    near(pixel(&pixels, 120, 48), red, "A right half");
    assert!(pixel(&pixels, 17, 17).3 < 8, "A corner is rounded");
    assert!(pixel(&pixels, 80, 15).3 == 0, "nothing above A");
    near(
        pixel(&pixels, 30, 160),
        red,
        "B near edge (crop is all red)",
    );
    near(pixel(&pixels, 80, 160), red, "B centre");
    // Untilted, B would cover x 16..144. Turned away, its right edge lands near x 136.
    assert!(pixel(&pixels, 140, 160).3 < 8, "B's far edge is pulled in");
    // Its near (left) half grows taller than 64: at x 40 its top is near y 127.1, above 128.
    assert!(pixel(&pixels, 40, 127).3 > 128, "B's near side is taller");

    // In place: the same textures take a new picture.
    gpu.write(&queue, &picture(Some((63, 102, 240))));
    let pixels = render(&gpu, &[a]);
    near(pixel(&pixels, 40, 48), red, "A after the in-place update");

    // Full range BT.601: code 255 with neutral chroma is white.
    let full = video_rgb_matrix(gpui::VideoColorMatrix::Bt601, true);
    let w = [1.0f32, 128.0 / 255.0, 128.0 / 255.0, 1.0];
    for row in full {
        let v: f32 = row.iter().zip(w).map(|(m, x)| m * x).sum();
        assert!((v - 1.0).abs() < 1e-4);
    }
}

#[test]
fn bt709_limited_matrix_matches_the_nesflow_picture_conversion() {
    // nesflow-codec Picture::rgb: R = 1.164(Y-16) + 1.793(Cr-128), G = .. - 0.213 Cb - 0.533 Cr,
    // B = .. + 2.112 Cb.
    let m = video_rgb_matrix(gpui::VideoColorMatrix::Bt709, false);
    let rgb = |y: f32, cb: f32, cr: f32| {
        let v = [y / 255.0, cb / 255.0, cr / 255.0, 1.0];
        m.map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum::<f32>() * 255.0)
    };
    for (y, cb, cr) in [
        (16.0, 128.0, 128.0),
        (235.0, 128.0, 128.0),
        (81.0, 90.0, 240.0),
    ] {
        let scale = 255.0 / 219.0 * (y - 16.0);
        let expected = [
            scale + 1.793 * (cr - 128.0),
            scale - 0.213 * (cb - 128.0) - 0.533 * (cr - 128.0),
            scale + 2.112 * (cb - 128.0),
        ];
        let got = rgb(y, cb, cr);
        for c in 0..3 {
            assert!(
                (got[c] - expected[c]).abs() < 0.6,
                "{got:?} vs {expected:?}"
            );
        }
    }
}
