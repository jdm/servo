/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use script_bindings::root::{Dom, DomRoot};
use servo_url::ImmutableOrigin;

use crate::dom::bindings::trace::HashMapTracedValues;
use crate::dom::sharedworkerglobalscope::{SharedWorkerGlobalScope, SharedWorkerGlobalScopeInit};

#[derive(JSTraceable)]
pub(crate) struct SharedWorkerManager {
    worker_globals: HashMapTracedValues<ImmutableOrigin, Dom<SharedWorkerGlobalScope>>,
}

impl SharedWorkerManager {
    pub(crate) fn new() -> SharedWorkerManager {
        SharedWorkerManager {
            worker_globals: Default::default(),
        }
    }

    pub(crate) fn worker_for_origin(&self, origin: &ImmutableOrigin) -> Option<&SharedWorkerGlobalScope> {
        self.worker_globals.get(origin).map(|v| &**v)
    }

    pub(crate) fn start_new_worker(&mut self, origin: ImmutableOrigin, init: SharedWorkerGlobalScopeInit) -> DomRoot<SharedWorkerGlobalScope> {
        let worker = SharedWorkerGlobalScope::new(init);
        self.worker_globals.insert(origin, worker.clone().as_traced());
        worker
    }
}
