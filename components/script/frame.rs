/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! A frame is either active and in the root frame tree, or inactive and cached, isolated
//! from any other frame tree. Cached frames can be retrieved by their pipeline, when a
//! particular frame is required. The constellation dictates the frame tree and changes
//! thereof; the script task does not discard cached frames until the pipeline is explicitly
//! exited.
//!
//! Active frames contain browsing contexts, from which the current document for that
//! frame can be obtained. Cached frames, on the other hand, refer only to documents.
//! There is no deduplication of cached frames, currently - multiple loads of the same
//! document will yield different cached frames containing different documents.

use dom::bindings::js::{JS, JSRef};
use dom::browsercontext::BrowserContext;
use dom::document::{Document, DocumentHelpers};
use dom::window::Window;
use msg::constellation_msg::{PipelineId, SubpageId};

use std::rc::Rc;

/// An inactive frame, with no actual tree structure.
#[jstraceable]
#[must_root]
pub struct CachedFrame {
    pipeline: PipelineId,
    pub children: Vec<PipelineId>,
    pub document: JS<Document>,
}

/// A cache for inactive frames.
#[jstraceable]
#[must_root]
pub struct FrameCache {
    cache: Vec<CachedFrame>
}

impl FrameCache {
    /// Create a new FrameCache.
    pub fn new() -> FrameCache {
        FrameCache {
            cache: vec!(),
        }
    }

    pub fn empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Recursively cache this frame all of its descendant frames.
    #[allow(unrooted_must_root)]
    pub fn add(&mut self, frame: Frame) {
        let Frame { context, children } = frame;
        let win = context.active_window().root();
        win.r().init_browser_context(None);
        let cached = CachedFrame {
            pipeline: context.active_pipeline(),
            children: children.iter().map(|c| c.context.active_pipeline()).collect(),
            document: JS::from_rooted(context.active_document()),
        };
        self.cache.push(cached);
        debug!("adding cached frame with id {:?}", context.active_pipeline());

        for child in children.into_iter() {
            self.add(child);
        }
    }

    /// Remove the matching inactive frame from the cache and return it.
    pub fn remove(&mut self, pipeline: PipelineId) -> Option<CachedFrame> {
        self.cache.iter().position(|frame| frame.pipeline == pipeline).map(|index| {
            debug!("removing cached frame with id {:?}", pipeline);
            self.cache.remove(index)
        })
    }

    /// Execute a callback for every known window object in the cache.
    pub fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        for frame in self.cache.iter() {
            let doc = frame.document.root();
            let win = doc.r().window().root();
            f(win.r())
        }
    }

    /// Retrieve a cached, matching frame.
    pub fn find(&self, pipeline: PipelineId) -> Option<&CachedFrame> {
        self.cache.iter().find(|frame| frame.pipeline == pipeline)
    }
}

/// A tree of active frames.
#[jstraceable]
#[must_root]
pub struct FrameTree {
    root_frame: Option<Frame>
}

impl FrameTree {
    /// Create a new FrameTree.
    pub fn new() -> FrameTree {
        FrameTree {
            root_frame: None,
        }
    }

    /// Returns true if there exists a root frame.
    pub fn has_root(&self) -> bool {
        self.root_frame.is_some()
    }

    /// Retrieve the browsing context at the root of the tree, if it exists.
    pub fn root_context(&self) -> Option<Rc<BrowserContext>> {
        self.root_frame.as_ref().map(|frame| frame.context.clone())
    }

    /// Execute a callback for every window object in the frame tree, including in the
    /// session history.
    pub fn for_all_windows<F>(&self, mut f: F) where F: FnMut(JSRef<Window>) {
        if let Some(ref frame) = self.root_frame {
            frame.for_all_windows(&mut f);
        }
    }

    /// Retrieve a matching frame from the tree. Panics if no match found.
    pub fn get(&self, id: PipelineId) -> &Frame {
        self.find(id, None).unwrap()
    }

