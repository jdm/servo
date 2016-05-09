/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use dom::bindings::cell::DOMRefCell;
use dom::bindings::conversions::StringificationBehavior;
use dom::bindings::conversions::{ToJSValConvertible, root_from_handleobject, FromJSValConvertible};
use dom::bindings::js::{JS, Root, RootedReference, MutHeapJSVal};
use dom::bindings::proxyhandler::{fill_property_descriptor, get_property_descriptor};
use dom::bindings::reflector::{Reflectable, Reflector};
use dom::bindings::utils::WindowProxyHandler;
use dom::bindings::utils::get_array_index_from_id;
use dom::document::Document;
use dom::element::Element;
use dom::window::{Window, post_message};
use js::glue::{CreateWrapperProxyHandler, ProxyTraps, NewWindowProxy};
use js::glue::{GetProxyPrivate, SetProxyExtra};
use js::jsapi::JSPropertySpec;
use js::jsapi::{Handle, HandleId, HandleObject, HandleValue, JSAutoCompartment, JSAutoRequest};
use js::jsapi::{JSContext, JSPROP_READONLY, JSErrNum, JSObject, PropertyDescriptor, JS_DefinePropertyById};
use js::jsapi::{JSJitInfo, JSFunctionSpec, JSNativeWrapper, JS_SetReservedSlot, JS_GetReservedSlot};
use js::jsapi::{JSPROP_ENUMERATE, JSCLASS_RESERVED_SLOTS_SHIFT, JSPROP_SHARED};
use js::jsapi::{JS_ForwardGetPropertyTo, JS_ForwardSetPropertyTo, JS_GetClass, JSClass};
use js::jsapi::{JS_GetOwnPropertyDescriptorById, JS_HasPropertyById, MutableHandle, CallArgs};
use js::jsapi::{MutableHandleValue, ObjectOpResult, RootedObject, RootedValue, JS_NewObject};
use js::jsval::{UndefinedValue, PrivateValue, ObjectValue, JSVal};
use js::rust::{define_methods, define_properties};
use js::{JSCLASS_IS_GLOBAL, JSCLASS_RESERVED_SLOTS_MASK};
use libc;
use msg::constellation_msg::PipelineId;
use std::os;

static CROSS_ORIGIN_CLASS: JSClass = JSClass {
    name: b"Window\0" as *const u8 as *const libc::c_char,
    flags:
        // JSCLASS_HAS_RESERVED_SLOTS(1)
        (1 & JSCLASS_RESERVED_SLOTS_MASK) << JSCLASS_RESERVED_SLOTS_SHIFT,
    addProperty: None,
    delProperty: None,
    getProperty: None,
    setProperty: None,
    enumerate: None,
    resolve: None,
    mayResolve: None,
    finalize: None,
    call: None,
    hasInstance: None,
    construct: None,
    trace: None,
    reserved: [0 as *mut os::raw::c_void; 23]
};

#[dom_struct]
pub struct BrowsingContext {
    reflector: Reflector,
    history: DOMRefCell<Vec<SessionHistoryEntry>>,
    active_index: usize,
    frame_element: Option<JS<Element>>,
    #[ignore_heap_size_of = "spidermonkey type"]
    parent: MutHeapJSVal,
    parent_pipeline: Option<PipelineId>,
}

#[allow(unsafe_code)]
unsafe extern "C" fn parent_(_cx: *mut JSContext, argc: libc::c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    *args.rval() = args.thisv().get();
    assert!(!(*args.rval()).is_undefined());
    return true;
}

#[allow(unsafe_code)]
unsafe extern fn post_message_(cx: *mut JSContext, argc: libc::c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let thisobj = args.thisv().to_object();
    let context_value = JS_GetReservedSlot(thisobj, 0).to_private();
    let browsing_context = &*(context_value as *const BrowsingContext);
    let destination = browsing_context.parent_pipeline.unwrap();
    let message = args.get(0);
    let origin = if let Ok(v) = FromJSValConvertible::from_jsval(cx, args.get(1), StringificationBehavior::Default) {
        v
    } else {
        return false;
    };
    let transfer = args.get(2);
    post_message(cx, message, origin, transfer, destination).is_ok()
}

const SMETHOD_SPECS: &'static [JSFunctionSpec] = &[
    JSFunctionSpec {
        name: b"postMessage\0" as *const u8 as *const libc::c_char,
        call: JSNativeWrapper { op: Some(post_message_), info: 0 as *const JSJitInfo },
        nargs: 3,
        flags: (JSPROP_ENUMERATE) as u16,
        selfHostedName: 0 as *const libc::c_char
    },
    JSFunctionSpec {
        name: 0 as *const libc::c_char,
        call: JSNativeWrapper { op: None, info: 0 as *const JSJitInfo },
        nargs: 0,
        flags: 0,
        selfHostedName: 0 as *const libc::c_char
    }
];

