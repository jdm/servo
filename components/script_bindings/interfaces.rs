use std::cell::RefCell;
use std::rc::Rc;
use std::thread::LocalKey;

use js::glue::JSPrincipalsCallbacks;
use js::jsapi::{CallArgs, JSContext, JSObject, HandleObject as RawHandleObject};
use js::rust::{HandleObject, MutableHandleObject};
use servo_url::{MutableOrigin, ServoUrl};

use crate::DomTypes;
use crate::conversions::{DerivedFrom, ToJSValConvertible};
use crate::error::Error;
use crate::realms::{AlreadyInRealm, InRealm};
use crate::reflector::{DomObject, DomObjectWrap};
use crate::root::DomRoot;
use crate::script_runtime::{CanGc, JSContext as SafeJSContext};
use crate::settings_stack::StackEntry;
use crate::utils::ProtoOrIfaceArray;

/// Operations that must be invoked from the generated bindings.
#[allow(unsafe_code)]
pub trait GlobalScopeHelpers<D: crate::DomTypes> {
    unsafe fn from_context(cx: *mut JSContext, realm: InRealm) -> DomRoot<D::GlobalScope>;
    fn get_cx() -> SafeJSContext;
    unsafe fn from_object(obj: *mut JSObject) -> DomRoot<D::GlobalScope>;
    fn from_reflector(
        reflector: &impl DomObject,
        realm: &AlreadyInRealm,
    ) -> DomRoot<D::GlobalScope>;

    unsafe fn from_object_maybe_wrapped(
        obj: *mut JSObject,
        cx: *mut JSContext,
    ) -> DomRoot<D::GlobalScope>;

    fn origin(&self) -> &MutableOrigin;

    fn incumbent() -> Option<DomRoot<D::GlobalScope>>;

    fn perform_a_microtask_checkpoint(&self, can_gc: CanGc);

    fn get_url(&self) -> ServoUrl;

    fn is_secure_context(&self) -> bool;
}

pub trait DocumentHelpers<D: DomTypes> {
    fn ensure_safe_to_run_script_or_layout(&self);
}

pub trait WindowHelpers {
    fn create_named_properties_object(
        cx: SafeJSContext,
        proto: HandleObject,
        object: MutableHandleObject,
    );
}

/// Operations that must be invoked from the generated bindings.
pub trait PromiseHelpers<D: crate::DomTypes> {
    fn new_resolved(
        global: &D::GlobalScope,
        cx: SafeJSContext,
        value: impl ToJSValConvertible,
    ) -> Rc<D::Promise>;
}

/// Operations that must be invoked from the generated bindings.
pub trait DomHelpers<D: DomTypes> {
    fn throw_dom_exception(cx: SafeJSContext, global: &D::GlobalScope, result: Error);
    fn report_pending_exception(cx: SafeJSContext, dispatch_event: bool, realm: InRealm, can_gc: CanGc);

    unsafe fn call_html_constructor<T: DerivedFrom<D::Element> + DomObject>(
        cx: SafeJSContext,
        args: &CallArgs,
        global: &D::GlobalScope,
        proto_id: crate::codegen::PrototypeList::ID,
        creator: unsafe fn(SafeJSContext, HandleObject, *mut ProtoOrIfaceArray),
        can_gc: CanGc,
    ) -> bool;

    fn settings_stack() -> &'static LocalKey<RefCell<Vec<StackEntry<D>>>>;

    fn push_new_element_queue();
    fn pop_current_element_queue(can_gc: CanGc);

    fn interface_map() -> &'static phf::Map<&'static [u8], fn(SafeJSContext, HandleObject)>;

    fn reflect_dom_object<T, U>(obj: Box<T>, global: &U, can_gc: CanGc) -> DomRoot<T> where T: DomObject + DomObjectWrap<D>, U: DerivedFrom<D::GlobalScope>;

    unsafe fn is_platform_object_same_origin(
        cx: SafeJSContext,
        obj: RawHandleObject,
    ) -> bool;

    fn principals_callbacks() -> &'static JSPrincipalsCallbacks;
}

pub trait TestBindingHelpers {
    fn condition_satisfied(cx: SafeJSContext, global: HandleObject) -> bool;
    fn condition_unsatisfied(cx: SafeJSContext, global: HandleObject) -> bool;
}

pub trait WebGL2RenderingContextHelpers {
    fn is_webgl2_enabled(cx: SafeJSContext, global: HandleObject) -> bool;
}
