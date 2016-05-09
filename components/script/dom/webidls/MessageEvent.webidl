/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// https://html.spec.whatwg.org/multipage/#messageevent
[Constructor(DOMString type, optional MessageEventInit eventInitDict)/*, Exposed=Window,Worker*/]
interface MessageEvent : Event {
  readonly attribute any data;
  readonly attribute DOMString origin;
  readonly attribute DOMString lastEventId;
  readonly attribute /*(*/Window /*or MessagePort)*/? source;
  //readonly attribute MessagePort[]? ports;
};

dictionary MessageEventInit : EventInit {
  any data = null;
  DOMString origin = "";
  DOMString lastEventId = "";
  /*(*/Window /*or MessagePort)*/? source = null;
  //sequence<MessagePort> ports;
};
