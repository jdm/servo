/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::CryptoKeyBinding::{CryptoKeyMethods, KeyType, KeyUsage};
use crate::dom::bindings::codegen::Bindings::SubtleCryptoBinding::KeyAlgorithm;
use crate::dom::bindings::reflector::{reflect_dom_object, Reflector};
use crate::dom::bindings::root::DomRoot;
use crate::dom::globalscope::GlobalScope;
use crate::script_runtime::JSContext;
use dom_struct::dom_struct;
use js::conversions::ToJSValConvertible;
use js::jsapi::{JSObject, Value};
use std::ptr::NonNull;

#[dom_struct]
pub struct CryptoKey {
    reflector_: Reflector,
    key_type: KeyType,
    extractable: bool,
    #[ignore_malloc_size_of = ""]
    algorithm: KeyAlgorithm,
    usages: Vec<KeyUsage>,
    
}

impl CryptoKey {
    fn new_inherited(key_type: KeyType, extractable: bool, algorithm: KeyAlgorithm, usages: Vec<KeyUsage>) -> CryptoKey {
        CryptoKey {
            reflector_: Reflector::new(),
            key_type,
            extractable,
            algorithm,
            usages,
        }
    }

    pub(crate) fn new(global: &GlobalScope, key_type: KeyType, extractable: bool, algorithm: KeyAlgorithm, usages: Vec<KeyUsage>) -> DomRoot<CryptoKey> {
        reflect_dom_object(Box::new(CryptoKey::new_inherited(key_type, extractable, algorithm, usages)), global)
    }
}

impl CryptoKeyMethods for CryptoKey {
    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Type(&self) -> KeyType {
        self.key_type.clone()
    }

    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Extractable(&self) -> bool {
        self.extractable
    }

    #[allow(unsafe_code)]
    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Algorithm(&self, cx: JSContext) -> NonNull<JSObject> {
        unsafe {
            rooted!(in(*cx) let mut algorithm: Value);
            self.algorithm.to_jsval(*cx, algorithm.handle_mut());
            NonNull::new(algorithm.to_object()).unwrap()
        }
    }

    #[allow(unsafe_code)]
    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Usages(&self, cx: JSContext) -> NonNull<JSObject> {
        unsafe {
            rooted!(in(*cx) let mut usages: Value);
            self.usages.to_jsval(*cx, usages.handle_mut());
            NonNull::new(usages.to_object()).unwrap()
        }
    }
}
