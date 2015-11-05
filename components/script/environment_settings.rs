/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![allow(dead_code)]

use dom::bindings::global::GlobalRoot;
use dom::bindings::js::Root;
use dom::bindings::trace::JSTraceable;
use dom::document::Document;
use encoding::EncodingRef;
use origin::Origin;
use script_task::ScriptChan;
use std::cell::RefCell;
use url::Url;

/// https://html.spec.whatwg.org/multipage/#https-state
pub enum HttpsState {
    Modern,
    Deprecated,
}

/// https://html.spec.whatwg.org/multipage/#environment-settings-object
pub trait EnvironmentSettings: JSTraceable {
    fn global(&self) -> GlobalRoot;
    fn responsible_event_loop(&self) -> Box<ScriptChan + Send>;
    fn responsible_document(&self) -> Option<Root<Document>>;
    fn api_url_character_encoding(&self) -> EncodingRef;
    fn api_base_url(&self) -> Url;
    fn origin(&self) -> Origin;
    fn effective_script_origin(&self) -> Origin;
    fn creation_url(&self) -> Url;
    fn https_state(&self) -> Option<HttpsState>;
    fn clone(&self) -> Box<EnvironmentSettings + 'static>;
}

#[derive(PartialEq)]
enum SettingsLabel {
    Candidate,
    NonCandidate,
}

thread_local!(static SCRIPT_SETTINGS_STACK: RefCell<ScriptSettingsStack> =
              RefCell::new(ScriptSettingsStack::new()));

/// https://html.spec.whatwg.org/multipage/#stack-of-script-settings-objects
pub struct ScriptSettingsStack {
    stack: Vec<(Box<EnvironmentSettings + 'static>, SettingsLabel)>,
}

impl ScriptSettingsStack {
    fn new() -> ScriptSettingsStack {
        ScriptSettingsStack {
            stack: vec![],
        }
    }

    pub fn push(object: Box<EnvironmentSettings + 'static>) {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            tls.borrow_mut().stack.push((object, SettingsLabel::NonCandidate));
        })
    }

    pub fn push_candidate(object: Box<EnvironmentSettings + 'static>) {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            tls.borrow_mut().stack.push((object, SettingsLabel::Candidate));
        })
    }

    pub fn pop_incumbent_settings_object() {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            let _ = tls.borrow_mut().stack.pop().unwrap();
        })
    }

    pub fn is_empty() -> bool {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            tls.borrow().stack.is_empty()
        })
    }

    /// https://html.spec.whatwg.org/multipage/#entry-settings-object
    pub fn entry_settings_object<F, R>(f: F) -> Option<R>
                                       where F: Fn(&(EnvironmentSettings + 'static)) -> R {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            tls.borrow()
               .stack
               .iter()
               .rev()
               .find(|entry| entry.1 == SettingsLabel::Candidate)
               .map(|entry| f(&*entry.0))
        })
    }

    /// https://html.spec.whatwg.org/multipage/#incumbent-settings-object
    pub fn incumbent_settings_object<F, R>(f: F) -> Option<R>
                                           where F: Fn(&(EnvironmentSettings + 'static)) -> R {
        SCRIPT_SETTINGS_STACK.with(|tls| {
            tls.borrow().stack.last().map(|entry| f(&*entry.0))
        })
    }
}
