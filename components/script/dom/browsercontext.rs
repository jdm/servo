/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use dom::bindings::conversions::unwrap_jsmanaged;
use dom::bindings::conversions::{ToJSValConvertible};
use dom::bindings::js::{JS, JSRef, Temporary, Root, MutNullableJS};
use dom::bindings::js::{OptionalRootable, OptionalRootedRootable, ResultRootable};
use dom::bindings::js::{OptionalRootedReference, OptionalOptionalRootedRootable};
use dom::bindings::proxyhandler::{get_property_descriptor, fill_property_descriptor};
use dom::bindings::utils::{Reflectable, WindowProxyHandler};
use dom::bindings::utils::get_array_index_from_id;
use dom::document::{Document, DocumentHelpers};
use dom::element::Element;
use dom::window::Window;
use dom::window::WindowHelpers;

use msg::constellation_msg::{PipelineId, SubpageId};

use js::jsapi::{JSContext, JSObject, jsid, JSPropertyDescriptor};
use js::jsapi::{JS_AlreadyHasOwnPropertyById, JS_ForwardGetPropertyTo};
use js::jsapi::{JS_GetPropertyDescriptorById, JS_DefinePropertyById};
use js::jsapi::{JS_SetPropertyById, JS_SetReservedSlot};
use js::jsval::{JSVal, ObjectValue};
use js::glue::{GetProxyPrivate};
use js::glue::{WrapperNew, CreateWrapperProxyHandler, ProxyTraps};
use js::rust::with_compartment;
use js::{JSRESOLVE_QUALIFIED, JSRESOLVE_ASSIGNING, JSSLOT_PROXY_PRIVATE};

use std::cell::Cell;
use std::ptr;

#[allow(raw_pointer_derive)]
#[jstraceable]
#[privatize]
#[must_root]
pub struct BrowserContext {
    active_document: MutNullableJS<Document>,
    pipeline: Cell<PipelineId>,
    subpage: Cell<Option<SubpageId>>,
    window_proxy: *mut JSObject,
    frame_element: MutNullableJS<Element>,
}

impl Drop for BrowserContext {
    fn drop(&mut self) {
        debug!("browser context for {:?} destroyed", self.pipeline.get());
    }
}

impl BrowserContext {
    #[allow(unrooted_must_root)]
    pub fn new(document: JSRef<Document>, frame_element: Option<JSRef<Element>>) -> BrowserContext {
        let win = document.window().root();
        BrowserContext {
            active_document: MutNullableJS::new(Some(document)),
            window_proxy: create_window_proxy(win.r()),
            frame_element: MutNullableJS::new(frame_element),
            pipeline: Cell::new(win.r().pipeline()),
            subpage: Cell::new(win.r().subpage()),
        }
    }

    pub fn clear(&self) {
        debug!("clearing browser context for {:?}", self.pipeline.get());
        self.active_document.clear();
        self.frame_element.clear();
    }

    pub fn active_document(&self) -> Temporary<Document> {
        self.active_document.get().unwrap()
    }

    pub fn active_window(&self) -> Temporary<Window> {
        let doc = self.active_document().root();
        doc.r().window()
    }

    pub fn frame_element(&self) -> Option<Temporary<Element>> {
        self.frame_element.get()
    }

    pub fn active_pipeline(&self) -> PipelineId {
        self.pipeline.get()
    }

    pub fn active_subpage(&self) -> Option<SubpageId> {
        self.subpage.get()
    }

    pub fn window_proxy(&self) -> *mut JSObject {
        assert!(!self.window_proxy.is_null());
        self.window_proxy
    }

    pub fn replace_active_document(&self, doc: JSRef<Document>) {
        self.active_document.assign(Some(doc));
        let win = doc.window().root();
        self.pipeline.set(win.r().pipeline());
        self.subpage.set(win.r().subpage());
        self.update_window_proxy(win.r());
    }

    #[allow(unsafe_blocks)]
    fn update_window_proxy(&self, win: JSRef<Window>) {
        unsafe {
            let reflector = ObjectValue(&*win.reflector().get_jsobject());
            JS_SetReservedSlot(self.window_proxy, JSSLOT_PROXY_PRIVATE, reflector);
        }
    }
}

#[allow(unsafe_blocks)]
fn create_window_proxy(win: JSRef<Window>) -> *mut JSObject {
    let WindowProxyHandler(handler) = win.windowproxy_handler();
    assert!(!handler.is_null());

    let parent = win.reflector().get_jsobject();
    let cx = win.get_cx();
    let wrapper = with_compartment(cx, parent, || unsafe {
        WrapperNew(cx, parent, handler)
    });
    assert!(!wrapper.is_null());
    wrapper
}

unsafe fn GetSubframeWindow(cx: *mut JSContext, proxy: *mut JSObject, id: jsid) -> Option<Temporary<Window>> {
    let index = get_array_index_from_id(cx, id);
    if let Some(index) = index {
        let target = GetProxyPrivate(proxy).to_object();
        let win: Root<Window> = unwrap_jsmanaged(target).unwrap().root();
        let mut found = false;
        return win.r().IndexedGetter(index, &mut found);
    }

    None
}

