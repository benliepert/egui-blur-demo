use std::sync::Arc;

use egui::ViewportId;
use raw_window_handle::HasDisplayHandle;
use tracing_setup::tracing::{debug, error, trace};
use wgpu::SurfaceError;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
};

use crate::{
    context::RenderContext, pipeline::WindowPipelineRegistry, surface::SurfaceRenderer,
    window_texture::WindowTexture,
};

pub struct Program<'a> {
    // These are Arc'd to avoid lifetime issues, since the surfaceRenderer's Surface
    // is created based on the window's lifetime, storing both here would create a
    // self-referential lifetime issue. render_ctx has the same issue.
    // TODO: determine if window needs to be stored here at all. It's not possible to
    // get it from the surface, but it might be possible to get it by other means (e.g.
    // if we can get the window based on the ID contained in window events)
    window: Arc<winit::window::Window>,
    render_ctx: Arc<RenderContext>,

    event_loop: Option<EventLoop<()>>,

    surface: SurfaceRenderer<'a>,

    egui_winit_bridge: egui_winit::State, // holds the egui context as well

    egui_wgpu_renderer: egui_wgpu::Renderer,

    ferris_img: egui::TextureHandle,
}

impl<'a> Program<'a> {
    pub async fn new() -> Self {
        let event_loop = EventLoop::new().expect("Failed to create event loop");
        let window_attributes = winit::window::Window::default_attributes()
            .with_inner_size(winit::dpi::LogicalSize::new(455., 330.))
            .with_title("Blur Rect Demo");

        // TODO: update this to use ActiveEventLoop instead that's more of a change, so I'm wary of doing it upfront
        let window = Arc::new(
            event_loop
                .create_window(window_attributes)
                .expect("Failed to create window"),
        );

        let render_ctx = Arc::new(RenderContext::new().await);
        let surface = SurfaceRenderer::from_window(window.clone(), render_ctx.clone());

        let egui_ctx = egui::Context::default();

        // TODO: dithering is off for now. Is that OK?
        let egui_wgpu_renderer =
            egui_wgpu::Renderer::new(render_ctx.get_device().0, surface.format(), None, 1, false);

        // Note: this only supports a single window for now
        let display_target = window.display_handle().unwrap();
        let native_pixels_per_point = Some(window.scale_factor() as f32);

        let ferris_img = Self::create_img(&egui_ctx);

        // TODO: is ROOT OK? only supports 1 window for now
        let viewport_id = ViewportId::ROOT;
        let egui_winit_bridge = egui_winit::State::new(
            egui_ctx,
            viewport_id,
            &display_target,
            native_pixels_per_point,
            None,
            None,
        );

        let mut res = Self {
            window,
            event_loop: Some(event_loop),

            render_ctx,
            surface,

            // egui_ctx,
            egui_wgpu_renderer,
            egui_winit_bridge,

            ferris_img,
        };

        res.generate_window_texture();

        res
    }

