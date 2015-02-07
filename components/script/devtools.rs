/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use devtools_traits;
use devtools_traits::{EvaluateJSReply, NodeInfo, Modification};
use dom::bindings::conversions::FromJSValConvertible;
use dom::bindings::conversions::StringificationBehavior;
use dom::bindings::js::{JSRef, Temporary, OptionalRootable};
use dom::bindings::codegen::InheritTypes::{NodeCast, ElementCast};
use dom::bindings::codegen::Bindings::DocumentBinding::DocumentMethods;
use dom::bindings::codegen::Bindings::DOMRectBinding::{DOMRectMethods};
use dom::bindings::codegen::Bindings::ElementBinding::{ElementMethods};
use dom::browsercontext::BrowserContext;
use dom::node::{Node, NodeHelpers};
use dom::window::{ScriptHelpers, WindowHelpers};
use dom::element::Element;

use std::rc::Rc;
use std::sync::mpsc::Sender;

pub fn handle_evaluate_js(context: &Rc<BrowserContext>, eval: String, reply: Sender<EvaluateJSReply>){
    let window = context.active_window().root();
    let cx = window.r().get_cx();
    let rval = window.r().evaluate_js_on_global_with_result(eval.as_slice());

    reply.send(if rval.is_undefined() {
        devtools_traits::VoidValue
    } else if rval.is_boolean() {
        devtools_traits::BooleanValue(rval.to_boolean())
    } else if rval.is_double() {
        devtools_traits::NumberValue(FromJSValConvertible::from_jsval(cx, rval, ()).unwrap())
    } else if rval.is_string() {
        //FIXME: use jsstring_to_str when jsval grows to_jsstring
        devtools_traits::StringValue(FromJSValConvertible::from_jsval(cx, rval, StringificationBehavior::Default).unwrap())
    } else if rval.is_null() {
        devtools_traits::NullValue
    } else {
        //FIXME: jsvals don't have an is_int32/is_number yet
        assert!(rval.is_object());
        panic!("object values unimplemented")
    }).unwrap();
}

pub fn handle_get_root_node(context: &Rc<BrowserContext>, reply: Sender<NodeInfo>) {
    let document = context.active_document().root();

    let node: JSRef<Node> = NodeCast::from_ref(document.r());
    reply.send(node.summarize()).unwrap();
}

pub fn handle_get_document_element(context: &Rc<BrowserContext>, reply: Sender<NodeInfo>) {
    let document = context.active_document().root();
    let document_element = document.r().GetDocumentElement().root().unwrap();

    let node: JSRef<Node> = NodeCast::from_ref(document_element.r());
    reply.send(node.summarize()).unwrap();
}

fn find_node_by_unique_id(context: &Rc<BrowserContext>, node_id: String) -> Temporary<Node> {
    let document = context.active_document().root();
    let node: JSRef<Node> = NodeCast::from_ref(document.r());

    for candidate in node.traverse_preorder() {
        if candidate.get_unique_id().as_slice() == node_id.as_slice() {
            return Temporary::from_rooted(candidate);
        }
    }

    panic!("couldn't find node with unique id {}", node_id)
}

pub fn handle_get_children(context: &Rc<BrowserContext>, node_id: String, reply: Sender<Vec<NodeInfo>>) {
    let parent = find_node_by_unique_id(context, node_id).root();
    let children = parent.r().children().map(|child| child.summarize()).collect();
    reply.send(children).unwrap();
}

pub fn handle_get_layout(context: &Rc<BrowserContext>, node_id: String, reply: Sender<(f32, f32)>) {
    let node = find_node_by_unique_id(context, node_id).root();
    let elem: JSRef<Element> = ElementCast::to_ref(node.r()).expect("should be getting layout of element");
    let rect = elem.GetBoundingClientRect().root();
    reply.send((rect.r().Width(), rect.r().Height())).unwrap();
}

pub fn handle_modify_attribute(context: &Rc<BrowserContext>, node_id: String, modifications: Vec<Modification>) {
    let node = find_node_by_unique_id(context, node_id).root();
    let elem: JSRef<Element> = ElementCast::to_ref(node.r()).expect("should be getting layout of element");

    for modification in modifications.iter(){
        match modification.newValue {
            Some(ref string) => {
                let _ = elem.SetAttribute(modification.attributeName.clone(), string.clone());
            },
            None => elem.RemoveAttribute(modification.attributeName.clone()),
        }
    }
}

pub fn handle_wants_live_notifications(context: &Rc<BrowserContext>, send_notifications: bool) {
    let win = context.active_window().root();
    win.r().set_devtools_wants_updates(send_notifications);
}
