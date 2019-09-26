/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::webgl_thread::{WebGLThread, WebGLThreadInit};
use canvas_traits::webgl::{WebGLMsg, WebGLSender, WebGLThreads, WebVRRenderHandler, webgl_channel};
use euclid::default::Size2D;
use fnv::FnvHashMap;
use gleam::gl;
use servo_config::pref;
use std::cell::RefCell;
use std::default::Default;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use surfman::{self, Context, Device, Surface, SurfaceTexture};
use webrender_traits::{WebrenderExternalImageApi, WebrenderExternalImageRegistry};
use webxr_api::WebGLExternalImageApi;

pub struct WebGLComm {
    pub webgl_threads: WebGLThreads,
    pub webxr_handler: Box<dyn webxr_api::WebGLExternalImageApi>,
    pub image_handler: Box<dyn WebrenderExternalImageApi>,
    pub output_handler: Option<Box<dyn webrender::OutputImageHandler>>,
}

impl WebGLComm {
    /// Creates a new `WebGLComm` object.
    pub fn new(
        device: Rc<Device>,
        context: Rc<RefCell<Context>>,
        webrender_gl: Rc<dyn gl::Gl>,
        webrender_api_sender: webrender_api::RenderApiSender,
        webvr_compositor: Option<Box<dyn WebVRRenderHandler>>,
        external_images: Arc<Mutex<WebrenderExternalImageRegistry>>,
        api_type: sparkle::gl::GlType,
    ) -> WebGLComm {
        println!("WebGLThreads::new()");
        let (sender, receiver) = webgl_channel::<WebGLMsg>().unwrap();

        // Make a front buffer.
        let front_buffer = Arc::new(FrontBuffer::new());

        // This implementation creates a single `WebGLThread` for all the pipelines.
        let init = WebGLThreadInit {
            webrender_api_sender,
            webvr_compositor,
            external_images,
            sender: sender.clone(),
            receiver,
            front_buffer: front_buffer.clone(),
            adapter: device.adapter(),
            api_type,
        };

        let output_handler = if pref!(dom.webgl.dom_to_texture.enabled) {
            Some(Box::new(OutputHandler::new(
                webrender_gl.clone(),
                sender.clone(),
            )))
        } else {
            None
        };

        let external = WebGLExternalImages::new(device,
                                                context,
                                                webrender_gl,
                                                front_buffer,
                                                sender.clone());

        WebGLThread::run_on_own_thread(init);

        WebGLComm {
            webgl_threads: WebGLThreads(sender),
            webxr_handler: external.sendable.clone_box(),
            image_handler: Box::new(external),
            output_handler: output_handler.map(|b| b as Box<_>),
        }
    }
}

/// Bridge between the webxr_api::ExternalImage callbacks and the WebGLThreads.
struct SendableWebGLExternalImages {
    webgl_channel: WebGLSender<WebGLMsg>,
}

impl SendableWebGLExternalImages {
    fn new(channel: WebGLSender<WebGLMsg>) -> Self {
        Self {
            webgl_channel: channel,
        }
    }
}

impl webxr_api::WebGLExternalImageApi for SendableWebGLExternalImages {
    fn lock(&self, _id: usize) -> Option<gl::GLsync> {
        // TODO(pcwalton)
        None
    }

    fn unlock(&self, _id: usize) {
        // TODO(pcwalton)
    }

    fn clone_box(&self) -> Box<dyn webxr_api::WebGLExternalImageApi> {
        Box::new(Self::new(self.webgl_channel.clone()))
    }
}

/// Bridge between the webrender::ExternalImage callbacks and the WebGLThreads.
struct WebGLExternalImages {
    device: Rc<Device>,
    context: Rc<RefCell<Context>>,
    webrender_gl: Rc<dyn gl::Gl>,
    front_buffer: Arc<FrontBuffer>,
    locked_front_buffer: Option<SurfaceTexture>,
    sendable: SendableWebGLExternalImages,
}

