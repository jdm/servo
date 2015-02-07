/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! A frame is either active and in the root frame tree, or inactive and cached, isolated
//! from any other frame tree. Cached frames can be retrieved by their pipeline and subpage
//! id combo, when a particular frame is required. The constellation dictates the frame
//! tree and changes thereof; the script task does not discard cached frames until the
//! pipeline is explicitly exited.

use dom::bindings::js::{JSRef, Temporary};
use dom::browsercontext::BrowserContext;
use dom::window::Window;
use msg::constellation_msg::{PipelineId, SubpageId};

use std::rc::Rc;

/// An inactive frame, with no actual tree structure.
pub struct CachedFrame {
    pipeline: PipelineId,
    subpage: Option<SubpageId>,
    /// The associated browsing context for this frame.
    pub context: Rc<BrowserContext>,
}

/// A cache for inactive frames.
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

    /// Recursively cache this frame all of its descendant frames.
    pub fn add(&mut self, frame: Frame) {
        let Frame { context, children } = frame;
        let cached = CachedFrame {
            pipeline: context.active_pipeline(),
            subpage: context.active_subpage(),
            context: context,
        };
        self.cache.push(cached);

        for child in children.into_iter() {
            self.add(child);
        }
    }

    /// Remove the matching inactive frame from the cache and return it.
    pub fn remove(&mut self, pipeline: PipelineId, subpage: Option<SubpageId>) -> Option<CachedFrame> {
        self.cache.iter().position(|frame| {
            frame.pipeline == pipeline && frame.subpage == subpage
        }).map(|index| {
            self.cache.remove(index)
        })
    }

    /// Execute a callback for every known window object in the cache.
    pub fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        for frame in self.cache.iter() {
            frame.context.session_history().for_all_windows(f);
        }
    }

    /// Retrieve a cached, matching frame.
    pub fn find(&self, pipeline: PipelineId, subpage: Option<SubpageId>) -> Option<&CachedFrame> {
        self.cache.iter().find(|frame| {
            frame.pipeline == pipeline && frame.subpage == subpage
        })
    }
}

/// A tree of active frames.
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

    pub fn has_root(&self) -> bool {
        self.root_frame.is_some()
    }

    /// Retrieve the window at the root of the tree. Panics if there is no root present.
    pub fn root_window(&self) -> Temporary<Window> {
        let frame = self.root_frame.as_ref().unwrap();
        frame.context.active_window()
    }

    /// Execute a callback for every window object in the frame tree, including in the
    /// session history.
    pub fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        if let Some(ref frame) = self.root_frame {
            frame.for_all_windows(f);
        }
    }

    /// Retrieve a matching frame from the tree. Panics if no match found.
    pub fn get(&self, id: PipelineId, sub: Option<SubpageId>) -> &Frame {
        self.find(id, sub).unwrap()
    }

    /// Retrieve a matching mutable frame from the tree. Panics if no match found.
    pub fn get_mut(&mut self, id: PipelineId, sub: Option<SubpageId>) -> &mut Frame {
        self.find_mut(id, sub).unwrap()
    }

    /// Retrieve a matching frame from the tree.
    pub fn find(&self, id: PipelineId, sub: Option<SubpageId>) -> Option<&Frame> {
        self.root_frame.as_ref().and_then(|frame| frame.find(id, sub))
    }

    /// Retrieve a matching mutable frame from the tree.
    pub fn find_mut(&mut self, id: PipelineId, sub: Option<SubpageId>) -> Option<&mut Frame> {
        self.root_frame.as_mut().and_then(|frame| frame.find_mut(id, sub))
    }

    /// Add a frame to the tree, parenting it to the optional provided matching frame.
    /// Panics if attempting to replace an existing root frame.
    pub fn add(&mut self, frame: Frame, parent: Option<(PipelineId, SubpageId)>) {
        if self.root_frame.is_none() {
            assert!(parent.is_none());
            self.root_frame = Some(frame);
            return;
        }

        // Any change of the root frame should explicitly remove it before replacing it.
        // Therefore, if we have a root frame, we expect a parent frame to be provided.
        let (parent_id, subpage_id) = parent.unwrap();

        let parent_frame = self.find_mut(parent_id, Some(subpage_id)).unwrap();
        parent_frame.add_child(frame);
    }

    /// Remove and return a matching frame from the tree. Panics if no such frame is present.
    pub fn remove(&mut self, id: PipelineId, subpage: Option<SubpageId>) -> Frame {
        let (clear_root, mut removed) = if let Some(ref mut frame) = self.root_frame {
            if frame.context.active_pipeline() == id {
                assert!(subpage.is_none());
                (true, None)
            } else {
                (false, frame.remove_child(id, subpage))
            }
        } else {
            (false, None)
        };
        if clear_root {
            removed = self.root_frame.take();
        }
        removed.unwrap()
    }
}

/// A frame in the frame tree.
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

    fn for_all_windows<F>(&self, f: &mut F) where F: FnMut(JSRef<Window>) {
        self.context.session_history().for_all_windows(f);
        for child in self.children.iter() {
            child.for_all_windows(f);
        }
    }

    fn find(&self, id: PipelineId, sub: Option<SubpageId>) -> Option<&Frame> {
        if self.context.active_pipeline() == id && self.context.active_subpage() == sub {
            return Some(self);
        }
        for child in self.children.iter() {
            let child_frame = child.find(id, sub);
            if child_frame.is_some() {
                return child_frame;
            }
        }
        None
    }

    fn find_mut(&mut self, id: PipelineId, sub: Option<SubpageId>) -> Option<&mut Frame> {
        if self.context.active_pipeline() == id && self.context.active_subpage() == sub {
            return Some(self);
        }
        for child in self.children.iter_mut() {
            let child_frame = child.find_mut(id, sub);
            if child_frame.is_some() {
                return child_frame;
            }
        }
        None
    }

    fn add_child(&mut self, child: Frame) {
        self.children.push(child);
    }

    fn remove_child(&mut self, id: PipelineId, subpage: Option<SubpageId>) -> Option<Frame> {
        let index = self.children.iter().position(|frame| {
            frame.context.active_pipeline() == id && frame.context.active_subpage() == subpage
        });
        if let Some(idx) = index {
            return Some(self.children.remove(idx));
        }
        for child in self.children.iter_mut() {
            let removed = child.remove_child(id, subpage);
            if removed.is_some() {
                return removed;
            }
        }
        None
    }
}
