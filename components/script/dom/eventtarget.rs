/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use dom::bindings::callback::{CallbackContainer, ExceptionHandling};
use dom::bindings::cell::DOMRefCell;
use dom::bindings::codegen::Bindings::EventHandlerBinding::EventHandlerNonNull;
use dom::bindings::codegen::Bindings::EventListenerBinding::EventListener;
use dom::bindings::codegen::Bindings::EventTargetBinding::EventTargetMethods;
use dom::bindings::codegen::Bindings::WindowBinding::WindowMethods;
use dom::bindings::codegen::InheritTypes::EventTargetTypeId;
use dom::bindings::conversions::Castable;
use dom::bindings::error::{Error, Fallible, report_pending_exception};
use dom::bindings::utils::{Reflectable, Reflector};
use dom::element::Element;
use dom::event::Event;
use dom::eventdispatcher::dispatch_event;
use dom::node::document_from_node;
use dom::virtualmethods::VirtualMethods;
use dom::window::Window;
use fnv::FnvHasher;
use js::jsapi::{CompileFunction, JS_GetFunctionObject};
use js::jsapi::{HandleObject, JSContext, RootedFunction};
use js::jsapi::{JSAutoCompartment, JSAutoRequest};
use js::rust::{AutoObjectVectorWrapper, CompileOptionsWrapper};
use libc::{c_char, size_t};
use script::{ErrorReporting, Script};
use std::borrow::{Cow, ToOwned};
use std::collections::HashMap;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::hash_state::DefaultState;
use std::default::Default;
use std::ffi::CString;
use std::rc::Rc;
use std::{intrinsics, ptr};
use url::Url;
use util::mem::HeapSizeOf;
use util::str::DOMString;

pub type EventHandler = EventHandlerNonNull;

#[derive(JSTraceable, Copy, Clone, PartialEq, HeapSizeOf)]
pub enum ListenerPhase {
    Capturing,
    Bubbling,
}

impl PartialEq for EventTargetTypeId {
    #[inline]
    fn eq(&self, other: &EventTargetTypeId) -> bool {
        match (*self, *other) {
            (EventTargetTypeId::Node(this_type), EventTargetTypeId::Node(other_type)) => {
                this_type == other_type
            }
            _ => self.eq_slow(other)
        }
    }
}

impl EventTargetTypeId {
    #[allow(unsafe_code)]
    fn eq_slow(&self, other: &EventTargetTypeId) -> bool {
        match (*self, *other) {
            (EventTargetTypeId::Node(this_type), EventTargetTypeId::Node(other_type)) => {
                this_type == other_type
            }
            (EventTargetTypeId::WorkerGlobalScope(this_type),
             EventTargetTypeId::WorkerGlobalScope(other_type)) => {
                this_type == other_type
            }
            (EventTargetTypeId::XMLHttpRequestEventTarget(this_type),
             EventTargetTypeId::XMLHttpRequestEventTarget(other_type)) => {
                this_type == other_type
            }
            (_, _) => {
                unsafe {
                    intrinsics::discriminant_value(self) == intrinsics::discriminant_value(other)
                }
            }
        }
    }
}

#[derive(JSTraceable, PartialEq, Clone)]
pub enum InlineEventListener {
    Uncompiled(Option<(DOMString, Url, usize)>),
    Compiled(Rc<EventHandler>),
}

impl InlineEventListener {
    fn get_compiled_handler(&mut self, owner: &EventTarget, ty: &str)
                            -> Option<Rc<EventHandler>> {
        match self {
            &mut InlineEventListener::Uncompiled(ref mut inner) => {
                let (source, url, line) = inner.take().unwrap();
                owner.get_compiled_event_handler(url, line, ty, source)
            }
            &mut InlineEventListener::Compiled(ref handler) => Some(handler.clone()),
        }
    }
}