impl WebGLExternalImages {
    fn new(device: Rc<Device>,
           context: Rc<RefCell<Context>>,
           webrender_gl: Rc<dyn gl::Gl>,    
           front_buffer: Arc<FrontBuffer>,
           channel: WebGLSender<WebGLMsg>)
           -> Self {
        Self {
            device,
            context,
            webrender_gl,
            front_buffer,
            locked_front_buffer: None,
            sendable: SendableWebGLExternalImages::new(channel),
        }
    }
}

impl WebrenderExternalImageApi for WebGLExternalImages {
    // FIXME(pcwalton): What is the ID for?
    fn lock(&mut self, id: u64) -> (u32, Size2D<i32>) {
        //self.context.borrow().debug("lock");
        let (gl_texture, size);
        match self.front_buffer.take() {
            None => {
                gl_texture = 0;
                size = Size2D::new(0, 0);
                self.locked_front_buffer = None;
            }
            Some(front_buffer) => {
                let mut context = self.context.borrow_mut();
                size = front_buffer.size();
                let locked_front_buffer =
                    self.device.create_surface_texture(&mut *context, front_buffer).unwrap();
                gl_texture = locked_front_buffer.gl_texture();
                self.locked_front_buffer = Some(locked_front_buffer);
            }
        }
        (gl_texture, size)
    }

    fn unlock(&mut self, id: u64) {
        //self.context.borrow().debug("unlock");
        self.sendable.unlock(id as usize);

        let locked_front_buffer = match self.locked_front_buffer.take() {
            None => return,
            Some(locked_front_buffer) => locked_front_buffer,
        };

        let mut context = self.context.borrow_mut();
        let locked_front_buffer = self.device
                                      .destroy_surface_texture(&mut *context, locked_front_buffer)
                                      .unwrap();

        let mut front_buffer_slot = self.front_buffer.0.lock().unwrap();
        if front_buffer_slot.is_none() {
            *front_buffer_slot = Some(locked_front_buffer);
            return;
        }

        let _ = self.device.destroy_surface(&mut *context, locked_front_buffer)/*.unwrap()*/;
    }
}

pub struct FrontBuffer(Mutex<Option<Surface>>);

impl FrontBuffer {
    fn new() -> FrontBuffer {
        FrontBuffer(Mutex::new(None))
    }

    fn take(&self) -> Option<Surface> {
        self.0.lock().unwrap().take()
    }

    pub(crate) fn lock(&self) -> MutexGuard<Option<Surface>> {
        self.0.lock().unwrap()
    }
}

/// struct used to implement DOMToTexture feature and webrender::OutputImageHandler trait.
//type OutputHandlerData = Option<(u32, Size2D<i32>)>;
struct OutputHandler {
    webrender_gl: Rc<dyn gl::Gl>,
    webgl_channel: WebGLSender<WebGLMsg>,
    // Used to avoid creating a new channel on each received WebRender request.
    sync_objects: FnvHashMap<webrender_api::PipelineId, gl::GLsync>,
}

impl OutputHandler {
    fn new(webrender_gl: Rc<dyn gl::Gl>, channel: WebGLSender<WebGLMsg>) -> Self {
        OutputHandler {
            webrender_gl,
            webgl_channel: channel,
            sync_objects: Default::default(),
        }
    }
}

/// Bridge between the WR frame outputs and WebGL to implement DOMToTexture synchronization.
impl webrender::OutputImageHandler for OutputHandler {
    fn lock(
        &mut self,
        id: webrender_api::PipelineId,
    ) -> Option<(u32, webrender_api::units::FramebufferIntSize)> {
        // Insert a fence in the WR command queue
        let gl_sync = self
            .webrender_gl
            .fence_sync(gl::SYNC_GPU_COMMANDS_COMPLETE, 0);
        self.sync_objects.insert(id, gl_sync);
        None
    }

    fn unlock(&mut self, id: webrender_api::PipelineId) {
        if let Some(gl_sync) = self.sync_objects.remove(&id) {
            // Flush the Sync object into the GPU's command queue to guarantee that it it's signaled.
            self.webrender_gl.flush();
            // Mark the sync object for deletion.
            self.webrender_gl.delete_sync(gl_sync);
        }
    }
}
