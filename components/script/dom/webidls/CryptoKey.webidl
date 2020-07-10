/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
/*
 * The origin of this IDL file is
 * https://w3c.github.io/webcrypto/#dfn-CryptoKey
 */

enum KeyType { "public", "private", "secret" };

enum KeyUsage { "encrypt", "decrypt", "sign", "verify", "deriveKey", "deriveBits", "wrapKey", "unwrapKey" };

[SecureContext,Exposed=(Window,Worker)]
interface CryptoKey {
  readonly attribute KeyType type;
  readonly attribute boolean extractable;
  readonly attribute object algorithm;
  readonly attribute object usages;
};