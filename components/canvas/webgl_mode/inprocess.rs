/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::webgl_thread::{WebGLThread, WebGLThreadInit};
use canvas_traits::webgl::{WebGLContextId, WebGLMsg, WebGLSender, WebGLThreads};
use canvas_traits::webgl::{WebVRRenderHandler, webgl_channel};
use canvas_traits::webgl::SwapChainId;
use euclid::default::Size2D;
use fnv::FnvHashMap;
use gleam::gl;
use servo_config::pref;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
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
    ) -> WebGLComm {
        println!("WebGLThreads::new()");
        let (sender, receiver) = webgl_channel::<WebGLMsg>().unwrap();

        // Make our front buffer table.
        let swap_chains = SwapChains::new();

        // This implementation creates a single `WebGLThread` for all the pipelines.
        let init = WebGLThreadInit {
            webrender_api_sender,
            webvr_compositor,
            external_images,
            sender: sender.clone(),
            receiver,
            swap_chains: swap_chains.clone(),
            adapter: device.adapter(),
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
                                                swap_chains,
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
    swap_chains: SwapChains,
    locked_front_buffers: FnvHashMap<SwapChainId, SurfaceTexture>,
    sendable: SendableWebGLExternalImages,
}

impl WebGLExternalImages {
    fn new(device: Rc<Device>,
           context: Rc<RefCell<Context>>,
           webrender_gl: Rc<dyn gl::Gl>,    
           swap_chains: SwapChains,
           channel: WebGLSender<WebGLMsg>)
           -> Self {
        Self {
            device,
            context,
            webrender_gl,
            swap_chains,
            locked_front_buffers: FnvHashMap::default(),
            sendable: SendableWebGLExternalImages::new(channel),
        }
    }
}

impl WebGLExternalImages {
    fn lock_swap_chain(&mut self, id: SwapChainId) -> Option<(u32, Size2D<i32>)> {
        let mut swap_chains = self.swap_chains.lock();
        let swap_chain = swap_chains.get_mut(&id)?;
        let front_buffer = swap_chain.pending_surface.take().or_else(|| {
            println!("*** no surface ready, presenting last one");
            swap_chain.presented_surfaces.pop()
        })?;

        let mut context = self.context.borrow_mut();
        let size = front_buffer.size();
        let front_buffer_texture = self.device
                                       .create_surface_texture(&mut *context, front_buffer)
                                       .unwrap();
        let gl_texture = front_buffer_texture.gl_texture();

        self.locked_front_buffers.insert(id, front_buffer_texture);

        Some((gl_texture, size))
    }

    fn unlock_swap_chain(&mut self, id: SwapChainId) -> Option<()> {
        let locked_front_buffer = self.locked_front_buffers.remove(&id)?;
        let mut context = self.context.borrow_mut();
        let locked_front_buffer = self.device
                                      .destroy_surface_texture(&mut *context, locked_front_buffer)
                                      .unwrap();

        self.swap_chains.push_presented_surface(id, locked_front_buffer);
        Some(())
        /*
        let mut front_buffer_slot = self.swap_chains.get(id);
        let mut locked_front_buffer_slot = front_buffer_slot.lock();
        if locked_front_buffer_slot.is_none() {
            *locked_front_buffer_slot = Some(locked_front_buffer);
            return;
        }

        println!("spuriously destroying surface");
        self.device.destroy_surface(&mut *context, locked_front_buffer).unwrap();
        */
    }
}

impl WebrenderExternalImageApi for WebGLExternalImages {
    fn lock(&mut self, id: u64) -> (u32, Size2D<i32>) {
        let id = SwapChainId::Context(WebGLContextId(id));
        self.lock_swap_chain(id).unwrap_or_default()
    }

    fn unlock(&mut self, id: u64) {
        let id = SwapChainId::Context(WebGLContextId(id));
        self.unlock_swap_chain(id);
    }
}

#[derive(Clone)]
pub struct SwapChains {
    table: Arc<Mutex<FnvHashMap<SwapChainId, SwapChain>>>,
}

pub(crate) struct SwapChain {
    pub(crate) pending_surface: Option<Surface>,
    pub(crate) presented_surfaces: Vec<Surface>,
}

impl SwapChains {
    fn new() -> SwapChains {
        SwapChains {
            table: Arc::new(Mutex::new(FnvHashMap::default())),
        }
    }

    fn push_presented_surface(&self, swap_id: SwapChainId, surface: Surface) {
        let mut table = self.lock();
        match table.entry(swap_id) {
            Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(SwapChain {
                    pending_surface: None,
                    presented_surfaces: vec![surface],
                });
            }
            Entry::Occupied(mut occupied_entry) => {
                occupied_entry.get_mut().presented_surfaces.push(surface);
            }
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<FnvHashMap<SwapChainId, SwapChain>> {
        self.table.lock().unwrap()
    }
}

/*
impl FrontBuffer {
    fn take(&self) -> Option<Surface> {
        self.surface.lock().unwrap().take()
    }

    pub(crate) fn lock(&self) -> MutexGuard<Option<Surface>> {
        self.surface.lock().unwrap()
    }
}
*/

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
