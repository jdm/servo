use js::jsapi::{GetCurrentRealmOrNull, JSAutoRealm};

use crate::interfaces::GlobalScopeHelpers;
use crate::reflector::DomObject;
use crate::script_runtime::JSContext;

pub struct AlreadyInRealm(());

impl AlreadyInRealm {
    #![allow(unsafe_code)]
    pub fn assert() -> AlreadyInRealm {
        /*unsafe {
            assert!(!GetCurrentRealmOrNull(*GlobalScope::get_cx()).is_null());
        }*/
        AlreadyInRealm(())
    }

    pub fn assert_for_cx(cx: JSContext) -> AlreadyInRealm {
        unsafe {
            assert!(!GetCurrentRealmOrNull(*cx).is_null());
        }
        AlreadyInRealm(())
    }
}

#[derive(Clone, Copy)]
pub enum InRealm<'a> {
    Already(&'a AlreadyInRealm),
    Entered(&'a JSAutoRealm),
}

impl InRealm<'_> {
    pub fn already(token: &AlreadyInRealm) -> InRealm {
        InRealm::Already(token)
    }

    pub fn entered(token: &JSAutoRealm) -> InRealm {
        InRealm::Entered(token)
    }
}

pub fn enter_realm<D: crate::DomTypes>(object: &impl DomObject) -> JSAutoRealm {
    JSAutoRealm::new(
        *D::GlobalScope::get_cx(),
        object.reflector().get_jsobject().get(),
    )
}
