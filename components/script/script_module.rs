/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The script module mod contains common traits and structs
//! related to `type=module` for script thread or worker threads.

use crate::compartments::{enter_realm, AlreadyInCompartment, InCompartment};
use crate::document_loader::LoadType;
use crate::dom::bindings::cell::DomRefCell;
use crate::dom::bindings::conversions::jsstring_to_str;
use crate::dom::bindings::error::report_pending_exception;
use crate::dom::bindings::error::Error;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::{Trusted, TrustedPromise};
use crate::dom::bindings::reflector::DomObject;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::settings_stack::AutoIncumbentScript;
use crate::dom::bindings::str::DOMString;
use crate::dom::bindings::trace::RootedTraceableBox;
use crate::dom::document::Document;
use crate::dom::element::Element;
use crate::dom::globalscope::GlobalScope;
use crate::dom::htmlscriptelement::{HTMLScriptElement, ScriptId, ScriptOrigin, ScriptType};
use crate::dom::node::document_from_node;
use crate::dom::performanceresourcetiming::InitiatorType;
use crate::dom::promise::Promise;
use crate::dom::promisenativehandler::{Callback, PromiseNativeHandler};
use crate::dom::worker::TrustedWorkerAddress;
use crate::network_listener::{self, NetworkListener};
use crate::network_listener::{PreInvoke, ResourceTimingListener};
use crate::task::TaskBox;
use crate::task_source::TaskSourceName;
use encoding_rs::{Encoding, UTF_8};
use ipc_channel::ipc;
use ipc_channel::router::ROUTER;
use js::glue::{AppendToAutoObjectVector, CreateAutoObjectVector};
use js::jsapi::Handle as RawHandle;
use js::jsapi::HandleObject;
use js::jsapi::HandleValue as RawHandleValue;
use js::jsapi::{AutoObjectVector, JSAutoRealm, JSObject, JSString};
use js::jsapi::{GetModuleResolveHook, JSRuntime, SetModuleResolveHook};
use js::jsapi::{GetRequestedModules, SetModuleMetadataHook};
use js::jsapi::{GetWaitForAllPromise, ModuleEvaluate, ModuleInstantiate, SourceText};
use js::jsapi::{Heap, JSContext, JS_ClearPendingException};
use js::jsapi::{SetModuleDynamicImportHook, SetScriptPrivateReferenceHooks};
use js::jsval::{JSVal, UndefinedValue};
use js::rust::jsapi_wrapped::{CompileModule, JS_GetArrayLength, JS_GetElement};
use js::rust::jsapi_wrapped::{GetRequestedModuleSpecifier, JS_GetPendingException};
use js::rust::wrappers::JS_SetPendingException;
use js::rust::CompileOptionsWrapper;
use js::rust::IntoHandle;
use js::rust::{Handle, HandleValue};
use net_traits::request::{Destination, ParserMetadata, Referrer, RequestBuilder, RequestMode};
use net_traits::{FetchMetadata, Metadata};
use net_traits::{FetchResponseListener, NetworkError};
use net_traits::{ResourceFetchTiming, ResourceTimingType};
use servo_url::ServoUrl;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use url::ParseError as UrlParseError;

pub fn get_source_text(source: Vec<u16>) -> SourceText<u16> {
    SourceText {
        units_: source.as_ptr() as *const _,
        length_: source.len() as u32,
        ownsUnits_: false,
        _phantom_0: PhantomData,
    }
}

#[allow(unsafe_code)]
unsafe fn gen_type_error(global: &GlobalScope, string: String) -> ModuleError {
    rooted!(in(*global.get_cx()) let mut thrown = UndefinedValue());
    Error::Type(string).to_jsval(*global.get_cx(), &global, thrown.handle_mut());

    return ModuleError::RawException(RootedTraceableBox::from_box(Heap::boxed(thrown.get())));
}

#[derive(JSTraceable)]
pub struct ModuleObject(Box<Heap<*mut JSObject>>);

impl ModuleObject {
    #[allow(unsafe_code)]
    pub fn handle(&self) -> HandleObject {
        unsafe { self.0.handle() }
    }
}

#[derive(JSTraceable)]
pub enum ModuleError {
    Network(NetworkError),
    RawException(RootedTraceableBox<Heap<JSVal>>),
}

impl Eq for ModuleError {}

impl PartialEq for ModuleError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Network(_), Self::RawException(_)) |
            (Self::RawException(_), Self::Network(_)) => false,
            _ => true,
        }
    }
}