#[derive(JSTraceable, Clone, PartialEq)]
pub enum EventListenerType {
    Additive(Rc<EventListener>),
    Inline(InlineEventListener),
}

pub enum CompiledEventListener {
    Listener(Rc<EventListener>),
    Handler(Rc<EventHandler>),
}

impl EventListenerType {
    fn get_compiled_listener(&mut self, owner: &EventTarget, ty: &str)
                             -> Option<CompiledEventListener> {
        match self {
            &mut EventListenerType::Inline(ref mut inline) =>
                inline.get_compiled_handler(owner, ty)
                      .map(|h| CompiledEventListener::Handler(h)),
            &mut EventListenerType::Additive(ref listener) =>
                Some(CompiledEventListener::Listener(listener.clone())),
        }
    }
}

impl HeapSizeOf for EventListenerType {
    fn heap_size_of_children(&self) -> usize {
        // FIXME: Rc<T> isn't HeapSizeOf and we can't ignore it due to #6870 and #6871
        0
    }
}

impl CompiledEventListener {
    pub fn call_or_handle_event<T: Reflectable>(&self,
                                                object: &T,
                                                event: &Event,
                                                exception_handle: ExceptionHandling) {
        match *self {
            CompiledEventListener::Listener(ref listener) => {
                let _ = listener.HandleEvent_(object, event, exception_handle);
            },
            CompiledEventListener::Handler(ref handler) => {
                let _ = handler.Call_(object, event, exception_handle);
            },
        }
    }
}

#[derive(JSTraceable, Clone, PartialEq, HeapSizeOf)]
#[privatize]
pub struct EventListenerEntry {
    phase: ListenerPhase,
    listener: EventListenerType,
}

#[derive(JSTraceable)]
struct EventListeners {
    num_uncompiled: usize,
    listeners: Vec<EventListenerEntry>,
}

impl Default for EventListeners {
    fn default() -> EventListeners {
        EventListeners {
            listeners: vec!(),
            num_uncompiled: 0,
        }
    }
}

impl EventListeners {
    fn get_inline_listener(&mut self, owner: &EventTarget, ty: &str) -> Option<Rc<EventHandler>> {
        let mut to_remove = None;

        for (idx, entry) in self.listeners.iter_mut().enumerate() {
            if let EventListenerType::Inline(ref mut inline) = entry.listener {
                let result = inline.get_compiled_handler(owner, ty);
                if result.is_some() {
                    return result;
                }

                to_remove = Some(idx);
                break;
            }
        }

        if let Some(idx) = to_remove {
            self.listeners.remove(idx);
        }
        None
    }

    fn get_listeners(&mut self, phase: Option<ListenerPhase>, owner: &EventTarget, ty: &str)
                     -> Vec<CompiledEventListener> {
        let mut to_remove = vec![];
        let result = self.listeners.iter_mut().enumerate().filter_map(|(idx, entry)| {
            if phase.is_none() || Some(entry.phase) == phase {
                if let Some(listener) = entry.listener.get_compiled_listener(owner, ty) {
                    Some(listener)
                } else {
                    to_remove.push(idx);
                    None
                }
            } else {
                None
            }
        }).collect();

        for (position, idx) in to_remove.iter().enumerate() {
            self.listeners.remove(idx - position);
        }

        result
    }
}

#[dom_struct]
pub struct EventTarget {
    reflector_: Reflector,
    #[ignore_heap_size_of = "XXXjdm"]
    handlers: DOMRefCell<HashMap<DOMString, EventListeners, DefaultState<FnvHasher>>>,
}

impl EventTarget {
    pub fn new_inherited() -> EventTarget {
        EventTarget {
            reflector_: Reflector::new(),
            handlers: DOMRefCell::new(Default::default()),
        }
    }

    pub fn get_listeners(&self, type_: &str) -> Option<Vec<CompiledEventListener>> {
        self.handlers.borrow_mut().get_mut(type_).map(|listeners| {
            listeners.get_listeners(None, self, type_)
        })
    }

