/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Various utilities to glue JavaScript and the DOM implementation together.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::thread::LocalKey;
use std::{ptr, slice, str};

use js::conversions::ToJSValConvertible;
use js::glue::{
    CallJitGetterOp, CallJitMethodOp, CallJitSetterOp, IsWrapper, JS_GetReservedSlot,
    UnwrapObjectDynamic, UnwrapObjectStatic, RUST_FUNCTION_VALUE_TO_JITINFO,
    JSPrincipalsCallbacks,
};
use js::jsapi::{
    AtomToLinearString, CallArgs, DOMCallbacks, ExceptionStackBehavior, GetLinearStringCharAt,
    GetLinearStringLength, GetNonCCWObjectGlobal, HandleId as RawHandleId,
    HandleObject as RawHandleObject, Heap, JSAtom, JSContext, JSJitInfo, JSObject, JSTracer,
    JS_ClearPendingException, JS_DeprecatedStringHasLatin1Chars, JS_EnumerateStandardClasses,
    JS_FreezeObject, JS_GetLatin1StringCharsAndLength, JS_IsExceptionPending, JS_IsGlobalObject,
    JS_ResolveStandardClass, MutableHandleIdVector as RawMutableHandleIdVector,
    MutableHandleValue as RawMutableHandleValue, ObjectOpResult, StringIsArrayIndex,
};
use js::jsval::{JSVal, UndefinedValue};
use js::rust::wrappers::{
    CallOriginalPromiseReject, JS_DeletePropertyById, JS_ForwardGetPropertyTo,
    JS_GetPendingException, JS_GetProperty, JS_GetPrototype, JS_HasProperty, JS_HasPropertyById,
    JS_SetPendingException, JS_SetProperty,
};
use js::rust::{
    get_object_class, is_dom_class, GCMethods, Handle, HandleId, HandleObject, HandleValue,
    MutableHandleValue, ToString,
};
use js::JS_CALLEE;
use script_bindings::interfaces::DomHelpers;
use script_bindings::realms::InRealm;
use script_bindings::reflector::DomObjectWrap;
use script_bindings::root::DomRoot;

use crate::dom::bindings::codegen::InterfaceObjectMap;
use crate::dom::bindings::codegen::PrototypeList::{self, PROTO_OR_IFACE_LENGTH};
use crate::dom::bindings::constructor::{call_html_constructor, push_new_element_queue, pop_current_element_queue};
use crate::dom::bindings::conversions::{
    jsstring_to_str, private_from_proto_check, DerivedFrom, PrototypeCheck,
};
use crate::dom::bindings::error::{throw_dom_exception, throw_invalid_this, Error, report_pending_exception};
use crate::dom::bindings::principals::PRINCIPALS_CALLBACKS;
use crate::dom::bindings::proxyhandler::is_platform_object_same_origin;
use crate::dom::bindings::reflector::{DomObject, reflect_dom_object};
use crate::dom::bindings::settings_stack::{self, StackEntry};
use crate::dom::bindings::str::DOMString;
use crate::dom::bindings::trace::trace_object;
use crate::dom::globalscope::GlobalScope;
use crate::dom::windowproxy::WindowProxyHandler;
use crate::script_runtime::{CanGc, JSContext as SafeJSContext};
use crate::DomTypes;

#[derive(JSTraceable, MallocSizeOf)]
/// Static data associated with a global object.
pub(crate) struct GlobalStaticData {
    #[ignore_malloc_size_of = "WindowProxyHandler does not properly implement it anyway"]
    /// The WindowProxy proxy handler for this global.
    pub(crate) windowproxy_handler: &'static WindowProxyHandler,
}

impl GlobalStaticData {
    /// Creates a new GlobalStaticData.
    pub(crate) fn new() -> GlobalStaticData {
        GlobalStaticData {
            windowproxy_handler: WindowProxyHandler::proxy_handler(),
        }
    }
}

pub(crate) use script_bindings::utils::*;

/// Returns a JSVal representing the frozen JavaScript array
pub(crate) fn to_frozen_array<T: ToJSValConvertible>(
    convertibles: &[T],
    cx: SafeJSContext,
    rval: MutableHandleValue,
) {
    unsafe { convertibles.to_jsval(*cx, rval) };

    rooted!(in(*cx) let obj = rval.to_object());
    unsafe { JS_FreezeObject(*cx, RawHandleObject::from(obj.handle())) };
}

