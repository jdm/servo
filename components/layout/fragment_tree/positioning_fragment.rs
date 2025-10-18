/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use app_units::Au;
use base::print_tree::PrintTree;
use malloc_size_of_derive::MallocSizeOf;
use servo_arc::Arc as ServoArc;
use style::properties::ComputedValues;

use super::{BaseFragment, BaseFragmentInfo, Fragment};
use crate::cell::ArcRefCell;
use crate::geom::PhysicalRect;

impl PositioningFragment {
    pub fn new_anonymous(
        style: ServoArc<ComputedValues>,
        rect: PhysicalRect<Au>,
        children: Vec<Fragment>,
    ) -> ArcRefCell<Self> {
        Self::new_with_base_fragment(BaseFragment::anonymous(), style, rect, children)
    }

    pub fn new_empty(
        base_fragment_info: BaseFragmentInfo,
        rect: PhysicalRect<Au>,
        style: ServoArc<ComputedValues>,
    ) -> ArcRefCell<Self> {
        Self::new_with_base_fragment(base_fragment_info.into(), style, rect, Vec::new())
    }

    fn new_with_base_fragment(
        base: BaseFragment,
        style: ServoArc<ComputedValues>,
        rect: PhysicalRect<Au>,
        children: Vec<Fragment>,
    ) -> ArcRefCell<Self> {
        ArcRefCell::new(PositioningFragment {
            base,
            style,
            rect,
            children,
            scrollable_overflow: None,
            cumulative_containing_block_rect: PhysicalRect::zero(),
        })
    }

    pub(crate) fn set_containing_block(&mut self, containing_block: &PhysicalRect<Au>) {
        self.cumulative_containing_block_rect = *containing_block;
    }

    pub fn offset_by_containing_block(&self, rect: &PhysicalRect<Au>) -> PhysicalRect<Au> {
        rect.translate(self.cumulative_containing_block_rect.origin.to_vector())
    }

    pub(crate) fn calculate_scrollable_overflow(&mut self) {
        self.scrollable_overflow = Some(self.children.iter().fold(
            PhysicalRect::zero(),
            |acc, child| {
                acc.union(
                    &child
                        .calculate_scrollable_overflow_for_parent()
                        .translate(self.rect.origin.to_vector()),
                )
            },
        ));
    }

    pub(crate) fn scrollable_overflow_for_parent(&self) -> PhysicalRect<Au> {
        self.scrollable_overflow.expect(
            "Should only call `scrollable_overflow_for_parent()` after calculating overflow",
        )
    }

    pub fn print(&self, tree: &mut PrintTree) {
        tree.new_level(format!(
            "PositioningFragment\
                \nbase={:?}\
                \nrect={:?}\
                \nscrollable_overflow={:?}",
            self.base, self.rect, self.scrollable_overflow
        ));

        for child in &self.children {
            child.print(tree);
        }
        tree.end_level();
    }
}
