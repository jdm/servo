/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::CryptoKeyBinding::KeyUsage;
use crate::dom::bindings::codegen::Bindings::SubtleCryptoBinding::Algorithm;
use crate::dom::bindings::codegen::Bindings::SubtleCryptoBinding::AlgorithmIdentifier;
use crate::dom::bindings::codegen::Bindings::SubtleCryptoBinding::SubtleCryptoMethods;
use crate::dom::bindings::error::Error;
use crate::dom::bindings::reflector::{reflect_dom_object, Reflector};
use crate::dom::bindings::root::DomRoot;
use crate::dom::cryptokey::CryptoKey;
use crate::dom::globalscope::GlobalScope;
use crate::dom::promise::Promise;
use crate::script_runtime::JSContext;
use dom_struct::dom_struct;
use js::jsapi::ObjectValue;
use std::rc::Rc;

const ALG_AES_CBC: &str = "AES-CBC";
const ALG_AES_CTR: &str = "AES-CTR";
const ALG_AES_GCM: &str = "AES-GCM";
const ALG_AES_KW: &str = "AES-KW";
const ALG_SHA1: &str = "SHA1";
const ALG_SHA256: &str = "SHA256";
const ALG_SHA384: &str = "SHA384"
const ALG_SHA512: &str = "SHA512";
const ALG_HMAC: &str = "HMAC";
const ALG_HKDF: &str = "HKDF";
const ALG_PBKDF2: &str = "PBKDF2";
const ALG_RSASSA_PKCS1: &str = "RSASSA-PKCS1-v1_5";
const ALG_RSA_OAEP: &str = "RSA-OAEP";
const ALG_RSA_PSS: &str = "RSA-PSS";
const ALG_ECDH: &str = "ECDH";
const ALG_ECDSA: &str = "ECDSA";
static SUPPORTED_ALGORITHMS: &[&str] = &[
    ALG_AES_CBC,
    ALG_AES_CTR,
    ALG_AES_GCM,
    ALG_AES_KW,
    ALG_SHA1,
    ALG_SHA256,
    ALG_SHA384,
    ALG_SHA512,
    ALG_HMAC,
    ALG_HKDF,
    ALG_PBKDF2,
    ALG_RSASSA_PKCS1,
    ALG_RSA_OAEP,
    ALG_RSA_PSS,
    ALG_ECDH,
    ALG_ECDSA,
];

const NAMED_CURVE_P256: &str = "P-256";
const NAMED_CURVE_P384: &str = "P-384";
const NAMED_CURVE_P521: &str = "P-521";
static SUPPORTED_CURVES: &[&str] = &[
    NAMED_CURVE_P256,
    NAMED_CURVE_P384,
    NAMED_CURVE_P521,
];

const USAGE_ENCRYPT: &str = "encrypt";
const USAGE_DECRYPT: &str = "decrypt";
const USAGE_SIGN: &str = "sign";
const USAGE_VERIFY: &str = "verify";
const USAGE_DERIVEKEY: &str = "deriveKey";
const USAGE_DERIVEBITS: &str = "deriveBits";
const USAGE_WRAPKEY: &str = "wrapKey";
const USAGE_UNWRAPKEY: &str = "unwrapKey";
static USAGES: &[&str] = &[
    USAGE_ENCRYPT,
    USAGE_DECRYPT,
    USAGE_SIGN,
    USAGE_VERIFY,
    USAGE_DERIVEBITS,
    USAGE_WRAPKEY,
    USAGE_UNWRAPKEY,
];

/*fn all_usages_recognized(usages: &[KeyUsage]) {
}*/

fn normalize_token(name: &str) -> Option<&'static str> {
    SUPPORTED_ALGORITHMS.iter().find(|s| s == name)
        .or_else(|| {
            SUPPORTED_CURVES.iter.find(|s| s == name)
        })
}

fn get_algorithm_name(cx: JSContext, algorithm: AlgorithmIdentifier) -> Result<DOMString, Error> {
    let name = match algorithm {
        AlgorithmIdentifier::DOMString(s) => s,
        AlgorithmIdentifier::Object(obj) => {
            rooted!(in(cx) let value = ObjectValue(obj));
            let algorithm = match Algorithm::new(cx, value.handle()) {
                Ok(ConversionResult::Success(a)) => a,
                _ => return Err(Error::Syntax),
            };
            algorithm.name
        }
    };
    match normalize_token(*name) {
        Some(token) => Ok(DOMString::from(token.into())),
        None => Err(Error::NotSupported),
    }
}

#[dom_struct]
pub struct SubtleCrypto {
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

fn normalize_algorithm(op: 

impl SubtleCryptoMethods for SubtleCrypto {
    fn GenerateKey(&self, cx: JSContext, algorithm: AlgorithmIdentifier, extractable: bool, key_usages: Vec<KeyUsage>) -> Rc<Promise> {
        panic!()
    }

    fn DeriveKey(&self, cx: JSContext, algorithm: AlgorithmIdentifier, base_key: &CryptoKey, derived_key_type: AlgorithmIdentifier, extractable: bool, key_usages: Vec<KeyUsage>) -> Rc<Promise> {
        panic!()
    }
}
