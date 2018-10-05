/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::proxyhandler::{has_property_on_prototype, fill_property_descriptor};
use crate::dom::bindings::root::Root;
use crate::dom::globalscope::GlobalScope;
use crate::dom::window::Window;
use crate::script_runtime::JSContext as SafeJSContext;
use js::conversions::jsstr_to_string;
use js::conversions::ToJSValConvertible;
use js::error::throw_type_error;
use js::glue::{CreateProxyHandler, NewProxyObject, ProxyTraps, RUST_JSID_IS_STRING};
use js::glue::RUST_JSID_TO_STRING;
use js::jsapi::{UndefinedHandleValue, JSContext, Handle, HandleObject};
use js::jsapi::{HandleId, MutableHandle, PropertyDescriptor, ObjectOpResult, AutoIdVector};
use js::jsapi::JS_SetImmutablePrototype;
use js::jsval::UndefinedValue;
use js::rust::IntoHandle;
use js::rust::{HandleObject as RustHandleObject, MutableHandleObject as RustMutableHandleObject};
use js::rust::{Handle as RustHandle, MutableHandle as RustMutableHandle};
use libc;
use std::ptr;

struct SyncWrapper(*const libc::c_void);
#[allow(unsafe_code)]
unsafe impl Sync for SyncWrapper {}

static NAMED_PROPERTY_TRAPS: ProxyTraps = ProxyTraps {
    enter: None,
    getOwnPropertyDescriptor: Some(get_own_property_descriptor),
    defineProperty: Some(define_property),
    ownPropertyKeys: Some(own_property_keys),
    delete_: Some(delete),
    enumerate: None,
    getPrototypeIfOrdinary: None,
    preventExtensions: Some(prevent_extensions),
    isExtensible: Some(is_extensible),
    has: None,
    get: None,
    set: None,
    call: None,
    construct: None,
    hasOwn: None,
    getOwnEnumerablePropertyKeys: None,
    nativeCall: None,
    hasInstance: None,
    objectClassIs: None,
    className: Some(class_name),
    fun_toString: None,
    boxedValue_unbox: None,
    defaultValue: None,
    trace: None,
    finalize: None,
    objectMoved: None,
    isCallable: None,
    isConstructor: None,
};

lazy_static! {
    static ref HANDLER: SyncWrapper = {
        // FIXME: proxies with non-null class pointers expected
        #[allow(unsafe_code)]
        unsafe { SyncWrapper(CreateProxyHandler(&NAMED_PROPERTY_TRAPS, ptr::null())) }
    };
}

#[allow(unsafe_code)]
unsafe extern "C" fn get_own_property_descriptor(
    cx: *mut JSContext,
    proxy: HandleObject,
    id: HandleId,
    desc: MutableHandle<PropertyDescriptor>
) -> bool {
    let cx = SafeJSContext::from_ptr(cx);
    if !RUST_JSID_IS_STRING(id) {
        // Nothing to do if we're resolving a non-string property.
        return true;
    }

    match has_property_on_prototype(cx, RustHandle::from_raw(proxy), id) {
        Ok(true) => return true,
        Ok(false) => (),
        Err(()) => return false,
    };

    let s = jsstr_to_string(*cx, RUST_JSID_TO_STRING(id));
    if s.is_empty() {
        return true;
    }

    let window = Root::downcast::<Window>(
        GlobalScope::from_object(proxy.get())
    ).expect("global is not a window");
    if let Some(obj) = window.NamedGetter(cx, s.into()) {
        rooted!(in(*cx) let mut val = UndefinedValue());
        obj.to_jsval(*cx, val.handle_mut());
        fill_property_descriptor(RustMutableHandle::from_raw(desc), proxy.get(), val.get(), 0);
    }
    return true;
}

#[allow(unsafe_code)]
unsafe extern "C" fn define_property(
    cx: *mut JSContext,
    _proxy: HandleObject,
    _id: HandleId,
    _desc: Handle<PropertyDescriptor>,
    _result: *mut ObjectOpResult
) -> bool {
    throw_type_error(cx, "Not allowed to define a property on the named properties object.");
    false
}

#[allow(unsafe_code)]
unsafe extern "C" fn own_property_keys(
    _cx: *mut JSContext,
    _proxy: HandleObject,
    _props: *mut AutoIdVector
) -> bool {
    //TODO
    true
}

#[allow(unsafe_code)]
unsafe extern "C" fn delete(
    _cx: *mut JSContext,
    _proxy: HandleObject,
    _id: HandleId,
    _result: *mut ObjectOpResult
) -> bool {
    //TODO
    false
}

#[allow(unsafe_code)]
unsafe extern "C" fn prevent_extensions(
    _cx: *mut JSContext,
    _proxy: HandleObject,
    _result: *mut ObjectOpResult
) -> bool {
    //TODO
    false
}

#[allow(unsafe_code)]
unsafe extern "C" fn is_extensible(
    _cx: *mut JSContext,
    _proxy: HandleObject,
    extensible: *mut bool
) -> bool {
    *extensible = true;
    true
}

#[allow(unsafe_code)]
unsafe extern "C" fn class_name(
    _cx: *mut JSContext,
    _proxy: HandleObject
) -> *const i8 {
    b"WindowProperties\0" as *const _ as *const i8
}

#[allow(unsafe_code)]
pub fn create(
    cx: SafeJSContext,
    proto: RustHandleObject,
    mut properties_obj: RustMutableHandleObject
) {
    unsafe {
        // TODO: mark proxy as singleton
        properties_obj.set(NewProxyObject(
            *cx,
            HANDLER.0,
            UndefinedHandleValue,
            proto.get(),
        ));
        assert!(!properties_obj.get().is_null());
        let mut succeeded = false;
        assert!(JS_SetImmutablePrototype(*cx, properties_obj.handle().into_handle(), &mut succeeded));
        assert!(succeeded);
    }
}