unsafe extern fn getOwnPropertyDescriptor(cx: *mut JSContext, proxy: *mut JSObject, id: jsid, set: bool, desc: *mut JSPropertyDescriptor) -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if let Some(window) = window {
        let window = window.root();
        (*desc).value = window.to_jsval(cx);
        fill_property_descriptor(&mut *desc, proxy, true);
        return true;
    }

    let target = GetProxyPrivate(proxy).to_object();
    let flags = if set { JSRESOLVE_ASSIGNING } else { 0 } | JSRESOLVE_QUALIFIED;
    // XXX This should be JS_GetOwnPropertyDescriptorById
    if JS_GetPropertyDescriptorById(cx, target, id, flags, desc) == 0 {
        return false;
    }

    if (*desc).obj != target {
        // Not an own property
        (*desc).obj = ptr::null_mut();
    } else {
        (*desc).obj = proxy;
    }

    true
}


unsafe extern fn defineProperty(cx: *mut JSContext, proxy: *mut JSObject, id: jsid, desc: *mut JSPropertyDescriptor) -> bool {
    if get_array_index_from_id(cx, id).is_some() {
        // Spec says to Reject whether this is a supported index or not,
        // since we have no indexed setter or indexed creator.  That means
        // throwing in strict mode (FIXME: Bug 828137), doing nothing in
        // non-strict mode.
        return true;
    }

    let target = GetProxyPrivate(proxy).to_object();
    JS_DefinePropertyById(cx, target, id, (*desc).value, (*desc).getter,
                          (*desc).setter, (*desc).attrs) != 0
}

unsafe extern fn hasOwn(cx: *mut JSContext, proxy: *mut JSObject, id: jsid, bp: *mut bool) -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if window.is_some() {
        *bp = true;
        return true;
    }

    let target = GetProxyPrivate(proxy).to_object();
    let mut found = 0;
    if JS_AlreadyHasOwnPropertyById(cx, target, id, &mut found) == 0 {
        return false;
    }

    *bp = found != 0;
    return true;
}

unsafe extern fn get(cx: *mut JSContext, proxy: *mut JSObject, receiver: *mut JSObject, id: jsid, vp: *mut JSVal) -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if let Some(window) = window {
        let window = window.root();
        *vp = window.to_jsval(cx);
        return true;
    }

    let target = GetProxyPrivate(proxy).to_object();
    JS_ForwardGetPropertyTo(cx, target, id, receiver, vp) != 0
}

unsafe extern fn set(cx: *mut JSContext, proxy: *mut JSObject, _receiver: *mut JSObject, id: jsid, _strict: bool, vp: *mut JSVal) -> bool {
    if get_array_index_from_id(cx, id).is_some() {
        // Reject (which means throw if and only if strict) the set.
        // FIXME: Throw
        return true;
    }

    // FIXME: The receiver should be used, we need something like JS_ForwardSetPropertyTo.
    let target = GetProxyPrivate(proxy).to_object();
    JS_SetPropertyById(cx, target, id, vp) != 0
}

static PROXY_HANDLER: ProxyTraps = ProxyTraps {
    getPropertyDescriptor: Some(get_property_descriptor
                                as unsafe extern "C" fn(*mut JSContext, *mut JSObject, jsid, bool, *mut JSPropertyDescriptor) -> bool),
    getOwnPropertyDescriptor: Some(getOwnPropertyDescriptor
                                   as unsafe extern "C" fn(*mut JSContext, *mut JSObject,
                                                           jsid, bool, *mut JSPropertyDescriptor)
                                                           -> bool),
    defineProperty: Some(defineProperty as unsafe extern "C" fn(*mut JSContext, *mut JSObject, jsid, *mut JSPropertyDescriptor) -> bool),
    getOwnPropertyNames: None,
    delete_: None,
    enumerate: None,

    has: None,
    hasOwn: Some(hasOwn as unsafe extern "C" fn(*mut JSContext, *mut JSObject, jsid, *mut bool) -> bool),
    get: Some(get as unsafe extern "C" fn(*mut JSContext, *mut JSObject, *mut JSObject, jsid, *mut JSVal) -> bool),
    set: Some(set as unsafe extern "C" fn(*mut JSContext, *mut JSObject, *mut JSObject, jsid, bool, *mut JSVal) -> bool),
    keys: None,
    iterate: None,

    call: None,
    construct: None,
    nativeCall: 0 as *const u8,
    hasInstance: None,
    typeOf: None,
    objectClassIs: None,
    obj_toString: None,
    fun_toString: None,
    //regexp_toShared: 0 as *u8,
    defaultValue: None,
    iteratorNext: None,
    finalize: None,
    getElementIfPresent: None,
    getPrototypeOf: None,
    trace: None
};

#[allow(unsafe_blocks)]
pub fn new_window_proxy_handler() -> WindowProxyHandler {
    unsafe {
        WindowProxyHandler(CreateWrapperProxyHandler(&PROXY_HANDLER))
    }
}
