//! winit event loop on the main thread: window, renderer, and the bridge to
//! the emulation thread. Space cycles test patterns (until VGA lands).

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
use crate::emu::{self, EmuEvent, EmuHandle, EmuSetup, HostEvent};
use crate::mailbox::Mailbox;

const TITLE: &str = "dosy";

struct Gpu {
    window: Arc<Window>,
    renderer: Renderer,
}

struct App {
    config: Config,
    gpu: Option<Gpu>,
    emu: EmuHandle,
    frames: Arc<Mailbox<VideoFrame>>,
    /// Last frame taken from the mailbox (buffer reused).
    frame: VideoFrame,
    has_frame: bool,
    error: Option<String>,
}

/// Starts the emulation thread, opens the window and runs until it closes.
pub fn run(config: Config, setup: EmuSetup) -> Result<(), String> {
    let event_loop = EventLoop::<EmuEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let frames = Arc::new(Mailbox::new(VideoFrame::blank(1, 1)));
    let emu = emu::spawn(setup, frames.clone(), move |e| {
        // Fails only once the event loop is gone (shutting down).
        let _ = proxy.send_event(e);
    })
    .map_err(|e| format!("cannot start emulation thread: {e}"))?;
    let mut app = App {
        config,
        gpu: None,
        emu,
        frames,
        frame: VideoFrame::blank(1, 1),
        has_frame: false,
        error: None,
    };
    let result = event_loop.run_app(&mut app).map_err(|e| e.to_string());
    app.emu.shutdown();
    result?;
    app.error.map_or(Ok(()), Err)
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, e: String) {
        self.error = Some(e);
        event_loop.exit();
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<Gpu, String> {
        let attrs = Window::default_attributes()
            .with_title(TITLE)
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
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        ));
        desc.backends = dosy_render::default_backends();
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let mut renderer = Renderer::new(&instance, surface, (size.width, size.height))
            .map_err(|e| e.to_string())?;
        renderer.set_config(render_config(&self.config, window.scale_factor()));
        if self.has_frame {
            renderer.upload(&self.frame);
        }
        Ok(renderer)
    }

    /// Takes the newest frame from the mailbox and uploads it.
    fn pull_frame(&mut self) {
        if !self.frames.take(&mut self.frame) {
            return;
        }
        if !self.config.aspect {
            self.frame.display_aspect = (0, 0); // square pixels
        }
        self.has_frame = true;
        if let Some(gpu) = &mut self.gpu {
            gpu.renderer.upload(&self.frame);
            gpu.window.request_redraw();
        }
    }
}

impl ApplicationHandler<EmuEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            match self.init(event_loop) {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(e) => self.fail(event_loop, e),
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: EmuEvent) {
        match event {
            EmuEvent::Frame => self.pull_frame(),
            EmuEvent::Paused(why) => {
                if let Some(gpu) = &self.gpu {
                    gpu.window.set_title(&format!("{TITLE} - paused: {why}"));
                }
                eprintln!("dosy: paused: {why}");
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gpu) = &mut self.gpu else { return };
        match event {
            WindowEvent::CloseRequested => {
                self.emu.shutdown();
                event_loop.exit();
            }
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
            } => self.emu.send(HostEvent::NextPattern),
            _ => {}
        }
    }
}