impl Ord for ModuleError {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Network(_), Self::RawException(_)) => Ordering::Greater,
            (Self::RawException(_), Self::Network(_)) => Ordering::Less,
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for ModuleError {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ModuleError {
    #[allow(unsafe_code)]
    pub fn handle(&self) -> Handle<JSVal> {
        match self {
            Self::Network(_) => unreachable!(),
            Self::RawException(exception) => exception.handle(),
        }
    }
}

impl Clone for ModuleError {
    fn clone(&self) -> Self {
        match self {
            Self::Network(network_error) => Self::Network(network_error.clone()),
            Self::RawException(exception) => Self::RawException(RootedTraceableBox::from_box(
                Heap::boxed(exception.get().clone()),
            )),
        }
    }
}

#[derive(JSTraceable)]
pub struct ModuleTree {
    url: ServoUrl,
    text: DomRefCell<DOMString>,
    record: DomRefCell<Option<ModuleObject>>,
    status: DomRefCell<ModuleStatus>,
    descendant_urls: DomRefCell<HashSet<ServoUrl>>,
    error: DomRefCell<Option<ModuleError>>,
    promise: DomRefCell<Option<Rc<Promise>>>,
}

impl ModuleTree {
    pub fn new(url: ServoUrl) -> Self {
        ModuleTree {
            url,
            text: DomRefCell::new(DOMString::new()),
            record: DomRefCell::new(None),
            status: DomRefCell::new(ModuleStatus::Initial),
            descendant_urls: DomRefCell::new(HashSet::new()),
            error: DomRefCell::new(None),
            promise: DomRefCell::new(None),
        }
    }

    pub fn get_promise(&self) -> &DomRefCell<Option<Rc<Promise>>> {
        &self.promise
    }

    pub fn set_promise(&self, promise: Rc<Promise>) {
        *self.promise.borrow_mut() = Some(promise);
    }

    pub fn get_status(&self) -> ModuleStatus {
        self.status.borrow().clone()
    }

    pub fn set_status(&self, status: ModuleStatus) {
        *self.status.borrow_mut() = status;
    }

    pub fn get_record(&self) -> &DomRefCell<Option<ModuleObject>> {
        &self.record
    }

    pub fn set_record(&self, record: ModuleObject) {
        *self.record.borrow_mut() = Some(record);
    }

    pub fn get_error(&self) -> &DomRefCell<Option<ModuleError>> {
        &self.error
    }

    pub fn set_error(&self, error: Option<ModuleError>) {
        *self.error.borrow_mut() = error;
    }

    pub fn get_text(&self) -> &DomRefCell<DOMString> {
        &self.text
    }

    pub fn set_text(&self, module_text: DOMString) {
        *self.text.borrow_mut() = module_text;
    }

    pub fn get_descendant_urls(&self) -> &DomRefCell<HashSet<ServoUrl>> {
        &self.descendant_urls
    }

    pub fn append_descendant_urls(&self, descendant_urls: HashSet<ServoUrl>) {
        self.descendant_urls.borrow_mut().extend(descendant_urls);
    }

    pub fn append_handler(&self, owner: ModuleOwner, module_url: ServoUrl) {
        let promise = self.promise.borrow();

        let resolve_this = owner.clone();
        let reject_this = owner.clone();

        let resolved_url = module_url.clone();
        let rejected_url = module_url.clone();

        let handler = PromiseNativeHandler::new(
            &owner.global(),
            Some(ModuleHandler::new(Box::new(
                task!(fetched_resolve: move || {
                    println!("fetched: {}", resolved_url.clone());
                    resolve_this.finish_module_load(Some(resolved_url));
                }),
            ))),
            Some(ModuleHandler::new(Box::new(
                task!(failure_reject: move || {
                    println!("failed");
                    reject_this.finish_module_load(Some(rejected_url));
                }),
            ))),
        );

        let _compartment = enter_realm(&*owner.global());
        AlreadyInCompartment::assert(&*owner.global());
        let _ais = AutoIncumbentScript::new(&*owner.global());

        let promise = promise.as_ref().unwrap();

        promise.append_native_handler(&handler);
    }
}

#[derive(Clone, Copy, Debug, JSTraceable, PartialEq, PartialOrd)]
pub enum ModuleStatus {
    Initial,
    Fetching,
    FetchFailed,
    FetchingDescendants,
    Finished,
}

impl ModuleTree {
    #[allow(unsafe_code)]
    /// https://html.spec.whatwg.org/multipage/#creating-a-module-script
    /// Step 7-11.
    fn compile_module_script(
        &self,
        global: &GlobalScope,
        module_script_text: DOMString,
        url: ServoUrl,
    ) -> Result<ModuleObject, ModuleError> {
        let module: Vec<u16> = module_script_text.encode_utf16().collect();

        let url_cstr = ffi::CString::new(url.as_str().as_bytes()).unwrap();

        let _ac = JSAutoRealm::new(*global.get_cx(), *global.reflector().get_jsobject());

        let compile_options = CompileOptionsWrapper::new(*global.get_cx(), url_cstr.as_ptr(), 1);

        rooted!(in(*global.get_cx()) let mut module_script = ptr::null_mut::<JSObject>());

        let mut source = get_source_text(module);

        unsafe {
            if !CompileModule(
                *global.get_cx(),
                compile_options.ptr,
                &mut source,
                &mut module_script.handle_mut(),
            ) {
                println!("fail to compile module script of {}", url);

                rooted!(in(*global.get_cx()) let mut exception = UndefinedValue());
                assert!(JS_GetPendingException(
                    *global.get_cx(),
                    &mut exception.handle_mut()
                ));
                JS_ClearPendingException(*global.get_cx());

                return Err(ModuleError::RawException(RootedTraceableBox::from_box(
                    Heap::boxed(exception.get()),
                )));
            }
        }

        println!("module script of {} compile done", url);

        self.resolve_requested_module_specifiers(
            &global,
            module_script.handle().into_handle(),
            global.api_base_url().clone(),
        )
        .map(|_| ModuleObject(Heap::boxed(*module_script)))
    }

    #[allow(unsafe_code)]
    pub fn instantiate_module_tree(
        &self,
        global: &GlobalScope,
        module_record: HandleObject,
    ) -> Result<(), ModuleError> {
        let _ac = JSAutoRealm::new(*global.get_cx(), *global.reflector().get_jsobject());

        unsafe {
            if !ModuleInstantiate(*global.get_cx(), module_record) {
                println!("fail to instantiate module");

                rooted!(in(*global.get_cx()) let mut exception = UndefinedValue());
                assert!(JS_GetPendingException(
                    *global.get_cx(),
                    &mut exception.handle_mut()
                ));
                JS_ClearPendingException(*global.get_cx());

                Err(ModuleError::RawException(RootedTraceableBox::from_box(
                    Heap::boxed(exception.get()),
                )))
            } else {
                println!("module instantiated successfully");

                Ok(())
            }
        }
    }

    #[allow(unsafe_code)]
    pub fn execute_module(
        &self,
        global: &GlobalScope,
        module_record: HandleObject,
    ) -> Result<(), ModuleError> {
        let _ac = JSAutoRealm::new(*global.get_cx(), *global.reflector().get_jsobject());

        unsafe {
            if !ModuleEvaluate(*global.get_cx(), module_record) {
                println!("fail to evaluate module");

                rooted!(in(*global.get_cx()) let mut exception = UndefinedValue());
                assert!(JS_GetPendingException(
                    *global.get_cx(),
                    &mut exception.handle_mut()
                ));
                JS_ClearPendingException(*global.get_cx());

                Err(ModuleError::RawException(RootedTraceableBox::from_box(
                    Heap::boxed(exception.get()),
                )))
            } else {
                println!("module evaluated successfully");
                Ok(())
            }
        }
    }

    #[allow(unsafe_code)]
    pub fn report_error(&self, global: &GlobalScope) {
        let module_error = self.error.borrow();

        if let Some(exception) = &*module_error {
            unsafe {
                JS_SetPendingException(*global.get_cx(), exception.handle());
                report_pending_exception(*global.get_cx(), true);
            }
        }
    }

    /// https://html.spec.whatwg.org/multipage/#fetch-the-descendants-of-a-module-script
    /// Step 5.
    pub fn resolve_requested_modules(
        &self,
        global: &GlobalScope,
        visited: &mut HashSet<ServoUrl>,
    ) -> Result<HashSet<ServoUrl>, ModuleError> {
        let status = self.get_status();

        assert_ne!(status, ModuleStatus::Initial);
        assert_ne!(status, ModuleStatus::Fetching);

        let record = self.record.borrow();

        if let Some(raw_record) = &*record {
            let valid_specifier_urls = self.resolve_requested_module_specifiers(
                &global,
                raw_record.handle(),
                global.api_base_url().clone(),
            );

            return valid_specifier_urls.map(|parsed_urls| {
                parsed_urls
                    .iter()
                    .filter_map(|parsed_url| {
                        if !visited.contains(&parsed_url) {
                            visited.insert(parsed_url.clone());

                            Some(parsed_url.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<HashSet<ServoUrl>>()
            });
        }

        unreachable!("Didn't have record while resolving its requested module")
    }

    #[allow(unsafe_code)]
    fn resolve_requested_module_specifiers(
        &self,
        global: &GlobalScope,
        module_object: HandleObject,
        base_url: ServoUrl,
    ) -> Result<HashSet<ServoUrl>, ModuleError> {
        let _ac = JSAutoRealm::new(*global.get_cx(), *global.reflector().get_jsobject());

        let mut specifier_urls = HashSet::new();

        unsafe {
            rooted!(in(*global.get_cx()) let requested_modules = GetRequestedModules(*global.get_cx(), module_object));

            let mut length = 0;

            if !JS_GetArrayLength(*global.get_cx(), requested_modules.handle(), &mut length) {
                let module_length_error =
                    gen_type_error(&global, "Wrong length of requested modules".to_owned());

                return Err(module_length_error);
            }

            for index in 0..length {
                rooted!(in(*global.get_cx()) let mut element = UndefinedValue());

                if !JS_GetElement(
                    *global.get_cx(),
                    requested_modules.handle(),
                    index,
                    &mut element.handle_mut(),
                ) {
                    let get_element_error =
                        gen_type_error(&global, "Failed to get requested module".to_owned());

                    return Err(get_element_error);
                }

                rooted!(in(*global.get_cx()) let specifier = GetRequestedModuleSpecifier(
                    *global.get_cx(), element.handle()
                ));

                let url = ModuleTree::resolve_module_specifier(
                    *global.get_cx(),
                    &base_url,
                    specifier.handle().into_handle(),
                );

                if url.is_err() {
                    let specifier_error =
                        gen_type_error(&global, "Wrong module specifier".to_owned());

                    return Err(specifier_error);
                }

                specifier_urls.insert(url.unwrap());
            }
        }

        Ok(specifier_urls)
    }

    /// The following module specifiers are allowed by the spec:
    ///  - a valid absolute URL
    ///  - a valid relative URL that starts with "/", "./" or "../"
    ///
    /// Bareword module specifiers are currently disallowed as these may be given
    /// special meanings in the future.
    /// https://html.spec.whatwg.org/multipage/#resolve-a-module-specifier
    #[allow(unsafe_code)]
    fn resolve_module_specifier(
        cx: *mut JSContext,
        url: &ServoUrl,
        specifier: RawHandle<*mut JSString>,
    ) -> Result<ServoUrl, UrlParseError> {
        let specifier_str = unsafe { jsstring_to_str(cx, *specifier) };

        // Step 1.
        if let Ok(specifier_url) = ServoUrl::parse(&specifier_str) {
            return Ok(specifier_url);
        }

        // Step 2.
        if !specifier_str.starts_with("/") &&
            !specifier_str.starts_with("./") &&
            !specifier_str.starts_with("../")
        {
            return Err(UrlParseError::InvalidDomainCharacter);
        }

        // Step 3.
        return ServoUrl::parse_with_base(Some(url), &specifier_str.clone());
    }

    /// https://html.spec.whatwg.org/multipage/#finding-the-first-parse-error
    fn find_first_parse_error(
        module_map: &HashMap<ServoUrl, Rc<ModuleTree>>,
        module_tree: &ModuleTree,
    ) -> Option<ModuleError> {
        // 4.
        let record = module_tree.get_record().borrow();

        if record.is_none() {
            let module_error = module_tree.get_error().borrow();

            return module_error.clone();
        }

        let descendant_urls = module_tree.get_descendant_urls().borrow();

        descendant_urls
            .iter()
            .filter_map(|url| {
                module_map.get(&url.clone()).and_then(|module| {
                    let module_error = module.get_error().borrow();

                    module_error.clone()
                })
            })
            .max()
    }
}

#[derive(JSTraceable, MallocSizeOf)]
struct ModuleHandler {
    #[ignore_malloc_size_of = "Measuring trait objects is hard"]
    task: DomRefCell<Option<Box<dyn TaskBox>>>,
}

impl ModuleHandler {
    pub fn new(task: Box<dyn TaskBox>) -> Box<dyn Callback> {
        Box::new(Self {
            task: DomRefCell::new(Some(task)),
        })
    }
}

impl Callback for ModuleHandler {
    fn callback(&self, _cx: *mut JSContext, _v: HandleValue) {
        let task = self.task.borrow_mut().take().unwrap();
        task.run_box();
    }
}

/// The owner of the module
/// It can be `worker` or `script` element
#[derive(Clone)]
pub enum ModuleOwner {
    #[allow(dead_code)]
    Worker(TrustedWorkerAddress),
    Window(Trusted<HTMLScriptElement>),
}

impl ModuleOwner {
    pub fn global(&self) -> DomRoot<GlobalScope> {
        match &self {
            ModuleOwner::Worker(worker) => (*worker.root().clone()).global(),
            ModuleOwner::Window(script) => (*script.root()).global(),
        }
    }

    fn gen_promise_with_final_handler(&self, module_url: Option<ServoUrl>) -> Rc<Promise> {
        let resolve_this = self.clone();
        let reject_this = self.clone();

        let resolved_url = module_url.clone();
        let rejected_url = module_url.clone();

        let handler = PromiseNativeHandler::new(
            &self.global(),
            Some(ModuleHandler::new(Box::new(
                task!(fetched_resolve: move || {
                    println!("fetched");
                    resolve_this.finish_module_load(resolved_url);
                }),
            ))),
            Some(ModuleHandler::new(Box::new(
                task!(failure_reject: move || {
                    println!("failed");
                    reject_this.finish_module_load(rejected_url);
                }),
            ))),
        );

        let compartment = enter_realm(&*self.global());
        let comp = InCompartment::Entered(&compartment);
        let _ais = AutoIncumbentScript::new(&*self.global());

        let promise = Promise::new_in_current_compartment(&self.global(), comp);

        promise.append_native_handler(&handler);

        promise
    }

    /// https://html.spec.whatwg.org/multipage/#fetch-the-descendants-of-and-link-a-module-script
    /// step 4-7.
    pub fn finish_module_load(&self, module_url: Option<ServoUrl>) {
        match &self {
            ModuleOwner::Worker(_) => unimplemented!(),
            ModuleOwner::Window(script) => {
                let global = self.global();

                let document = document_from_node(&*script.root());

                let base_url = document.base_url();

                let module_map = global.get_module_map().borrow();

                let script_src = script
                    .root()
                    .upcast::<Element>()
                    .get_attribute(&ns!(), &local_name!("src"))
                    .map(|attr| base_url.join(&attr.value()).ok())
                    .unwrap_or(None);

                let (module_tree, mut load, load_type) =
                    if let Some(script_src) = script_src.clone() {
                        let module_tree = module_map.get(&script_src.clone()).unwrap().clone();

                        let load = Ok(ScriptOrigin::external(
                            module_tree.get_text().borrow().clone(),
                            script_src.clone(),
                            ScriptType::Module,
                        ));

                        println!(
                            "Going to finish external script from {}",
                            script_src.clone()
                        );

                        (module_tree, load, LoadType::Script(script_src.clone()))
                    } else {
                        let module_tree = {
                            let inline_module_map = global.get_inline_module_map().borrow();
                            inline_module_map
                                .get(&script.root().get_script_id())
                                .unwrap()
                                .clone()
                        };

                        let load = Ok(ScriptOrigin::internal(
                            module_tree.get_text().borrow().clone(),
                            base_url.clone(),
                            ScriptType::Module,
                        ));

                        println!("Going to finish internal script from {}", base_url.clone());

                        (module_tree, load, LoadType::Script(base_url.clone()))
                    };

                {
                    // FIXME: while having a `failed` descendant, the Promise all will
                    //        be rejected directly without waiting fetching descendants.
                    let descendant_urls = module_tree.get_descendant_urls().borrow();
                    let is_any_fetching = descendant_urls.iter().any(|descendant_url| {
                        let descendant_tree = module_map.get(&descendant_url.clone());
                        let descendant_status = descendant_tree.unwrap().get_status();
                        descendant_status < ModuleStatus::FetchFailed
                    });
                    if is_any_fetching {
                        return;
                    }
                }

                module_tree.set_status(ModuleStatus::Finished);

                if script_src == module_url {
                    let module_error =
                        ModuleTree::find_first_parse_error(&*module_map, &module_tree);

                    match module_error {
                        None => {
                            let module_record = module_tree.get_record().borrow();
                            if let Some(record) = &*module_record {
                                let instantiated =
                                    module_tree.instantiate_module_tree(&global, record.handle());

                                if let Err(exception) = instantiated {
                                    module_tree.set_error(Some(exception.clone()));
                                }
                            }
                        },
                        Some(ModuleError::RawException(exception)) => {
                            module_tree.set_error(Some(ModuleError::RawException(exception)));
                        },
                        Some(ModuleError::Network(network_error)) => {
                            module_tree
                                .set_error(Some(ModuleError::Network(network_error.clone())));

                            // Change the `result` load of the script into `network` error
                            load = Err(network_error);
                        },
                    };

                    let r#async = script
                        .root()
                        .upcast::<Element>()
                        .has_attribute(&local_name!("async"));

                    if !r#async && (&*script.root()).get_parser_inserted() {
                        document.deferred_script_loaded(&*script.root(), load);
                    } else if !r#async && !(&*script.root()).get_non_blocking() {
                        document.asap_in_order_script_loaded(&*script.root(), load);
                    } else {
                        document.asap_script_loaded(&*script.root(), load);
                    };

                    document.finish_load(load_type);
                }
            },
        }
    }
}

/// The context required for asynchronously loading an external module script source.
struct ModuleContext {
    /// The owner of the module that initiated the request.
    owner: ModuleOwner,
    /// The response body received to date.
    data: Vec<u8>,
    /// The response metadata received to date.
    metadata: Option<Metadata>,
    /// The initial URL requested.
    url: ServoUrl,
    /// Destination of current module context
    destination: Destination,
    /// Indicates whether the request failed, and why
    status: Result<(), NetworkError>,
    /// Timing object for this resource
    resource_timing: ResourceFetchTiming,
}

impl FetchResponseListener for ModuleContext {
    fn process_request_body(&mut self) {} // TODO(cybai): Perhaps add custom steps to perform fetch here?

    fn process_request_eof(&mut self) {} // TODO(cybai): Perhaps add custom steps to perform fetch here?

    fn process_response(&mut self, metadata: Result<FetchMetadata, NetworkError>) {
        self.metadata = metadata.ok().map(|meta| match meta {
            FetchMetadata::Unfiltered(m) => m,
            FetchMetadata::Filtered { unsafe_, .. } => unsafe_,
        });

        let status_code = self
            .metadata
            .as_ref()
            .and_then(|m| match m.status {
                Some((c, _)) => Some(c),
                _ => None,
            })
            .unwrap_or(0);

        self.status = match status_code {
            0 => Err(NetworkError::Internal(
                "No http status code received".to_owned(),
            )),
            200..=299 => Ok(()), // HTTP ok status codes
            _ => Err(NetworkError::Internal(format!(
                "HTTP error code {}",
                status_code
            ))),
        };
    }

    fn process_response_chunk(&mut self, mut chunk: Vec<u8>) {
        if self.status.is_ok() {
            self.data.append(&mut chunk);
        }
    }

    /// <https://html.spec.whatwg.org/multipage/#fetch-a-single-module-script>
    /// Step 8-14
    #[allow(unsafe_code)]
    fn process_response_eof(&mut self, response: Result<ResourceFetchTiming, NetworkError>) {
        let global = self.owner.global();

        let load = response.and(self.status.clone()).map(|_| {
            let meta = self.metadata.take().unwrap();

            let encoding = meta
                .charset
                .and_then(|encoding| Encoding::for_label(encoding.as_bytes()))
                .unwrap_or(UTF_8);

            // Step 12-1 & 13-1.
            let (source_text, _, _) = encoding.decode(&self.data);
            ScriptOrigin::external(
                DOMString::from(source_text),
                meta.final_url,
                ScriptType::Module,
            )
        });

        if let Err(err) = load {
            // Step 9.
            println!("Failed to fetch {}", self.url.clone());
            let module_tree = {
                let module_map = global.get_module_map().borrow();
                module_map.get(&self.url.clone()).unwrap().clone()
            };

            module_tree.set_status(ModuleStatus::FetchFailed);

            module_tree.set_error(Some(ModuleError::Network(err)));

            let promise = module_tree.get_promise().borrow();
            promise.as_ref().unwrap().resolve_native(&());

            return;
        }

        // TODO: Step 12 & 13. HANDLE MIME TYPE CHECKING

        // Step 14.
        if let Ok(ref resp_mod_script) = load {
            let module_tree = {
                let module_map = global.get_module_map().borrow();
                module_map.get(&self.url.clone()).unwrap().clone()
            };

            module_tree.set_text(resp_mod_script.text());

            let compiled_module = module_tree.compile_module_script(
                &global,
                resp_mod_script.text(),
                self.url.clone(),
            );

            match compiled_module {
                Err(exception) => {
                    module_tree.set_error(Some(exception));

                    let promise = module_tree.get_promise().borrow();
                    promise.as_ref().unwrap().resolve_native(&());

                    return;
                },
                Ok(record) => {
                    module_tree.set_record(record);

                    let mut visited = HashSet::new();
                    visited.insert(self.url.clone());

                    let descendant_results = fetch_module_descendants_and_link(
                        &self.owner,
                        &module_tree,
                        self.destination.clone(),
                        visited,
                    );

                    // Resolve the request of this module tree promise directly
                    // when there's no descendant
                    if descendant_results.is_none() {
                        let promise = module_tree.get_promise().borrow();
                        promise.as_ref().unwrap().resolve_native(&());
                    }
                },
            }
        }
    }

    fn resource_timing_mut(&mut self) -> &mut ResourceFetchTiming {
        &mut self.resource_timing
    }

    fn resource_timing(&self) -> &ResourceFetchTiming {
        &self.resource_timing
    }

    fn submit_resource_timing(&mut self) {
        network_listener::submit_timing(self)
    }
}

impl ResourceTimingListener for ModuleContext {
    fn resource_timing_information(&self) -> (InitiatorType, ServoUrl) {
        let initiator_type = InitiatorType::LocalName("module".to_string());
        (initiator_type, self.url.clone())
    }

    fn resource_timing_global(&self) -> DomRoot<GlobalScope> {
        self.owner.global()
    }
}

impl PreInvoke for ModuleContext {}

#[allow(unsafe_code)]
/// A function to register module hooks (e.g. listening on resolving modules,
/// getting module metadata, getting script private reference and resolving dynamic import)
pub unsafe fn EnsureModuleHooksInitialized(rt: *mut JSRuntime) {
    if GetModuleResolveHook(rt).is_some() {
        return;
    }

    SetModuleResolveHook(rt, Some(HostResolveImportedModule));
    SetModuleMetadataHook(rt, None);
    SetScriptPrivateReferenceHooks(rt, None, None);

    SetModuleDynamicImportHook(rt, None);
}

#[allow(unsafe_code)]
/// https://tc39.github.io/ecma262/#sec-hostresolveimportedmodule
/// https://html.spec.whatwg.org/multipage/#hostresolveimportedmodule(referencingscriptormodule%2C-specifier)
unsafe extern "C" fn HostResolveImportedModule(
    cx: *mut JSContext,
    _reference_private: RawHandleValue,
    specifier: RawHandle<*mut JSString>,
) -> *mut JSObject {
    let global_scope = GlobalScope::from_context(cx);

    // Step 2.
    let base_url = global_scope.api_base_url();

    // TODO: Step 3.

    // Step 5.
    let url = ModuleTree::resolve_module_specifier(*global_scope.get_cx(), &base_url, specifier);

    // Step 6.
    assert!(url.is_ok());

    let parsed_url = url.unwrap();

    // Step 4 & 7.
    let module_map = global_scope.get_module_map().borrow();

    let module_tree = module_map.get(&parsed_url);

    // Step 9.
    assert!(module_tree.is_some());

    let fetched_module_object = module_tree.unwrap().get_record().borrow();

    // Step 8.
    assert!(fetched_module_object.is_some());

    // Step 10.
    if let Some(record) = &*fetched_module_object {
        return record.handle().get();
    }

    unreachable!()
}

/// https://html.spec.whatwg.org/multipage/#fetch-a-module-script-tree
pub fn fetch_external_module_script(
    owner: ModuleOwner,
    url: ServoUrl,
    destination: Destination,
) -> Rc<Promise> {
    // Step 1.
    fetch_single_module_script(
        owner,
        url,
        destination,
        Referrer::Client,
        ParserMetadata::NotParserInserted,
        true,
    )
}

/// https://html.spec.whatwg.org/multipage/#internal-module-script-graph-fetching-procedure
pub fn perform_internal_module_script_fetch(
    owner: ModuleOwner,
    url: ServoUrl,
    destination: Destination,
    visited: HashSet<ServoUrl>,
    referrer: Referrer,
    parser_metadata: ParserMetadata,
    top_level_module_fetch: bool,
) -> Rc<Promise> {
    // Step 1.
    assert!(visited.get(&url).is_some());

    // Step 2.
    fetch_single_module_script(
        owner,
        url,
        destination,
        referrer,
        parser_metadata,
        top_level_module_fetch,
    )
}

/// https://html.spec.whatwg.org/multipage/#fetch-a-single-module-script
pub fn fetch_single_module_script(
    owner: ModuleOwner,
    url: ServoUrl,
    destination: Destination,
    referrer: Referrer,
    parser_metadata: ParserMetadata,
    top_level_module_fetch: bool,
) -> Rc<Promise> {
    {
        // Step 1.
        let global = owner.global();
        let module_map = global.get_module_map().borrow();

        println!("Start to fetch {}", url.clone());

        if let Some(module_tree) = module_map.get(&url.clone()) {
            let status = module_tree.get_status();

            let promise = module_tree.get_promise().borrow();

            println!(
                "Meet a fetched url: {:?}, {}, {:?}",
                status,
                url.clone(),
                promise.is_none()
            );

            assert!(promise.is_some());

            module_tree.append_handler(owner.clone(), url.clone());

            // Step 2.
            if status == ModuleStatus::Fetching || status == ModuleStatus::FetchingDescendants {
                // TODO: queue a network task ?
                return promise.as_ref().unwrap().clone();
            }

            // Step 3.
            if status == ModuleStatus::Finished {
                //  asynchronously complete this algorithm with moduleMap[url], and abort these steps
                let promise = promise.as_ref().unwrap();
                return promise.clone();
            }
        }
    }

    let global = owner.global();

    let module_tree = ModuleTree::new(url.clone());
    module_tree.set_status(ModuleStatus::Fetching);

    let promise = owner.gen_promise_with_final_handler(Some(url.clone()));

    module_tree.set_promise(promise.clone());

    // Step 4.
    global.set_module_map(url.clone(), module_tree);

    // Step 5-6.
    let mode = match destination.clone() {
        Destination::Worker | Destination::SharedWorker if top_level_module_fetch => {
            RequestMode::SameOrigin
        },
        _ => RequestMode::NoCors,
    };

    let document: Option<DomRoot<Document>> = match &owner {
        ModuleOwner::Worker(_) => None,
        ModuleOwner::Window(script) => Some(document_from_node(&*script.root())),
    };

    // Step 7-8.
    let request = RequestBuilder::new(url.clone())
        .destination(destination.clone())
        .referrer(Some(referrer))
        .parser_metadata(parser_metadata)
        .mode(mode);

    let context = Arc::new(Mutex::new(ModuleContext {
        owner,
        data: vec![],
        metadata: None,
        url: url.clone(),
        destination: destination.clone(),
        status: Ok(()),
        resource_timing: ResourceFetchTiming::new(ResourceTimingType::Resource),
    }));

    let (action_sender, action_receiver) = ipc::channel().unwrap();

    let listener = NetworkListener {
        context,
        task_source: global.networking_task_source(),
        canceller: Some(global.task_canceller(TaskSourceName::Networking)),
    };

    ROUTER.add_route(
        action_receiver.to_opaque(),
        Box::new(move |message| {
            listener.notify_fetch(message.to().unwrap());
        }),
    );

    if let Some(doc) = document {
        doc.fetch_async(LoadType::Script(url), request, action_sender);
    }

    promise
}

#[allow(unsafe_code)]
/// https://html.spec.whatwg.org/multipage/#fetch-an-inline-module-script-graph
pub fn fetch_inline_module_script(
    owner: ModuleOwner,
    module_script_text: DOMString,
    url: ServoUrl,
    script_id: ScriptId,
) {
    let global = owner.global();

    let module_tree = ModuleTree::new(url.clone());

    let promise = owner.gen_promise_with_final_handler(None);

    module_tree.set_promise(promise.clone());

    let compiled_module =
        module_tree.compile_module_script(&global, module_script_text, url.clone());

    match compiled_module {
        Ok(record) => {
            module_tree.set_record(record);

            let descendant_results = fetch_module_descendants_and_link(
                &owner,
                &module_tree,
                Destination::Script,
                HashSet::new(),
            );

            global.set_inline_module_map(script_id, module_tree);

            if descendant_results.is_none() {
                promise.resolve_native(&());
            }
        },
        Err(exception) => {
            module_tree.set_error(Some(exception));
            global.set_inline_module_map(script_id, module_tree);
            promise.resolve_native(&());
        },
    }
}

/// https://html.spec.whatwg.org/multipage/#fetch-the-descendants-of-and-link-a-module-script
/// Step 1-3.
#[allow(unsafe_code)]
fn fetch_module_descendants_and_link(
    owner: &ModuleOwner,
    module_tree: &ModuleTree,
    destination: Destination,
    visited: HashSet<ServoUrl>,
) -> Option<Rc<Promise>> {
    let descendant_results = fetch_module_descendants(owner, module_tree, destination, visited);

    match descendant_results {
        Ok(descendants) => {
            if descendants.len() > 0 {
                unsafe {
                    let global = owner.global();

                    let _compartment = enter_realm(&*global);
                    AlreadyInCompartment::assert(&*global);
                    let _ais = AutoIncumbentScript::new(&*global);

                    let abv = CreateAutoObjectVector(*global.get_cx());

                    for descendant in descendants {
                        assert!(AppendToAutoObjectVector(
                            abv as *mut AutoObjectVector,
                            descendant.promise_obj().get()
                        ));
                    }

                    rooted!(in(*global.get_cx()) let raw_promise_all = GetWaitForAllPromise(*global.get_cx(), abv));

                    let promise_all =
                        Promise::new_with_js_promise(raw_promise_all.handle(), global.get_cx());

                    let promise = module_tree.get_promise().borrow();
                    let promise = promise.as_ref().unwrap().clone();

                    let resolve_promise = TrustedPromise::new(promise.clone());
                    let reject_promise = TrustedPromise::new(promise.clone());

                    let handler = PromiseNativeHandler::new(
                        &global,
                        Some(ModuleHandler::new(Box::new(
                            task!(all_fetched_resolve: move || {
                                println!("promise all fetched");
                                let promise = resolve_promise.root();
                                promise.resolve_native(&());
                            }),
                        ))),
                        Some(ModuleHandler::new(Box::new(
                            task!(all_failure_reject: move || {
                                println!("promise all failed");
                                let promise = reject_promise.root();
                                promise.reject_native(&());
                            }),
                        ))),
                    );

                    promise_all.append_native_handler(&handler);

                    return Some(promise_all);
                }
            }
        },
        Err(err) => {
            module_tree.set_error(Some(err));
        },
    }

    None
}

#[allow(unsafe_code)]
/// https://html.spec.whatwg.org/multipage/#fetch-the-descendants-of-a-module-script
fn fetch_module_descendants(
    owner: &ModuleOwner,
    module_tree: &ModuleTree,
    destination: Destination,
    mut visited: HashSet<ServoUrl>,
) -> Result<Vec<Rc<Promise>>, ModuleError> {
    println!("Start to load dependencies of {}", module_tree.url.clone());

    let global = owner.global();

    module_tree.set_status(ModuleStatus::FetchingDescendants);

    module_tree
        .resolve_requested_modules(&global, &mut visited)
        .map(|requested_urls| {
            module_tree.append_descendant_urls(requested_urls.clone());

            requested_urls
                .iter()
                .map(|requested_url| {
                    println!("Requested url: {}", requested_url);

                    {
                        let module_map = global.get_module_map().borrow();
                        let descendant_tree = module_map.get(&requested_url.clone());
                        if let Some(module_tree) = descendant_tree {
                            let status = module_tree.get_status();
                            let promise = module_tree.get_promise().borrow();

                            println!("See existing descendant: {}", requested_url.clone());

                            return match promise.as_ref() {
                                Some(p) => {
                                    println!(
                                        "Visited array of requested url {}: it's visited {:?} and status is {:?}",
                                        requested_url.clone(),
                                        visited.contains(&requested_url.clone()),
                                        status.clone()
                                    );

                                    if visited.contains(&requested_url.clone()) {
                                        match status {
                                            ModuleStatus::Initial | ModuleStatus::Fetching => {},
                                            ModuleStatus::FetchingDescendants => {
                                                let descendant_urls = module_tree.get_descendant_urls().borrow();
                                                let all_gt_fetching_descendants =
                                                    descendant_urls.iter().all(|descendant_url| {
                                                        let descendant_tree = module_map.get(&descendant_url.clone());
                                                        let descendant_status = descendant_tree.unwrap().get_status();
                                                        descendant_status >= ModuleStatus::FetchingDescendants
                                                    });

                                                if all_gt_fetching_descendants {
                                                    let module_error = module_tree.get_error().borrow();

                                                    if module_error.is_some() {
                                                        p.resolve_native(&());
                                                    } else {
                                                        p.resolve_native(&());
                                                    }
                                                }
                                            },
                                            ModuleStatus::FetchFailed | ModuleStatus::Finished => {
                                                let module_error = module_tree.get_error().borrow();

                                                if module_error.is_some() {
                                                    p.resolve_native(&());
                                                } else {
                                                    p.resolve_native(&());
                                                }
                                            },
                                        }
                                    }

                                    p.clone()
                                },
                                None => {
                                    debug!("Got a None promise but it exists in module map");

                                    rooted!(in(*global.get_cx()) let mut undefined_result = UndefinedValue());

                                    let module_error = module_tree.get_error().borrow();

                                    let p = if module_error.is_some() {
                                        Promise::new_rejected(
                                            &global,
                                            global.get_cx(),
                                            undefined_result.handle(),
                                        )
                                    } else {
                                        Promise::new_resolved(
                                            &global,
                                            global.get_cx(),
                                            undefined_result.handle(),
                                        )
                                    };

                                    p.unwrap().clone()
                                },
                            };
                        }
                    }

                    perform_internal_module_script_fetch(
                        owner.clone(),
                        requested_url.clone(),
                        destination.clone(),
                        visited.clone(),
                        Referrer::Client,
                        ParserMetadata::NotParserInserted,
                        true,
                    )
                })
                .collect()
        })
}