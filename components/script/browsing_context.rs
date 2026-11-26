use std::rc::Rc;

use embedder_traits::ViewportDetails;
use js::context::JSContext;
use script_bindings::inheritance::Castable;
use script_bindings::root::DomRoot;
use script_bindings::str::DOMString;
use script_traits::{DocumentActivity, WebViewState};
use servo_base::id::{BrowsingContextId, PipelineId};
use servo_url::ServoUrl;

use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::document::{Document, HasBrowsingContext, IsHTMLDocument};
use crate::dom::element::Element;
use crate::dom::globalscope::GlobalScope;
use crate::dom::servoparser::populate_about_blank;
use crate::event_loop::document_loader::{DocumentLoader, LoadType};
use crate::event_loop::script_thread::ScriptThread;
use crate::navigation::{
    determine_creation_sandboxing_flags, determine_iframe_element_referrer_policy,
    determine_the_origin,
};

pub(crate) struct InitialBrowsingContextDetails {
    pub(crate) webview_state: Rc<WebViewState>,
    pub(crate) parent_info: Option<PipelineId>,
    pub(crate) viewport_details: ViewportDetails,
    pub(crate) opener: Option<BrowsingContextId>,
    pub(crate) inherited_secure_context: Option<bool>,
    pub(crate) frame_name: Option<DOMString>,
    pub(crate) new_pipeline_id: PipelineId,
    pub(crate) browsing_context_id: BrowsingContextId,
}

/// <https://html.spec.whatwg.org/multipage/#creating-a-new-browsing-context>
pub(crate) fn create_a_new_browsing_context_and_document(
    cx: &mut JSContext,
    creator: Option<&Document>,
    embedder: Option<&Element>,
    details: InitialBrowsingContextDetails,
) -> DomRoot<Document> {
    // Step 1. Let browsingContext be a new browsing context.
    // TODO
    // Step 2. Let unsafeContextCreationTime be the unsafe shared current time.
    // TODO

    // Step 3. Let creatorOrigin be null.
    // Step 4. Let creatorBaseURL be null.
    // Step 5. If creator is non-null:
    // Step 5.1. Set creatorOrigin to creator's origin.
    // Step 5.2. Set creatorBaseURL to creator's document base URL.
    let creator_origin = creator.map(|creator| (*creator.origin()).clone());
    let creator_base_url = creator.map(|creator| creator.base_url());
    // Step 5.3. Set browsingContext's virtual browsing context group ID to
    //   creator's browsing context's top-level browsing context's virtual
    //   browsing context group ID.
    // TODO

    // Step 6. Let sandboxFlags be the result of determining the creation
    //   sandboxing flags given browsingContext and embedder.
    // Note: browsingContext's sandboxing flags are always empty in step 1,
    //   so we simplify by not providing one here.
    let sandbox_flags = determine_creation_sandboxing_flags(None, embedder);

    // Step 7. Let origin be the result of determining the origin given
    //   about:blank, sandboxFlags, and creatorOrigin.
    let about_blank = ServoUrl::parse("about:blank").unwrap();
    let origin = determine_the_origin(Some(&about_blank), sandbox_flags, creator_origin);

    // Step 8. Let permissionsPolicy be the result of creating a permissions
    //   policy given embedder and origin.
    // TODO

    // Step 9. Let agent be the result of obtaining a similar-origin window agent
    //   given origin, group, and false.
    // TODO

    // Step 10. Let realm execution context be the result of creating a new realm
    //   given agent and the following customizations:
    // * For the global object, create a new Window object.
    // * For the global this binding, use browsingContext's WindowProxy object.

    // Step 11. Let topLevelCreationURL be the return value of running these steps:
    // Step 11.1. If embedder is non-null, then return embedder's relevant settings object's top-level creation URL.
    // Step 11.2. If creator is non-null, then return creator's relevant settings object's top-level creation URL.
    // Step 11.3. Return about:blank.
    let top_level_creation_url = embedder
        .and_then(|embedder| embedder.global().top_level_creation_url())
        .or_else(|| creator.and_then(|creator| creator.global().top_level_creation_url()))
        .unwrap_or_else(|| about_blank.clone());

    // Step 12. Let topLevelOrigin be origin if embedder is null; otherwise embedder's relevant settings object's top-level origin.
    // TODO
    // Step 13. Set up a window environment settings object with about:blank, realm execution context, null, topLevelCreationURL, and topLevelOrigin.
    //let new_pipeline_id = new_pipeline_info.new_pipeline_id.clone();
    let window = ScriptThread::create_realm(
        cx,
        &*details.webview_state,
        details.new_pipeline_id,
        details.parent_info,
        details.viewport_details,
        origin.clone(),
        about_blank.clone(),
        top_level_creation_url,
        details.inherited_secure_context,
        details.browsing_context_id,
        details.opener,
        details.frame_name,
    );

    // Step 14. Let loadTimingInfo be a new document load timing info with its navigation start time set to the result of calling coarsen time with unsafeContextCreationTime and the new environment settings object's cross-origin isolated capability.
    // TODO

    // Step 15. Let document be a new Document, with:
    let loader = DocumentLoader::new_with_threads(
        window.upcast::<GlobalScope>().resource_threads().clone(),
        None,
    );
    let document = Document::new(
        cx,
        &window,
        HasBrowsingContext::Yes,
        Some(about_blank.clone()),
        creator_base_url,
        origin,
        IsHTMLDocument::HTMLDocument,
        "text/html".parse().ok(),
        None,                          // last modified
        DocumentActivity::FullyActive, //XXXjdm
        loader,
        creator.map(|creator| creator.url().as_str().to_owned()),
        None,  /* status code */
        true,  /* is about blank */
        true,  /* allow declarative shadow roots */
        None,  /* inherited insecure requests policy */
        false, /* has trustworthy ancestor origin */
        ScriptThread::custom_element_reaction_stack(),
        sandbox_flags,
        details.new_pipeline_id,
        window.layout().image_cache(),
    );
    window.init_document(&document);

    ScriptThread::track_document(&document);

    // Step 16. Let iframeReferrerPolicy be the result of determining the iframe element referrer policy given embedder.
    let iframe_referrer_policy = determine_iframe_element_referrer_policy(embedder);

    // Step 17. Set document's internal ancestor origin objects list to the result of running the internal ancestor origin objects list creation steps given document and iframeReferrerPolicy.
    document.set_internal_ancestor_origin_objects_list(
        document.internal_ancestor_origin_objects_list_creation_steps(&iframe_referrer_policy),
    );

    // Step 18. Set document's ancestor origins list to the result of running the ancestor origins list creation steps given document.
    document.set_ancestor_origins_list(&document.ancestor_origins_list_creation_steps(cx));

    // Step 19. If creator is non-null:
    // Step 19.1. Set document's referrer to the serialization of creator's URL.
    // Step 19.2. Set document's policy container to a clone of creator's policy container.
    if let Some(creator) = creator {
        document.set_policy_container(creator.policy_container().clone());
    }
    // Step 19.3. If creator's origin is same origin with creator's relevant settings object's top-level origin, then set document's opener policy to creator's browsing context's top-level browsing context's active document's opener policy.
    // TODO

    // Step 20. Assert: document's URL and document's relevant settings object's creation URL are about:blank.
    debug_assert!(document.url() == about_blank);
    debug_assert!(document.global().creation_url() == about_blank);

    // Step 21. Mark document as ready for post-load tasks.
    // TODO

    // Step 22. Populate with html/head/body given document.
    populate_about_blank(cx, &document);

    // Step 23. Make active document.
    // Note: already occurred in ScriptThread::create_realm.
    //ScriptThread::make_active(&document);

    // Step 24. Completely finish loading document.
    document.completely_finish_loading();

    // Step 25. Return browsingContext and document.
    document
}

