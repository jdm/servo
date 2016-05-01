/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use dom::bindings::codegen::Bindings::TextMetricsBinding::{self, TextMetricsMethods};
use dom::bindings::global::GlobalRef;
use dom::bindings::js::Root;
use dom::bindings::num::Finite;
use dom::bindings::reflector::{Reflector, reflect_dom_object};

#[dom_struct]
pub struct TextMetrics {
    reflector_: Reflector,
    width: f32,
}

impl TextMetrics {
    fn new_inherited(width: f32) -> TextMetrics {
        TextMetrics {
            reflector_: Reflector::new(),
            width: width,
        }
    }

    pub fn new(global: GlobalRef, width: f32) -> Root<TextMetrics> {
        reflect_dom_object(box TextMetrics::new_inherited(width),
                           global,
                           TextMetricsBinding::Wrap)
    }
}

impl TextMetricsMethods for TextMetrics {
    fn Width(&self) -> Finite<f64> {
        Finite::wrap(self.width as f64)
    }
}