    pub fn get_listeners_for(&self, type_: &str, desired_phase: ListenerPhase)
                             -> Option<Vec<CompiledEventListener>> {
        self.handlers.borrow_mut().get_mut(type_).map(|listeners| {
            listeners.get_listeners(Some(desired_phase), self, type_)
        })
    }

    pub fn dispatch_event_with_target(&self,
                                  target: &EventTarget,
                                  event: &Event) -> bool {
        dispatch_event(self, Some(target), event)
    }

    pub fn dispatch_event(&self, event: &Event) -> bool {
        dispatch_event(self, None, event)
    }

    pub fn set_inline_event_listener(&self,
                                 ty: DOMString,
                                 listener: Option<InlineEventListener>) {
        let mut handlers = self.handlers.borrow_mut();
        let entries = match handlers.entry(ty) {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(Default::default()),
        };

        let idx = entries.listeners.iter().position(|ref entry| {
            match entry.listener {
                EventListenerType::Inline(_) => true,
                _ => false,
            }
        });

        match idx {
            Some(idx) => {
                match listener {
                    Some(listener) => entries.listeners[idx].listener = EventListenerType::Inline(listener),
                    None => {
                        entries.listeners.remove(idx);
                    }
                }
            }
            None => {
                if listener.is_some() {
                    entries.listeners.push(EventListenerEntry {
                        phase: ListenerPhase::Bubbling,
                        listener: EventListenerType::Inline(listener.unwrap()),
                    });
                }
            }
        }
    }

    fn get_inline_event_listener(&self, ty: DOMString) -> Option<Rc<EventHandler>> {
        let mut handlers = self.handlers.borrow_mut();
        handlers.get_mut(&ty).and_then(|entry| entry.get_inline_listener(self, &ty))
    }

    pub fn set_event_handler_uncompiled(&self,
                                        url: Url,
                                        line: usize,
                                        ty: &str,
                                        source: DOMString) {
        self.set_inline_event_listener(ty.to_owned(),
                                       Some(InlineEventListener::Uncompiled(
                                           Some((source, url, line)))));
    }

    // https://html.spec.whatwg.org/multipage/#getting-the-current-value-of-the-event-handler
    #[allow(unsafe_code)]
    pub fn get_compiled_event_handler(&self,
                                      url: Url,
                                      lineno: usize,
                                      ty: &str,
                                      source: DOMString)
                                      -> Option<(Rc<EventHandler>)> {
        // Step 1.1
        let element = self.downcast::<Element>();
        let document = match element {
            Some(element) => document_from_node(element),
            None => self.downcast::<Window>().unwrap().Document(),
        };

        // TODO step 1.2 (browsing context/scripting enabled)

        // Step 1.3
        let body: Vec<u16> = source.utf16_units().collect();

        // TODO step 1.5 (form owner)

        // Step 1.6
        let window = document.window();
        let script_settings = window.environment_settings();

        let url_serialized = CString::new(url.serialize()).unwrap();
        let name = CString::new(ty).unwrap();

        let nargs = 1; //XXXjdm not true for onerror
        static mut ARG_NAMES: [*const c_char; 1] = [b"event\0" as *const u8 as *const c_char];

        let cx = window.get_cx();

        let options = CompileOptionsWrapper::new(cx, url_serialized.as_ptr(), lineno as u32);
        // TODO step 1.10.1-3 (document, form owner, element in scope chain)
        let scopechain = AutoObjectVectorWrapper::new(cx);

        let _ar = JSAutoRequest::new(cx);
        let _ac = JSAutoCompartment::new(cx, window.reflector().get_jsobject().get());
        let mut handler = RootedFunction::new(cx, ptr::null_mut());
        let rv = unsafe {
            CompileFunction(cx,
                            scopechain.ptr,
                            options.ptr,
                            name.as_ptr(),
                            nargs,
                            ARG_NAMES.as_mut_ptr(),
                            body.as_ptr(),
                            body.len() as size_t,
                            handler.handle_mut())
        };
        if !rv || handler.ptr.is_null() {
            // Step 1.8.2
            report_pending_exception(cx, self.reflector().get_jsobject().get());
            // Step 1.8.1 / 1.8.3
            return None;
        }

        // Step 1.11-13
        let script = Script {
            settings_object: script_settings,
            code_entry_point: Cow::Owned(source),
            source_url: url,
            line: lineno,
            errors: ErrorReporting::DontMuteErrors,
        };

        let funobj = unsafe { JS_GetFunctionObject(handler.ptr) };
        assert!(!funobj.is_null());
        // Step 1.14
        Some(EventHandlerNonNull::new(funobj, script.settings_object.clone()))
    }