const SATTRIBUTE_SPECS: &'static [JSPropertySpec] = &[
    JSPropertySpec {
        name: b"parent\0" as *const u8 as *const libc::c_char,
        flags: (JSPROP_ENUMERATE | JSPROP_SHARED) as u8,
        getter: JSNativeWrapper { op: Some(parent_), info: 0 as *const JSJitInfo },
        setter: JSNativeWrapper { op: None, info: 0 as *const JSJitInfo },
    },
    JSPropertySpec {
        name: 0 as *const libc::c_char,
        flags: 0,
        getter: JSNativeWrapper { op: None, info: 0 as *const JSJitInfo },
        setter: JSNativeWrapper { op: None, info: 0 as *const JSJitInfo },
    },
];

impl BrowsingContext {
    pub fn new_inherited(frame_element: Option<&Element>,
                         parent_pipeline: Option<PipelineId>) -> BrowsingContext {
        BrowsingContext {
            reflector: Reflector::new(),
            history: DOMRefCell::new(vec![]),
            active_index: 0,
            frame_element: frame_element.map(JS::from_ref),
            parent: MutHeapJSVal::new(),
            parent_pipeline: parent_pipeline,
        }
    }

    #[allow(unsafe_code)]
    pub fn new(window: &Window, frame_element: Option<&Element>, parent_pipeline: Option<PipelineId>)
               -> Root<BrowsingContext> {
        unsafe {
            let WindowProxyHandler(handler) = window.windowproxy_handler();
            assert!(!handler.is_null());

            let cx = window.get_cx();
            let _ar = JSAutoRequest::new(cx);
            let parent = window.reflector().get_jsobject();
            assert!(!parent.get().is_null());
            assert!(((*JS_GetClass(parent.get())).flags & JSCLASS_IS_GLOBAL) != 0);
            let _ac = JSAutoCompartment::new(cx, parent.get());
            let window_proxy = RootedObject::new(cx,
                NewWindowProxy(cx, parent, handler));
            assert!(!window_proxy.ptr.is_null());

            let create_cross_origin_parent = parent_pipeline.is_some() && frame_element.is_none();

            let object = box BrowsingContext::new_inherited(frame_element, parent_pipeline);

            let raw = Box::into_raw(object);
            SetProxyExtra(window_proxy.ptr, 0, &PrivateValue(raw as *const _));

            (*raw).init_reflector(window_proxy.ptr);

            let context = Root::from_ref(&*raw);

            if create_cross_origin_parent {
                // Create a cross origin reflector with the browsing context reflector
                // in its reserved slot.
                let obj = RootedObject::new(cx, JS_NewObject(cx, &CROSS_ORIGIN_CLASS));
                let obj = obj.handle();
                assert!(!obj.is_null());
                define_methods(cx, obj, SMETHOD_SPECS).unwrap();
                define_properties(cx, obj, SATTRIBUTE_SPECS).unwrap();
                JS_SetReservedSlot(*obj, 0, PrivateValue((&*context) as *const _ as *const libc::c_void));
                context.parent.set(ObjectValue(&**obj));
            }

            context
        }
    }

    pub fn init(&self, document: &Document) {
        assert!(self.history.borrow().is_empty());
        assert_eq!(self.active_index, 0);
        self.history.borrow_mut().push(SessionHistoryEntry::new(document));
    }

    pub fn active_document(&self) -> Root<Document> {
        Root::from_ref(&*self.history.borrow()[self.active_index].document)
    }

    pub fn active_window(&self) -> Root<Window> {
        Root::from_ref(self.active_document().window())
    }

    pub fn frame_element(&self) -> Option<&Element> {
        self.frame_element.r()
    }

    pub fn parent(&self) -> JSVal {
        self.parent.get()
    }

    pub fn window_proxy(&self) -> *mut JSObject {
        let window_proxy = self.reflector.get_jsobject();
        assert!(!window_proxy.get().is_null());
        window_proxy.get()
    }
}

// This isn't a DOM struct, just a convenience struct
// without a reflector, so we don't mark this as #[dom_struct]
#[must_root]
#[privatize]
#[derive(JSTraceable, HeapSizeOf)]
pub struct SessionHistoryEntry {
    document: JS<Document>,
    children: Vec<JS<BrowsingContext>>,
}

impl SessionHistoryEntry {
    fn new(document: &Document) -> SessionHistoryEntry {
        SessionHistoryEntry {
            document: JS::from_ref(document),
            children: vec![],
        }
    }
}

