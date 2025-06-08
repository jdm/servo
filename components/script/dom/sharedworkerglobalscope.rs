/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use constellation_traits::WorkerGlobalScopeInit;
use crossbeam_channel::{Receiver};
use devtools_traits::DevtoolScriptControlMsg;
use dom_struct::dom_struct;
use net_traits::request::InsecureRequestsPolicy;
use script_bindings::root::DomRoot;
use script_bindings::script_runtime::JSContext as SafeJSContext;
use script_bindings::str::DOMString;
use servo_url::ServoUrl;

use crate::dom::abstractworker::WorkerScriptMsg;
use crate::dom::bindings::codegen::Bindings::SharedWorkerGlobalScopeBinding;
use crate::dom::bindings::codegen::Bindings::SharedWorkerGlobalScopeBinding::SharedWorkerGlobalScopeMethods;
use crate::dom::bindings::codegen::Bindings::WorkerBinding::WorkerType;
#[cfg(feature = "webgpu")]
use crate::dom::webgpu::identityhub::IdentityHub;
use crate::dom::workerglobalscope::WorkerGlobalScope;
use crate::script_runtime::Runtime;

pub(crate) struct SharedWorkerScriptMsg(WorkerScriptMsg);

pub(crate) struct SharedWorkerGlobalScopeInit {
    init: WorkerGlobalScopeInit,
    worker_name: DOMString,
    worker_type: WorkerType,
    worker_url: ServoUrl,
    from_devtools_receiver: IpcReceiver<DevtoolScriptControlMsg>,
    runtime: Runtime,
    closing: Arc<AtomicBool>,
    #[cfg(feature = "webgpu")] gpu_id_hub: Arc<IdentityHub>,
    insecure_requests_policy: InsecureRequestsPolicy,
}

#[dom_struct]
pub(crate) struct SharedWorkerGlobalScope {
    workerglobalscope: WorkerGlobalScope,
}

impl SharedWorkerGlobalScope {
    fn new_inherited(
        init: SharedWorkerGlobalScopeInit,
    ) -> SharedWorkerGlobalScope {
        SharedWorkerGlobalScope {
            workerglobalscope: WorkerGlobalScope::new_inherited(
                init.init,
                init.worker_name,
                init.worker_type,
                init.worker_url,
                init.runtime,
                init.from_devtools_receiver,
                init.closing,
                #[cfg(feature = "webgpu")]
                init.gpu_id_hub,
                init.insecure_requests_policy,
            ),
            references: Arc::new(AtomicU32::new(0)),
        }
    }

    #[allow(unsafe_code)]
    pub(crate) fn new(
        init: SharedWorkerGlobalScopeInit,
    ) -> DomRoot<SharedWorkerGlobalScope> {
        let cx = init.runtime.cx();
        let scope = Box::new(SharedWorkerGlobalScope::new_inherited(init));
        unsafe {
            SharedWorkerGlobalScopeBinding::Wrap::<crate::DomTypeHolder>(
                SafeJSContext::from_ptr(cx),
                scope,
            )
        }
    }
}

impl SharedWorkerGlobalScopeMethods<crate::DomTypeHolder> for SharedWorkerGlobalScope {
    fn Name(&self) -> DOMString {
        DOMString::from("".to_owned())
    }

    fn Close(&self) {
    }

    event_handler!(error, GetOnconnect, SetOnconnect);
}
