/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use actor::{Actor, ActorRegistry};
use protocol::JsonPacketStream;

use rustc_serialize::json;
use std::net::TcpStream;

#[derive(RustcEncodable)]
struct ReconfigureReply {
    from: String,
}

#[derive(RustcEncodable)]
struct PausedReply {
    from: String,
    __type__: String,
    actor: String,
    why: String,
}

struct PauseActor {
    name: String,
}

impl Actor for PauseActor {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn handle_message(&self,
                      _registry: &ActorRegistry,
                      _msg_type: &str,
                      _msg: &json::Object,
                      _stream: &mut TcpStream) -> Result<bool, ()> {
        Ok(false)
    }
}

pub struct ThreadActor {
    pub name: String,
}

impl Actor for ThreadActor {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn handle_message(&self,
                      registry: &ActorRegistry,
                      msg_type: &str,
                      _msg: &json::Object,
                      stream: &mut TcpStream) -> Result<bool, ()> {
        Ok(match msg_type {
            "attach" => {
                let pause = PauseActor {
                    name: registry.new_name("pause"),
                };
                let msg = PausedReply {
                    from: self.name(),
                    __type__: "paused".to_string(),
                    actor: pause.name(),
                    why: "attached".to_string(),
                };
                registry.register_later(box pause);
                stream.write_json_packet(&msg);
                true
            }

            "reconfigure" => {
                let msg = ReconfigureReply {
                    from: self.name(),
                };
                stream.write_json_packet(&msg);
                true
            },

            "sources" => {
                
            },

            _ => false,
        })
    }
}

struct SourceActor {
}