    /// Retrieve a matching frame from the tree.
    pub fn find(&self, id: PipelineId, sub: Option<SubpageId>) -> Option<&Frame> {
        self.root_frame.as_ref().and_then(|frame| {
            if let Some(sub) = sub {
                frame.find_by_subframe(id, sub)
            } else {
                frame.find(id)
            }
        })
    }

    /// Retrieve a matching mutable frame from the tree.
    pub fn find_mut(&mut self, id: PipelineId) -> Option<&mut Frame> {
        self.root_frame.as_mut().and_then(|frame| frame.find_mut(id))
    }

    /// Add a frame to the tree, parenting it to the optional provided matching frame.
    /// Panics if attempting to replace an existing root frame.
    #[allow(unrooted_must_root)]
    pub fn add(&mut self, frame: Frame, parent: Option<PipelineId>) {
        if self.root_frame.is_none() {
            assert!(parent.is_none());
            debug!("assigning root frame with id {:?}", frame.context.active_pipeline());
            self.root_frame = Some(frame);
            return;
        }

        // Any change of the root frame should explicitly remove it before replacing it.
        // Therefore, if we have a root frame, we expect a parent frame to be provided.
        let parent_id = parent.unwrap();

        let parent_frame = self.find_mut(parent_id).unwrap();
        debug!("adding child frame with id {:?}", frame.context.active_pipeline());
        parent_frame.children.push(frame);
    }

    /// Remove and return a matching frame from the tree. Panics if no such frame is present.
    pub fn maybe_remove(&mut self, id: PipelineId) -> Option<Frame> {
        let (clear_root, mut removed) = if let Some(ref mut frame) = self.root_frame {
            if frame.context.active_pipeline() == id {
                (true, None)
            } else {
                (false, frame.remove_child(id))
            }
        } else {
            (false, None)
        };
        if clear_root {
            debug!("removing root frame with id {:?}", id);
            removed = self.root_frame.take();
        }
        removed
    }

    pub fn remove(&mut self, id: PipelineId) -> Frame {
        self.maybe_remove(id).unwrap()
    }
}

/// A frame in the frame tree.
#[jstraceable]
#[must_root]
pub struct Frame {
    /// The associated browsing context for this frame.
    pub context: Rc<BrowserContext>,
    children: Vec<Frame>,
}

impl Frame {
    /// Create a new frame for the provided browsing context.
    pub fn new(context: Rc<BrowserContext>) -> Frame {
        Frame {
            context: context,
            children: vec!(),
        }
    }

    pub fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        let win = self.context.active_window().root();
        f(win.r());
        for child in self.children.iter() {
            child.for_all_windows(f);
        }
    }

    fn find_by_subframe(&self, id: PipelineId, sub: SubpageId) -> Option<&Frame> {
        if self.context.active_pipeline() == id {
            for child in self.children.iter() {
                if child.context.active_subpage() == Some(sub) {
                    return Some(child);
                }
            }
            return None;
        }
        for child in self.children.iter() {
            let child_frame = child.find_by_subframe(id, sub);
            if child_frame.is_some() {
                return child_frame;
            }
        }
        None
    }

    fn find(&self, id: PipelineId) -> Option<&Frame> {
        if self.context.active_pipeline() == id {
            return Some(self);
        }
        for child in self.children.iter() {
            let child_frame = child.find(id);
            if child_frame.is_some() {
                return child_frame;
            }
        }
        None
    }

    fn find_mut(&mut self, id: PipelineId) -> Option<&mut Frame> {
        if self.context.active_pipeline() == id {
            return Some(self);
        }
        for child in self.children.iter_mut() {
            let child_frame = child.find_mut(id);
            if child_frame.is_some() {
                return child_frame;
            }
        }
        None
    }

    fn remove_child(&mut self, id: PipelineId) -> Option<Frame> {
        let index = self.children.iter().position(|frame| {
            frame.context.active_pipeline() == id
        });
        if let Some(idx) = index {
            return Some(self.children.remove(idx));
        }
        for child in self.children.iter_mut() {
            let removed = child.remove_child(id);
            if removed.is_some() {
                debug!("removing child frame with id {:?}", id);
                return removed;
            }
        }
        None
    }
}