/// <https://html.spec.whatwg.org/multipage/#creating-a-new-auxiliary-browsing-context>
pub(crate) fn create_a_new_auxiliary_browsing_context_and_document(
    cx: &mut JSContext,
    opener: &Document,
    details: InitialBrowsingContextDetails,
) -> DomRoot<Document> {
    // Step 1. Let openerTopLevelBrowsingContext be opener's top-level traversable's active browsing context.
    // Step 2. Let group be openerTopLevelBrowsingContext's group.
    // Step 3. Assert: group is non-null, as navigating invokes this directly.
    // TODO

    // Step 4. Let browsingContext and document be the result of creating a new browsing context and document with opener's active document, null, and group.
    let document = create_a_new_browsing_context_and_document(cx, Some(opener), None, details);

    // Step 5. Set browsingContext's is auxiliary to true.
    // Step 6. Append browsingContext to group.
    // TODO

    // Step 7. Set browsingContext's opener browsing context to opener.
    // Note: Already performed as part of create_a_new_browsing_context_and_document.

    // Step 8. Set browsingContext's virtual browsing context group ID to openerTopLevelBrowsingContext's virtual browsing context group ID.
    // Step 9. Set browsingContext's opener origin at creation to opener's active document's origin.
    // TODO

    // Step 10. Return browsingContext and document.
    document
}

/// <https://html.spec.whatwg.org/multipage/#creating-a-new-top-level-browsing-context>
pub(crate) fn create_a_new_top_level_browsing_context_and_document(
    cx: &mut JSContext,
    details: InitialBrowsingContextDetails,
) -> DomRoot<Document> {
    // Step 1. Let group and document be the result of creating a new browsing context group and document.
    let document = create_a_new_browsing_context_and_document(cx, None, None, details);

    // Step 2. Return group's browsing context set[0] and document.
    document
}
