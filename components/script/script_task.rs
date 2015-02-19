/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! The script task is the task that owns the DOM in memory, runs JavaScript, and spawns parsing
//! and layout tasks.

#![allow(unsafe_blocks)]

use dom::bindings::cell::DOMRefCell;
use dom::bindings::codegen::Bindings::DocumentBinding::{DocumentMethods, DocumentReadyState};
use dom::bindings::codegen::Bindings::EventBinding::EventMethods;
use dom::bindings::codegen::Bindings::EventTargetBinding::EventTargetMethods;
use dom::bindings::codegen::InheritTypes::{ElementCast, EventTargetCast, HTMLIFrameElementCast, NodeCast, EventCast};
use dom::bindings::conversions::FromJSValConvertible;
use dom::bindings::conversions::StringificationBehavior;
use dom::bindings::global::GlobalRef;
use dom::bindings::js::{JS, JSRef, OptionalRootable, RootedReference, Temporary};
use dom::bindings::js::{RootCollection, RootCollectionPtr};
use dom::bindings::refcounted::{LiveDOMReferences, Trusted, TrustedReference};
use dom::bindings::structuredclone::StructuredCloneData;
use dom::bindings::trace::JSTraceable;
use dom::bindings::utils::{wrap_for_same_compartment, pre_wrap};
use dom::browsercontext::BrowserContext;
use dom::document::{Document, IsHTMLDocument, DocumentHelpers, DocumentSource};
use dom::element::{Element, ElementTypeId, ActivationElementHelpers};
use dom::event::{Event, EventHelpers, EventBubbles, EventCancelable};
use dom::uievent::UIEvent;
use dom::eventtarget::{EventTarget, EventTargetHelpers};
use dom::htmlelement::HTMLElementTypeId;
use dom::htmliframeelement::HTMLIFrameElement;
use dom::keyboardevent::KeyboardEvent;
use dom::mouseevent::MouseEvent;
use dom::node::{self, Node, NodeHelpers, NodeDamage, NodeTypeId, window_from_node};
use dom::window::{Window, WindowHelpers, ScriptHelpers};
use dom::worker::{Worker, TrustedWorkerAddress};
use frame::{Frame, FrameTree, FrameCache};
use parse::html::{HTMLInput, parse_html};
use layout_interface::{ScriptLayoutChan, LayoutChan, ReflowGoal, ReflowQueryType};
use layout_interface;
use timers::TimerId;
use devtools;

use devtools_traits::{DevtoolsControlChan, DevtoolsControlPort, NewGlobal, GetRootNode, DevtoolsPageInfo};
use devtools_traits::{DevtoolScriptControlMsg, EvaluateJS, GetDocumentElement};
use devtools_traits::{GetChildren, GetLayout, ModifyAttribute, WantsLiveNotifications};
use script_traits::CompositorEvent;
use script_traits::CompositorEvent::{ResizeEvent, ReflowEvent, ClickEvent};
use script_traits::CompositorEvent::{MouseDownEvent, MouseUpEvent};
use script_traits::CompositorEvent::{MouseMoveEvent, KeyEvent};
use script_traits::{NewLayoutInfo, OpaqueScriptLayoutChannel};
use script_traits::{ConstellationControlMsg, ScriptControlChan};
use script_traits::ScriptTaskFactory;
use msg::compositor_msg::ReadyState::{FinishedLoading, Loading, PerformingLayout};
use msg::compositor_msg::{LayerId, ScriptListener};
use msg::constellation_msg::{ConstellationChan};
use msg::constellation_msg::{LoadData, NavigationDirection, PipelineId, SubpageId};
use msg::constellation_msg::{Failure, Msg, WindowSizeData, Key, KeyState};
use msg::constellation_msg::{KeyModifiers, SUPER, SHIFT, CONTROL, ALT};
use msg::constellation_msg::{PipelineExitType};
use msg::constellation_msg::Msg as ConstellationMsg;
use net::image_cache_task::ImageCacheTask;
use net::resource_task::{ResourceTask, ControlMsg, LoadResponse};
use net::resource_task::LoadData as NetLoadData;
use net::storage_task::StorageTask;
use util::geometry::to_frac_px;
use util::smallvec::SmallVec;
use util::str::DOMString;
use util::task::{spawn_named, spawn_named_with_send_on_failure};
use util::task_state;

use geom::Rect;
use geom::point::{Point2D};
use hyper::header::{Header, Headers, HeaderFormat};
use hyper::header::shared::util as header_util;
use js::jsapi::{JS_SetWrapObjectCallbacks, JS_SetGCZeal, JS_DEFAULT_ZEAL_FREQ, JS_GC};
use js::jsapi::{JSContext, JSRuntime, JSObject, JS_SetExtraGCRootsTracer};
use js::jsapi::{JS_SetGCParameter, JSGC_MAX_BYTES, JSTracer};
use js::jsapi::{JS_SetGCCallback, JSGCStatus, JSGC_BEGIN, JSGC_END};
use js::rust::{Cx, RtUtils};
use js;
use url::Url;

use libc;
use std::any::Any;
use std::borrow::ToOwned;
use std::cell::{RefCell, Cell};
use std::fmt::{self, Show};
use std::num::ToPrimitive;
use std::ptr;
use std::rc::Rc;
use std::sync::mpsc::{channel, Sender, Receiver, Select};
use std::u32;
use time::{Tm, strptime};

thread_local!(pub static STACK_ROOTS: Cell<Option<RootCollectionPtr>> = Cell::new(None));
thread_local!(pub static SCRIPT_TASK: Rc<RefCell<Option<*const ScriptTask>>> = Rc::new(RefCell::new(None)));

struct InProgressLoad {
    pipeline_id: PipelineId,
    subpage_id: Option<(PipelineId, (SubpageId, Option<SubpageId>))>,
    window_size: WindowSizeData,
    layout_chan: LayoutChan,
    clip_rect: Option<Rect<f32>>,
    url: Url,
}

impl InProgressLoad {
    fn new(id: PipelineId,
           subpage_id: Option<(PipelineId, (SubpageId, Option<SubpageId>))>,
           layout_chan: LayoutChan,
           window_size: WindowSizeData,
           url: Url) -> InProgressLoad {
        InProgressLoad {
            pipeline_id: id,
            subpage_id: subpage_id,
            layout_chan: layout_chan,
            window_size: window_size,
            clip_rect: None,
            url: url,
        }
    }
}

#[derive(Copy)]
pub enum TimerSource {
    FromWindow(PipelineId),
    FromWorker
}

pub trait Runnable {
    fn handler(self: Box<Self>);
}

/// Messages used to control script event loops, such as ScriptTask and
/// DedicatedWorkerGlobalScope.
pub enum ScriptMsg {
    /// Acts on a fragment URL load on the specified pipeline (only dispatched
    /// to ScriptTask).
    TriggerFragment(PipelineId, Url),
    /// Begins a content-initiated load on the specified pipeline (only
    /// dispatched to ScriptTask).
    TriggerLoad(PipelineId, LoadData),
    /// Instructs the script task to send a navigate message to
    /// the constellation (only dispatched to ScriptTask).
    Navigate(NavigationDirection),
    /// Fires a JavaScript timeout
    /// TimerSource must be FromWindow when dispatched to ScriptTask and
    /// must be FromWorker when dispatched to a DedicatedGlobalWorkerScope
    FireTimer(TimerSource, TimerId),
    /// Notifies the script that a window associated with a particular pipeline
    /// should be closed (only dispatched to ScriptTask).
    ExitWindow(PipelineId),
    /// Message sent through Worker.postMessage (only dispatched to
    /// DedicatedWorkerGlobalScope).
    DOMMessage(StructuredCloneData),
    /// Sends a message to the Worker object (dispatched to all tasks) regarding error.
    WorkerDispatchErrorEvent(TrustedWorkerAddress, DOMString, DOMString, u32, u32),
    /// Generic message that encapsulates event handling.
    RunnableMsg(Box<Runnable+Send>),
    /// A DOM object's last pinned reference was removed (dispatched to all tasks).
    RefcountCleanup(TrustedReference),
    /// The final network response for a page has arrived.
    PageFetchComplete(PipelineId, Option<SubpageId>, LoadResponse),
}

