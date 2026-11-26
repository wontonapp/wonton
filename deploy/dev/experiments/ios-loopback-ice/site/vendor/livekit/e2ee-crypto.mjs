// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 15232-15428 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { EncryptedPacketPayload } from './protobuf-messages-room.mjs';

const ENCRYPTION_ALGORITHM = 'AES-GCM';
// How many consecutive frames can fail decrypting before a particular key gets marked as invalid
const DECRYPTION_FAILURE_TOLERANCE = 10;
// flag set to indicate that e2ee has been setup for sender/receiver;
const E2EE_FLAG = 'lk_e2ee';
/**
 * trackId a receiver's/sender's encoded stream pipeline was last set up for.
 * Needed to recover the right cryptor when a transceiver gets reused.
 */
const E2EE_TRACK_ID = 'lk_e2ee_track_id';
const SALT = 'LKFrameEncryptionKey';
const KEY_PROVIDER_DEFAULTS = {
  sharedKey: false,
  ratchetSalt: SALT,
  ratchetWindowSize: 8,
  failureTolerance: DECRYPTION_FAILURE_TOLERANCE,
  keyringSize: 16,
  keySize: 128
};var KeyProviderEvent;
(function (KeyProviderEvent) {
  KeyProviderEvent["SetKey"] = "setKey";
  /** Event for requesting to ratchet the key used to encrypt the stream */
  KeyProviderEvent["RatchetRequest"] = "ratchetRequest";
  /** Emitted when a key is ratcheted. Could be after auto-ratcheting on decryption failure or
   *  following a `RatchetRequest`, will contain the ratcheted key material */
  KeyProviderEvent["KeyRatcheted"] = "keyRatcheted";
})(KeyProviderEvent || (KeyProviderEvent = {}));
var KeyHandlerEvent;
(function (KeyHandlerEvent) {
  /** Emitted when a key has been ratcheted. Is emitted when any key has been ratcheted
   * i.e. when the FrameCryptor tried to ratchet when decryption is failing  */
  KeyHandlerEvent["KeyRatcheted"] = "keyRatcheted";
})(KeyHandlerEvent || (KeyHandlerEvent = {}));
var EncryptionEvent;
(function (EncryptionEvent) {
  EncryptionEvent["ParticipantEncryptionStatusChanged"] = "participantEncryptionStatusChanged";
  EncryptionEvent["EncryptionError"] = "encryptionError";
})(EncryptionEvent || (EncryptionEvent = {}));
var CryptorEvent;
(function (CryptorEvent) {
  CryptorEvent["Error"] = "cryptorError";
})(CryptorEvent || (CryptorEvent = {}));function isE2EESupported() {
  return isInsertableStreamSupported() || isScriptTransformSupported();
}
function isScriptTransformSupported() {
  // @ts-ignore
  return typeof window !== 'undefined' && typeof window.RTCRtpScriptTransform !== 'undefined';
}
function isInsertableStreamSupported() {
  return typeof window !== 'undefined' && typeof window.RTCRtpSender !== 'undefined' &&
  // @ts-ignore
  typeof window.RTCRtpSender.prototype.createEncodedStreams !== 'undefined';
}
function isVideoFrame(frame) {
  return 'type' in frame;
}
function importKey(keyBytes_1) {
  return __awaiter(this, arguments, void 0, function (keyBytes) {
    let algorithm = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {
      name: ENCRYPTION_ALGORITHM
    };
    let usage = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : 'encrypt';
    return function* () {
      // https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/importKey
      return crypto.subtle.importKey('raw', keyBytes, algorithm, false, usage === 'derive' ? ['deriveBits', 'deriveKey'] : ['encrypt', 'decrypt']);
    }();
  });
}
function createKeyMaterialFromString(password) {
  return __awaiter(this, void 0, void 0, function* () {
    let enc = new TextEncoder();
    const keyMaterial = yield crypto.subtle.importKey('raw', enc.encode(password), {
      name: 'PBKDF2'
    }, false, ['deriveBits', 'deriveKey']);
    return keyMaterial;
  });
}
function createKeyMaterialFromBuffer(cryptoBuffer) {
  return __awaiter(this, void 0, void 0, function* () {
    const keyMaterial = yield crypto.subtle.importKey('raw', cryptoBuffer, 'HKDF', false, ['deriveBits', 'deriveKey']);
    return keyMaterial;
  });
}
function getAlgoOptions(algorithmName, salt) {
  const textEncoder = new TextEncoder();
  const encodedSalt = textEncoder.encode(salt);
  switch (algorithmName) {
    case 'HKDF':
      return {
        name: 'HKDF',
        salt: encodedSalt,
        hash: 'SHA-256',
        info: new ArrayBuffer(128)
      };
    case 'PBKDF2':
      {
        return {
          name: 'PBKDF2',
          salt: encodedSalt,
          hash: 'SHA-256',
          iterations: 100000
        };
      }
    default:
      throw new Error("algorithm ".concat(algorithmName, " is currently unsupported"));
  }
}
/**
 * Derives a set of keys from the master key.
 * See https://tools.ietf.org/html/draft-omara-sframe-00#section-4.3.1
 */
