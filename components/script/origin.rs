/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![allow(unsafe_code)]

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use url::{Url, Host};

/// A representation of an [origin](https://url.spec.whatwg.org/#origin)
#[derive(Clone, JSTraceable, HeapSizeOf)]
pub struct Origin {
    #[ignore_heap_size_of = "Rc<T> has unclear ownership semantics"]
    repr: Rc<OriginRepresentation>,
}

#[derive(Clone, JSTraceable)]
enum OriginRepresentation {
    OpaqueIdentifier { id: u64, debug_repr: String },
    Tuple { scheme: String, host: Host, port: u16, },
    Alias(Rc<OriginRepresentation>),
}

lazy_static! {
    static ref NEXT_OPAQUE_ID: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
}

fn next_opaque_id() -> u64 {
    //FIXME: this won't be globally unique in multiprocess
    let mut id_storage = NEXT_OPAQUE_ID.lock().unwrap();
    let cur_id = *id_storage;
    *id_storage += 1;
    cur_id
}

impl Origin {
    /// https://url.spec.whatwg.org/#origin
    pub fn from_url(url: &Url) -> Origin {
        match &url.scheme[..] {
            "blob" => {
                match Url::parse(&url.path().unwrap()[0][..]) {
                    Ok(parsed) => Origin::from_url(&parsed),
                    Err(_) => Origin::opaque_identifier(url.clone()),
                }
            }

            "ftp" | "gopher" | "http" | "https" | "ws" | "wss" => {
                Origin {
                    repr: Rc::new(OriginRepresentation::Tuple {
                        scheme: url.scheme.clone(),
                        host: url.host().unwrap().clone(),
                        port: url.port_or_default().unwrap(),
                    }),
                }
            }

            // Call this out specially from the default case just to be clear
            // that file URLs are cross-origin to each other.
            "file" => {
                Origin::opaque_identifier(url.clone())
            }

            _ => {
                Origin::opaque_identifier(url.clone())
            }
        }
    }

    pub fn opaque_identifier(url: Url) -> Origin {
        let opaque_id = OriginRepresentation::OpaqueIdentifier {
            id: next_opaque_id(),
            debug_repr: url.serialize(),
        };
        Origin {
            repr: Rc::new(opaque_id),
        }
    }

    pub fn alias(&self) -> Origin {
        Origin {
            repr: Rc::new(OriginRepresentation::Alias(self.repr.clone())),
        }
    }

}

trait Dealias {
    fn dealiased(&self) -> Self;
}

impl Dealias for Rc<OriginRepresentation> {
    fn dealiased(&self) -> Rc<OriginRepresentation> {
        match **self {
            OriginRepresentation::Alias(ref aliased) => aliased.dealiased(),
            _ => self.clone(),
        }
    }
}

impl PartialEq for Origin {
    fn eq(&self, other: &Origin) -> bool {
        match (&*self.repr.dealiased(), &*other.repr.dealiased()) {
            (&OriginRepresentation::OpaqueIdentifier { id: id1, .. },
             &OriginRepresentation::OpaqueIdentifier { id: id2, .. }) => id1 == id2,
            (&OriginRepresentation::Tuple { scheme: ref scheme1, host: ref host1, port: ref port1 },
             &OriginRepresentation::Tuple { scheme: ref scheme2, host: ref host2, port: ref port2 }) =>
                scheme1 == scheme2 && host1 == host2 && port1 == port2,
            _ => false,
        }
    }
}

/// Data that is strongly associated with a particular origin, such that it must only
/// be accessible from callers that can claim association with the same origin.
pub struct OriginTainted<T> {
    origin: Origin,
    data: T,
}

impl<T> OriginTainted<T> {
    /// Taint this data such that is will be considered cross-origin to every origin.
    /// The provided url is merely for providing useful debugging information.
    pub fn taint(&mut self, other: Url) {
        self.origin = Origin::opaque_identifier(other);
    }

    /// Attempt to gain access to the enclosed data. Returns None if the provided origin
    /// is not considered same-origin to the current associated taint.
    pub fn access(&self, other: Origin) -> Option<&T> {
        if self.origin == other {
            return Some(&self.data);
        }
        None
    }

    /// Access the enclosed data without performing any origin-related checks.
    /// Use with extreme caution.
    pub fn bypass_same_origin_check(&self) -> &T {
        &self.data
    }
}
