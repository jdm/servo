/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::SubtleCryptoBinding::SubtleCryptoMethods;
use crate::dom::bindings::reflector::{reflect_dom_object, Reflector};
use dom_struct::dom_struct;

#[dom_struct]
struct SubtleCrypto {
    reflector_: Reflector,
}

impl SubtleCrypto {
    fn new_inherited() -> SubtleCrypto {
        SubtleCrypto {
            reflector_: Reflector::new(),
        }
    }

    pub(crate) fn new(global: &GlobalScope) -> DomRoot<SubtleCrypto> {
        reflect_dom_object(Box::new(SubtleCrypto::new_inherited()), global)
    }
}

impl SubtleCryptoMethods for SubtleCrypto {
    fn DeriveKey() {
    }
}
