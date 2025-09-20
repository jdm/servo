use servo_url::ServoUrl;

use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::document::Document;
use crate::dom::globalscope::GlobalScope;
use crate::fetch::BlobResolver;

pub(crate) enum RelativeTo<'a> {
    Document(&'a Document),
    Global(&'a GlobalScope),
}

pub(crate) fn parse_url(url: &str, relative_to: RelativeTo) -> Result<ServoUrl, url::ParseError> {
    let global;
    let (base, resource_sender) = match relative_to {
        RelativeTo::Document(doc) => {
            global = doc.global();
            (doc.base_url(), global.resource_threads())
        },
        RelativeTo::Global(global) => (global.api_base_url(), global.resource_threads()),
    };
    ServoUrl::parse_with_base_and_blob_store(Some(&base), url, &BlobResolver(&resource_sender.core_thread))
}
