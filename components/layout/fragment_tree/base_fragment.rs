/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use bitflags::bitflags;
use html5ever::local_name;
pub use layout_api::{BaseFragment, FragmentFlags, Tag};
use layout_api::combine_id_with_fragment_type;
use layout_api::wrapper_traits::{
    PseudoElementChain, ThreadSafeLayoutElement, ThreadSafeLayoutNode,
};
use malloc_size_of::malloc_size_of_is_0;
use malloc_size_of_derive::MallocSizeOf;
use script::layout_dom::ServoThreadSafeLayoutNode;
use style::dom::OpaqueNode;
use style::selector_parser::PseudoElement;

use crate::dom_traversal::NodeAndStyleInfo;

impl BaseFragment {
    pub(crate) fn anonymous() -> Self {
        BaseFragment {
            tag: None,
            flags: FragmentFlags::empty(),
        }
    }

    pub(crate) fn is_anonymous(&self) -> bool {
        self.tag.is_none()
    }
}

/// Information necessary to construct a new BaseFragment.
#[derive(Clone, Copy, Debug, MallocSizeOf)]
pub(crate) struct BaseFragmentInfo {
    /// The tag to use for the new BaseFragment, if it is not an anonymous Fragment.
    pub tag: Option<Tag>,

    /// The flags to use for the new BaseFragment.
    pub flags: FragmentFlags,
}

impl BaseFragmentInfo {
    pub(crate) fn anonymous() -> Self {
        Self {
            tag: None,
            flags: FragmentFlags::empty(),
        }
    }

    pub(crate) fn new_for_testing(id: usize) -> Self {
        Self {
            tag: Some(Tag {
                node: OpaqueNode(id),
                pseudo_element_chain: Default::default(),
            }),
            flags: FragmentFlags::empty(),
        }
    }
}

impl From<&NodeAndStyleInfo<'_>> for BaseFragmentInfo {
    fn from(info: &NodeAndStyleInfo) -> Self {
        info.node.into()
    }
}

impl From<ServoThreadSafeLayoutNode<'_>> for BaseFragmentInfo {
    fn from(node: ServoThreadSafeLayoutNode) -> Self {
        let pseudo_element_chain = node.pseudo_element_chain();
        let mut flags = FragmentFlags::empty();

        // Anonymous boxes should not have a tag, because they should not take part in hit testing.
        //
        // TODO(mrobinson): It seems that anonymous boxes should take part in hit testing in some
        // cases, but currently this means that the order of hit test results isn't as expected for
        // some WPT tests. This needs more investigation.
        if matches!(
            pseudo_element_chain.innermost(),
            Some(PseudoElement::ServoAnonymousBox) |
                Some(PseudoElement::ServoAnonymousTable) |
                Some(PseudoElement::ServoAnonymousTableCell) |
                Some(PseudoElement::ServoAnonymousTableRow)
        ) {
            return Self::anonymous();
        }

        if let Some(element) = node.as_html_element() {
            if element.is_body_element_of_html_element_root() {
                flags.insert(FragmentFlags::IS_BODY_ELEMENT_OF_HTML_ELEMENT_ROOT);
            }

            match element.get_local_name() {
                &local_name!("br") => {
                    flags.insert(FragmentFlags::IS_BR_ELEMENT);
                },
                &local_name!("table") | &local_name!("th") | &local_name!("td") => {
                    flags.insert(FragmentFlags::IS_TABLE_TH_OR_TD_ELEMENT);
                },
                _ => {},
            }

            if ThreadSafeLayoutElement::is_root(&element) {
                flags.insert(FragmentFlags::IS_ROOT_ELEMENT);
            }
        };

        Self {
            tag: Some(node.into()),
            flags,
        }
    }
}

impl From<BaseFragmentInfo> for BaseFragment {
    fn from(info: BaseFragmentInfo) -> Self {
        Self {
            tag: info.tag,
            flags: info.flags,
        }
    }
}

impl Tag {
    pub(crate) fn to_display_list_fragment_id(self) -> u64 {
        combine_id_with_fragment_type(self.node.id(), self.pseudo_element_chain.primary.into())
    }
}

impl From<ServoThreadSafeLayoutNode<'_>> for Tag {
    fn from(node: ServoThreadSafeLayoutNode<'_>) -> Self {
        Self {
            node: node.opaque(),
            pseudo_element_chain: node.pseudo_element_chain(),
        }
    }
}
