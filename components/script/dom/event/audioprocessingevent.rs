use dom_struct::dom_struct;
use js::context::JSContext;
use js::rust::HandleObject;
use script_bindings::domstring::DOMString;
use script_bindings::inheritance::Castable;
use script_bindings::num::Finite;
use script_bindings::reflector::reflect_dom_object_with_proto;
use script_bindings::root::{Dom, DomRoot};
use stylo_atoms::Atom;

use crate::dom::audio::audiobuffer::AudioBuffer;
use crate::dom::bindings::codegen::Bindings::AudioProcessingEventBinding::{
    AudioProcessingEventInit, AudioProcessingEventMethods,
};
use crate::dom::bindings::codegen::Bindings::EventBinding::EventMethods;
use crate::dom::event::Event;
use crate::dom::window::Window;

#[dom_struct]
pub(crate) struct AudioProcessingEvent {
    event: Event,
    input_buffer: Dom<AudioBuffer>,
    output_buffer: Dom<AudioBuffer>,
    playback_time: Finite<f64>,
}

impl AudioProcessingEvent {
    fn new_inherited(
        input_buffer: &AudioBuffer,
        output_buffer: &AudioBuffer,
        playback_time: Finite<f64>,
    ) -> AudioProcessingEvent {
        AudioProcessingEvent {
            event: Event::new_inherited(),
            input_buffer: Dom::from_ref(input_buffer),
            output_buffer: Dom::from_ref(output_buffer),
            playback_time: playback_time,
        }
    }

    pub(crate) fn new(
        cx: &mut JSContext,
        window: &Window,
        type_: Atom,
        input_buffer: &AudioBuffer,
        output_buffer: &AudioBuffer,
        playback_time: Finite<f64>,
        bubbles: bool,
        cancelable: bool,
    ) -> DomRoot<AudioProcessingEvent> {
        Self::new_with_proto(
            cx,
            window,
            None,
            type_,
            input_buffer,
            output_buffer,
            playback_time,
            bubbles,
            cancelable,
        )
    }

    fn new_with_proto(
        cx: &mut JSContext,
        window: &Window,
        proto: Option<HandleObject>,
        type_: Atom,
        input_buffer: &AudioBuffer,
        output_buffer: &AudioBuffer,
        playback_time: Finite<f64>,
        bubbles: bool,
        cancelable: bool,
    ) -> DomRoot<AudioProcessingEvent> {
        let ev = reflect_dom_object_with_proto(
            cx,
            Box::new(AudioProcessingEvent::new_inherited(
                input_buffer,
                output_buffer,
                playback_time,
            )),
            window,
            proto,
        );
        {
            let event = ev.upcast::<Event>();
            event.init_event(type_, bubbles, cancelable);
        }
        ev
    }
}

impl AudioProcessingEventMethods<crate::DomTypeHolder> for AudioProcessingEvent {
    fn Constructor(
        cx: &mut JSContext,
        window: &Window,
        proto: Option<HandleObject>,
        type_: DOMString,
        init: &AudioProcessingEventInit,
    ) -> DomRoot<AudioProcessingEvent> {
        AudioProcessingEvent::new_with_proto(
            cx,
            window,
            proto,
            Atom::from(type_),
            &init.inputBuffer,
            &init.outputBuffer,
            init.playbackTime,
            init.parent.bubbles,
            init.parent.cancelable,
        )
    }

    fn PlaybackTime(&self) -> Finite<f64> {
        self.playback_time
    }

    fn InputBuffer(&self) -> DomRoot<AudioBuffer> {
        self.input_buffer.as_rooted()
    }

    fn OutputBuffer(&self) -> DomRoot<AudioBuffer> {
        self.output_buffer.as_rooted()
    }

    fn IsTrusted(&self) -> bool {
        self.event.IsTrusted()
    }
}
