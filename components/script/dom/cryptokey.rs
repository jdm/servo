/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::CryptoKeyBinding::{CryptoKeyMethods, KeyType, KeyUsage};
use crate::dom::bindings::codegen::Bindings::SubtleCryptoBindings::KeyAlgorithm;
use crate::dom::bindings::reflector::{reflect_dom_object, Reflector};
use crate::script_runtime::JSContext;
use dom_struct::dom_struct;
use js::jsapi::JSValue;
use js::rust::IntoHandle;

#[dom_struct]
struct CryptoKey {
    reflector_: Reflector,
    key_type: KeyType,
    extractable: bool,
    algorithm: KeyAlgorithm,
    usages: Vec<KeyUsage>,
    
}

impl CryptoKey {
    fn new_inherited(key_type: KeyType, extractable: bool, algorithm: KeyAlgorithm, usages: Vec<KeyUsage>) -> CryptoKey {
        CryptoKey {
            reflector_: Reflector::new(),
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

    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Algorithm(&self, cx: JSContext) -> JSValue {
        rooted!(in(*cx) let mut algorithm: JSValue);
        unsafe {
            self.algorithm.to_jsval(*cx, algorithm.handle_mut().into_handle());
        }
        algorithm
    }

    #[allow(unsafe_code)]
    // https://w3c.github.io/webcrypto/#cryptokey-interface-members
    fn Usages(&self, cx: JSContext) -> JSValue {
        rooted!(in(*cx) let mut usages: JSValue);
        unsafe {
            self.usages.to_jsval(*cx, usages.handle_mut());
        }
        usages.get()
    }
}
