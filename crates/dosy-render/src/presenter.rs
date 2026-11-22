//! Surface-independent GPU drawing: uploads indexed frames and draws them
//! into any color target. [`crate::Renderer`] wraps it with a window surface;
//! tests use it with an offscreen texture.

use dosy_core::video::{Rgb8, VideoFrame};

use crate::RenderConfig;
use crate::layout::viewport;

struct FrameTexture {
    texture: wgpu::Texture,
    size: (u32, u32),
    bind_group: wgpu::BindGroup,
}

/// Palette-lookup + scaling pipeline for one target format.
pub struct Presenter {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    palette: wgpu::Texture,
    palette_view: wgpu::TextureView,
    frame: Option<FrameTexture>,
    aspect: (u32, u32),
    frame_allocations: u32,
}

impl Presenter {
    /// Builds the pipeline for color targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/present.wgsl"));
        let tex_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dosy frame+palette"),
            entries: &[
                tex_entry(0, wgpu::TextureSampleType::Uint),
                tex_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dosy present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dosy present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        // Palette entries are sRGB values. On an sRGB target, sample them
        // through an sRGB texture so the output encoding round-trips.
        let palette_format = if format.is_srgb() {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dosy palette"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: palette_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_view = palette.create_view(&Default::default());
        Self {
            format,
            pipeline,
            layout,
            palette,
            palette_view,
            frame: None,
            aspect: (4, 3),
            frame_allocations: 0,
        }
    }

    /// Target format this presenter draws into.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// How many frame textures have been created. Same-size uploads reuse
    /// the existing texture.
    pub fn frame_allocations(&self) -> u32 {
        self.frame_allocations
    }

    /// Uploads pixels and palette. Frames whose `pixels` don't match
    /// `width * height` are ignored.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame) {
        let (w, h) = (frame.width, frame.height);
        if w == 0 || h == 0 || frame.pixels.len() != (w as usize) * (h as usize) {
            return;
        }
        if self.frame.as_ref().map(|f| f.size) != Some((w, h)) {
            self.frame = Some(self.create_frame_texture(device, w, h));
            self.frame_allocations += 1;
        }
        let tex = &self.frame.as_ref().expect("allocated above").texture;
        queue.write_texture(
            tex.as_image_copy(),
            &frame.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            extent(w, h),
        );
        queue.write_texture(
            self.palette.as_image_copy(),
            &palette_rgba(&frame.palette),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(1),
            },
            extent(256, 1),
        );
        self.aspect = frame.display_aspect;
    }

    /// Clears `view` (of size `target`) to the background and draws the last
    /// uploaded frame into its viewport.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        target: (u32, u32),
        config: &RenderConfig,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dosy present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color(config.background, self.format)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let Some(frame) = &self.frame else { return };
        let Some(r) = viewport(
            frame.size,
            self.aspect,
            target,
            config.padding,
            config.scale,
        ) else {
            return;
        };
        pass.set_viewport(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &frame.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn create_frame_texture(&self, device: &wgpu::Device, w: u32, h: u32) -> FrameTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dosy frame"),
            size: extent(w, h),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dosy frame+palette"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.palette_view),
                },
            ],
        });
        FrameTexture {
            texture,
            size: (w, h),
            bind_group,
        }
    }
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

fn palette_rgba(palette: &[Rgb8; 256]) -> Vec<u8> {
    palette.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect()
}

/// Background as a clear color. Clear values are linear on sRGB targets.
fn clear_color(c: Rgb8, format: wgpu::TextureFormat) -> wgpu::Color {
    let ch = |v: u8| {
        let v = v as f64 / 255.0;
        if !format.is_srgb() {
            v
        } else if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    wgpu::Color {
        r: ch(c.r),
        g: ch(c.g),
        b: ch(c.b),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_color_linearizes_on_srgb_targets() {
        let c = Rgb8 {
            r: 255,
            g: 128,
            b: 0,
        };
        let unorm = clear_color(c, wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!((unorm.r, unorm.b), (1.0, 0.0));
        assert!((unorm.g - 128.0 / 255.0).abs() < 1e-9);
        let srgb = clear_color(c, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert_eq!((srgb.r, srgb.b), (1.0, 0.0));
        assert!((srgb.g - 0.2158).abs() < 1e-3);
    }

    #[test]
    fn palette_is_rgba() {
        let mut p = [Rgb8::default(); 256];
        p[1] = Rgb8 { r: 1, g: 2, b: 3 };
        let v = palette_rgba(&p);
        assert_eq!(v.len(), 1024);
        assert_eq!(&v[0..8], &[0, 0, 0, 255, 1, 2, 3, 255]);
    }
}
