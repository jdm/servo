/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;

use malloc_size_of_derive::MallocSizeOf;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::StreamId;
use crate::actor::{Actor, ActorError, ActorRegistry, new_actor_name};
use crate::actors::browsing_context::BrowsingContextActor;
use crate::actors::inspector::accessible::{AccessibleActor, AccessibleActorMsg};
use crate::protocol::ClientRequest;

#[derive(Serialize)]
struct HideTabbingOrderReply {
    from: String,
}

#[derive(Serialize)]
struct ChildrenReply {
    from: String,
    children: Vec<AccessibleActorMsg>,
}

#[derive(Serialize)]
struct UnhighlightReply {
    from: String,
}

#[derive(MallocSizeOf)]
pub(crate) struct AccessibleWalkerActor {
    name: String,
    children: Vec<String>,
}

impl AccessibleWalkerActor {
    pub fn register(registry: &ActorRegistry, browsing_context_actor: String) -> Arc<Self> {
        let document_accessible = AccessibleActor::register(registry, browsing_context_actor);
        let name = new_actor_name::<Self>();
        let actor = Self {
            name,
            children: vec![document_accessible.name().into()],
        };
        registry.register::<Self>(actor)
    }
}

impl Actor for AccessibleWalkerActor {
    fn name(&self) -> &str {
        &self.name
    }

    fn handle_message(
        &self,
        request: ClientRequest,
        registry: &ActorRegistry,
        msg_type: &str,
        _msg: &Map<String, Value>,
        _id: StreamId,
    ) -> Result<(), ActorError> {
        match msg_type {
            "hideTabbingOrder" => {
                let msg = HideTabbingOrderReply {
                    from: self.name().into(),
                };
                request.reply_final(&msg)?
            },
            "children" => {
                let msg = ChildrenReply {
                    from: self.name().into(),
                    children: self
                        .children
                        .iter()
                        .map(|name| registry.encode::<AccessibleActor, _>(name))
                        .collect(),
                };
                request.reply_final(&msg)?
            },
            "unhighlight" => {
                let msg = UnhighlightReply {
                    from: self.name().into(),
                };
                request.reply_final(&msg)?
            },
            "highlightAccessible" => {
                let msg = UnhighlightReply {
                    from: self.name().into(),
                };
                request.reply_final(&msg)?
            },
            _ => return Err(ActorError::UnrecognizedPacketType),
        };
        Ok(())
    }
}
