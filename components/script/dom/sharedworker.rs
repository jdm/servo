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
    droppable: DroppableSharedWorker,
}

struct DroppableSharedWorker {
    sender: Sender<SharedWorkerScriptMsg>,
    reference: Arc<AtomicU32>,
}

impl Drop for DroppableSharedWorker {
    fn drop(&mut self) {
        if self.reference.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = self.sender.send(SharedWorkerScriptMsg::Teardown);
        }
    }
}

impl SharedWorker {
    fn new_inherited(sender: Sender<SharedWorkerScriptMsg>, _closing: Arc<AtomicBool>, reference: Arc<AtomicU32>) -> SharedWorker {
        SharedWorker {
            eventtarget: EventTarget::new_inherited(),
            droppable: DroppableSharedWorker {
                reference,
                sender,
            }
        }
    }

    fn new(
        window: &Window,
        proto: Option<HandleObject>,
        sender: Sender<SharedWorkerScriptMsg>,
        closing: Arc<AtomicBool>,
        reference: Arc<AtomicU32>,
        can_gc: CanGc,
    ) -> DomRoot<SharedWorker> {
        reflect_dom_object_with_proto(
            Box::new(SharedWorker::new_inherited(sender, closing, reference)),
            window,
            proto,
            can_gc,
        )
    }
}

impl SharedWorkerMethods<crate::DomTypeHolder> for SharedWorker {
    fn Constructor(
        window: &Window,
        proto: Option<HandleObject>,
        can_gc: CanGc,
        script_url: TrustedScriptURLOrUSVString,
        name_or_options: StringOrWorkerOptions,
    ) -> Fallible<DomRoot<SharedWorker>> {
        // Step 1. Let compliantScriptURL be the result of invoking the Get Trusted Type compliant string algorithm with TrustedScriptURL, this's relevant global object, scriptURL, "SharedWorker constructor", and "script".
        let global = window.upcast();
        let compliant_script_url = TrustedScriptURL::get_trusted_script_url_compliant_string(
            global,
            script_url,
            "SharedWorker",
            "constructor",
            CanGc::note(),
        );

        // Step 2. If options is a DOMString, set options to a new WorkerOptions dictionary whose name member is set to the value of options and whose other members are set to their default values.
        let options = match name_or_options {
            StringOrWorkerOptions::String(name) => WorkerOptions {
                name,
                ..Default::default()
            },
            StringOrWorkerOptions::WorkerOptions(options) => options,
        };

        // Step 3. Let outside settings be the current settings object.

        // Step 4. Let urlRecord be the result of encoding-parsing a URL given compliantScriptURL, relative to outside settings.
        // Step 5. If urlRecord is failure, then throw a "SyntaxError" DOMException.
        let Ok(url_record) = global.api_base_url().join(&compliant_script_url) else {
            return Err(Error::Syntax);
        };

        let init = SharedWorkerGlobalScopeInit {
            init: prepare_workerscope_init(global, todo!(), None),
            worker_name: options.name.clone(),
            worker_type: options.type_,
            worker_url: compliant_script_url,
            from_devtools_receiver: todo!(),
            runtime: todo!(),
            closing: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "webgpu")] gpu_id_hub: global.wgpu_id_hub(),
            insecure_requests_policy: global.insecure_requests_policy(),
        };

        Ok(ScriptThread::construct_shared_worker(global.origin(), init))
    }

    event_handler!(error, GetOnerror, SetOnerror);    
}
