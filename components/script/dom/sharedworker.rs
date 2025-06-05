/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;
use dom_struct::dom_struct;
use js::rust::HandleObject;
use script_bindings::codegen::GenericUnionTypes::StringOrWorkerOptions;
use script_bindings::root::DomRoot;
use script_bindings::script_runtime::CanGc;

use crate::dom::bindings::codegen::Bindings::SharedWorkerBinding::SharedWorkerMethods;
use crate::dom::bindings::codegen::UnionTypes::TrustedScriptURLOrUSVString;
use crate::dom::bindings::reflector::{DomGlobal, reflect_dom_object_with_proto};
use crate::dom::eventtarget::EventTarget;
use crate::dom::sharedworkerglobalscope::SharedWorkerScriptMsg;
use crate::dom::window::Window;

#[dom_struct]
pub(crate) struct SharedWorker {
    eventtarget: EventTarget,    
}

impl SharedWorker {
    fn new_inherited(_sender: Sender<SharedWorkerScriptMsg>, _closing: Arc<AtomicBool>) -> SharedWorker {
        SharedWorker {
            eventtarget: EventTarget::new_inherited(),
        }
    }

    fn new(
        window: &Window,
        proto: Option<HandleObject>,
        sender: Sender<SharedWorkerScriptMsg>,
        closing: Arc<AtomicBool>,
        can_gc: CanGc,
    ) -> DomRoot<SharedWorker> {
        reflect_dom_object_with_proto(
            Box::new(SharedWorker::new_inherited(sender, closing)),
            window,
            proto,
            can_gc,
        )
    }
}

impl SharedWorkerMethods<crate::DomTypeHolder> for SharedWorker {
    fn Constructor(
        _window: &Window,
        _proto: Option<HandleObject>,
        _can_gc: CanGc,
        _script_url: TrustedScriptURLOrUSVString,
        _name_or_options: StringOrWorkerOptions,
    ) -> DomRoot<SharedWorker> {
        todo!()
    }

    event_handler!(error, GetOnerror, SetOnerror);    
}
