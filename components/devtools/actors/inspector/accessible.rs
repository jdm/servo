/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;

use malloc_size_of_derive::MallocSizeOf;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::StreamId;
use crate::actor::{Actor, ActorEncode, ActorError, ActorRegistry, new_actor_name};
use crate::actors::browsing_context::BrowsingContextActor;
use crate::protocol::ClientRequest;

#[derive(Serialize)]
struct AuditReply {
    from: String,
    audit: Option<String>,
}

#[derive(Serialize)]
struct HydrateReply {
    from: String,
    properties: Map<String, Value>,
}

#[derive(Serialize)]
struct AccessibleRelation {
    #[serde(rename = "type")]
    type_: String,
    targets: Vec<AccessibleActorMsg>,
}

#[derive(Serialize)]
struct GetRelationsReply {
    from: String,
    relations: Vec<AccessibleRelation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessibleActorMsg {
    actor: String,
    role: String,
    level: Option<String>,
    name: Option<String>,
    use_child_target_to_fetch_children: bool,
    child_count: u32,
    checks: Map<String, Value>,
}

#[derive(MallocSizeOf)]
enum NodeActor {
    BrowsingContextRoot(String),
    Node(String),
}

#[derive(MallocSizeOf)]
pub(crate) struct AccessibleActor {
    name: String,
    role: String,
    level: Option<String>,
    acc_name: Option<String>,
    child_count: u32,
    node_actor: NodeActor,
}

impl Actor for AccessibleActor {
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
            "audit" => {
                let msg = AuditReply {
                    from: self.name().into(),
                    audit: None,
                };
                request.reply_final(&msg)?
            },
            "hydrate" => {
                let msg = HydrateReply {
                    from: self.name().into(),
                    properties: Default::default(),
                };
                request.reply_final(&msg)?
            },
            "getRelations" => {
                let msg = GetRelationsReply {
                    from: self.name().into(),
                    relations: Default::default(),
                };
                request.reply_final(&msg)?
            },
            _ => return Err(ActorError::UnrecognizedPacketType),
        };
        Ok(())
    }
}

impl AccessibleActor {
    pub fn node_actor(&self, registry: &ActorRegistry) -> String {
        match &self.node_actor {
            NodeActor::BrowsingContextRoot(browsing_context) => {
                let browsing_context_actor =
                    registry.find::<BrowsingContextActor>(&browsing_context);
                let root_node = browsing_context_actor.root_node(registry).unwrap();
                root_node.actor.clone()
            },
            NodeActor::Node(_) => unreachable!(),
        }
    }

    pub fn register(registry: &ActorRegistry, browsing_context: String) -> Arc<Self> {
        let name = new_actor_name::<Self>();
        let actor = Self {
            name,
            role: "".to_string(),
            level: None,
            acc_name: None,
            child_count: 0,
            node_actor: NodeActor::BrowsingContextRoot(browsing_context),
        };
        registry.register::<Self>(actor)
    }
}

impl ActorEncode<AccessibleActorMsg> for AccessibleActor {
    fn encode(&self, _: &ActorRegistry) -> AccessibleActorMsg {
        AccessibleActorMsg {
            actor: self.name().into(),
            role: self.role.clone(),
            level: self.level.clone(),
            name: self.acc_name.clone(),
            use_child_target_to_fetch_children: false,
            child_count: self.child_count,
            checks: Default::default(),
        }
    }
}
