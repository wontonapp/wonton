// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 28502-28644 行按职责切出，行为不变。

/** Sends a `streamTrailer` packet, marking the end of a stream. */
import { __awaiter } from './tslib.mjs';
import { DataPacket } from './protobuf-messages-room.mjs';
import { DataStream_Trailer } from './protobuf-messages-stream.mjs';
import { LivekitReasonedError } from './errors.mjs';
import { DataChannelKind } from './data-channel.mjs';

function sendStreamTrailer(streamId, destinationIdentities, engine) {
  return __awaiter(this, void 0, void 0, function* () {
    const trailerPacket = new DataPacket({
      destinationIdentities,
      value: {
        case: 'streamTrailer',
        value: new DataStream_Trailer({
          streamId
        })
      }
    });
    yield engine.sendDataPacket(trailerPacket, DataChannelKind.RELIABLE);
  });
}// Number type sizes
const U8_LENGTH_BYTES = 1;
const U16_LENGTH_BYTES = 2;
const U32_LENGTH_BYTES = 4;
const U64_LENGTH_BYTES = 8;
/// Constants used for serialization and deserialization.
const SUPPORTED_VERSION = 0;
const BASE_HEADER_LEN = 12;
// Bitfield shifts and masks for header flags
const VERSION_SHIFT = 5;
const VERSION_MASK = 0x07;
const FRAME_MARKER_SHIFT = 3;
const FRAME_MARKER_MASK = 0x3;
const FRAME_MARKER_START = 0x2;
const FRAME_MARKER_FINAL = 0x1;
const FRAME_MARKER_INTER = 0x0;
const FRAME_MARKER_SINGLE = 0x3;
const EXT_WORDS_INDICATOR_SIZE = 2;
const EXT_FLAG_SHIFT = 0x2;
const EXT_FLAG_MASK = 0x1;
const EXT_TAG_PADDING = 0;var DataTrackDeserializeErrorReason;
(function (DataTrackDeserializeErrorReason) {
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["TooShort"] = 0] = "TooShort";
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["HeaderOverrun"] = 1] = "HeaderOverrun";
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["MissingExtWords"] = 2] = "MissingExtWords";
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["UnsupportedVersion"] = 3] = "UnsupportedVersion";
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["InvalidHandle"] = 4] = "InvalidHandle";
  DataTrackDeserializeErrorReason[DataTrackDeserializeErrorReason["MalformedExt"] = 5] = "MalformedExt";
})(DataTrackDeserializeErrorReason || (DataTrackDeserializeErrorReason = {}));
class DataTrackDeserializeError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(19, message, options);
    this.name = 'DataTrackDeserializeError';
    this.reason = reason;
    this.reasonName = DataTrackDeserializeErrorReason[reason];
  }
  static tooShort() {
    return new DataTrackDeserializeError('Too short to contain a valid header', DataTrackDeserializeErrorReason.TooShort);
  }
  static headerOverrun() {
    return new DataTrackDeserializeError('Header exceeds total packet length', DataTrackDeserializeErrorReason.HeaderOverrun);
  }
  static missingExtWords() {
    return new DataTrackDeserializeError('Extension word indicator is missing', DataTrackDeserializeErrorReason.MissingExtWords);
  }
  static unsupportedVersion(version) {
    return new DataTrackDeserializeError("Unsupported version ".concat(version), DataTrackDeserializeErrorReason.UnsupportedVersion);
  }
  static invalidHandle(cause) {
    return new DataTrackDeserializeError("invalid track handle: ".concat(cause.message), DataTrackDeserializeErrorReason.InvalidHandle, {
      cause
    });
  }
  static malformedExt(tag) {
    return new DataTrackDeserializeError("Extension with tag ".concat(tag, " is malformed"), DataTrackDeserializeErrorReason.MalformedExt);
  }
}
var DataTrackSerializeErrorReason;
(function (DataTrackSerializeErrorReason) {
  DataTrackSerializeErrorReason[DataTrackSerializeErrorReason["TooSmallForHeader"] = 0] = "TooSmallForHeader";
  DataTrackSerializeErrorReason[DataTrackSerializeErrorReason["TooSmallForPayload"] = 1] = "TooSmallForPayload";
})(DataTrackSerializeErrorReason || (DataTrackSerializeErrorReason = {}));
class DataTrackSerializeError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(19, message, options);
    this.name = 'DataTrackSerializeError';
    this.reason = reason;
    this.reasonName = DataTrackSerializeErrorReason[reason];
  }
  static tooSmallForHeader() {
    return new DataTrackSerializeError('Buffer cannot fit header', DataTrackSerializeErrorReason.TooSmallForHeader);
  }
  static tooSmallForPayload() {
    return new DataTrackSerializeError('Buffer cannot fit payload', DataTrackSerializeErrorReason.TooSmallForPayload);
  }
}/** An abstract class implementing common behavior related to data track binary serialization. */
class Serializable {
  /** Encodes the instance as binary and returns the data as a Uint8Array. */
  toBinary() {
    const lengthBytes = this.toBinaryLengthBytes();
    const output = new ArrayBuffer(lengthBytes);
    const view = new DataView(output);
    const writtenBytes = this.toBinaryInto(view);
    if (lengthBytes !== writtenBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("".concat(this.constructor.name, ".toBinary: written bytes (").concat(writtenBytes, " bytes) not equal to allocated array buffer length (").concat(lengthBytes, " bytes)."));
    }
    return new Uint8Array(output);
  }
}var DataTrackExtensionTag;
(function (DataTrackExtensionTag) {
  DataTrackExtensionTag[DataTrackExtensionTag["UserTimestamp"] = 2] = "UserTimestamp";
  DataTrackExtensionTag[DataTrackExtensionTag["E2ee"] = 1] = "E2ee";
})(DataTrackExtensionTag || (DataTrackExtensionTag = {}));
class DataTrackExtension extends Serializable {}
class DataTrackUserTimestampExtension extends DataTrackExtension {
  constructor(timestamp) {
    super();
    this.timestamp = timestamp;
  }
  toBinaryLengthBytes() {
    return U8_LENGTH_BYTES /* tag */ + U8_LENGTH_BYTES /* length */ + DataTrackUserTimestampExtension.lengthBytes;
  }
  toBinaryInto(dataView) {
    let byteIndex = 0;
    dataView.setUint8(byteIndex, DataTrackUserTimestampExtension.tag);
    byteIndex += U8_LENGTH_BYTES;
    dataView.setUint8(byteIndex, DataTrackUserTimestampExtension.lengthBytes);
    byteIndex += U8_LENGTH_BYTES;
    dataView.setBigUint64(byteIndex, this.timestamp);
    byteIndex += U64_LENGTH_BYTES;
    const totalLengthBytes = this.toBinaryLengthBytes();
    if (byteIndex !== totalLengthBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("DataTrackUserTimestampExtension.toBinaryInto: Wrote ".concat(byteIndex, " bytes but expected length was ").concat(totalLengthBytes, " bytes"));
    }
    return byteIndex;
  }
  toJSON() {
    return {
      tag: DataTrackUserTimestampExtension.tag,
      lengthBytes: DataTrackUserTimestampExtension.lengthBytes,
      timestamp: this.timestamp
    };
  }
}
DataTrackUserTimestampExtension.tag = DataTrackExtensionTag.UserTimestamp;
DataTrackUserTimestampExtension.lengthBytes = 8;
export { BASE_HEADER_LEN, DataTrackDeserializeError, DataTrackDeserializeErrorReason, DataTrackExtension, DataTrackExtensionTag, DataTrackSerializeError, DataTrackSerializeErrorReason, DataTrackUserTimestampExtension, EXT_FLAG_MASK, EXT_FLAG_SHIFT, EXT_TAG_PADDING, EXT_WORDS_INDICATOR_SIZE, FRAME_MARKER_FINAL, FRAME_MARKER_INTER, FRAME_MARKER_MASK, FRAME_MARKER_SHIFT, FRAME_MARKER_SINGLE, FRAME_MARKER_START, SUPPORTED_VERSION, Serializable, U16_LENGTH_BYTES, U32_LENGTH_BYTES, U64_LENGTH_BYTES, U8_LENGTH_BYTES, VERSION_MASK, VERSION_SHIFT, sendStreamTrailer };