    fn create_img(ctx: &egui::Context) -> egui::TextureHandle {
        let image = image::load_from_memory(include_bytes!("cuddlyferris.png").as_slice()).unwrap();
        let size = [image.width() as _, image.height() as _];
        let image_buffer = image.to_rgba8();
        let pixels: image::FlatSamples<&[u8]> = image_buffer.as_flat_samples();

        ctx.load_texture(
            "cuddlyferris",
            egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()),
            egui::TextureOptions::LINEAR,
        )
    }

    pub fn run(mut self) {
        let event_loop = self.event_loop.take().unwrap();

        // TODO: update this to use run_app?
        // this keeps it simple though and reduces the risk of breakage for now
        // might want to add a return value too. Just follow some example
        let res = event_loop.run(move |event, _event_loop| {
            trace!("Event loop received event: {:?}", event);
            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::RedrawRequested => self.handle_redraw_request(),
                    _ => self.handle_window_event(&event),
                },

                // https://github.com/rust-windowing/winit/issues/2900
                // said to migrate from MainEventsCleared to this
                Event::AboutToWait => {
                    self.window.request_redraw();
                }
                _ => (),
            }
        });

        match res {
            Ok(_) => debug!("Event loop exited successfully"),
            Err(e) => error!("{:?}", e),
        }
    }

    fn resize(&mut self, new_inner_size: PhysicalSize<u32>, new_scale_factor: Option<f64>) {
        if let Some(new_scale_factor) = new_scale_factor {
            self.surface.set_scale_factor(new_scale_factor)
        }

        self.surface.resize(new_inner_size, &self.render_ctx);
        self.generate_window_texture();
    }

    fn generate_window_texture(&mut self) {
        let window_texture = WindowTexture::from_surface(&self.surface, &self.render_ctx);

        let _ = self
            .egui_wgpu_renderer
            .callback_resources
            .insert(window_texture);
    }

    pub fn handle_window_event(&mut self, event: &winit::event::WindowEvent) {
        match event {
            winit::event::WindowEvent::Resized(new_size) => {
                self.resize(*new_size, None);
            }

            winit::event::WindowEvent::ScaleFactorChanged {
                scale_factor: _, ..
            } => {
                todo!("handle scale factor change");
                // self.egui_winit_bridge
                //     .set_pixels_per_point(*scale_factor as f32);
                // self.resize(**new_inner_size, Some(*scale_factor));
            }

            _ => (),
        }

        let response = self.egui_winit_bridge.on_window_event(&self.window, event);

        if !response.consumed {
            match event {
                winit::event::WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == winit::event::ElementState::Pressed {
                        if let winit::keyboard::Key::Character(ref ch) = event.logical_key {
                            if ch == "q" {
                                self.exit();
                            }
                        }
                    }
                }
                winit::event::WindowEvent::CloseRequested => self.exit(),
                _ => (),
            }
        }
    }

    fn exit(&mut self) {
        todo!("Implement event loop exit");
        // if let Some(event_loop) = self.event_loop.take() {
        //     event_loop.exit();
        // }
    }

    pub fn handle_redraw_request(&mut self) {
        match self.draw() {
            Ok(_) => {}
            Err(SurfaceError::Lost) => self.surface.reconfigure(&self.render_ctx),
            Err(SurfaceError::OutOfMemory) => self.exit(),
            Err(e) => eprintln!("{:?}", e),
        }
    }

    pub fn draw(&mut self) -> Result<(), SurfaceError> {
        trace!("Drawing frame");

        let raw_input = self.egui_winit_bridge.take_egui_input(&self.window);

        let full_output = self
            .egui_winit_bridge
            .egui_ctx()
            .run(raw_input, |ctx| crate::ui::ui_main(ctx, &self.ferris_img));

        // TODO: is this correct?
        let pixels_per_point = self.surface.get_scale_fac() as f32;
        let paint_jobs = self
            .egui_winit_bridge
            .egui_ctx()
            .tessellate(full_output.shapes, pixels_per_point);
        let screen_descriptor = self.surface.screen_descriptor();

        let (device, queue) = self.render_ctx.get_device();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("EGUI Render Encoder"),
        });

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_wgpu_renderer
                .update_texture(device, queue, *id, image_delta)
        }

        // one difference between this and the egui-wgpu impl is that that one
        // stores the renderer in an Arc<RwLock<>>
        let _data = {
            self.egui_wgpu_renderer.update_buffers(
                device,
                queue,
                &mut encoder,
                &paint_jobs,
                &screen_descriptor,
            )
        };

        {
            // get the write lock in a binding so that it persists until we're done using wt
            let resources = &mut self.egui_wgpu_renderer.callback_resources;
            let wt = resources.get::<WindowTexture>().unwrap();

            let descriptor = wgpu::RenderPassDescriptor {
                label: Some("EGUI Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &wt.view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            };

            trace!("First render pass");
            let render_pass = encoder.begin_render_pass(&descriptor);
            self.egui_wgpu_renderer.render(
                &mut render_pass.forget_lifetime(),
                &paint_jobs,
                &screen_descriptor,
            );
        }

        let output = self.surface.get_current_texture(&self.render_ctx)?;

        {
            let surface_view = output
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            trace!("Second render pass");
            let mut copy_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("EGUI copy render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            let resources = &mut self.egui_wgpu_renderer.callback_resources;

            let WindowPipelineRegistry {
                copy_pipeline,
                copy_bind_group,
                ..
            } = &resources
                .get::<WindowTexture>()
                .unwrap()
                .pipeline_registry();

            copy_render_pass.set_pipeline(&copy_pipeline);
            copy_render_pass.set_bind_group(0, copy_bind_group, &[]);
            copy_render_pass.draw(0..4, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &full_output.textures_delta.free {
            self.egui_wgpu_renderer.free_texture(id);
        }

        self.egui_winit_bridge
            .handle_platform_output(&self.window, full_output.platform_output);

        trace!("Frame drawn");

        Ok(())
    }
}