    pub fn set_event_handler_common<T: CallbackContainer>(
        &self, ty: &str, listener: Option<Rc<T>>)
    {
        let event_listener = listener.map(|listener|
                                          InlineEventListener::Compiled(
                                              EventHandlerNonNull::new(listener.callback(),
                                                                       listener.script_settings())));
        self.set_inline_event_listener(ty.to_owned(), event_listener);
    }

    pub fn get_event_handler_common<T: CallbackContainer>(&self, ty: &str) -> Option<Rc<T>> {
        let listener = self.get_inline_event_listener(ty.to_owned());
        listener.map(|listener| CallbackContainer::new(listener.parent.callback(),
                                                       listener.script_settings()))
    }

    pub fn has_handlers(&self) -> bool {
        !self.handlers.borrow().is_empty()
    }
}

impl EventTargetMethods for EventTarget {
    // https://dom.spec.whatwg.org/#dom-eventtarget-addeventlistener
    fn AddEventListener(&self,
                        ty: DOMString,
                        listener: Option<Rc<EventListener>>,
                        capture: bool) {
        match listener {
            Some(listener) => {
                let mut handlers = self.handlers.borrow_mut();
                let entry = match handlers.entry(ty) {
                    Occupied(entry) => entry.into_mut(),
                    Vacant(entry) => entry.insert(Default::default()),
                };

                let phase = if capture { ListenerPhase::Capturing } else { ListenerPhase::Bubbling };
                let new_entry = EventListenerEntry {
                    phase: phase,
                    listener: EventListenerType::Additive(listener)
                };
                if !entry.listeners.contains(&new_entry) {
                    entry.listeners.push(new_entry);
                }
            },
            _ => (),
        }
    }

    // https://dom.spec.whatwg.org/#dom-eventtarget-removeeventlistener
    fn RemoveEventListener(&self,
                           ty: DOMString,
                           listener: Option<Rc<EventListener>>,
                           capture: bool) {
        match listener {
            Some(ref listener) => {
                let mut handlers = self.handlers.borrow_mut();
                let entry = handlers.get_mut(&ty);
                for entry in entry {
                    let phase = if capture { ListenerPhase::Capturing } else { ListenerPhase::Bubbling };
                    let old_entry = EventListenerEntry {
                        phase: phase,
                        listener: EventListenerType::Additive(listener.clone())
                    };
                    if let Some(position) = entry.listeners.iter().position(|e| *e == old_entry) {
                        entry.listeners.remove(position);
                    }
                }
            },
            _ => (),
        }
    }

    // https://dom.spec.whatwg.org/#dom-eventtarget-dispatchevent
    fn DispatchEvent(&self, event: &Event) -> Fallible<bool> {
        if event.dispatching() || !event.initialized() {
            return Err(Error::InvalidState);
        }
        event.set_trusted(false);
        Ok(self.dispatch_event(event))
    }
}

impl VirtualMethods for EventTarget {
    fn super_type(&self) -> Option<&VirtualMethods> {
        None
    }
}
