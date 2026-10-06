/*#![cfg_attr(crown, feature(register_tool))]
#![cfg_attr(crown, register_tool(crown))]*/

#[macro_use]
extern crate servo_jstraceable_derive;
#[macro_use]
extern crate malloc_size_of_derive;

use servo_malloc_size_of as malloc_size_of;

//use servo_dom_struct::dom_struct;
pub(crate) use js::gc::Traceable as JSTraceable;
pub(crate) use script_bindings::inheritance::HasParent;
pub(crate) use script_bindings::reflector::{DomObject, MutDomObject, Reflector};

//pub(crate) use script_bindings::trace::JSTracable;

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
    use crate::dom::foopy::Foopy;

    trait_set::trait_set! {
        pub trait Equivalence = DomTypes<
            Foopy = Foopy<Self>,
        >;
    }
}

pub mod foopy {
    use std::marker::PhantomData;

    use servo_deny_public_fields as deny_public_fields;
    //use script_bindings::inheritance::HasParent;
    //use servo_malloc_size_of::MallocSizeOf;
    use script_bindings::reflector::Reflector;
    use script_bindings::codegen::GenericBindings::FoopyBinding::Foopy_Binding::FoopyMethods;
    use script_bindings::str::USVString;
    use script_bindings::DomTypes;

    use crate::traits::Equivalence;

    #[servo_dom_struct::dom_struct(no_crown)]
    pub struct Foopy<D: DomTypes> {
        reflector_: Reflector,
        #[no_trace = "PhantomData does not exist"]
        marker: PhantomData<D>,
    }

    impl<D: Equivalence> FoopyMethods<D> for Foopy<D> {
        fn Foopy(&self) -> USVString { todo!() }
    }
}

pub mod dom {
    pub(crate) mod types {}
    pub(crate) mod bindings {
        pub(crate) use script_bindings::*;
    }

    pub use crate::foopy;
}