/// A cloneable interface for communicating with an event loop.
pub trait ScriptChan {
    /// Send a message to the associated event loop.
    fn send(&self, msg: ScriptMsg);
    /// Clone this handle.
    fn clone(&self) -> Box<ScriptChan+Send>;
}

/// Encapsulates internal communication within the script task.
#[jstraceable]
pub struct NonWorkerScriptChan(pub Sender<ScriptMsg>);

impl ScriptChan for NonWorkerScriptChan {
    fn send(&self, msg: ScriptMsg) {
        let NonWorkerScriptChan(ref chan) = *self;
        chan.send(msg)/*.unwrap()*/.ok();
    }

    fn clone(&self) -> Box<ScriptChan+Send> {
        let NonWorkerScriptChan(ref chan) = *self;
        box NonWorkerScriptChan((*chan).clone())
    }
}

impl NonWorkerScriptChan {
    /// Creates a new script chan.
    pub fn new() -> (Receiver<ScriptMsg>, Box<NonWorkerScriptChan>) {
        let (chan, port) = channel();
        (port, box NonWorkerScriptChan(chan))
    }
}

pub struct StackRootTLS;

impl StackRootTLS {
    pub fn new(roots: &RootCollection) -> StackRootTLS {
        STACK_ROOTS.with(|ref r| {
            r.set(Some(RootCollectionPtr(roots as *const _)))
        });
        StackRootTLS
    }
}

impl Drop for StackRootTLS {
    fn drop(&mut self) {
        STACK_ROOTS.with(|ref r| r.set(None));
    }
}

/// Information for an entire page. Pages are top-level browsing contexts and can contain multiple
/// frames.
///
/// FIXME: Rename to `Page`, following WebKit?
//TODO(jdm) need to ensure frame tree and cached frames are traced
pub struct ScriptTask {
    /// The current, active frame tree
    frame_tree: DOMRefCell<FrameTree>,
    /// The cache of inactive frames
    frame_cache: DOMRefCell<FrameCache>,
    /// A list of data pertaining to loads that have not yet received a network response
    incomplete_loads: DOMRefCell<Vec<InProgressLoad>>,
    /// A handle to the image cache task.
    image_cache_task: ImageCacheTask,
    /// A handle to the resource task.
    resource_task: ResourceTask,
    /// A handle to the storage task.
    storage_task: StorageTask,

    /// The port on which the script task receives messages (load URL, exit, etc.)
    port: Receiver<ScriptMsg>,
    /// A channel to hand out to script task-based entities that need to be able to enqueue
    /// events in the event queue.
    chan: NonWorkerScriptChan,

    /// A channel to hand out to tasks that need to respond to a message from the script task.
    control_chan: ScriptControlChan,

    /// The port on which the constellation and layout tasks can communicate with the
    /// script task.
    control_port: Receiver<ConstellationControlMsg>,

    /// For communicating load url messages to the constellation
    constellation_chan: ConstellationChan,
    /// A handle to the compositor for communicating ready state messages.
    compositor: DOMRefCell<Box<ScriptListener+'static>>,

    /// For providing instructions to an optional devtools server.
    devtools_chan: Option<DevtoolsControlChan>,
    /// For receiving commands from an optional devtools server. Will be ignored if
    /// no such server exists.
    devtools_port: DevtoolsControlPort,
    devtools_sender: Sender<DevtoolScriptControlMsg>,

    /// The JavaScript runtime.
    js_runtime: js::rust::rt,
    /// The JSContext.
    js_context: DOMRefCell<Option<Rc<Cx>>>,

    mouse_over_targets: DOMRefCell<Option<Vec<JS<Node>>>>
}

/// In the event of task failure, all data on the stack runs its destructor. However, there
/// are no reachable, owning pointers to the DOM memory, so it never gets freed by default
/// when the script task fails. The ScriptMemoryFailsafe uses the destructor bomb pattern
/// to forcibly tear down the JS compartments for pages associated with the failing ScriptTask.
struct ScriptMemoryFailsafe<'a> {
    owner: Option<&'a ScriptTask>,
}

impl<'a> ScriptMemoryFailsafe<'a> {
    fn neuter(&mut self) {
        self.owner = None;
    }

    fn new(owner: &'a ScriptTask) -> ScriptMemoryFailsafe<'a> {
        ScriptMemoryFailsafe {
            owner: Some(owner),
        }
    }
}

#[unsafe_destructor]
impl<'a> Drop for ScriptMemoryFailsafe<'a> {
    fn drop(&mut self) {
        match self.owner {
            Some(owner) => {
                unsafe {
                    let mut clear = |&:win: JSRef<Window>| {
                        win.clear_js_context_for_script_deallocation();
                    };
                    owner.frame_cache
                         .borrow_for_script_deallocation()
                         .for_all_windows(&mut clear);
                    owner.frame_tree
                         .borrow_for_script_deallocation()
                         .for_all_windows(clear);
                    *owner.js_context.borrow_for_script_deallocation() = None;
                }
            }
            None => (),
        }
    }
}

trait PrivateScriptTaskHelpers {
    fn click_event_filter_by_disabled_state(&self) -> bool;
}

impl<'a> PrivateScriptTaskHelpers for JSRef<'a, Node> {
    fn click_event_filter_by_disabled_state(&self) -> bool {
        match self.type_id() {
            NodeTypeId::Element(ElementTypeId::HTMLElement(HTMLElementTypeId::HTMLButtonElement)) |
            NodeTypeId::Element(ElementTypeId::HTMLElement(HTMLElementTypeId::HTMLInputElement)) |
            // NodeTypeId::Element(ElementTypeId::HTMLKeygenElement) |
            NodeTypeId::Element(ElementTypeId::HTMLElement(HTMLElementTypeId::HTMLOptionElement)) |
            NodeTypeId::Element(ElementTypeId::HTMLElement(HTMLElementTypeId::HTMLSelectElement)) |
            NodeTypeId::Element(ElementTypeId::HTMLElement(HTMLElementTypeId::HTMLTextAreaElement)) if self.get_disabled_state() => true,
            _ => false
        }
    }
}

impl ScriptTaskFactory for ScriptTask {
    fn create_layout_channel(_phantom: Option<&mut ScriptTask>) -> OpaqueScriptLayoutChannel {
        let (chan, port) = channel();
        ScriptLayoutChan::new(chan, port)
    }

    fn clone_layout_channel(_phantom: Option<&mut ScriptTask>, pair: &OpaqueScriptLayoutChannel) -> Box<Any+Send> {
        box pair.sender() as Box<Any+Send>
    }

    fn create<C>(_phantom: Option<&mut ScriptTask>,
                 id: PipelineId,
                 compositor: C,
                 layout_chan: &OpaqueScriptLayoutChannel,
                 control_chan: ScriptControlChan,
                 control_port: Receiver<ConstellationControlMsg>,
                 constellation_chan: ConstellationChan,
                 failure_msg: Failure,
                 resource_task: ResourceTask,
                 storage_task: StorageTask,
                 image_cache_task: ImageCacheTask,
                 devtools_chan: Option<DevtoolsControlChan>,
                 window_size: WindowSizeData,
                 load_data: LoadData)
                 where C: ScriptListener + Send + 'static {
        let ConstellationChan(const_chan) = constellation_chan.clone();
        let (script_chan, script_port) = channel();
        let layout_chan = LayoutChan(layout_chan.sender());
        spawn_named_with_send_on_failure("ScriptTask", task_state::SCRIPT, move || {
            let script_task = ScriptTask::new(box compositor as Box<ScriptListener>,
                                              script_port,
                                              NonWorkerScriptChan(script_chan),
                                              control_chan,
                                              control_port,
                                              constellation_chan,
                                              resource_task,
                                              storage_task,
                                              image_cache_task,
                                              devtools_chan);
            SCRIPT_TASK.with(|ref r| {
                *r.borrow_mut() = Some(&script_task as *const _);
            });
            let mut failsafe = ScriptMemoryFailsafe::new(&script_task);

            let new_load = InProgressLoad::new(id, None, layout_chan, window_size,
                                               load_data.url.clone());
            script_task.start_page_load(new_load, load_data);

            script_task.start();

            SCRIPT_TASK.with(|ref r| {
                *r.borrow_mut() = None;
            });
            // This must always be the very last operation performed before the task completes
            failsafe.neuter();
        }, ConstellationMsg::Failure(failure_msg), const_chan);
    }
}

