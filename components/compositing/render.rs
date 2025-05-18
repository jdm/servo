/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::path::PathBuf;
use std::rc::Rc;

use base::id::PipelineId;
use euclid::Size2D;
use profile_traits::mem::{Report, ReportKind};
use profile_traits::path;
use webrender::{CaptureBits, RenderApi, Renderer, Transaction};
use webrender_api::{DocumentId, FontKey, FontInstanceKey, HitTestFlags, HitTestResult, ImageKey, PipelineId as WebRenderPipelineId, Epoch as WebRenderEpoch};
use webrender_api::units::{DevicePixel, WorldPoint};
use wr_malloc_size_of::MallocSizeOfOps;

use crate::compositor::WebRenderDebugOption;

pub(crate) struct WebRenderRenderer {
    /// The WebRender [`RenderApi`] interface used to communicate with WebRender.
    webrender_api: RenderApi,

    /// The active webrender document.
    webrender_document: DocumentId,

    /// The GL bindings for webrender
    webrender_gl: Rc<dyn gleam::gl::Gl>,

    /// The webrender renderer.
    webrender: Option<Renderer>,
}

impl WebRenderRenderer {
    pub(crate) fn new(
        api: RenderApi,
        document: DocumentId,
        gl: Rc<dyn gleam::gl::Gl>,
        renderer: Renderer,
    ) -> Self {
        Self {
            webrender_api: api,
            webrender_document: document,
            webrender_gl: gl,
            webrender: Some(renderer),
        }
    }

    pub(crate) fn report_memory(&self, ops: MallocSizeOfOps) -> Vec<Report> {
        let report = self.webrender_api.report_memory(ops);
        vec![
            Report {
                path: path!["webrender", "fonts"],
                kind: ReportKind::ExplicitJemallocHeapSize,
                size: report.fonts,
            },
            Report {
                path: path!["webrender", "images"],
                kind: ReportKind::ExplicitJemallocHeapSize,
                size: report.images,
            },
            Report {
                path: path!["webrender", "display-list"],
                kind: ReportKind::ExplicitJemallocHeapSize,
                size: report.display_list,
            },
        ]
    }

    pub(crate) fn gl_info(&self) -> (String, String) {
        (
            self.webrender_gl.get_string(gleam::gl::RENDERER),
            self.webrender_gl.get_string(gleam::gl::VERSION),
        )
    }

    pub(crate) fn save_capture(&self, capture_path: PathBuf) {
        println!("Saving WebRender capture to {capture_path:?}");
        self.webrender_api.save_capture(capture_path, CaptureBits::all());
    }

    pub(crate) fn toggle_webrender_debug(&mut self, option: WebRenderDebugOption) {
        let Some(webrender) = self.webrender.as_mut() else {
            return;
        };
        let mut flags = webrender.get_debug_flags();
        let flag = match option {
            WebRenderDebugOption::Profiler => {
                webrender::DebugFlags::PROFILER_DBG |
                    webrender::DebugFlags::GPU_TIME_QUERIES |
                    webrender::DebugFlags::GPU_SAMPLE_QUERIES
            },
            WebRenderDebugOption::TextureCacheDebug => webrender::DebugFlags::TEXTURE_CACHE_DBG,
            WebRenderDebugOption::RenderTargetDebug => webrender::DebugFlags::RENDER_TARGET_DBG,
        };
        flags.toggle(flag);
        webrender.set_debug_flags(flags);
    }

    pub(crate) fn render(&mut self, size: Size2D<i32, DevicePixel>) {
        if let Some(webrender) = self.webrender.as_mut() {
            webrender.render(size, 0 /* buffer_age */).ok();
        }
    }

    pub(crate) fn update(&mut self) {
        if let Some(webrender) = self.webrender.as_mut() {
            webrender.update();
        }
    }

    pub(crate) fn generate_font_instance_key(&self) -> FontInstanceKey {
        self.webrender_api.generate_font_instance_key()
    }

    pub(crate) fn generate_font_key(&self) -> FontKey {
        self.webrender_api.generate_font_key()
    }

    pub(crate) fn generate_image_key(&self) -> ImageKey {
        self.webrender_api.generate_image_key()
    }

    pub(crate) fn flush_scene_builder(&self) {
        self.webrender_api.flush_scene_builder();
    }

    pub(crate) fn current_epoch(&self, id: PipelineId) -> Option<WebRenderEpoch> {
        self.webrender.as_ref().and_then(|wr| wr.current_epoch(self.webrender_document, id.into()))
    }

    pub(crate) fn deinit(&mut self) {
        if let Some(webrender) = self.webrender.take() {
            webrender.deinit();
        }
    }

    pub(crate) fn send_transaction(&mut self, transaction: Transaction) {
        self.webrender_api
            .send_transaction(self.webrender_document, transaction);
    }

    pub(crate) fn hit_test(
        &self,
        pipeline_id: Option<WebRenderPipelineId>,
        world_point: WorldPoint,
        flags: HitTestFlags,
    ) -> HitTestResult {
        self.webrender_api.hit_test(self.webrender_document, pipeline_id, world_point, flags)
    }

    pub(crate) fn clear_background(&self, color: [f64; 4]) {
        let gl = &self.webrender_gl;
        self.assert_gl_framebuffer_complete();

        // Always clear the entire RenderingContext, regardless of how many WebViews there are
        // or where they are positioned. This is so WebView actually clears even before the
        // first WebView is ready.
        gl.clear_color(
            color[0] as f32,
            color[1] as f32,
            color[2] as f32,
            color[3] as f32,
        );
        gl.clear(gleam::gl::COLOR_BUFFER_BIT);
    }

    #[track_caller]
    pub(crate) fn assert_no_gl_error(&self) {
        debug_assert_eq!(
            self.webrender_gl.get_error(),
            gleam::gl::NO_ERROR
        );
    }

    #[track_caller]
    pub(crate) fn assert_gl_framebuffer_complete(&self) {
        debug_assert_eq!(
            (
                self.webrender_gl.get_error(),
                self.webrender_gl.check_frame_buffer_status(gleam::gl::FRAMEBUFFER)
            ),
            (gleam::gl::NO_ERROR, gleam::gl::FRAMEBUFFER_COMPLETE)
        );
    }
}
