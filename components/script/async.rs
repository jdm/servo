/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Helpers for interacting with the DOM asynchronously.

use dom::bindings::refcounted::Trusted;
use dom::bindings::reflector::Reflectable;
use script_thread::Runnable;

/// Convert a value into one that can be safely stored for an unbounded period
/// of time without being traced by the GC.
pub trait ToAsync {
    type Output: Send + 'static;
    fn into_async(self) -> Self::Output;
}

impl ToAsync for &'static str {
    type Output = &'static str;
    fn into_async(self) -> Self {
        self
    }
}

impl<T: ToAsync> ToAsync for (T,) {
    type Output = (T::Output,);
    fn into_async(self) -> (T::Output,) {
        (self.0.into_async(),)
    }
}

impl<T: ToAsync, U: ToAsync> ToAsync for (T,U) {
    type Output = (T::Output, U::Output);
    fn into_async(self) -> (T::Output, U::Output) {
        (self.0.into_async(), self.1.into_async())
    }
}

impl<'a, T: Reflectable + 'static> ToAsync for &'a T {
    type Output = Trusted<T>;
    fn into_async(self) -> Trusted<T> {
        Trusted::new(self)
    }
}

/// Encapsulation of a closure to run asynchronously, that will be passed
/// a tuple of values as an argument.
pub struct AsyncAction<F, T> {
    f: F,
    args: T,
}

impl<F: Fn(T::Output) + Send + 'static, T: ToAsync> AsyncAction<F, T> {
    /// Create a new instance of an AsyncAction.
    pub fn new(f: F, args: T) -> AsyncAction<F, T::Output> {
        AsyncAction {
            f: f,
            args: args.into_async(),
        }
    }
}

impl<F: Fn(T), T> Runnable for AsyncAction<F, T> {
    fn handler(self: Box<AsyncAction<F, T>>) {
        let this = *self;
        (this.f)(this.args);
    }
}

pub fn do_soon<F, T, Q>(&self, f: F, args: T, queue: &Q) where F: Fn(T::Output) + Send + 'static,
                                                               T: ToAsync,
                                                               Q: ScriptChan {
    let runnable = box AsyncAction::new(f, args);
    let msg = CommonScriptMsg::RunnableMsg(ScriptThreadEventCategory::DomEvent, runnable);
    let _ = queue.send(msg);
    }
}
