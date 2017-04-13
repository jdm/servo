/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

pub fn float_to_fixed(before: usize, f: f64) -> i32 {
    ((1i32 << before) as f64 * f) as i32
}

pub fn fixed_to_float(before: usize, f: i32) -> f64 {
    f as f64 * 1.0f64 / ((1i32 << before) as f64)
}

pub fn is_bidi_control(c: char) -> bool {
    match c {
        '\u{202A}'...'\u{202E}' => true,
        '\u{2066}'...'\u{2069}' => true,
        '\u{200E}' | '\u{200F}' | '\u{061C}' => true,
        _ => false
    }
}
