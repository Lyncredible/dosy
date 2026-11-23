//! winit event loop: one window, one renderer. Space cycles test patterns.

use std::sync::Arc;

use dosy_config::Config;
use dosy_core::video::VideoFrame;
use dosy_render::{RenderError, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::cli::render_config;
use crate::pattern;

struct Gpu {
    window: Arc<Window>,
    renderer: Renderer,
}

struct App {
    config: Config,
    frames: Vec<VideoFrame>,
    current: usize,
    gpu: Option<Gpu>,
    error: Option<String>,
}

/// Opens the window and runs until it is closed.
pub fn run(config: Config) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut frames = vec![pattern::bars_320x200(), pattern::text_720x400()];
    if !config.aspect {
        for f in &mut frames {
            f.display_aspect = (0, 0); // square pixels
        }
    }
    let mut app = App {
        config,
        frames,
        current: 0,
        gpu: None,
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, e: String) {
        self.error = Some(e);
        event_loop.exit();
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<Gpu, String> {
        let attrs = Window::default_attributes()
            .with_title("dosy")
            .with_inner_size(LogicalSize::new(960.0, 720.0));
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
        let renderer = self.make_renderer(event_loop, &window)?;
        window.request_redraw();
        Ok(Gpu { window, renderer })
    }

    /// New instance, surface and renderer for `window`, with the current
    /// config and frame. Also used to recover from a lost surface.
    fn make_renderer(
        &self,
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
    ) -> Result<Renderer, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let mut renderer = Renderer::new(&instance, surface, (size.width, size.height))
            .map_err(|e| e.to_string())?;
        renderer.set_config(render_config(&self.config, window.scale_factor()));
        renderer.upload(&self.frames[self.current]);
        Ok(renderer)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            match self.init(event_loop) {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(e) => self.fail(event_loop, e),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gpu) = &mut self.gpu else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.renderer.resize(size.width, size.height);
                gpu.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.renderer
                    .set_config(render_config(&self.config, scale_factor));
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => match gpu.renderer.render() {
                Ok(()) => {}
                Err(RenderError::SurfaceLost) => {
                    // Recreate the surface and renderer, then redraw once.
                    let window = gpu.window.clone();
                    match self.make_renderer(event_loop, &window) {
                        Ok(r) => {
                            if let Some(gpu) = &mut self.gpu {
                                gpu.renderer = r;
                            }
                            window.request_redraw();
                        }
                        Err(e) => self.fail(event_loop, e),
                    }
                }
                Err(e) => self.fail(event_loop, e.to_string()),
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Space),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                self.current = (self.current + 1) % self.frames.len();
                gpu.renderer.upload(&self.frames[self.current]);
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}