function deriveKeys(material, options) {
  return __awaiter(this, void 0, void 0, function* () {
    const algorithmOptions = getAlgoOptions(material.algorithm.name, options.ratchetSalt);
    // https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/deriveKey#HKDF
    // https://developer.mozilla.org/en-US/docs/Web/API/HkdfParams
    const encryptionKey = yield crypto.subtle.deriveKey(algorithmOptions, material, {
      name: ENCRYPTION_ALGORITHM,
      length: options.keySize
    }, false, ['encrypt', 'decrypt']);
    return {
      material,
      encryptionKey
    };
  });
}
function createE2EEKey() {
  return window.crypto.getRandomValues(new Uint8Array(32));
}
/**
 * Ratchets a key. See
 * https://tools.ietf.org/html/draft-omara-sframe-00#section-4.3.5.1
 */
function ratchet(material, salt) {
  return __awaiter(this, void 0, void 0, function* () {
    const algorithmOptions = getAlgoOptions(material.algorithm.name, salt);
    // https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/deriveBits
    return crypto.subtle.deriveBits(algorithmOptions, material, 256);
  });
}
function needsRbspUnescaping(frameData) {
  for (var i = 0; i < frameData.length - 3; i++) {
    if (frameData[i] == 0 && frameData[i + 1] == 0 && frameData[i + 2] == 3) return true;
  }
  return false;
}
function parseRbsp(stream) {
  const dataOut = [];
  var length = stream.length;
  for (var i = 0; i < stream.length;) {
    // Be careful about over/underflow here. byte_length_ - 3 can underflow, and
    // i + 3 can overflow, but byte_length_ - i can't, because i < byte_length_
    // above, and that expression will produce the number of bytes left in
    // the stream including the byte at i.
    if (length - i >= 3 && !stream[i] && !stream[i + 1] && stream[i + 2] == 3) {
      // Two rbsp bytes.
      dataOut.push(stream[i++]);
      dataOut.push(stream[i++]);
      // Skip the emulation byte.
      i++;
    } else {
      // Single rbsp byte.
      dataOut.push(stream[i++]);
    }
  }
  return new Uint8Array(dataOut);
}
const kZerosInStartSequence = 2;
const kEmulationByte = 3;
function writeRbsp(data_in) {
  const dataOut = [];
  var numConsecutiveZeros = 0;
  for (var i = 0; i < data_in.length; ++i) {
    var byte = data_in[i];
    if (byte <= kEmulationByte && numConsecutiveZeros >= kZerosInStartSequence) {
      // Need to escape.
      dataOut.push(kEmulationByte);
      numConsecutiveZeros = 0;
    }
    dataOut.push(byte);
    if (byte == 0) {
      ++numConsecutiveZeros;
    } else {
      numConsecutiveZeros = 0;
    }
  }
  return new Uint8Array(dataOut);
}
function asEncryptablePacket(packet) {
  var _a, _b, _c, _d, _e;
  if (((_a = packet.value) === null || _a === void 0 ? void 0 : _a.case) !== 'sipDtmf' && ((_b = packet.value) === null || _b === void 0 ? void 0 : _b.case) !== 'metrics' && ((_c = packet.value) === null || _c === void 0 ? void 0 : _c.case) !== 'speaker' && ((_d = packet.value) === null || _d === void 0 ? void 0 : _d.case) !== 'transcription' && ((_e = packet.value) === null || _e === void 0 ? void 0 : _e.case) !== 'encryptedPacket') {
    return new EncryptedPacketPayload({
      value: packet.value
    });
  }
  return undefined;
}
export { CryptorEvent, DECRYPTION_FAILURE_TOLERANCE, E2EE_FLAG, E2EE_TRACK_ID, ENCRYPTION_ALGORITHM, EncryptionEvent, KEY_PROVIDER_DEFAULTS, KeyHandlerEvent, KeyProviderEvent, SALT, asEncryptablePacket, createE2EEKey, createKeyMaterialFromBuffer, createKeyMaterialFromString, deriveKeys, getAlgoOptions, importKey, isE2EESupported, isInsertableStreamSupported, isScriptTransformSupported, isVideoFrame, kEmulationByte, kZerosInStartSequence, needsRbspUnescaping, parseRbsp, ratchet, writeRbsp };
