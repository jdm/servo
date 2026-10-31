/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The origin of this IDL file is
 * https://webaudio.github.io/web-audio-api/#AudioProcessingEvent
 */

[Exposed=Window]
interface AudioProcessingEvent : Event {
    constructor (DOMString type, AudioProcessingEventInit eventInitDict);
    readonly attribute double playbackTime;
    readonly attribute AudioBuffer inputBuffer;
    readonly attribute AudioBuffer outputBuffer;
};

dictionary AudioProcessingEventInit : EventInit {
    required double playbackTime;
    required AudioBuffer inputBuffer;
    required AudioBuffer outputBuffer;
};