unsafe extern "C" fn debug_gc_callback(_rt: *mut JSRuntime, status: JSGCStatus) {
    match status {
        JSGC_BEGIN => task_state::enter(task_state::IN_GC),
        JSGC_END   => task_state::exit(task_state::IN_GC),
        _ => (),
    }
}

impl ScriptTask {
    fn get() -> *const ScriptTask {
        SCRIPT_TASK.with(|ref r| {
            r.borrow().unwrap()
        })
    }

    /// Find an active matching frame in the current frame tree and pass it as an argument to
    /// the provided callback.
    pub fn find_frame<F,R>(id: PipelineId, sub: Option<SubpageId>, f: F) -> Option<R> where F: Fn(&Frame) -> R {
        unsafe {
            (*ScriptTask::get()).frame_tree.borrow().find(id, sub).map(|frame| f(frame))
        }
    }

    /// Creates a new script task.
    pub fn new(compositor: Box<ScriptListener+'static>,
               port: Receiver<ScriptMsg>,
               chan: NonWorkerScriptChan,
               control_chan: ScriptControlChan,
               control_port: Receiver<ConstellationControlMsg>,
               constellation_chan: ConstellationChan,
               resource_task: ResourceTask,
               storage_task: StorageTask,
               img_cache_task: ImageCacheTask,
               devtools_chan: Option<DevtoolsControlChan>)
               -> ScriptTask {
        let (js_runtime, js_context) = ScriptTask::new_rt_and_cx();
        let wrap_for_same_compartment = wrap_for_same_compartment as
            unsafe extern "C" fn(*mut JSContext, *mut JSObject) -> *mut JSObject;
        let pre_wrap = pre_wrap as
            unsafe extern fn(*mut JSContext, *mut JSObject, *mut JSObject,
                             libc::c_uint) -> *mut JSObject;

        unsafe {
            // JS_SetWrapObjectCallbacks clobbers the existing wrap callback,
            // and JSCompartment::wrap crashes if that happens. The only way
            // to retrieve the default callback is as the result of
            // JS_SetWrapObjectCallbacks, which is why we call it twice.
            let callback = JS_SetWrapObjectCallbacks((*js_runtime).ptr,
                                                     None,
                                                     Some(wrap_for_same_compartment),
                                                     None);
            JS_SetWrapObjectCallbacks((*js_runtime).ptr,
                                      callback,
                                      Some(wrap_for_same_compartment),
                                      Some(pre_wrap));
        }

        let (devtools_sender, devtools_receiver) = channel();
        ScriptTask {
            frame_tree: DOMRefCell::new(FrameTree::new()),
            frame_cache: DOMRefCell::new(FrameCache::new()),
            incomplete_loads: DOMRefCell::new(vec!()),

            image_cache_task: img_cache_task,
            resource_task: resource_task,
            storage_task: storage_task,

            port: port,
            chan: chan,
            control_chan: control_chan,
            control_port: control_port,
            constellation_chan: constellation_chan,
            compositor: DOMRefCell::new(compositor),
            devtools_chan: devtools_chan,
            devtools_port: devtools_receiver,
            devtools_sender: devtools_sender,

            js_runtime: js_runtime,
            js_context: DOMRefCell::new(Some(js_context)),
            mouse_over_targets: DOMRefCell::new(None)
        }
    }

    pub fn new_rt_and_cx() -> (js::rust::rt, Rc<Cx>) {
        LiveDOMReferences::initialize();
        let js_runtime = js::rust::rt();
        assert!({
            let ptr: *mut JSRuntime = (*js_runtime).ptr;
            !ptr.is_null()
        });

        unsafe {
            JS_SetExtraGCRootsTracer((*js_runtime).ptr,
                                     Some(trace_frames as unsafe extern "C" fn(*mut js::jsapi::JSTracer, *mut libc::types::common::c95::c_void)),
                                     ptr::null_mut());
        }

        // Unconstrain the runtime's threshold on nominal heap size, to avoid
        // triggering GC too often if operating continuously near an arbitrary
        // finite threshold. This leaves the maximum-JS_malloc-bytes threshold
        // still in effect to cause periodical, and we hope hygienic,
        // last-ditch GCs from within the GC's allocator.
        unsafe {
            JS_SetGCParameter(js_runtime.ptr, JSGC_MAX_BYTES, u32::MAX);
        }

        let js_context = js_runtime.cx();
        assert!({
            let ptr: *mut JSContext = (*js_context).ptr;
            !ptr.is_null()
        });
        js_context.set_default_options_and_version();
        js_context.set_logging_error_reporter();
        unsafe {
            JS_SetGCZeal((*js_context).ptr, 0, JS_DEFAULT_ZEAL_FREQ);
        }

        // Needed for debug assertions about whether GC is running.
        if !cfg!(ndebug) {
            unsafe {
                JS_SetGCCallback(js_runtime.ptr,
                    Some(debug_gc_callback as unsafe extern "C" fn(*mut JSRuntime, JSGCStatus)));
            }
        }

        (js_runtime, js_context)
    }

    pub fn get_cx(&self) -> *mut JSContext {
        (**self.js_context.borrow().as_ref().unwrap()).ptr
    }

    /// Starts the script task. After calling this method, the script task will loop receiving
    /// messages on its port.
    pub fn start(&self) {
        while self.handle_msgs() {
            // Go on...
        }
    }

    fn for_all_known_windows<F>(&self, mut f: F) where F: FnMut(JSRef<Window>) {
        self.frame_cache.borrow().for_all_windows(&mut f);
        self.frame_tree.borrow().for_all_windows(f);
    }

    fn find_active_pipeline(&self, pipeline: PipelineId) -> Option<Temporary<Document>> {
        self.frame_tree.borrow().find(pipeline, None).map(|frame| {
            frame.context.active_document()
        })
    }

    fn find_any_matching_pipeline(&self, pipeline: PipelineId) -> Option<Temporary<Document>> {
        let doc = self.find_active_pipeline(pipeline);
        if doc.is_some() {
            return doc;
        }
        self.frame_cache.borrow().find(pipeline).map(|frame| {
            Temporary::new(frame.document.clone())
        })
    }

