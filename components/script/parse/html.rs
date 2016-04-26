/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![allow(unrooted_must_root)]

use document_loader::DocumentLoader;
use dom::bindings::codegen::Bindings::DocumentBinding::DocumentMethods;
use dom::bindings::codegen::Bindings::HTMLTemplateElementBinding::HTMLTemplateElementMethods;
use dom::bindings::codegen::Bindings::NodeBinding::NodeMethods;
use dom::bindings::inheritance::{Castable, CharacterDataTypeId, NodeTypeId};
use dom::bindings::js::{JS, Root, RootedReference};
use dom::characterdata::CharacterData;
use dom::comment::Comment;
use dom::document::Document;
use dom::document::{DocumentSource, IsHTMLDocument};
use dom::documenttype::DocumentType;
use dom::element::{Element, ElementCreator};
use dom::htmlformelement::HTMLFormElement;
use dom::htmlscriptelement::HTMLScriptElement;
use dom::htmltemplateelement::HTMLTemplateElement;
use dom::node::Node;
use dom::node::{document_from_node, window_from_node};
use dom::processinginstruction::ProcessingInstruction;
use dom::servohtmlparser;
use dom::servohtmlparser::{FragmentContext, ServoHTMLParser};
use dom::text::Text;
use html5ever::Attribute;
use html5ever::serialize::TraversalScope;
use html5ever::serialize::TraversalScope::{ChildrenOnly, IncludeNode};
use html5ever::serialize::{AttrRef, Serializable, Serializer};
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::{NextParserState, NodeOrText, QuirksMode, TreeSink};
use msg::constellation_msg::PipelineId;
use parse::Parser;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Write};
use string_cache::QualName;
use url::Url;
use util::str::DOMString;

pub enum ServoNodeOrText {
    Node(usize),
    Text(String),
}

pub struct ServoAttribute {
    name: QualName,
    value: String,
}

pub enum ParserOperation {
    GetTemplateContents(usize, usize),
    CreateElement(QualName, Vec<ServoAttribute>, usize),
    CreateComment(String, usize),
    Insert(usize, Option<usize>, ServoNodeOrText),
    AppendDoctypeToDocument(String, String, String),
    AddAttrsIfMissing(usize, Vec<ServoAttribute>),
    RemoveFromParent(usize),
    MarkScriptAlreadyStarted(usize),
    CompleteScript(usize),
    ReparentChild(usize, usize),
}

