/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;

use dom_struct::dom_struct;
use js::context::JSContext;
use script_bindings::inheritance::Castable;
use script_bindings::num::Finite;
use script_bindings::refcounted::Trusted;
use script_bindings::reflector::reflect_dom_object;
use script_bindings::root::DomRoot;
use servo_media::audio::audio_node::{AudioNodeInit, AudioNodeMessage};
use servo_media::audio::block::{Block, Chunk, FRAMES_PER_BLOCK_USIZE, FrameIterator};
use servo_media::audio::graph::NodeId;
use servo_media::audio::script_processor_node::{
    ScriptProcessorNodeMessage, ScriptProcessorNodeOptions,
};
use stylo_atoms::Atom;

use crate::dom::audio::audiobuffer::AudioBuffer;
use crate::dom::audio::audionode::AudioNode;
use crate::dom::audio::baseaudiocontext::BaseAudioContext;
use crate::dom::audionode::UnwrappedAudioNodeOptions;
use crate::dom::bindings::codegen::Bindings::AudioNodeBinding::{
    AudioNodeMethods, ChannelCountMode, ChannelInterpretation,
};
use crate::dom::bindings::codegen::Bindings::ScriptProcessorNodeBinding::ScriptProcessorNodeMethods;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::event::Event;
use crate::dom::event::audioprocessingevent::AudioProcessingEvent;
use crate::dom::window::Window;

#[dom_struct]
pub(crate) struct ScriptProcessorNode {
    node: AudioNode,
    buffer_size: u32,
}

impl ScriptProcessorNode {
    fn new_inherited(
        context: &BaseAudioContext,
        node_id: Option<NodeId>,
        options: UnwrappedAudioNodeOptions,
        number_of_inputs: u32,
        number_of_outputs: u32,
        buffer_size: u32,
    ) -> Self {
        ScriptProcessorNode {
            node: AudioNode::new_inherited(
                node_id, context, options, number_of_inputs,
                number_of_outputs,
            ),
            buffer_size,
        }
    }

    pub(crate) fn new(
        cx: &mut JSContext,
        window: &Window,
        context: &BaseAudioContext,
        number_of_inputs: u32,
        number_of_outputs: u32,
        buffer_size: u32,
    ) -> DomRoot<ScriptProcessorNode> {
        let node_options = UnwrappedAudioNodeOptions {
            count: number_of_inputs,
            mode: ChannelCountMode::Clamped_max,
            interpretation: ChannelInterpretation::Speakers,
        };
        let buffer_size = if buffer_size > 0 { buffer_size } else { 4096 };
        let audio_node_init =
            AudioNodeInit::ScriptProcessorNode(ScriptProcessorNodeOptions { buffer_size});
        let node_id =
            AudioNode::generate_audio_node_id(cx, &node_options, audio_node_init, context);

        let node = reflect_dom_object(
            cx,
            Box::new(ScriptProcessorNode::new_inherited(
                context,
                node_id,
                node_options,
                number_of_inputs,
                number_of_outputs,
                buffer_size,
            )),
            window,
        );

        let trusted = Trusted::new(&*node);
        let task_source = window
            .task_manager()
            .dom_manipulation_task_source()
            .to_sendable();
        let callback = Box::new(move |buffer: Vec<Chunk>, sample_rate, time| {
            //println!("queueing task");
            let node = trusted.clone();
            task_source.queue(task!(fire_event: move |cx| {
                //println!("preparing event");
                let node = node.root();
                let global = node.global();
                let window = global.as_window();
                let length = (buffer.len() * FRAMES_PER_BLOCK_USIZE) as u32;
                let mut data = vec![vec![]];
                let num_chunks = buffer.len();
                for mut chunk in buffer {
                    let mut iter = FrameIterator::new(&mut chunk.blocks[0]);
                    while let Some(frame) = iter.next() {
                        data[0].extend(frame.get_frame());
                    }
                }
                let num_inputs = node.upcast::<AudioNode>().NumberOfInputs();
                let num_outputs = node.upcast::<AudioNode>().NumberOfOutputs();
                let input_buffer = AudioBuffer::new_with_channel_data(
                    cx,
                    window,
                    //num_inputs,
                    length,
                    sample_rate,
                    data.clone(),
                );
                let output_buffer = AudioBuffer::new_with_channel_data(
                    cx,
                    window,
                    //num_outputs,
                    length,
                    sample_rate,
                    data,
                );
                let event = AudioProcessingEvent::new(
                    cx,
                    window,
                    Atom::from("audioprocess"),
                    &input_buffer,
                    &output_buffer,
                    Finite::new(time).expect("should always have finite time"),
                    false,
                    false,
                );
                event.upcast::<Event>().set_trusted(true);
                event.upcast::<Event>().fire(cx, node.upcast());

                let data = output_buffer.get_channels(cx).map(|buffer| buffer.buffers.clone()).unwrap_or_default();
                let mut chunks = vec![];
                for i in 0..num_chunks {
                    let mut block = Block::empty();
                    block.push_chan(&data[i * FRAMES_PER_BLOCK_USIZE]);
                    chunks.push(Chunk::new(block));
                }
                node.upcast::<AudioNode>().message(
                    AudioNodeMessage::ScriptProcessorNode(
                        ScriptProcessorNodeMessage::ProcessedBuffer(chunks)
                    )
                );
            }));
        });
        node.upcast::<AudioNode>()
            .message(AudioNodeMessage::ScriptProcessorNode(
                ScriptProcessorNodeMessage::SetupCallback(callback),
            ));

        node
    }
}

impl ScriptProcessorNodeMethods<crate::DomTypeHolder> for ScriptProcessorNode {
    fn BufferSize(&self) -> i32 {
        self.buffer_size as i32
    }

    event_handler!(audioprocess, GetOnaudioprocess, SetOnaudioprocess);
}
