/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;

use malloc_size_of_derive::MallocSizeOf;
use serde::Serialize;
use serde_json::{Map, Value};
use servo_config::pref;

use crate::StreamId;
use crate::actor::{Actor, ActorError, ActorRegistry, new_actor_name};
use crate::protocol::ClientRequest;

#[derive(Serialize)]
struct BootstrapReply {
    from: String,
    state: BootstrapConfiguration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapConfiguration {
    can_be_disabled: bool,
    can_be_enabled: bool,
}

#[derive(MallocSizeOf)]
pub(crate) struct ParentAccessibilityActor {
    name: String,
}

impl ParentAccessibilityActor {
    pub fn register(registry: &ActorRegistry) -> Arc<Self> {
        let name = new_actor_name::<Self>();
        let actor = ParentAccessibilityActor { name };
        registry.register::<Self>(actor)
    }
}

impl Actor for ParentAccessibilityActor {
    fn name(&self) -> &str {
        &self.name
    }
    fn handle_message(
        &self,
        request: ClientRequest,
        _registry: &ActorRegistry,
        msg_type: &str,
        _msg: &Map<String, Value>,
        _id: StreamId,
    ) -> Result<(), ActorError> {
        match msg_type {
            "bootstrap" => {
                let msg = BootstrapReply {
                    from: self.name().into(),
                    state: BootstrapConfiguration {
                        can_be_enabled: !pref!(accessibility_enabled),
                        can_be_disabled: pref!(accessibility_enabled),
                    },
                };
                request.reply_final(&msg)?
            },
            "enable" => return Err(ActorError::UnrecognizedPacketType),
            "disable" => return Err(ActorError::UnrecognizedPacketType),
            _ => return Err(ActorError::UnrecognizedPacketType),
        };
        Ok(())
    }
}
