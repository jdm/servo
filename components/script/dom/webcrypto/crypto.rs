/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use dom_struct::dom_struct;
use js::context::{JSContext, NoGC};
use js::jsapi::{Heap, JSObject, Type};
use js::rust::CustomAutoRooterGuard;
use js::typedarray::{ArrayBufferView, ArrayBufferViewU8, HeapArrayBufferView, TypedArray};
use rand::TryRng;
use rand::rngs::SysRng;
use script_bindings::error::InterfaceError;
use script_bindings::reflector::{Reflector, reflect_dom_object_with_cx};
use script_bindings::trace::RootedTraceableBox;
use uuid::Uuid;

use crate::dom::bindings::codegen::Bindings::CryptoBinding::CryptoMethods;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{DomRoot, MutNullableDom};
use crate::dom::bindings::str::DOMString;
use crate::dom::globalscope::GlobalScope;
use crate::dom::subtlecrypto::SubtleCrypto;

// https://developer.mozilla.org/en-US/docs/Web/API/Crypto
#[dom_struct]
pub(crate) struct Crypto {
    reflector_: Reflector,
    subtle: MutNullableDom<SubtleCrypto>,
}

impl Crypto {
    fn new_inherited() -> Crypto {
        Crypto {
            reflector_: Reflector::new(),
            subtle: MutNullableDom::default(),
        }
    }

    pub(crate) fn new(cx: &mut JSContext, global: &GlobalScope) -> DomRoot<Crypto> {
        reflect_dom_object_with_cx(Box::new(Crypto::new_inherited()), global, cx)
    }
}

pub(crate) enum CryptoError {
    NonIntegerBuffer,
    InputTooLong,
    RandomGenerationFailed,
}

impl From<CryptoError> for String {
    fn from(error: CryptoError) -> String {
        match error {
            CryptoError::NonIntegerBuffer => "Non-integer buffer provided",
            CryptoError::InputTooLong => "Input buffer exceeded 64kb",
            CryptoError::RandomGenerationFailed => "Failed to generate random values",
        }.into()
    }
}

impl script_bindings::error::InterfaceScopedError for Crypto {
    type IdlError = CryptoError;
}

impl CryptoMethods<crate::DomTypeHolder> for Crypto {
    /// <https://w3c.github.io/webcrypto/#dfn-Crypto-attribute-subtle>
    fn Subtle(&self, cx: &mut js::context::JSContext) -> DomRoot<SubtleCrypto> {
        self.subtle
            .or_init(|| SubtleCrypto::new(cx, &self.global()))
    }

    #[expect(unsafe_code)]
    /// <https://w3c.github.io/webcrypto/#Crypto-method-getRandomValues>
    fn GetRandomValues(
        &self,
        no_gc: &NoGC,
        mut input: CustomAutoRooterGuard<ArrayBufferView>,
    ) -> Result<RootedTraceableBox<HeapArrayBufferView>, InterfaceError<CryptoError>> {
        let array_type = input.get_array_type();

        if !is_integer_buffer(array_type) {
            Err(InterfaceError::TypeMismatch(CryptoError::NonIntegerBuffer))
        } else {
            let data = input.as_mut_slice_safe(no_gc).unwrap_or(&mut []);
            if data.len() > 65536 {
                return Err(InterfaceError::QuotaExceeded(CryptoError::InputTooLong));
            }

            if SysRng.try_fill_bytes(data).is_err() {
                return Err(InterfaceError::Operation(CryptoError::RandomGenerationFailed));
            }

            let underlying_object = unsafe { input.underlying_object() };
            TypedArray::<ArrayBufferViewU8, Box<Heap<*mut JSObject>>>::from(*underlying_object)
                .map(RootedTraceableBox::new)
                .map_err(|_| InterfaceError::JSFailed)
        }
    }

    /// <https://w3c.github.io/webcrypto/#Crypto-method-randomUUID>
    fn RandomUUID(&self) -> DOMString {
        let uuid = Uuid::new_v4();
        uuid.hyphenated()
            .encode_lower(&mut Uuid::encode_buffer())
            .to_owned()
            .into()
    }
}

fn is_integer_buffer(array_type: Type) -> bool {
    matches!(
        array_type,
        Type::Uint8 |
            Type::Uint8Clamped |
            Type::Int8 |
            Type::Uint16 |
            Type::Int16 |
            Type::Uint32 |
            Type::Int32 |
            Type::BigInt64 |
            Type::BigUint64
    )
}
