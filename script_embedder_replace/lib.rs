/*#![cfg_attr(crown, feature(register_tool))]
#![cfg_attr(crown, register_tool(crown))]*/

#[macro_use]
extern crate servo_jstraceable_derive;
#[macro_use]
extern crate malloc_size_of_derive;

use serde::{Deserialize, Serialize};
use servo_base::generic_channel::{GenericCallback, GenericSender};
use servo_malloc_size_of as malloc_size_of;

//use servo_dom_struct::dom_struct;
pub(crate) use js::gc::Traceable as JSTraceable;
pub(crate) use script_bindings::inheritance::HasParent;
pub(crate) use script_bindings::reflector::{DomObject, MutDomObject, Reflector};

//pub(crate) use script_bindings::trace::JSTracable;

#[derive(Serialize, Deserialize)]
pub enum Message {
    Foopy(GenericSender<String>),
    Ping(GenericCallback<()>),
}

#[derive(Serialize, Deserialize)]
pub enum EmbedderMessage {
    BootstrapChannel(GenericSender<ipc_channel::ipc::IpcSender<Message>>),
}

#[expect(non_snake_case)]
pub(crate) mod codegen {
    #[expect(unused)]
    pub(crate) mod Bindings {
        use std::ptr;

        use script_bindings::DomTypes;
        use script_bindings::codegen::PrototypeList;
        use script_bindings::conversions::IDLInterface;
        use script_bindings::reflector::Reflector;
        use script_bindings::utils::DOMClass;
        use crate::dom::foopy::Foopy;
        use crate::traits::Equivalence;

        include!(concat!(
            env!("OUT_DIR"),
            "/ConcreteBindings/FoopyBinding.rs"
        ));
        include!(concat!(env!("OUT_DIR"), "/ConcreteInheritTypes.rs"));
    }
}

pub mod traits {
    use script_bindings::DomTypes;
    use script_bindings::interfaces::PromiseCallbackTrait;
    use script_bindings::reflector::DomGlobalGeneric;
    use crate::dom::foopy::Foopy;

    trait_set::trait_set! {
        pub trait Equivalence = DomTypes<
            Foopy = Foopy<Self>,
            Foopy: DomGlobalGeneric<Self>,
        >;

        pub trait ScriptPromise<D: DomTypes> =
            PromiseCallbackTrait<D, Foopy<D>, ()>
    }
}

pub mod foopy {
    use std::marker::PhantomData;

    use js::context::JSContext;
    use js::rust::HandleObject;
    use servo_base::generic_channel;
    use servo_deny_public_fields as deny_public_fields;
    //use script_bindings::inheritance::HasParent;
    //use servo_malloc_size_of::MallocSizeOf;
    use script_bindings::reflector::{Reflector, reflect_dom_object_with_wrap};
    use script_bindings::codegen::GenericBindings::FoopyBinding::Foopy_Binding::Wrap as FoopyWrap;
    use script_bindings::codegen::GenericBindings::FoopyBinding::Foopy_Binding::FoopyMethods;
    use script_bindings::inheritance::Castable;
    use script_bindings::interfaces::{GlobalScopeHelpers, PromiseHelpers, PromiseCallbackTrait};
    use script_bindings::reflector::DomGlobalGeneric;
    use script_bindings::root::DomRoot;
    use script_bindings::routed_promise::RoutedPromiseListener;
    use script_bindings::str::USVString;
    use script_bindings::DomTypes;
    use servo_constellation_traits::ScriptToConstellationMessage;

    use crate::traits::{Equivalence, ScriptPromise};
    use crate::{EmbedderMessage, Message};

    #[servo_dom_struct::dom_struct(no_crown)]
    pub struct Foopy<D: DomTypes> {
        reflector_: Reflector,
        #[no_trace = "PhantomData does not exist"]
        marker: PhantomData<D>,
        #[no_trace = ""]
        channel: ipc_channel::ipc::IpcSender<Message>,
    }

    impl<D> FoopyMethods<D> for Foopy<D>
    where
        D: Equivalence,
        <D::Promise as PromiseHelpers<D>>::StackRoot: ScriptPromise<D>,
    {
        fn Foopy(&self) -> USVString {
            let (sender, receiver) = generic_channel::channel().unwrap();
            let _ = self.channel.send(Message::Foopy(sender));
            receiver.recv().unwrap().into()
        }

        fn Roundtrip(&self, cx: &mut JSContext) -> <<D as script_bindings::DomTypes>::Promise as script_bindings::interfaces::PromiseHelpers<D>>::StackRoot {
            let global = self.global_from_reflector();
            let promise = D::Promise::new(cx, &global);
            let callback = promise.callback_promise_dom_manipulation_task_source(self);

            let _ = self.channel.send(Message::Ping(callback));
            promise
        }

        fn Constructor(
            cx: &mut JSContext,
            window: &D::Window,
            _proto: Option<HandleObject>,
        ) -> DomRoot<Foopy<D>> {
            let global = window.upcast::<D::GlobalScope>();
            let channel = global.script_to_constellation_chan();
            let (sender, receiver) = generic_channel::channel().unwrap();
            let msg = EmbedderMessage::BootstrapChannel(sender);
            let serialized = serde_json::to_string(&msg).unwrap().into_bytes();
            let _ = channel.send(ScriptToConstellationMessage::Embedder(serialized));
            let this = Self {
                reflector_: Reflector::new(),
                marker: PhantomData,
                channel: receiver.recv().unwrap(),
            };
            reflect_dom_object_with_wrap::<D, _, _>(
                cx,
                Box::new(this),
                window,
                FoopyWrap::<D>,
            )
        }
    }

    impl<D: Equivalence> RoutedPromiseListener<D, ()> for Foopy<D>
    where
        Self: DomGlobalGeneric<D>,
    //EventHandlerNonNull<D>: CallbackContainer,
    {
        fn handle_response(
            &self,
            cx: &mut js::context::JSContext,
            response: (),
            promise: &<D::Promise as PromiseHelpers<D>>::StackRoot,
        ) {
            promise.resolve_native(cx, &response);
        }
    }
}

pub mod dom {
    pub(crate) mod types {}
    pub(crate) mod bindings {
        pub(crate) use script_bindings::*;
    }

    pub use crate::foopy;
}
