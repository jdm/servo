/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::RTCRtpSenderBinding::RTCRtpSenderMethods;
use crate::dom::bindings::codegen::Bindings::RTCRtpSenderBinding::{
    RTCRtpParameters, RTCRtcpParameters,  RTCRtpSendParameters
};
use crate::dom::bindings::reflector::{reflect_dom_object, DomObject, Reflector};
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::globalscope::GlobalScope;
use crate::dom::promise::Promise;
use dom_struct::dom_struct;
use std::rc::Rc;

#[dom_struct]
pub struct RTCRtpSender {
    reflector_: Reflector
}

impl RTCRtpSender {
    fn new_inherited() -> Self {
        Self {
            reflector_: Reflector::new(),
        }
    }

    pub(crate) fn new(global: &GlobalScope) -> DomRoot<Self> {
        reflect_dom_object(Box::new(Self::new_inherited()), global)
    }
}

impl RTCRtpSenderMethods for RTCRtpSender {
    fn GetParameters(&self) -> RTCRtpSendParameters {
        RTCRtpSendParameters {
            parent: RTCRtpParameters {
                headerExtensions: vec![],
                rtcp: RTCRtcpParameters {
                    cname: None,
                    reducedSize: None,
                },
                codecs: vec![],
            },
            transactionId: DOMString::new(),
            encodings: vec![],
        }
    }

    fn SetParameters(&self, _parameters: &RTCRtpSendParameters) -> Rc<Promise> {
        let promise = Promise::new(&self.global());
        promise.resolve_native(&());
        promise
    }
}
