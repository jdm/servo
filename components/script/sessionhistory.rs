/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! An implementation of the concept of session history:
//! https://html.spec.whatwg.org/multipage/browsers.html#the-session-history-of-browsing-contexts

use dom::bindings::js::{JS, Temporary, JSRef};
use dom::document::{Document, DocumentHelpers};
use dom::node::window_from_node;
use dom::window::Window;

#[jstraceable]
#[must_root]
/// A session history encapsulation for a particular browsing context.
pub struct SessionHistory {
    history: Vec<JS<Document>>,
    active_index: uint,
}

impl SessionHistory {
    /// Create a new session history.
    pub fn new() -> SessionHistory {
        SessionHistory {
            history: vec!(),
            active_index: 0,
        }
    }

    fn actual_index(&self) -> uint {
        self.active_index - 1
    }

    /// Append a new session history entry.
    pub fn push(&mut self, document: JSRef<Document>) {
        self.history.push(JS::from_rooted(document));
        self.active_index += 1;
    }

    /// Replace the current session history entry.
    pub fn replace(&mut self, document: JSRef<Document>) {
        let idx = self.actual_index();
        self.history[idx] = JS::from_rooted(document);
    }

    /// Retrieve the associated document for the current session history entry.
    pub fn active_document(&self) -> Temporary<Document> {
        let idx = self.actual_index();
        Temporary::new(self.history[idx].clone())
    }

    /// Retrieve the associated window for the current session history entry.
    pub fn active_window(&self) -> Temporary<Window> {
        let doc = self.active_document().root();
        doc.r().window()
    }

    /// Execute a callback for every known window object in this session history.
    pub fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        for doc in self.history.iter() {
            let doc = doc.root();
            let win = window_from_node(doc.r()).root();
            f(win.r());
        }
    }
}
