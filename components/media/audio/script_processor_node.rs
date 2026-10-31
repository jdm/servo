/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use malloc_size_of_derive::MallocSizeOf;

use crate::audio_node::{AudioNodeEngine, AudioNodeMessage, AudioNodeType, BlockInfo, ChannelInfo};
use crate::block::{Chunk, FRAMES_PER_BLOCK_USIZE};

#[derive(Clone, MallocSizeOf)]
pub struct ScriptProcessorNodeOptions {
    pub buffer_size: u32,
}

impl std::fmt::Debug for ScriptProcessorNodeOptions {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        todo!()
    }
}

#[derive(MallocSizeOf)]
pub enum ScriptProcessorNodeMessage {
    ProcessedBuffer(Vec<Chunk>),
    SetupCallback(#[ignore_malloc_size_of = ""] ScriptProcessorCallback),
}

pub type ScriptProcessorCallback = Box<dyn Fn(Vec<Chunk>, f32, f64) + Send + 'static>;

#[derive(AudioNodeCommon, MallocSizeOf)]
pub(crate) struct ScriptProcessorNode {
    buffer_size: u32,
    channel_info: ChannelInfo,
    #[ignore_malloc_size_of = ""]
    input_buffer_info: Option<BlockInfo>,
    input_buffer: Vec<Chunk>,
    output_buffer: Vec<Chunk>,
    #[ignore_malloc_size_of = "can't measure opaque callbacks"]
    callback: Option<ScriptProcessorCallback>,
}

impl ScriptProcessorNode {
    pub fn new(options: ScriptProcessorNodeOptions, channel_info: ChannelInfo) -> Self {
        Self {
            buffer_size: options.buffer_size,
            callback: None,
            channel_info,
            input_buffer: Default::default(),
            output_buffer: Default::default(),
            input_buffer_info: None,
        }
    }
}

impl AudioNodeEngine for ScriptProcessorNode {
    fn node_type(&self) -> AudioNodeType {
        AudioNodeType::ScriptProcessorNode
    }

    fn process(&mut self, inputs: Chunk, info: &BlockInfo) -> Chunk {
        if self.input_buffer_info.is_none() {
            self.input_buffer_info = Some(*info);
        }
        if !inputs.blocks.is_empty() && !inputs.blocks[0].is_empty() {
            //println!("pushing");
            self.input_buffer.push(inputs);
        }
        if let Some(ref callback) = self.callback &&
            self.input_buffer.len() * FRAMES_PER_BLOCK_USIZE >= self.buffer_size as usize
        {
            //println!("callback time");
            let chunk_info = self.input_buffer_info.take().expect("must be some");
            (callback)(std::mem::take(&mut self.input_buffer), chunk_info.sample_rate, chunk_info.time);
        }
        if !self.output_buffer.is_empty() {
            //println!("returning output block");
            self.output_buffer.remove(0)
        } else {
            Chunk::explicit_silence()
        }
    }

    fn message_specific(&mut self, message: AudioNodeMessage, _sample_rate: f32) {
        //println!("got message");
        match message {
            AudioNodeMessage::ScriptProcessorNode(ScriptProcessorNodeMessage::SetupCallback(
                callback,
            )) => self.callback = Some(callback),
            AudioNodeMessage::ScriptProcessorNode(ScriptProcessorNodeMessage::ProcessedBuffer(
                buffer,
            )) => self.output_buffer.extend(buffer.into_iter()),
            _ => {},
        }
    }
}