    /// Handle incoming control messages.
    fn handle_msgs(&self) -> bool {
        let roots = RootCollection::new();
        let _stack_roots_tls = StackRootTLS::new(&roots);

        // Handle pending resize events.
        // Gather them first to avoid a double mut borrow on self.
        let mut resizes = vec!();

        {
            self.for_all_known_windows(|win| {
                // Only process a resize if layout is idle.
                if win.layout_is_idle() {
                    if let Some(size) = win.steal_resize_event() {
                        resizes.push((win.pipeline(), size));
                    }
                }
            });
        }

        for (id, size) in resizes.into_iter() {
            self.handle_event(id, ResizeEvent(size));
        }

        enum MixedMessage {
            FromConstellation(ConstellationControlMsg),
            FromScript(ScriptMsg),
            FromDevtools(DevtoolScriptControlMsg),
        }

        // Store new resizes, and gather all other events.
        let mut sequential = vec!();

        // Receive at least one message so we don't spinloop.
        let mut event = {
            let sel = Select::new();
            let mut port1 = sel.handle(&self.port);
            let mut port2 = sel.handle(&self.control_port);
            let mut port3 = sel.handle(&self.devtools_port);
            unsafe {
                port1.add();
                port2.add();
                if self.devtools_chan.is_some() {
                    port3.add();
                }
            }
            let ret = sel.wait();
            if ret == port1.id() {
                MixedMessage::FromScript(self.port.recv().unwrap())
            } else if ret == port2.id() {
                MixedMessage::FromConstellation(self.control_port.recv().unwrap())
            } else if ret == port3.id() {
                MixedMessage::FromDevtools(self.devtools_port.recv().unwrap())
            } else {
                panic!("unexpected select result")
            }
        };

        // Squash any pending resize and reflow events in the queue.
        loop {
            match event {
                // This has to be handled before the ResizeMsg below,
                // otherwise the page may not have been added to the
                // child list yet, causing the find() to fail.
                MixedMessage::FromConstellation(ConstellationControlMsg::AttachLayout(new_layout_info)) => {
                    self.handle_new_layout(new_layout_info);
                }
                MixedMessage::FromConstellation(ConstellationControlMsg::Resize(id, size)) => {
                    self.handle_resize(id, size);
                }
                MixedMessage::FromConstellation(ConstellationControlMsg::Viewport(id, rect)) => {
                    self.handle_viewport(id, rect);
                }
                _ => {
                    sequential.push(event);
                }
            }

            // If any of our input sources has an event pending, we'll perform another iteration
            // and check for more resize events. If there are no events pending, we'll move
            // on and execute the sequential non-resize events we've seen.
            match self.control_port.try_recv() {
                Err(_) => match self.port.try_recv() {
                    Err(_) => match self.devtools_port.try_recv() {
                        Err(_) => break,
                        Ok(ev) => event = MixedMessage::FromDevtools(ev),
                    },
                    Ok(ev) => event = MixedMessage::FromScript(ev),
                },
                Ok(ev) => event = MixedMessage::FromConstellation(ev),
            }
        }

        // Process the gathered events.
        for msg in sequential.into_iter() {
            match msg {
                MixedMessage::FromConstellation(ConstellationControlMsg::ExitPipeline(id, exit_type)) => {
                    if self.handle_exit_pipeline_msg(id, exit_type) {
                        return false
                    }
                },
                MixedMessage::FromConstellation(inner_msg) => self.handle_msg_from_constellation(inner_msg),
                MixedMessage::FromScript(inner_msg) => self.handle_msg_from_script(inner_msg),
                MixedMessage::FromDevtools(inner_msg) => self.handle_msg_from_devtools(inner_msg),
            }
        }

        true
    }

    fn handle_msg_from_constellation(&self, msg: ConstellationControlMsg) {
        match msg {
            ConstellationControlMsg::AttachLayout(_) =>
                panic!("should have handled AttachLayout already"),
            ConstellationControlMsg::MakeActive(id, parent) =>
                self.handle_make_active(id, parent),
            ConstellationControlMsg::MakeInactive(id) =>
                self.handle_make_inactive(id),
            ConstellationControlMsg::SendEvent(id, event) =>
                self.handle_event(id, event),
            ConstellationControlMsg::ReflowComplete(id, reflow_id) =>
                self.handle_reflow_complete_msg(id, reflow_id),
            ConstellationControlMsg::ResizeInactive(id, new_size) =>
                self.handle_resize_inactive_msg(id, new_size),
            ConstellationControlMsg::Viewport(..) =>
                panic!("should have handled Viewport already"),
            ConstellationControlMsg::Resize(..) =>
                panic!("should have handled Resize already"),
            ConstellationControlMsg::ExitPipeline(..) =>
                panic!("should have handled ExitPipeline already"),
            ConstellationControlMsg::GetTitle(pipeline_id) =>
                self.handle_get_title_msg(pipeline_id),
        }
    }

    fn handle_msg_from_script(&self, msg: ScriptMsg) {
        match msg {
            ScriptMsg::TriggerLoad(id, load_data) =>
                self.trigger_load(id, load_data),
            ScriptMsg::TriggerFragment(id, url) =>
                self.trigger_fragment(id, url),
            ScriptMsg::FireTimer(TimerSource::FromWindow(id), timer_id) =>
                self.handle_fire_timer_msg(id, timer_id),
            ScriptMsg::FireTimer(TimerSource::FromWorker, _) =>
                panic!("Worker timeouts must not be sent to script task"),
            ScriptMsg::Navigate(direction) =>
                self.handle_navigate_msg(direction),
            ScriptMsg::ExitWindow(id) =>
                self.handle_exit_window_msg(id),
            ScriptMsg::DOMMessage(..) =>
                panic!("unexpected message"),
            ScriptMsg::WorkerDispatchErrorEvent(addr, msg, file_name,line_num, col_num) =>
                Worker::handle_error_message(addr, msg, file_name, line_num, col_num),
            ScriptMsg::RunnableMsg(runnable) =>
                runnable.handler(),
            ScriptMsg::RefcountCleanup(addr) =>
                LiveDOMReferences::cleanup(self.get_cx(), addr),
            ScriptMsg::PageFetchComplete(id, subpage, response) =>
                self.handle_page_fetch_complete(id, subpage, response),
        }
    }

