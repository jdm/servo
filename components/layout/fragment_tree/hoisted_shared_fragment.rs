/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use app_units::Au;
use malloc_size_of_derive::MallocSizeOf;

use super::Fragment;
use crate::geom::PhysicalRect;

impl HoistedSharedFragment {
    pub(crate) fn new(original_static_position_rect: PhysicalRect<Au>) -> Self {
        Self {
            fragment: Default::default(),
            original_static_position_rect,
        }
    }
}