/// Returns wether `obj` is a platform object using dynamic unwrap
/// <https://heycam.github.io/webidl/#dfn-platform-object>
#[allow(dead_code)]
pub(crate) fn is_platform_object_dynamic(obj: *mut JSObject, cx: *mut JSContext) -> bool {
    is_platform_object(obj, &|o| unsafe {
        UnwrapObjectDynamic(o, cx, /* stopAtWindowProxy = */ false)
    })
}

/// Returns wether `obj` is a platform object using static unwrap
/// <https://heycam.github.io/webidl/#dfn-platform-object>
pub(crate) fn is_platform_object_static(obj: *mut JSObject) -> bool {
    is_platform_object(obj, &|o| unsafe { UnwrapObjectStatic(o) })
}

fn is_platform_object(
    obj: *mut JSObject,
    unwrap_obj: &dyn Fn(*mut JSObject) -> *mut JSObject,
) -> bool {
    unsafe {
        // Fast-path the common case
        let mut clasp = get_object_class(obj);
        if is_dom_class(&*clasp) {
            return true;
        }
        // Now for simplicity check for security wrappers before anything else
        if IsWrapper(obj) {
            let unwrapped_obj = unwrap_obj(obj);
            if unwrapped_obj.is_null() {
                return false;
            }
            clasp = get_object_class(obj);
        }
        // TODO also check if JS_IsArrayBufferObject
        is_dom_class(&*clasp)
    }
}

unsafe extern "C" fn instance_class_has_proto_at_depth(
    clasp: *const js::jsapi::JSClass,
    proto_id: u32,
    depth: u32,
) -> bool {
    let domclass: *const DOMJSClass = clasp as *const _;
    let domclass = &*domclass;
    domclass.dom_class.interface_chain[depth as usize] as u32 == proto_id
}

#[allow(missing_docs)] // FIXME
pub(crate) const DOM_CALLBACKS: DOMCallbacks = DOMCallbacks {
    instanceClassMatchesProto: Some(instance_class_has_proto_at_depth),
};

// Generic method for returning c_char from caller
pub(crate) trait AsCCharPtrPtr {
    fn as_c_char_ptr(&self) -> *const c_char;
}

impl AsCCharPtrPtr for [u8] {
    fn as_c_char_ptr(&self) -> *const c_char {
        self as *const [u8] as *const c_char
    }
}

impl DomHelpers<crate::DomTypeHolder> for crate::DomTypeHolder {
    fn throw_dom_exception(
        cx: SafeJSContext,
        global: &<crate::DomTypeHolder as DomTypes>::GlobalScope,
        result: Error,
    ) {
        throw_dom_exception(cx, global, result, CanGc::note())
    }

    unsafe fn call_html_constructor<
        T: DerivedFrom<<crate::DomTypeHolder as DomTypes>::Element> + DomObject,
    >(
        cx: SafeJSContext,
        args: &CallArgs,
        global: &<crate::DomTypeHolder as DomTypes>::GlobalScope,
        proto_id: PrototypeList::ID,
        creator: unsafe fn(SafeJSContext, HandleObject, *mut ProtoOrIfaceArray),
        can_gc: CanGc,
    ) -> bool {
        call_html_constructor::<T>(cx, args, global, proto_id, creator, can_gc)
    }

    fn settings_stack() -> &'static LocalKey<RefCell<Vec<StackEntry<crate::DomTypeHolder>>>> {
        &settings_stack::STACK
    }

    fn report_pending_exception(cx: SafeJSContext, dispatch_event: bool, realm: InRealm, can_gc: CanGc) {
        report_pending_exception(cx, dispatch_event, realm, can_gc)
    }

    fn push_new_element_queue() {
        push_new_element_queue()
    }

    fn pop_current_element_queue(can_gc: CanGc) {
        pop_current_element_queue(can_gc)
    }

    fn interface_map() -> &'static phf::Map<&'static [u8], for<'a> fn(SafeJSContext, HandleObject)> {
        &InterfaceObjectMap::MAP
    }

    fn reflect_dom_object<T, U>(obj: Box<T>, global: &U, can_gc: CanGc) -> DomRoot<T> where T: DomObject + DomObjectWrap<crate::DomTypeHolder>, U: DerivedFrom<GlobalScope> {
        reflect_dom_object(obj, global, can_gc)
    }

    unsafe fn is_platform_object_same_origin(cx: SafeJSContext, obj: RawHandleObject) -> bool {
        is_platform_object_same_origin(cx, obj)
    }

    fn principals_callbacks() -> &'static JSPrincipalsCallbacks {
        &PRINCIPALS_CALLBACKS
    }
}
