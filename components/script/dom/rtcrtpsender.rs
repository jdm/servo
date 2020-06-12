/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::dom::bindings::codegen::Bindings::RTCRtpSenderBinding::RTCRtpSenderMethods;
use crate::dom::bindings::codegen::Bindings::RTCRtpSenderBinding::{
    RTCRtpCodecParameters, RTCRtpParameters,RTCRtcpParameters,  RTCRtpSendParameters
};
use crate::dom::bindings::reflector::{reflect_dom_object, Reflector};
use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::str::DOMString;
use crate::dom::globalscope::GlobalScope;
use dom_struct::dom_struct;

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
                    payloadType: 0,
                    mimeType: DOMString::new(),
                    clockRate: 0,
                    channels: 0,
                    sdpFmtpLine: DOMString::new(),
                },
                codecs: vec![],
            },
            transactionId: DOMString::new(),
            encodings: vec![],
        }
    }
}
