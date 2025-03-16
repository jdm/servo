/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::RefCell;
use std::marker::PhantomData;
use std::thread;

use js::jsapi::{GetScriptedCallerGlobal, HideScriptedCaller, JSTracer, UnhideScriptedCaller};
use js::rust::Runtime;
use script_bindings::interfaces::{DomHelpers, GlobalScopeHelpers};

use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::trace::JSTraceable;
use crate::dom::globalscope::GlobalScope;
use crate::script_runtime::CanGc;
use crate::DomTypes;

thread_local!(pub(super) static STACK: RefCell<Vec<StackEntry<crate::DomTypeHolder>>> = const {
    RefCell::new(Vec::new())
});

pub(crate) use script_bindings::settings_stack::{StackEntry, StackEntryKind};

/// Traces the script settings stack.
pub(crate) unsafe fn trace(tracer: *mut JSTracer) {
    STACK.with(|stack| {
        stack.borrow().trace(tracer);
    })
}

pub(crate) fn is_execution_stack_empty() -> bool {
    STACK.with(|stack| stack.borrow().is_empty())
}

pub(crate) type AutoEntryScript = GenericAutoEntryScript<crate::DomTypeHolder>;

/// Returns the ["entry"] global object.
///
/// ["entry"]: https://html.spec.whatwg.org/multipage/#entry
pub(crate) fn entry_global() -> DomRoot<GlobalScope> {
    STACK
        .with(|stack| {
            stack
                .borrow()
                .iter()
                .rev()
                .find(|entry| entry.kind == StackEntryKind::Entry)
                .map(|entry| DomRoot::from_ref(&*entry.global))
        })
        .unwrap()
}

pub(crate) type AutoIncumbentScript = GenericAutoIncumbentScript<crate::DomTypeHolder>;

pub(crate) use script_bindings::settings_stack::*;
