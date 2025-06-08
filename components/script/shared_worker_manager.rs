/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use script_bindings::root::{Dom, DomRoot};
use servo_url::ImmutableOrigin;

use crate::dom::bindings::trace::HashMapTracedValues;
use crate::dom::sharedworkerglobalscope::{SharedWorkerGlobalScope, SharedWorkerGlobalScopeInit};

struct SharedWorkerData {
    sender: Sender<SharedWorkerScriptMsg>,
    constructor_url: ServoUrl,
    closing: Arc<AtomicBool>,
    references: Arc<AtomicU32>,
    name: DOMString,
}

#[derive(JSTraceable)]
pub(crate) struct SharedWorkerManager {
    worker_globals: HashMapTracedValues<ImmutableOrigin, Vec<SharedWorkerData>>,
}

impl SharedWorkerManager {
    pub(crate) fn new() -> SharedWorkerManager {
        SharedWorkerManager {
            worker_globals: Default::default(),
        }
    }

    /// <https://html.spec.whatwg.org/multipage/#dom-sharedworker>
    pub(crate) fn create_worker(
        &mut self,
        origin: ImmutableOrigin,
        init: SharedWorkerGlobalScopeInit,
        can_gc: CanGc,
    ) -> DomRoot<SharedWorker> {
        // Step 11. Enqueue the following steps to the shared worker manager:
        // Step 11.1. Let worker global scope be null.
        // Step 11.2. For each scope in the list of all SharedWorkerGlobalScope objects:
        // Step 11.2.1. Let worker storage key be the result of running obtain a storage key for non-storage purposes given scope's relevant settings object.
        let worker_data = None;
        let origin_entry = self
            .worker_globals
            .entry(origin)
            .or_insert_with(Default::default);
        for data in &origin_entry {
            // Step 11.2.2. If all of the following are true:
            // * worker storage key equals outside storage key;
            // * scope's closing flag is false;
            // * scope's constructor url equals urlRecord; and
            // * scope's name equals the value of options's name member,
            if !data.closing.load(Ordering::Relaxed) &&
                data.constructor_url == init.worker_url &&
                data.name == init.worker_name
            {
                worker_data = Some(data);
                break
            }
        }

        // Step 11.3. If worker global scope is not null, but the user agent has
        // been configured to disallow communication between the worker represented
        // by the worker global scope and the scripts whose settings object is
        // outside settings, then set worker global scope to null.
        //---
        // No such configuration exists.

        // Step 11.4. If worker global scope is not null, then check if worker
        // global scope's type and credentials match the options values. If not,
        // queue a task to fire an event named error and abort these steps.
        // TODO

        // Step 11.5. If worker global scope is not null, then run these subsubsteps:
        if let Some(worker_data) = worker_data {
            // Step 11.5.1. Let settings object be the relevant settings object for worker global scope.
            // Step 11.5.2. Let workerIsSecureContext be true if settings object is a secure context; otherwise, false.
            // Step 11.5.3. If workerIsSecureContext is not callerIsSecureContext, then queue a task to fire an event named error at worker and abort these steps.
            // Step 11.5.4. Associate worker with worker global scope.
            // Step 11.5.5. Let inside port be a new MessagePort in settings object's realm.
            // Step 11.5.6. Entangle outside port and inside port.
            // Step 11.5.7. Queue a task, using the DOM manipulation task source,
            //   to fire an event named connect at worker global scope, using
            //   MessageEvent, with the data attribute initialized to the empty
            //   string, the ports attribute initialized to a new frozen array
            //   containing only inside port, and the source attribute initialized
            //   to inside port.

            let _ = data.references.fetch_add(1, Ordering::SeqCst);
        }

        // Step 11.6 Otherwise, in parallel, run a worker given worker, urlRecord, outside settings, outside port, and option.
        if worker_data.is_none() {
            let (worker_sender, worker_receiver) = unbounded();
            let data = SharedWorkerData {
                sender: worker_sender.clone(),
                constructor_url: init.worker_url.clone(),
                closing: init.closing.clone(),
                references: Arc::new(AtomicU32::new(1)),
                name: init.worker_name.clone(),
            };
            let _ = DedicatedWorkerGlobalScope::run_worker_scope(
                init.init,
                init.worker_url,
                init.from_devtools_receiver,
                todo!(), // trusted worker address
                window.event_loop_sender(),
                worker_sender,
                worker_receiver,
                todo!(), // worker script load origin
                init.worker_name,
                init.worker_type,
                init.closing,
                window.image_cache(),
                Some(window.window_proxy().browsing_context_id()),
                init.gpu_id_hub,
                todo!(), // control receiver
                todo!(), // js context sender
                init.insecure_requests_policy,
                window.upcast::<GlobalScope>().policy_container(),
            );
            origin_entry.push(data);
            worker_data = origin_entry.last();
        }

        let worker_data = worker_data.expect("A worker sender should have been created by this point");

        // Step 12. Return worker.
        SharedWorker::new(
            window,
            None,
            worker_data.sender.clone(),
            worker_data.closing.clone(),
            CanGc::note(),
        )
    }
}