    fn handle_msg_from_devtools(&self, msg: DevtoolScriptControlMsg) {
        let frame_tree = self.frame_tree.borrow();
        match msg {
            EvaluateJS(id, s, reply) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_evaluate_js(context, s, reply)
            }
            GetRootNode(id, reply) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_get_root_node(context, reply)
            }
            GetDocumentElement(id, reply) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_get_document_element(context, reply)
            }
            GetChildren(id, node_id, reply) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_get_children(context, node_id, reply)
            }
            GetLayout(id, node_id, reply) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_get_layout(context, node_id, reply)
            }
            ModifyAttribute(id, node_id, modifications) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_modify_attribute(context, node_id, modifications)
            }
            WantsLiveNotifications(id, to_send) => {
                let context = &frame_tree.get(id).context;
                devtools::handle_wants_live_notifications(context, to_send)
            }
        }
    }

    fn handle_resize(&self, id: PipelineId, size: WindowSizeData) {
        if let Some(doc) = self.find_any_matching_pipeline(id) {
            let doc = doc.root();
            let win = doc.r().window().root();
            win.r().set_resize_event(size);
            return;
        }
        let mut loads = self.incomplete_loads.borrow_mut();
        if let Some(ref mut load) = loads.iter_mut().find(|load| load.pipeline_id == id) {
            load.window_size = size;
            return;
        }
        panic!("resize sent to nonexistent pipeline");
    }

    fn handle_viewport(&self, id: PipelineId, rect: Rect<f32>) {
        if let Some(doc) = self.find_any_matching_pipeline(id) {
            let doc = doc.root();
            let window = window_from_node(doc.r()).root();
            if window.r().set_page_clip_rect_with_new_viewport(rect) {
                self.force_reflow(doc.r());
            }
            return;
        }

        let mut loads = self.incomplete_loads.borrow_mut();
        if let Some(ref mut load) = loads.iter_mut().find(|load| load.pipeline_id == id) {
            load.clip_rect = Some(rect);
            return;
        }
        panic!("Page rect message sent to nonexistent pipeline");
    }

    fn handle_new_layout(&self, new_layout_info: NewLayoutInfo) {
        let NewLayoutInfo {
            new_pipeline_id,
            subpage_id,
            layout_chan,
            window_size,
            load_data
        } = new_layout_info;

        let chan = layout_chan.downcast_ref::<Sender<layout_interface::Msg>>().unwrap();
        let layout_chan = LayoutChan(chan.clone());
        let subpage_id = (subpage_id.0, (subpage_id.1, subpage_id.2));
        let new_load = InProgressLoad::new(new_pipeline_id, Some(subpage_id), layout_chan,
                                           window_size, load_data.url.clone());
        self.start_page_load(new_load, load_data);
    }

    /// Handles a timer that fired.
    fn handle_fire_timer_msg(&self, id: PipelineId, timer_id: TimerId) {
        let doc = self.find_active_pipeline(id)
                      .expect("ScriptTask: received fire timer msg for a pipeline ID \
                               not associated with this script task. This is a bug.").root();
        let window = window_from_node(doc.r()).root();
        window.r().handle_fire_timer(timer_id);
    }

    /// Handles a notification that reflow completed.
    fn handle_reflow_complete_msg(&self, pipeline_id: PipelineId, reflow_id: uint) {
        debug!("Script: Reflow {:?} complete for {:?}", reflow_id, pipeline_id);
        let doc = self.find_active_pipeline(pipeline_id).expect(
            "ScriptTask: received a load message for a layout channel that is not associated \
             with this script task. This is a bug.").root();
        let window = window_from_node(doc.r()).root();
        window.r().handle_reflow_complete_msg(reflow_id);

        self.compositor.borrow_mut().set_ready_state(pipeline_id, FinishedLoading);
    }

    /// Handles a navigate forward or backward message.
    /// TODO(tkuehn): is it ever possible to navigate only on a subframe?
    fn handle_navigate_msg(&self, direction: NavigationDirection) {
        let ConstellationChan(ref chan) = self.constellation_chan;
        chan.send(ConstellationMsg::Navigate(direction)).unwrap();
    }

    /// Window was resized, but this script was not active, so don't reflow yet
    fn handle_resize_inactive_msg(&self, id: PipelineId, new_size: WindowSizeData) {
        let doc = self.find_any_matching_pipeline(id)
                      .expect("Received resize message for PipelineId not associated \
                               with a page in the page tree. This is a bug.").root();
        let window = window_from_node(doc.r()).root();
        window.r().handle_resize_inactive_msg(new_size);
    }

    /// We have gotten a window.close from script, which we pass on to the compositor.
    /// We do not shut down the script task now, because the compositor will ask the
    /// constellation to shut down the pipeline, which will clean everything up
    /// normally. If we do exit, we will tear down the DOM nodes, possibly at a point
    /// where layout is still accessing them.
    fn handle_exit_window_msg(&self, _: PipelineId) {
        debug!("script task handling exit window msg");

        // TODO(tkuehn): currently there is only one window,
        // so this can afford to be naive and just shut down the
        // compositor. In the future it'll need to be smarter.
        self.compositor.borrow_mut().close();
    }

    fn handle_page_fetch_complete(&self, id: PipelineId, subpage: Option<SubpageId>,
                                  response: LoadResponse) {
        let idx = self.incomplete_loads.borrow().iter().position(|&:load| {
            load.pipeline_id == id && load.subpage_id.map(|sub| (sub.1).0) == subpage
        }).unwrap();
        let load = self.incomplete_loads.borrow_mut().remove(idx);
        self.load(response, load);
    }

    /// Handles a request for the window title.
    fn handle_get_title_msg(&self, pipeline_id: PipelineId) {
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        doc.r().send_title_to_compositor();
    }

    /// Handles a request to exit the script task and shut down layout.
    /// Returns true if the script task should shut down and false otherwise.
    fn handle_exit_pipeline_msg(&self, id: PipelineId, exit_type: PipelineExitType) -> bool {
        // If root is being exited, shut down all pages
        let root_id = self.frame_tree.borrow().root_context().map(|c| c.active_pipeline());
        if root_id == Some(id) {
            debug!("shutting down layout for root page {:?}", id);
            *self.js_context.borrow_mut() = None;
        }

        // otherwise find just the matching page and exit all sub-pages
        self.shut_down_layout(id, (*self.js_runtime).ptr, exit_type);
        return !self.frame_tree.borrow().has_root() && self.frame_cache.borrow().empty();
    }

    /// The entry point to document loading. Defines bindings, sets up the window and document
    /// objects, parses HTML and CSS, and kicks off initial layout.
    fn load(&self,
            response: LoadResponse,
            incomplete: InProgressLoad) {
        let final_url = response.metadata.final_url.clone();
        debug!("ScriptTask: loading {:?} on page {:?}", incomplete.url, incomplete.pipeline_id);

        assert!(incomplete.subpage_id.is_none() || self.frame_tree.borrow().has_root());

        let frame_element = incomplete.subpage_id.and_then(|(parent_id, (subpage_id, _))| {
          // In the case a parent id exists but the matching page
          // cannot be found, this means the page exists in a different
          // script task (due to origin) so it shouldn't be returned.
          // TODO: window.parent will continue to return self in that
          // case, which is wrong. We should be returning an object that
          // denies access to most properties (per
          // https://github.com/servo/servo/issues/3939#issuecomment-62287025).
          self.find_active_pipeline(parent_id).and_then(|doc| {
            let match_iframe = |&:&node: &JSRef<HTMLIFrameElement>| {
              node.subpage_id().map_or(false, |id| id == subpage_id)
            };

            let doc = doc.root();
            let doc: JSRef<Node> = NodeCast::from_ref(doc.r());

            doc.traverse_preorder()
               .filter_map(|node| HTMLIFrameElementCast::to_ref(node))
               .find(match_iframe)
               .map(|node| Temporary::from_rooted(ElementCast::from_ref(node)))
          })
        }).root();

        let is_javascript = incomplete.url.scheme.as_slice() == "javascript";

        self.compositor.borrow_mut().set_ready_state(incomplete.pipeline_id, Loading);

        let last_modified = response.metadata.headers.as_ref().and_then(|headers| {
            headers.get().map(|&LastModified(ref tm)| tm.clone())
        });

        let cx = self.js_context.borrow();
        let cx = cx.as_ref().unwrap();

        // Create the window and document objects.
        let window = Window::new(cx.clone(),
                                 self.chan.clone(),
                                 self.control_chan.clone(),
                                 self.compositor.borrow_mut().dup(),
                                 self.image_cache_task.clone(),
                                 self.resource_task.clone(),
                                 self.storage_task.clone(),
                                 self.devtools_chan.clone(),
                                 self.constellation_chan.clone(),
                                 incomplete.layout_chan,
                                 incomplete.pipeline_id,
                                 incomplete.subpage_id.map(|p| (p.1).0),
                                 incomplete.window_size).root();

        let document = Document::new(window.r(), Some(final_url.clone()),
                                     IsHTMLDocument::HTMLDocument, None,
                                     DocumentSource::FromParser).root();
        if let Some(tm) = last_modified {
            document.r().set_last_modified(dom_last_modified(&tm));
        }

        let root_context = self.frame_tree.borrow().root_context();
        let context = match (incomplete.subpage_id, root_context) {
            // We're replacing an existing subframe.
            (Some((parent_pipeline, (_, Some(old_subpage)))), Some(_)) => {
                let mut frame_tree = self.frame_tree.borrow_mut();
                let mut frame_cache = self.frame_cache.borrow_mut();
                let context = {
                    let frame = frame_tree.find(parent_pipeline, Some(old_subpage)).unwrap();
                    frame.context.clone()
                };
                frame_cache.add(frame_tree.remove(context.active_pipeline()));
                context.replace_active_document(document.r());
                frame_tree.add(Frame::new(context.clone()), Some(parent_pipeline));
                context
            }
            // Either we're attaching a new subframe or there's no root frame,
            // so we need a new browsing context.
            (Some(_), Some(_)) | (None, None) => {
                let context = Rc::new(BrowserContext::new(document.r(), frame_element.r()));
                self.frame_tree.borrow_mut().add(Frame::new(context.clone()),
                                                 incomplete.subpage_id.map(|(a, _)| a));
                context
            }
            // We're replacing the content in the root browsing context.
            (None, Some(context)) => {
                let mut frame_tree = self.frame_tree.borrow_mut();
                let mut frame_cache = self.frame_cache.borrow_mut();
                frame_cache.add(frame_tree.remove(context.active_pipeline()));
                context.replace_active_document(document.r());
                frame_tree.add(Frame::new(context.clone()), None);
                context
            }
            (Some(_), None) =>
                panic!("We should never end up with a subframe replacement and no root frame."),
        };
        window.r().init_browser_context(Some(context));

        let parse_input = if is_javascript {
            assert!(final_url.serialize() == "about:blank");
            parse_html(document.r(), HTMLInput::InputUrl(response), &final_url);

            let evalstr = incomplete.url.non_relative_scheme_data().unwrap();
            let jsval = window.r().evaluate_js_on_global_with_result(evalstr);
            let strval = FromJSValConvertible::from_jsval(self.get_cx(), jsval,
                                                          StringificationBehavior::Empty);
            HTMLInput::InputString(strval.unwrap_or("".to_owned()))
        } else {
            HTMLInput::InputUrl(response)
        };

        parse_html(document.r(), parse_input, &final_url);

        document.r().set_ready_state(DocumentReadyState::Interactive);
        self.compositor.borrow_mut().set_ready_state(incomplete.pipeline_id, PerformingLayout);

        // Kick off the initial reflow of the page.
        debug!("kicking off initial reflow of {:?}", final_url);
        document.r().content_changed(NodeCast::from_ref(document.r()),
                                     NodeDamage::OtherNodeDamage);
        window.r().flush_layout(ReflowGoal::ForDisplay, ReflowQueryType::NoQuery);

        // https://html.spec.whatwg.org/multipage/#the-end step 4
        let addr: Trusted<Document> = Trusted::new(self.get_cx(), document.r(), self.chan.clone());

        self.chan.send(ScriptMsg::RunnableMsg(box DocumentProgressHandler {
            addr: addr.clone(),
            task: DocumentProgressTask::DOMContentLoaded,
        }));

        // We have no concept of a document loader right now, so just dispatch the
        // "load" event as soon as we've finished executing all scripts parsed during
        // the initial load.

        // https://html.spec.whatwg.org/multipage/#the-end step 7
        self.chan.send(ScriptMsg::RunnableMsg(box DocumentProgressHandler {
            addr: addr,
            task: DocumentProgressTask::Load,
        }));

        window.r().set_fragment_name(final_url.fragment.clone());

        let ConstellationChan(ref chan) = self.constellation_chan;
        chan.send(ConstellationMsg::LoadComplete).unwrap();

        // Notify devtools that a new script global exists.
        match self.devtools_chan {
            None => {}
            Some(ref chan) => {
                let page_info = DevtoolsPageInfo {
                    title: document.r().Title(),
                    url: final_url
                };
                chan.send(NewGlobal(incomplete.pipeline_id, self.devtools_sender.clone(),
                                    page_info)).unwrap();
            }
        }
    }

    fn scroll_fragment_point(&self, pipeline_id: PipelineId, node: JSRef<Element>) {
        let node: JSRef<Node> = NodeCast::from_ref(node);
        let rect = node.get_bounding_content_box();
        let point = Point2D(to_frac_px(rect.origin.x).to_f32().unwrap(),
                            to_frac_px(rect.origin.y).to_f32().unwrap());
        // FIXME(#2003, pcwalton): This is pretty bogus when multiple layers are involved.
        // Really what needs to happen is that this needs to go through layout to ask which
        // layer the element belongs to, and have it send the scroll message to the
        // compositor.
        self.compositor.borrow_mut().scroll_fragment_point(pipeline_id, LayerId::null(), point);
    }

    /// Reflows non-incrementally.
    fn force_reflow(&self, doc: JSRef<Document>) {
        doc.dirty_all_nodes();
        let win = doc.window().root();
        win.r().reflow(ReflowGoal::ForDisplay,
                       ReflowQueryType::NoQuery);
    }

    #[allow(unrooted_must_root)]
    fn handle_make_active(&self, pipeline_id: PipelineId, parent: Option<PipelineId>) {
        if self.frame_tree.borrow().find(pipeline_id, None).is_some() {
            // In all likelihood we're navigating through session history, so we're trying
            // to activate a frame which is already active.
            return;
        }

        // If we're changing the active root frame, just stash the current active frame.
        if parent.is_none() {
            let active = self.frame_tree.borrow().root_context().unwrap().active_pipeline();
            let old_frame = self.frame_tree.borrow_mut().remove(active);
            self.frame_cache.borrow_mut().add(old_frame);
        }

        // TODO: Since we lack subframe session history, this currently only supports
        //       recreating a subframe tree under the assumption that there are no
        //       existing subframes to replace.

        // We do not throw away frames yet, so there's no reason it should not be found.
        let new_frame = self.frame_cache.borrow_mut().remove(pipeline_id).unwrap();

        let new_doc = new_frame.document.root();
        let new_win = new_doc.r().window().root();
        let new_context = new_win.r().browser_context();
        self.frame_tree.borrow_mut().add(Frame::new(new_context), parent);
    }

    #[allow(unrooted_must_root)]
    fn handle_make_inactive(&self, pipeline_id: PipelineId) {
        // We should never attempt to make an inactive frame active.
        assert!(self.frame_tree.borrow().find(pipeline_id, None).is_some());

        let old_frame = self.frame_tree.borrow_mut().remove(pipeline_id);
        assert!(self.frame_tree.borrow().has_root());
        self.frame_cache.borrow_mut().add(old_frame);
    }

    /// This is the main entry point for receiving and dispatching DOM events.
    ///
    /// TODO: Actually perform DOM event dispatch.
    fn handle_event(&self, pipeline_id: PipelineId, event: CompositorEvent) {
        match event {
            ResizeEvent(new_size) => {
                self.handle_resize_event(pipeline_id, new_size);
            }

            ReflowEvent(nodes) => {
                // FIXME(pcwalton): This event seems to only be used by the image cache task, and
                // the interaction between it and the image holder is really racy. I think that, in
                // order to fix this race, we need to rewrite the image cache task to make the
                // image holder responsible for the lifecycle of image loading instead of having
                // the image holder and layout task both be observers. Then we can have the DOM
                // image element observe the state of the image holder and have it send reflows
                // via the normal dirtying mechanism, and ultimately remove this event.
                //
                // See the implementation of `Width()` and `Height()` in `HTMLImageElement` for
                // fallout of this problem.
                for node in nodes.iter() {
                    let node_to_dirty = node::from_untrusted_node_address(self.js_runtime.ptr,
                                                                          *node).root();
                    let document = self.find_active_pipeline(pipeline_id).unwrap().root();
                    document.r().content_changed(node_to_dirty.r(),
                                                 NodeDamage::OtherNodeDamage);
                }

                self.handle_reflow_event(pipeline_id);
            }

            ClickEvent(_button, point) => {
                self.handle_click_event(pipeline_id, _button, point);
            }

            MouseDownEvent(..) => {}
            MouseUpEvent(..) => {}
            MouseMoveEvent(point) => {
                self.handle_mouse_move_event(pipeline_id, point);
            }

            KeyEvent(key, state, modifiers) => {
                self.dispatch_key_event(key, state, modifiers, pipeline_id);
            }
        }
    }

    /// The entry point for all key processing for web content
    fn dispatch_key_event(&self, key: Key,
                          state: KeyState,
                          modifiers: KeyModifiers,
                          pipeline_id: PipelineId) {
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        let focused = doc.r().get_focused_element().root();
        let body = doc.r().GetBody().root();
        let window = window_from_node(doc.r()).root();

        let target: JSRef<EventTarget> = match (&focused, &body) {
            (&Some(ref focused), _) => EventTargetCast::from_ref(focused.r()),
            (&None, &Some(ref body)) => EventTargetCast::from_ref(body.r()),
            (&None, &None) => EventTargetCast::from_ref(window.r()),
        };

        let ctrl = modifiers.contains(CONTROL);
        let alt = modifiers.contains(ALT);
        let shift = modifiers.contains(SHIFT);
        let meta = modifiers.contains(SUPER);

        let is_composing = false;
        let is_repeating = state == KeyState::Repeated;
        let ev_type = match state {
            KeyState::Pressed | KeyState::Repeated => "keydown",
            KeyState::Released => "keyup",
        }.to_owned();

        let props = KeyboardEvent::key_properties(key, modifiers);

        let keyevent = KeyboardEvent::new(window.r(), ev_type, true, true,
                                          Some(window.r()), 0,
                                          props.key.to_owned(), props.code.to_owned(),
                                          props.location, is_repeating, is_composing,
                                          ctrl, alt, shift, meta,
                                          None, props.key_code).root();
        let event = EventCast::from_ref(keyevent.r());
        let _ = target.DispatchEvent(event);
        let mut prevented = event.DefaultPrevented();

        // https://dvcs.w3.org/hg/dom3events/raw-file/tip/html/DOM3-Events.html#keys-cancelable-keys
        if state != KeyState::Released && props.is_printable() && !prevented {
            // https://dvcs.w3.org/hg/dom3events/raw-file/tip/html/DOM3-Events.html#keypress-event-order
            let event = KeyboardEvent::new(window.r(), "keypress".to_owned(),
                                           true, true, Some(window.r()),
                                           0, props.key.to_owned(), props.code.to_owned(),
                                           props.location, is_repeating, is_composing,
                                           ctrl, alt, shift, meta,
                                           props.char_code, 0).root();
            let ev = EventCast::from_ref(event.r());
            let _ = target.DispatchEvent(ev);
            prevented = ev.DefaultPrevented();
            // TODO: if keypress event is canceled, prevent firing input events
        }

        if !prevented {
            self.compositor.borrow_mut().send_key_event(key, state, modifiers);
        }

        // This behavior is unspecced
        // We are supposed to dispatch synthetic click activation for Space and/or Return,
        // however *when* we do it is up to us
        // I'm dispatching it after the key event so the script has a chance to cancel it
        // https://www.w3.org/Bugs/Public/show_bug.cgi?id=27337
        match key {
            Key::Space if !prevented && state == KeyState::Released => {
                let maybe_elem: Option<JSRef<Element>> = ElementCast::to_ref(target);
                maybe_elem.map(|el| el.as_maybe_activatable().map(|a| a.synthetic_click_activation(ctrl, alt, shift, meta)));
            }
            Key::Enter if !prevented && state == KeyState::Released => {
                let maybe_elem: Option<JSRef<Element>> = ElementCast::to_ref(target);
                maybe_elem.map(|el| el.as_maybe_activatable().map(|a| a.implicit_submission(ctrl, alt, shift, meta)));
            }
            _ => ()
        }

        window.r().flush_layout(ReflowGoal::ForDisplay, ReflowQueryType::NoQuery);
    }

    /// The entry point for content to notify that a new load has been requested
    /// for the given pipeline.
    fn trigger_load(&self, pipeline_id: PipelineId, load_data: LoadData) {
        let ConstellationChan(ref const_chan) = self.constellation_chan;
        const_chan.send(ConstellationMsg::LoadUrl(pipeline_id, load_data)).unwrap();
    }

    /// The entry point for content to notify that a fragment url has been requested
    /// for the given pipeline.
    fn trigger_fragment(&self, pipeline_id: PipelineId, url: Url) {
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        match doc.r().find_fragment_node(url.fragment.unwrap()).root() {
            Some(node) => {
                self.scroll_fragment_point(pipeline_id, node.r());
            }
            None => {}
        }
    }


    fn handle_resize_event(&self, pipeline_id: PipelineId, new_size: WindowSizeData) {
        let doc = self.find_any_matching_pipeline(pipeline_id).unwrap().root();
        let window = window_from_node(doc.r()).root();
        window.r().set_window_size(new_size);

        self.force_reflow(doc.r());

        let fragment_node = window.r().steal_fragment_name()
                                  .and_then(|name| doc.r().find_fragment_node(name))
                                  .root();
        match fragment_node {
            Some(node) => self.scroll_fragment_point(pipeline_id, node.r()),
            None => {}
        }

        // http://dev.w3.org/csswg/cssom-view/#resizing-viewports
        // https://dvcs.w3.org/hg/dom3events/raw-file/tip/html/DOM3-Events.html#event-type-resize
        let uievent = UIEvent::new(window.r(),
                                   "resize".to_owned(), false,
                                   false, Some(window.r()),
                                   0i32).root();
        let event: JSRef<Event> = EventCast::from_ref(uievent.r());

        let wintarget: JSRef<EventTarget> = EventTargetCast::from_ref(window.r());
        event.fire(wintarget);
    }

    fn handle_reflow_event(&self, pipeline_id: PipelineId) {
        debug!("script got reflow event");
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        self.force_reflow(doc.r());
    }

    fn handle_click_event(&self, pipeline_id: PipelineId, _button: uint, point: Point2D<f32>) {
        debug!("ClickEvent: clicked at {:?}", point);
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        match doc.r().hit_test(&point) {
            Some(node_address) => {
                debug!("node address is {:?}", node_address.0);

                let temp_node =
                    node::from_untrusted_node_address(
                        self.js_runtime.ptr, node_address).root();

                let maybe_elem: Option<JSRef<Element>> = ElementCast::to_ref(temp_node.r());
                let maybe_node = match maybe_elem {
                    Some(element) => Some(element),
                    None => temp_node.r().ancestors().filter_map(ElementCast::to_ref).next(),
                };

                match maybe_node {
                    Some(el) => {
                        let node: JSRef<Node> = NodeCast::from_ref(el);
                        debug!("clicked on {:?}", node.debug_str());
                        // Prevent click event if form control element is disabled.
                        if node.click_event_filter_by_disabled_state() { return; }
                        let window = window_from_node(doc.r()).root();
                        doc.r().begin_focus_transaction();

                        let event =
                            Event::new(GlobalRef::Window(window.r()),
                                       "click".to_owned(),
                                       EventBubbles::Bubbles,
                                       EventCancelable::Cancelable).root();
                        // https://dvcs.w3.org/hg/dom3events/raw-file/tip/html/DOM3-Events.html#trusted-events
                        event.r().set_trusted(true);
                        // https://html.spec.whatwg.org/multipage/interaction.html#run-authentic-click-activation-steps
                        el.authentic_click_activation(event.r());

                        doc.r().commit_focus_transaction();
                        window.r().flush_layout(ReflowGoal::ForDisplay, ReflowQueryType::NoQuery);
                    }
                    None => {}
                }
            }

            None => {}
        }
    }


    fn handle_mouse_move_event(&self, pipeline_id: PipelineId, point: Point2D<f32>) {
        let doc = self.find_active_pipeline(pipeline_id).unwrap().root();
        match doc.r().get_nodes_under_mouse(&point) {
            Some(node_address) => {
                let mut target_list = vec!();
                let mut target_compare = false;

                let mouse_over_targets = &mut *self.mouse_over_targets.borrow_mut();
                match *mouse_over_targets {
                    Some(ref mut mouse_over_targets) => {
                        for node in mouse_over_targets.iter_mut() {
                            let node = node.root();
                            node.r().set_hover_state(false);
                        }
                    }
                    None => {}
                }

                if node_address.len() > 0 {
                  let top_most_node =
                      node::from_untrusted_node_address(self.js_runtime.ptr, node_address[0]).root();

                      let x = point.x.to_i32().unwrap_or(0);
                      let y = point.y.to_i32().unwrap_or(0);

                      let window = window_from_node(doc.r()).root();
                      let mouse_event = MouseEvent::new(window.r(),
                          "mousemove".to_owned(),
                          true,
                          true,
                          Some(window.r()),
                          0i32,
                          x, y, x, y,
                          false, false, false, false,
                          0i16,
                          None).root();

                      let event: JSRef<Event> = EventCast::from_ref(mouse_event.r());
                      let target: JSRef<EventTarget> = EventTargetCast::from_ref(top_most_node.r());
                      event.fire(target);
                }

                for node_address in node_address.iter() {
                    let temp_node =
                        node::from_untrusted_node_address(self.js_runtime.ptr, *node_address).root();

                    let maybe_node = temp_node.r().ancestors().find(|node| node.is_element());
                    match maybe_node {
                        Some(node) => {
                            node.set_hover_state(true);
                            match *mouse_over_targets {
                                Some(ref mouse_over_targets) if !target_compare => {
                                    target_compare =
                                        !mouse_over_targets.contains(&JS::from_rooted(node));
                                }
                                _ => {}
                            }
                            target_list.push(JS::from_rooted(node));
                        }
                        None => {}
                    }
                }
                match *mouse_over_targets {
                    Some(ref mouse_over_targets) => {
                        if mouse_over_targets.len() != target_list.len() {
                            target_compare = true
                        }
                    }
                    None => target_compare = true,
                }

                if target_compare {
                    if mouse_over_targets.is_some() {
                        self.force_reflow(doc.r())
                    }
                    *mouse_over_targets = Some(target_list);
                }
            }

            None => {}
        }
    }

    fn start_page_load(&self, incomplete: InProgressLoad, mut load_data: LoadData) {
        let id = incomplete.pipeline_id.clone();
        let subpage = incomplete.subpage_id.clone().map(|p| (p.1).0);
        let script_chan = self.chan.clone();
        let resource_task = self.resource_task.clone();

        spawn_named(format!("fetch for {:?}", load_data.url), move || {
            if load_data.url.scheme.as_slice() == "javascript" {
                load_data.url = Url::parse("about:blank").unwrap();
            }

            let (input_chan, input_port) = channel();
            resource_task.send(ControlMsg::Load(NetLoadData {
                url: load_data.url,
                method: load_data.method,
                headers: Headers::new(),
                preserved_headers: load_data.headers,
                data: load_data.data,
                cors: None,
                consumer: input_chan,
            })).unwrap();

            let load_response = input_port.recv().unwrap();
            script_chan.send(ScriptMsg::PageFetchComplete(id, subpage, load_response));
        });

        self.incomplete_loads.borrow_mut().push(incomplete);
    }

    /// Shuts down layout for the given page tree.
    #[allow(unrooted_must_root)]
    fn shut_down_layout(&self, pipeline: PipelineId, rt: *mut JSRuntime,
                        exit_type: PipelineExitType) {
        let mut channels = vec!();

        {
            fn shut_down_window(win: JSRef<Window>, clear_context: bool) -> Option<LayoutChan> {
                // Tell the layout task to begin shutting down, and wait until it
                // processed this message.
                let (response_chan, response_port) = channel();
                let LayoutChan(ref chan) = win.layout_chan();
                debug!("sending prepare to exit");
                let task_present = chan.send(layout_interface::Msg::PrepareToExit(response_chan)).is_ok();
                if task_present {
                    response_port.recv().unwrap();
                }

                if clear_context {
                    win.browser_context().clear();
                }
                win.clear_js_context();
                if task_present {
                    Some(win.layout_chan())
                } else {
                    None
                }
            }

            /*let root_pipeline_matches = self.frame_tree.borrow()
                                                       .root_context()
                                                       .map(|c| c.active_pipeline() == pipeline)
                                                       .unwrap_or(false);*/
            if let Some(frame) = self.frame_tree.borrow_mut().maybe_remove(pipeline) {
                //let frame = self.frame_tree.borrow_mut().remove(pipeline);
                debug!("shutting down all child windows in frame tree for pipeline {:?}", pipeline);
                let mut f = |&mut :win: JSRef<Window>| {
                    if let Some(chan) = shut_down_window(win, true) {
                        channels.push(chan);
                    }
                };
                frame.for_all_windows(&mut f);
            }

            let mut frame_cache = self.frame_cache.borrow_mut();
            if let Some(frame) = frame_cache.remove(pipeline) {
                fn shut_down_doc(doc: JSRef<Document>, channels: &mut Vec<LayoutChan>) {
                    let win = doc.window().root();
                    if let Some(chan) = shut_down_window(win.r(), false) {
                        channels.push(chan);
                    }
                }

                debug!("shutting down window for cached pipeline {:?}", pipeline);
                let doc = frame.document.root();
                shut_down_doc(doc.r(), &mut channels);

                let mut children = frame.children;
                while let Some(child) = children.pop() {
                    debug!("shutting down child window for cached pipeline {:?}", child);
                    let frame = frame_cache.remove(child).unwrap();
                    let doc = frame.document.root();
                    shut_down_doc(doc.r(), &mut channels);
                    children.extend(frame.children.into_iter());
                }
            }
        }

        // Force a GC to make sure that our DOM reflectors are released before we tell
        // layout to exit.
        unsafe {
            JS_GC(rt);
        }

        // Destroy the layout task. If there were node leaks, layout will now crash safely.
        for LayoutChan(chan) in channels.into_iter() {
            debug!("shutting down a layout task");
            chan.send(layout_interface::Msg::ExitNow(exit_type)).ok();
        }
    }
}

