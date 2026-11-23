use super::*;

const W: u32 = 10;
const H: u32 = 8;

fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = default_backends();
    let instance = wgpu::Instance::new(desc);
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    request_device(&adapter).ok()
}

/// Renders into a W x H Rgba8Unorm texture and returns its RGBA rows.
fn render_to_vec(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    p: &Presenter,
    cfg: &RenderConfig,
) -> Vec<[u8; 4]> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row = 256u32; // COPY_BYTES_PER_ROW_ALIGNMENT
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let view = target.create_view(&Default::default());
    let mut enc = device.create_command_encoder(&Default::default());
    p.draw(&mut enc, &view, (W, H), cfg);
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    buf.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("poll");
    let data = buf.slice(..).get_mapped_range().expect("mapped range");
    let mut out = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let i = (y * row + x * 4) as usize;
            out.push([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter"]
fn headless_render_reads_back_palette_colors() {
    let Some((device, queue)) = headless() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut p = Presenter::new(&device, wgpu::TextureFormat::Rgba8Unorm);

    // 2x2 square-pixel frame: red green / blue black.
    let mut frame = VideoFrame::blank(2, 2);
    frame.display_aspect = (1, 1);
    frame.pixels = vec![1, 2, 3, 0];
    frame.palette[1] = Rgb8 { r: 255, g: 0, b: 0 };
    frame.palette[2] = Rgb8 { r: 0, g: 255, b: 0 };
    frame.palette[3] = Rgb8 { r: 0, g: 0, b: 255 };
    p.upload(&device, &queue, &frame);

    // 10x8 window, padding 1px left/right -> 8x8 area, 4x integer scale at x=1.
    let cfg = RenderConfig {
        background: Rgb8 {
            r: 255,
            g: 255,
            b: 255,
        },
        padding: (1, 0),
        scale: ScaleMode::Integer,
    };
    let px = render_to_vec(&device, &queue, &p, &cfg);
    let at = |x: u32, y: u32| px[(y * W + x) as usize];
    assert_eq!(at(0, 0), [255, 255, 255, 255]); // padding
    assert_eq!(at(9, 7), [255, 255, 255, 255]);
    assert_eq!(at(1, 0), [255, 0, 0, 255]);
    assert_eq!(at(4, 3), [255, 0, 0, 255]);
    assert_eq!(at(5, 0), [0, 255, 0, 255]);
    assert_eq!(at(8, 3), [0, 255, 0, 255]);
    assert_eq!(at(1, 4), [0, 0, 255, 255]);
    assert_eq!(at(8, 7), [0, 0, 0, 255]);

    // Same size reuses the texture; a new size reallocates.
    assert_eq!(p.frame_allocations(), 1);
    frame.seq += 1;
    p.upload(&device, &queue, &frame);
    assert_eq!(p.frame_allocations(), 1);
    p.upload(&device, &queue, &VideoFrame::blank(4, 2));
    assert_eq!(p.frame_allocations(), 2);
    // Malformed frames are ignored.
    let mut bad = VideoFrame::blank(4, 4);
    bad.pixels.pop();
    p.upload(&device, &queue, &bad);
    assert_eq!(p.frame_allocations(), 2);
}

#[test]
fn render_config_defaults() {
    let c = RenderConfig::default();
    assert_eq!(c.scale, ScaleMode::Integer);
    assert_eq!(c.padding, (0, 0));
    assert_eq!(c.background, Rgb8::default());
}
