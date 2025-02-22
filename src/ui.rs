use egui::{epaint::Shadow, *};
use tracing_setup::tracing::{trace, warn};
use wgpu::RenderPassDescriptor;
// need to include self since the instrument macro expands to refer to it
use tracing_setup::tracing::{self, instrument};

use crate::window_texture::WindowTexture;

#[derive(Clone, Copy)]
pub struct CallbackTraitImplementer {
    // the blur window rect, which will be used in the callback funcs
    window_rect: Rect,
}

impl CallbackTraitImplementer {
    #[instrument(level = "trace", name = "blur pass", skip(self, encoder, wt))]
    pub fn first_pass(&self, encoder: &mut wgpu::CommandEncoder, wt: &WindowTexture) {
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: wt.back_view(),
                resolve_target: None,
                ops: wgpu::Operations {
                    // clear the offscreen texture before writing the new blurred content
                    load: wgpu::LoadOp::Clear(wgpu::Color::default()),
                    // store the blurred output in the back_view
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        trace!("Pass started");

        let rect = self.window_rect;
        // NOTE: the prepare function ensured the uniform buffer that our shader uses is up to date
        let min = (rect.min.to_vec2() * wt.pixels_per_point() as f32).round();
        let max = (rect.max.to_vec2() * wt.pixels_per_point() as f32).round();

        render_pass.set_viewport(min.x, min.y, max.x - min.x, max.y - min.y, 0.0, 1.0);

        let reg = wt.pipeline_registry();
        render_pass.set_pipeline(&reg.blur_rect_pipeline);
        render_pass.set_bind_group(0, &reg.blur_rect_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }

    #[instrument(level = "trace", name = "copy pass full", skip(self, encoder, wt))]
    pub fn second_pass_old(&self, encoder: &mut wgpu::CommandEncoder, wt: &WindowTexture) {
        /* Operating on render_pass directly here results in lifetime issues since it's borrowing part of the WindowTexture,
            which comes from resources. ie
            render_pass.begin_new_render_pass(...); // expects wt resource to live for 'static, since that's render_pass's lifetime

            Using the encoder to start a new render pass results in the parameters getting moved/copied before this function
            completes, so there's no lifetime issues.
        */
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: wt.view(), // this must hold the app output - why?
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        trace!("Pass started");

        let size = wt.physical_size();
        render_pass.set_viewport(0.0, 0.0, size.width as f32, size.height as f32, 0.0, 1.0);

        let reg = wt.pipeline_registry();
        render_pass.set_pipeline(&reg.copy_back_pipeline);
        render_pass.set_bind_group(0, &reg.copy_back_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }

    #[instrument(
        level = "trace",
        name = "copy pass partial",
        skip(self, render_pass, wt)
    )]
    fn second_pass(&self, render_pass: &mut wgpu::RenderPass<'static>, wt: &WindowTexture) {
        trace!("Pass started");

        let size = wt.physical_size();
        render_pass.set_viewport(0.0, 0.0, size.width as f32, size.height as f32, 0.0, 1.0);

        let reg = wt.pipeline_registry();
        render_pass.set_pipeline(&reg.copy_back_pipeline);
        render_pass.set_bind_group(0, &reg.copy_back_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

/*
Ideas: try doing everything in prepare. and do nothing in paint
*/

impl egui_wgpu::CallbackTrait for CallbackTraitImplementer {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        trace!("prepare() callback");
        let wt = resources
            .get::<WindowTexture>()
            .expect("WindowTexture resource not found");

        // this ensures that the uniform buffer our shader uses is up to date
        // with the latest blur window size
        wt.pipeline_registry().set_rect(self.window_rect, queue);

        // self.first_pass(egui_encoder, wt);
        // self.second_pass_old(egui_encoder, wt);
        // self.second_pass(egui_encoder, wt);

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        // let wt = resources
        //     .get::<WindowTexture>()
        //     .expect("WindowTexture resource not found");

        // trace!("paint() callback");

        // // TODO: This should be using a color attachment to wt.view()
        // // but I can't change the render pass to use that since it's already been started...
        // self.second_pass(render_pass, wt);
    }
}

// Janky: passing in rect so we can get the blur window size elsewhere...
pub fn ui_main(
    ctx: &egui::Context,
    image: &egui::TextureHandle,
) -> Option<CallbackTraitImplementer> {
    trace!("Drawing main UI");

    egui::CentralPanel::default().show(&ctx, |ui| {
        ui.heading("This is a test");
        ui.image(image);
        let _ = ui.button("Test");
    });

    // windows are on the middle layer
    let layer = LayerId::new(Order::Middle, Id::from("test_window_bg"));

    let blur_window = egui::Window::new("Test")
        .id(layer.id)
        .frame(
            Frame::window(&ctx.style())
                .fill(Color32::TRANSPARENT)
                .shadow(Shadow::NONE),
        )
        .resizable(true)
        .default_size(vec2(200., 260.))
        .show(ctx, |ui| {
            ui.allocate_space(ui.available_size());
        })
        .unwrap()
        .response;

    // in the original implementation, the callback was added to the painter before the blur window
    // which allowed it to be drawn under it. However, I don't know of a clean way to do this without
    // storing the rect and being off by one frame the first time and if it's moved/resized.
    let painter = ctx.layer_painter(layer);
    let rect = blur_window.rect;
    if rect.size().length() > 0.0 {
        let callback = CallbackTraitImplementer { window_rect: rect };
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            callback.clone(),
        ));
        Some(callback)
    } else {
        None
    }
}