//FIXME(seanmonstar): uplift to Hyper
#[derive(Clone)]
struct LastModified(pub Tm);

impl Header for LastModified {
    #[inline]
    fn header_name(_: Option<LastModified>) -> &'static str {
        "Last-Modified"
    }

    // Parses an RFC 2616 compliant date/time string,
    fn parse_header(raw: &[Vec<u8>]) -> Option<LastModified> {
        header_util::from_one_raw_str(raw).and_then(|s: String| {
            let s = s.as_slice();
            strptime(s, "%a, %d %b %Y %T %Z").or_else(|_| {
                strptime(s, "%A, %d-%b-%y %T %Z")
            }).or_else(|_| {
                strptime(s, "%c")
            }).ok().map(|tm| LastModified(tm))
        })
    }
}

impl HeaderFormat for LastModified {
    // a localized date/time string in a format suitable
    // for document.lastModified.
    fn fmt_header(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let LastModified(ref tm) = *self;
        match tm.tm_utcoff {
            0 => tm.rfc822().fmt(f),
            _ => tm.to_utc().rfc822().fmt(f)
        }
    }
}

fn dom_last_modified(tm: &Tm) -> String {
    format!("{}", tm.to_local().strftime("%m/%d/%Y %H:%M:%S").unwrap())
}