#[allow(unsafe_code)]
unsafe fn GetSubframeWindow(cx: *mut JSContext,
                            proxy: HandleObject,
                            id: HandleId)
                            -> Option<Root<Window>> {
    let index = get_array_index_from_id(cx, id);
    if let Some(index) = index {
        let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
        let win = root_from_handleobject::<Window>(target.handle()).unwrap();
        let mut found = false;
        return win.IndexedGetter(index, &mut found);
    }

    None
}

#[allow(unsafe_code)]
unsafe extern "C" fn getOwnPropertyDescriptor(cx: *mut JSContext,
                                              proxy: HandleObject,
                                              id: HandleId,
                                              desc: MutableHandle<PropertyDescriptor>)
                                              -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if let Some(window) = window {
        let mut val = RootedValue::new(cx, UndefinedValue());
        window.to_jsval(cx, val.handle_mut());
        (*desc.ptr).value = val.ptr;
        fill_property_descriptor(&mut *desc.ptr, *proxy.ptr, JSPROP_READONLY);
        return true;
    }

    let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
    if !JS_GetOwnPropertyDescriptorById(cx, target.handle(), id, desc) {
        return false;
    }

    assert!(desc.get().obj.is_null() || desc.get().obj == target.ptr);
    if desc.get().obj == target.ptr {
        desc.get().obj = *proxy.ptr;
    }

    true
}

#[allow(unsafe_code)]
unsafe extern "C" fn defineProperty(cx: *mut JSContext,
                                    proxy: HandleObject,
                                    id: HandleId,
                                    desc: Handle<PropertyDescriptor>,
                                    res: *mut ObjectOpResult)
                                    -> bool {
    if get_array_index_from_id(cx, id).is_some() {
        // Spec says to Reject whether this is a supported index or not,
        // since we have no indexed setter or indexed creator.  That means
        // throwing in strict mode (FIXME: Bug 828137), doing nothing in
        // non-strict mode.
        (*res).code_ = JSErrNum::JSMSG_CANT_DEFINE_WINDOW_ELEMENT as ::libc::uintptr_t;
        return true;
    }

    let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
    JS_DefinePropertyById(cx, target.handle(), id, desc, res)
}

#[allow(unsafe_code)]
unsafe extern "C" fn has(cx: *mut JSContext,
                         proxy: HandleObject,
                         id: HandleId,
                         bp: *mut bool)
                         -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if window.is_some() {
        *bp = true;
        return true;
    }

    let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
    let mut found = false;
    if !JS_HasPropertyById(cx, target.handle(), id, &mut found) {
        return false;
    }

    *bp = found;
    true
}

#[allow(unsafe_code)]
unsafe extern "C" fn get(cx: *mut JSContext,
                         proxy: HandleObject,
                         receiver: HandleValue,
                         id: HandleId,
                         vp: MutableHandleValue)
                         -> bool {
    let window = GetSubframeWindow(cx, proxy, id);
    if let Some(window) = window {
        window.to_jsval(cx, vp);
        return true;
    }

    let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
    JS_ForwardGetPropertyTo(cx, target.handle(), id, receiver, vp)
}

#[allow(unsafe_code)]
unsafe extern "C" fn set(cx: *mut JSContext,
                         proxy: HandleObject,
                         id: HandleId,
                         v: HandleValue,
                         receiver: HandleValue,
                         res: *mut ObjectOpResult)
                         -> bool {
    if get_array_index_from_id(cx, id).is_some() {
        // Reject (which means throw if and only if strict) the set.
        (*res).code_ = JSErrNum::JSMSG_READ_ONLY as ::libc::uintptr_t;
        return true;
    }

    let target = RootedObject::new(cx, GetProxyPrivate(*proxy.ptr).to_object());
    JS_ForwardSetPropertyTo(cx,
                            target.handle(),
                            id,
                            v,
                            receiver,
                            res)
}

static PROXY_HANDLER: ProxyTraps = ProxyTraps {
    enter: None,
    getOwnPropertyDescriptor: Some(getOwnPropertyDescriptor),
    defineProperty: Some(defineProperty),
    ownPropertyKeys: None,
    delete_: None,
    enumerate: None,
    preventExtensions: None,
    isExtensible: None,
    has: Some(has),
    get: Some(get),
    set: Some(set),
    call: None,
    construct: None,
    getPropertyDescriptor: Some(get_property_descriptor),
    hasOwn: None,
    getOwnEnumerablePropertyKeys: None,
    nativeCall: None,
    hasInstance: None,
    objectClassIs: None,
    className: None,
    fun_toString: None,
    boxedValue_unbox: None,
    defaultValue: None,
    trace: None,
    finalize: None,
    objectMoved: None,
    isCallable: None,
    isConstructor: None,
};

#[allow(unsafe_code)]
pub fn new_window_proxy_handler() -> WindowProxyHandler {
    unsafe {
        WindowProxyHandler(CreateWrapperProxyHandler(&PROXY_HANDLER))
    }
}
