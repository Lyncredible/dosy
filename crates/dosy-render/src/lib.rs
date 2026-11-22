//! GPU presenter for [`dosy_core::video::VideoFrame`]s (wgpu). The app hands
//! in a `wgpu::Surface`; this crate never depends on a windowing library.
//! See `docs/ARCHITECTURE.md` ("Renderer").

pub mod layout;
mod presenter;

use std::fmt;

use dosy_core::video::{Rgb8, VideoFrame};

pub use layout::{Rect, ScaleMode};
pub use presenter::Presenter;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Presentation settings (mapped from the app config).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderConfig {
    /// Color for padding and letterbox areas.
    pub background: Rgb8,
    /// Padding in pixels on each side: (left/right, top/bottom).
    pub padding: (u32, u32),
    pub scale: ScaleMode,
}

#[derive(Debug)]
pub enum RenderError {
    /// No GPU adapter can drive the surface.
    NoAdapter(String),
    /// Device creation failed.
    Device(String),
    /// The surface has no format/present mode usable with the adapter.
    SurfaceUnsupported,
    /// The surface was lost and must be recreated by the app.
    SurfaceLost,
    /// A validation error was raised while acquiring the surface texture.
    Validation,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            Self::Device(e) => write!(f, "GPU device creation failed: {e}"),
            Self::SurfaceUnsupported => f.write_str("window surface not supported by the GPU"),
            Self::SurfaceLost => f.write_str("window surface lost"),
            Self::Validation => f.write_str("GPU validation error acquiring the surface"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Creates a device and queue on `adapter` with downlevel-friendly limits.
pub fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), RenderError> {
    let desc = wgpu::DeviceDescriptor {
        label: Some("dosy"),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults()
            .using_resolution(adapter.limits()),
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&desc))
        .map_err(|e| RenderError::Device(e.to_string()))
}

/// Draws frames into a window surface.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    presenter: Presenter,
    config: RenderConfig,
}

impl Renderer {
    /// Picks an adapter for `surface`, creates the device and configures the
    /// surface at `size` (physical pixels). Blocks until the GPU is ready.
    pub fn new(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        size: (u32, u32),
    ) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let (device, queue) = request_device(&adapter)?;
        let mut surface_config = surface
            .get_default_config(&adapter, size.0.max(1), size.1.max(1))
            .ok_or(RenderError::SurfaceUnsupported)?;
        surface_config.present_mode = wgpu::PresentMode::AutoVsync;
        let presenter = Presenter::new(&device, surface_config.format);
        let mut r = Self {
            surface,
            device,
            queue,
            surface_config,
            presenter,
            config: RenderConfig::default(),
        };
        r.resize(size.0, size.1);
        Ok(r)
    }

    pub fn config(&self) -> &RenderConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: RenderConfig) {
        self.config = config;
    }

    /// Reconfigures the surface. A zero size (minimized window) is ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Uploads a frame; textures are reused while the size stays the same.
    pub fn upload(&mut self, frame: &VideoFrame) {
        self.presenter.upload(&self.device, &self.queue, frame);
    }

    /// Draws the last uploaded frame and presents. Frames that can't be
    /// acquired right now (timeout, occluded, outdated) are skipped.
    pub fn render(&mut self) -> Result<(), RenderError> {
        use wgpu::CurrentSurfaceTexture as Cst;
        let (texture, suboptimal) = match self.surface.get_current_texture() {
            Cst::Success(t) => (t, false),
            Cst::Suboptimal(t) => (t, true),
            Cst::Timeout | Cst::Occluded => return Ok(()),
            Cst::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Cst::Lost => return Err(RenderError::SurfaceLost),
            Cst::Validation => return Err(RenderError::Validation),
        };
        let view = texture.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let size = (self.surface_config.width, self.surface_config.height);
        self.presenter.draw(&mut encoder, &view, size, &self.config);
        self.queue.submit([encoder.finish()]);
        self.queue.present(texture);
        if suboptimal {
            self.surface.configure(&self.device, &self.surface_config);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