enum DocumentProgressTask {
    DOMContentLoaded,
    Load,
}

struct DocumentProgressHandler {
    addr: Trusted<Document>,
    task: DocumentProgressTask,
}

impl DocumentProgressHandler {
    fn dispatch_dom_content_loaded(&self) {
        let document = self.addr.to_temporary().root();
        let window = document.r().window().root();
        let event = Event::new(GlobalRef::Window(window.r()), "DOMContentLoaded".to_owned(),
                               EventBubbles::DoesNotBubble,
                               EventCancelable::NotCancelable).root();
        let doctarget: JSRef<EventTarget> = EventTargetCast::from_ref(document.r());
        let _ = doctarget.DispatchEvent(event.r());
    }

    fn set_ready_state_complete(&self) {
        let document = self.addr.to_temporary().root();
        document.r().set_ready_state(DocumentReadyState::Complete);
    }

    fn dispatch_load(&self) {
        let document = self.addr.to_temporary().root();
        let window = document.r().window().root();
        let event = Event::new(GlobalRef::Window(window.r()), "load".to_owned(),
                               EventBubbles::DoesNotBubble,
                               EventCancelable::NotCancelable).root();
        let wintarget: JSRef<EventTarget> = EventTargetCast::from_ref(window.r());
        let doctarget: JSRef<EventTarget> = EventTargetCast::from_ref(document.r());
        event.r().set_trusted(true);
        let _ = wintarget.dispatch_event_with_target(doctarget, event.r());
    }
}

impl Runnable for DocumentProgressHandler {
    fn handler(self: Box<DocumentProgressHandler>) {
        match self.task {
            DocumentProgressTask::DOMContentLoaded => {
                self.dispatch_dom_content_loaded();
            }
            DocumentProgressTask::Load => {
                self.set_ready_state_complete();
                self.dispatch_load();
            }
        }
    }
}

pub unsafe extern fn trace_frames(tracer: *mut JSTracer, _data: *mut libc::c_void) {
    let script_task = &*ScriptTask::get();
    script_task.frame_tree.borrow_for_gc_trace().trace(tracer);
    script_task.frame_cache.borrow_for_gc_trace().trace(tracer);
}
