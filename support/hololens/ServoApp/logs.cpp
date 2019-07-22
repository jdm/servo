/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#include "pch.h"

void log(const char *format, ...) {
  char buf[4096], *p = buf;
  va_list args;
  int n;

  va_start(args, format);
  n = vsnprintf(p, sizeof buf - 3, format, args);
  va_end(args);

  p += (n < 0) ? sizeof buf - 3 : n;

  while (p > buf && isspace(p[-1])) {
    *--p = '\0';
  }

  *p++ = '\r';
  *p++ = '\n';
  *p = '\0';

  OutputDebugStringA(buf);
}

char* gl_error_string(EGLint error) {
  switch (error) {
  case EGL_SUCCESS:
    return "No error";
  case EGL_NOT_INITIALIZED:
    return "EGL not initialized or failed to initialize";
  case EGL_BAD_ACCESS:
    return "Resource inaccessible";
  case EGL_BAD_ALLOC:
    return "Cannot allocate resources";
  case EGL_BAD_ATTRIBUTE:
    return "Unrecognized attribute or attribute value";
  case EGL_BAD_CONTEXT:
    return "Invalid EGL context";
  case EGL_BAD_CONFIG:
    return "Invalid EGL frame buffer configuration";
  case EGL_BAD_CURRENT_SURFACE:
    return "Current surface is no longer valid";
  case EGL_BAD_DISPLAY:
    return "Invalid EGL display";
  case EGL_BAD_SURFACE:
    return "Invalid surface";
  case EGL_BAD_MATCH:
    return "Inconsistent arguments";
  case EGL_BAD_PARAMETER:
    return "Invalid argument";
  case EGL_BAD_NATIVE_PIXMAP:
    return "Invalid native pixmap";
  case EGL_BAD_NATIVE_WINDOW:
    return "Invalid native window";
  case EGL_CONTEXT_LOST:
    return "Context lost";
  }
  return "Unknown error";
}

void log_gl_error(GLenum const err) {
  log("GL ERROR: %s", gl_error_string(err));
}