pub fn process_op(nodes: &mut HashMap<usize, JS<Node>>, op: ParserOperation) {
    let document = Root::from_ref(&**nodes.get(&0).unwrap());
    let document = document.downcast::<Document>().unwrap();
    match op {
        ParserOperation::GetTemplateContents(target, contents) => {
            let target = Root::from_ref(&**nodes.get(&target).unwrap());
            let template = target.downcast::<HTMLTemplateElement>()
                                 .expect("tried to get template contents of \
                                          non-HTMLTemplateElement in HTML parsing");
            assert!(nodes.insert(contents, JS::from_ref(template.Content().upcast())).is_none());
        }

        ParserOperation::CreateElement(name, attrs, id) => {
            let elem = Element::create(name, None, &*document,
                                       ElementCreator::ParserCreated);

            for attr in attrs {
                elem.set_attribute_from_parser(attr.name,
                                               DOMString::from(attr.value),
                                               None);
            }

            nodes.insert(id, JS::from_rooted(&Root::upcast(elem)));
        }

        ParserOperation::CreateComment(text, id) => {
            let comment = Comment::new(DOMString::from(text), &*document);
            nodes.insert(id, JS::from_rooted(&Root::upcast(comment)));
        }

        ParserOperation::Insert(parent, reference_child, new_node) => {
            let parent = &**nodes.get(&parent).unwrap();
            let reference_child = reference_child.map(|r| &**nodes.get(&r).unwrap());
            match new_node {
                ServoNodeOrText::Node(n) => {
                    let n = &**nodes.get(&n).unwrap();
                    assert!(parent.InsertBefore(&n, reference_child).is_ok());
                }
                ServoNodeOrText::Text(t) => {
                    let text = Text::new(DOMString::from(t), &parent.owner_doc());
                    assert!(parent.InsertBefore(text.upcast(), reference_child).is_ok());
                }
            }
        }

        ParserOperation::AppendDoctypeToDocument(name, public_id, system_id) => {
            let doctype = DocumentType::new(
                DOMString::from(name), Some(DOMString::from(public_id)),
                Some(DOMString::from(system_id)), document);
            document.upcast::<Node>().AppendChild(doctype.upcast()).expect("Appending failed");
        }

        ParserOperation::AddAttrsIfMissing(id, attrs) => {
            let target = &**nodes.get(&id).unwrap();
            let elem = target.downcast::<Element>()
                             .expect("tried to set attrs on non-Element in HTML parsing");
            for attr in attrs {
                elem.set_attribute_from_parser(attr.name, DOMString::from(attr.value), None);
            }
        }

        ParserOperation::RemoveFromParent(target) => {
            let target = &**nodes.get(&target).unwrap();
            if let Some(ref parent) = target.GetParentNode() {
                parent.RemoveChild(&*target).unwrap();
            }
        }

        ParserOperation::MarkScriptAlreadyStarted(node) => {
            let node = &**nodes.get(&node).unwrap();
            let script = node.downcast::<HTMLScriptElement>();
            script.map(|script| script.mark_already_started());
        }

        ParserOperation::CompleteScript(node) => {
            let node = &**nodes.get(&node).unwrap();
            let script = node.downcast::<HTMLScriptElement>();
            if let Some(script) = script {
                let _ = script.prepare();
            }
        }

        ParserOperation::ReparentChild(node, new_parent) => {
            let node = &**nodes.get(&node).unwrap();
            let new_parent = &**nodes.get(&new_parent).unwrap();
            while let Some(ref child) = node.GetFirstChild() {
                new_parent.AppendChild(child.r()).unwrap();
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ParseNode {
    pub id: usize,
}

#[derive(JSTraceable, HeapSizeOf)]
pub struct ParseNodeData {
    pub qual_name: Option<QualName>,
    pub parent: Option<usize>,
}

impl<'a> TreeSink for servohtmlparser::Sink {
    type Output = Self;
    fn finish(self) -> Self { self }

    type Handle = ParseNode;

    fn get_document(&mut self) -> ParseNode {
        ParseNode {
            id: 0
        }
    }

    fn get_template_contents(&self, target: ParseNode) -> ParseNode {
        let contents = self.new_parse_node();
        self.enqueue(ParserOperation::GetTemplateContents(target.id, contents.id));
        contents
    }

    fn same_node(&self, x: ParseNode, y: ParseNode) -> bool {
        x == y
    }

    fn elem_name(&self, target: ParseNode) -> QualName {
        self.parse_data
            .borrow()
            .get(&target.id)
            .unwrap()
            .qual_name
            .clone()
            .expect("tried to get name of non-Element in HTML parsing")
    }

    fn create_element(&mut self, name: QualName, attrs: Vec<Attribute>)
                      -> ParseNode {
        let node = self.new_parse_node();
        self.parse_data.borrow_mut().get_mut(&node.id).unwrap().qual_name = Some(name.clone());
        let attrs = attrs.into_iter().map(|a| ServoAttribute { name: a.name, value: a.value.into() }).collect();
        self.enqueue(ParserOperation::CreateElement(name, attrs, node.id));
        node
    }

    fn create_comment(&mut self, text: StrTendril) -> ParseNode {
        let node = self.new_parse_node();
        self.enqueue(ParserOperation::CreateComment(String::from(text), node.id));
        node
    }

    fn append_before_sibling(&mut self,
            sibling: ParseNode,
            new_node: NodeOrText<ParseNode>) -> Result<(), NodeOrText<ParseNode>> {
        // If there is no parent return the node to the parser
        let parent = match self.parse_data.borrow().get(&sibling.id).unwrap().parent {
            Some(p) => p,
            None => return Err(new_node),
        };
        if let NodeOrText::AppendNode(ref n) = new_node {
            self.parse_data.borrow_mut().get_mut(&n.id).unwrap().parent = Some(parent);
        }
        let new_node = match new_node {
            NodeOrText::AppendNode(n) => ServoNodeOrText::Node(n.id),
            NodeOrText::AppendText(t) => ServoNodeOrText::Text(t.into()),
        };
        self.enqueue(ParserOperation::Insert(parent, Some(sibling.id), new_node));
        Ok(())
    }

    fn parse_error(&mut self, msg: Cow<'static, str>) {
        debug!("Parse error: {}", msg);
    }

    fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.document.set_quirks_mode(mode);
    }

    fn append(&mut self, parent: ParseNode, child: NodeOrText<ParseNode>) {
        // FIXME(#3701): Use a simpler algorithm and merge adjacent text nodes
        let child = match child {
            NodeOrText::AppendNode(n) => ServoNodeOrText::Node(n.id),
            NodeOrText::AppendText(t) => ServoNodeOrText::Text(t.into()),
        };
        self.enqueue(ParserOperation::Insert(parent.id, None, child));
    }

    fn append_doctype_to_document(&mut self, name: StrTendril, public_id: StrTendril,
                                  system_id: StrTendril) {
        self.enqueue(ParserOperation::AppendDoctypeToDocument(name.into(),
                                                              public_id.into(),
                                                              system_id.into()));
    }

    fn add_attrs_if_missing(&mut self, target: ParseNode, attrs: Vec<Attribute>) {
        let attrs = attrs.into_iter().map(|a| ServoAttribute { name: a.name, value: a.value.into() }).collect();
        self.enqueue(ParserOperation::AddAttrsIfMissing(target.id, attrs));
    }

    fn remove_from_parent(&mut self, target: ParseNode) {
        self.parse_data.borrow_mut().get_mut(&target.id).unwrap().parent = None;
        self.enqueue(ParserOperation::RemoveFromParent(target.id));
    }

    fn mark_script_already_started(&mut self, node: ParseNode) {
        self.enqueue(ParserOperation::MarkScriptAlreadyStarted(node.id));
    }

    fn complete_script(&mut self, node: ParseNode) -> NextParserState {
        self.enqueue(ParserOperation::CompleteScript(node.id));
        NextParserState::Continue
    }

    fn reparent_children(&mut self, node: ParseNode, new_parent: ParseNode) {
        for data in self.parse_data.borrow_mut().values_mut() {
            if data.parent == Some(node.id) {
                data.parent = Some(new_parent.id);
            }
        }
        self.enqueue(ParserOperation::ReparentChild(node.id, new_parent.id));
    }
}

impl<'a> Serializable for &'a Node {
    fn serialize<'wr, Wr: Write>(&self, serializer: &mut Serializer<'wr, Wr>,
                                 traversal_scope: TraversalScope) -> io::Result<()> {
        let node = *self;
        match (traversal_scope, node.type_id()) {
            (_, NodeTypeId::Element(..)) => {
                let elem = node.downcast::<Element>().unwrap();
                let name = QualName::new(elem.namespace().clone(),
                                         elem.local_name().clone());
                if traversal_scope == IncludeNode {
                    let attrs = elem.attrs().iter().map(|attr| {
                        let qname = QualName::new(attr.namespace().clone(),
                                                  attr.local_name().clone());
                        let value = attr.value().clone();
                        (qname, value)
                    }).collect::<Vec<_>>();
                    let attr_refs = attrs.iter().map(|&(ref qname, ref value)| {
                        let ar: AttrRef = (&qname, &**value);
                        ar
                    });
                    try!(serializer.start_elem(name.clone(), attr_refs));
                }

                let children = if let Some(tpl) = node.downcast::<HTMLTemplateElement>() {
                    // https://github.com/w3c/DOM-Parsing/issues/1
                    tpl.Content().upcast::<Node>().children()
                } else {
                    node.children()
                };

                for handle in children {
                    try!(handle.r().serialize(serializer, IncludeNode));
                }

                if traversal_scope == IncludeNode {
                    try!(serializer.end_elem(name.clone()));
                }
                Ok(())
            },

            (ChildrenOnly, NodeTypeId::Document(_)) => {
                for handle in node.children() {
                    try!(handle.r().serialize(serializer, IncludeNode));
                }
                Ok(())
            },

            (ChildrenOnly, _) => Ok(()),

            (IncludeNode, NodeTypeId::DocumentType) => {
                let doctype = node.downcast::<DocumentType>().unwrap();
                serializer.write_doctype(&doctype.name())
            },

            (IncludeNode, NodeTypeId::CharacterData(CharacterDataTypeId::Text)) => {
                let cdata = node.downcast::<CharacterData>().unwrap();
                serializer.write_text(&cdata.data())
            },

            (IncludeNode, NodeTypeId::CharacterData(CharacterDataTypeId::Comment)) => {
                let cdata = node.downcast::<CharacterData>().unwrap();
                serializer.write_comment(&cdata.data())
            },

            (IncludeNode, NodeTypeId::CharacterData(CharacterDataTypeId::ProcessingInstruction)) => {
                let pi = node.downcast::<ProcessingInstruction>().unwrap();
                let data = pi.upcast::<CharacterData>().data();
                serializer.write_processing_instruction(&pi.target(), &data)
            },

            (IncludeNode, NodeTypeId::DocumentFragment) => Ok(()),

            (IncludeNode, NodeTypeId::Document(_)) => panic!("Can't serialize Document node itself"),
        }
    }
}

pub enum ParseContext<'a> {
    Fragment(FragmentContext<'a>),
    Owner(Option<PipelineId>),
}

pub fn parse_html(document: &Document,
                  input: DOMString,
                  url: Url,
                  context: ParseContext) {
    let parser = match context {
        ParseContext::Owner(owner) =>
            ServoHTMLParser::new(Some(url), document, owner),
        ParseContext::Fragment(fc) =>
            ServoHTMLParser::new_for_fragment(Some(url), document, fc),
    };
    parser.parse_chunk(String::from(input));
}

// https://html.spec.whatwg.org/multipage/#parsing-html-fragments
pub fn parse_html_fragment(context_node: &Node,
                           input: DOMString,
                           output: &Node) {
    let window = window_from_node(context_node);
    let context_document = document_from_node(context_node);
    let context_document = context_document.r();
    let url = context_document.url();

    // Step 1.
    let loader = DocumentLoader::new(&*context_document.loader());
    let document = Document::new(window.r(), None, Some(url.clone()),
                                 IsHTMLDocument::HTMLDocument,
                                 None, None,
                                 DocumentSource::FromParser,
                                 loader);

    // Step 2.
    document.set_quirks_mode(context_document.quirks_mode());

    // Step 11.
    let form = context_node.inclusive_ancestors()
                           .find(|element| element.is::<HTMLFormElement>());
    let fragment_context = FragmentContext {
        context_elem: context_node,
        form_elem: form.r(),
    };
    parse_html(document.r(), input, url.clone(), ParseContext::Fragment(fragment_context));

    // Step 14.
    let root_element = document.GetDocumentElement().expect("no document element");
    for child in root_element.upcast::<Node>().children() {
        output.AppendChild(child.r()).unwrap();
    }
}
