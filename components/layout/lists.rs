/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
use style::properties::style_structs;
use style::values::computed::Image;

use crate::context::LayoutContext;
use crate::dom_traversal::{NodeAndStyleInfo, PseudoElementContentItem};
use crate::replaced::ReplacedContents;

/// <https://drafts.csswg.org/css-lists/#content-property>
pub(crate) fn make_marker<'dom>(
    context: &LayoutContext,
    info: &NodeAndStyleInfo<'dom>,
    list_index: u32,
) -> Option<(NodeAndStyleInfo<'dom>, Vec<PseudoElementContentItem>)> {
    let marker_info =
        info.with_pseudo_element(context, style::selector_parser::PseudoElement::Marker)?;
    let style = &marker_info.style;
    let list_style = style.get_list();

    // https://drafts.csswg.org/css-lists/#marker-image
    let marker_image = || match &list_style.list_style_image {
        Image::Url(url) => Some(vec![
            PseudoElementContentItem::Replaced(ReplacedContents::from_image_url(
                marker_info.node,
                context,
                url,
            )?),
            PseudoElementContentItem::Text(" ".into()),
        ]),
        // XXX: Non-None image types unimplemented.
        Image::ImageSet(..) |
        Image::Gradient(..) |
        Image::CrossFade(..) |
        Image::PaintWorklet(..) |
        Image::None => None,
        Image::LightDark(..) => unreachable!("light-dark() should be disabled"),
    };
    let content = marker_image().or_else(|| {
        Some(vec![PseudoElementContentItem::Text(
            marker_string(list_style, list_index)?.into(),
        )])
    })?;

    Some((marker_info, content))
}

/// <https://drafts.csswg.org/css-lists/#marker-string>
fn marker_string(style: &style_structs::List, index: u32) -> Option<String> {
    match style.list_style_type {
        ListStyleType::None => None,
        // TODO: Using non-breaking space here is a bit of a hack to give a bit of margin to outside
        // markers, but really we should be setting `white-space: pre` on them instead.
        // See https://github.com/w3c/csswg-drafts/issues/4891.
        ListStyleType::Disc => Some("•\u{00a0}".to_string()),
        ListStyleType::Circle => Some("◦\u{00a0}".to_string()),
        ListStyleType::Square => Some("▪\u{00a0}".to_string()),
        ListStyleType::DisclosureOpen => Some("▾\u{00a0}".to_string()),
        ListStyleType::DisclosureClosed => Some("‣\u{00a0}".to_string()),
        ListStyleType::Decimal => Some(format!("{}.\u{00a0}", index.to_string())),
        ListStyleType::LowerAlpha => Some(format!(
            "{}.\u{00a0}",
            char::from(b"a"[0] + (index % 26) as u8)
        )),
        ListStyleType::UpperAlpha => Some(format!(
            "{}.\u{00a0}",
            char::from(b"A"[0] + (index % 26) as u8)
        )),
        ListStyleType::ArabicIndic |
        ListStyleType::Bengali |
        ListStyleType::Cambodian |
        ListStyleType::CjkDecimal |
        ListStyleType::Devanagari |
        ListStyleType::Gujarati |
        ListStyleType::Gurmukhi |
        ListStyleType::Kannada |
        ListStyleType::Khmer |
        ListStyleType::Lao |
        ListStyleType::Malayalam |
        ListStyleType::Mongolian |
        ListStyleType::Myanmar |
        ListStyleType::Oriya |
        ListStyleType::Persian |
        ListStyleType::Telugu |
        ListStyleType::Thai |
        ListStyleType::Tibetan |
        ListStyleType::CjkEarthlyBranch |
        ListStyleType::CjkHeavenlyStem |
        ListStyleType::LowerGreek |
        ListStyleType::Hiragana |
        ListStyleType::HiraganaIroha |
        ListStyleType::Katakana |
        ListStyleType::KatakanaIroha => {
            // TODO: Implement support for counters.
            None
        },
    }
}
