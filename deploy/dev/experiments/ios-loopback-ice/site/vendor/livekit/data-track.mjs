// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 28644-30011 行按职责切出，行为不变。
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { LoggerNames } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { LivekitReasonedError } from './errors.mjs';
import { Future } from './utils-misc.mjs';
import { DataTrackTimestamp, WrapAroundUnsignedInt } from './device.mjs';
import { DataTrackHandle, DataTrackHandleError, DataTrackHandleErrorReason, abortSignalAny, abortSignalTimeout, coerceToDataView } from './data-track-core.mjs';
import { BASE_HEADER_LEN, DataTrackDeserializeError, DataTrackExtension, DataTrackExtensionTag, DataTrackSerializeError, DataTrackUserTimestampExtension, EXT_FLAG_MASK, EXT_FLAG_SHIFT, EXT_TAG_PADDING, EXT_WORDS_INDICATOR_SIZE, FRAME_MARKER_FINAL, FRAME_MARKER_INTER, FRAME_MARKER_MASK, FRAME_MARKER_SHIFT, FRAME_MARKER_SINGLE, FRAME_MARKER_START, SUPPORTED_VERSION, Serializable, U16_LENGTH_BYTES, U32_LENGTH_BYTES, U8_LENGTH_BYTES, VERSION_MASK, VERSION_SHIFT } from './data-streams.mjs';


class DataTrackE2eeExtension extends DataTrackExtension {
  constructor(keyIndex, iv) {
    super();
    this.keyIndex = keyIndex;
    this.iv = iv;
  }
  toBinaryLengthBytes() {
    return U8_LENGTH_BYTES /* tag */ + U8_LENGTH_BYTES /* length */ + DataTrackE2eeExtension.lengthBytes;
  }
  toBinaryInto(dataView) {
    let byteIndex = 0;
    dataView.setUint8(byteIndex, DataTrackE2eeExtension.tag);
    byteIndex += U8_LENGTH_BYTES;
    dataView.setUint8(byteIndex, DataTrackE2eeExtension.lengthBytes);
    byteIndex += U8_LENGTH_BYTES;
    dataView.setUint8(byteIndex, this.keyIndex);
    byteIndex += U8_LENGTH_BYTES;
    for (let i = 0; i < this.iv.length; i += 1) {
      dataView.setUint8(byteIndex, this.iv[i]);
      byteIndex += U8_LENGTH_BYTES;
    }
    const totalLengthBytes = this.toBinaryLengthBytes();
    if (byteIndex !== totalLengthBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("DataTrackE2eeExtension.toBinaryInto: Wrote ".concat(byteIndex, " bytes but expected length was ").concat(totalLengthBytes, " bytes"));
    }
    return byteIndex;
  }
  toJSON() {
    return {
      tag: DataTrackE2eeExtension.tag,
      lengthBytes: DataTrackE2eeExtension.lengthBytes,
      keyIndex: this.keyIndex,
      iv: this.iv
    };
  }
}
DataTrackE2eeExtension.tag = DataTrackExtensionTag.E2ee;
DataTrackE2eeExtension.lengthBytes = 13;
class DataTrackExtensions extends Serializable {
  constructor() {
    let opts = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {};
    super();
    this.userTimestamp = opts.userTimestamp;
    this.e2ee = opts.e2ee;
  }
  toBinaryLengthBytes() {
    let lengthBytes = 0;
    if (this.userTimestamp) {
      lengthBytes += this.userTimestamp.toBinaryLengthBytes();
    }
    if (this.e2ee) {
      lengthBytes += this.e2ee.toBinaryLengthBytes();
    }
    return lengthBytes;
  }
  toBinaryInto(dataView) {
    let byteIndex = 0;
    if (this.e2ee) {
      const e2eeBytes = this.e2ee.toBinaryInto(dataView);
      byteIndex += e2eeBytes;
    }
    if (this.userTimestamp) {
      const userTimestampBytes = this.userTimestamp.toBinaryInto(new DataView(dataView.buffer, dataView.byteOffset + byteIndex));
      byteIndex += userTimestampBytes;
    }
    const totalLengthBytes = this.toBinaryLengthBytes();
    if (byteIndex !== totalLengthBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("DataTrackExtensions.toBinaryInto: Wrote ".concat(byteIndex, " bytes but expected length was ").concat(totalLengthBytes, " bytes"));
    }
    return byteIndex;
  }
  static fromBinary(input) {
    const dataView = coerceToDataView(input);
    let userTimestamp;
    let e2ee;
    let byteIndex = 0;
    while (dataView.byteLength - byteIndex >= U8_LENGTH_BYTES + U8_LENGTH_BYTES) {
      const tag = dataView.getUint8(byteIndex);
      byteIndex += U8_LENGTH_BYTES;
      const lengthBytes = dataView.getUint8(byteIndex);
      byteIndex += U8_LENGTH_BYTES;
      if (tag === EXT_TAG_PADDING) {
        // Skip padding
        continue;
      }
      switch (tag) {
        case DataTrackExtensionTag.UserTimestamp:
          if (dataView.byteLength - byteIndex < DataTrackUserTimestampExtension.lengthBytes) {
            throw DataTrackDeserializeError.malformedExt(tag);
          }
          userTimestamp = new DataTrackUserTimestampExtension(dataView.getBigUint64(byteIndex));
          byteIndex += lengthBytes;
          break;
        case DataTrackExtensionTag.E2ee:
          if (dataView.byteLength - byteIndex < DataTrackE2eeExtension.lengthBytes) {
            throw DataTrackDeserializeError.malformedExt(tag);
          }
          const keyIndex = dataView.getUint8(byteIndex);
          const iv = new Uint8Array(12);
          for (let i = 0; i < iv.length; i += 1) {
            let byteIndexForIv = byteIndex;
            byteIndexForIv += U8_LENGTH_BYTES; // key index
            byteIndexForIv += i * U8_LENGTH_BYTES; // Index into iv array
            iv[i] = dataView.getUint8(byteIndexForIv);
          }
          e2ee = new DataTrackE2eeExtension(keyIndex, iv);
          byteIndex += lengthBytes;
          break;
        default:
          // Skip over unknown extensions (forward compatible).
          if (dataView.byteLength - byteIndex < lengthBytes) {
            throw DataTrackDeserializeError.malformedExt(tag);
          }
          byteIndex += lengthBytes;
          break;
      }
    }
    // NOTE: padding bytes after extension data is intentionally not being processed
    return [new DataTrackExtensions({
      userTimestamp,
      e2ee
    }), dataView.byteLength];
  }
  toJSON() {
    var _a, _b, _c, _d;
    return {
      userTimestamp: (_b = (_a = this.userTimestamp) === null || _a === void 0 ? void 0 : _a.toJSON()) !== null && _b !== void 0 ? _b : null,
      e2ee: (_d = (_c = this.e2ee) === null || _c === void 0 ? void 0 : _c.toJSON()) !== null && _d !== void 0 ? _d : null
    };
  }
}const DataTrackFrameInternal = {
  from(frame) {
    return {
      payload: frame.payload,
      extensions: new DataTrackExtensions({
        userTimestamp: frame.userTimestamp ? new DataTrackUserTimestampExtension(frame.userTimestamp) : undefined
      })
    };
  },
  /** Converts from a DataTrackFrameInternal -> DataTrackFrame. Some internal information is
   * discarded like e2ee encrption extension data. */
  lossyIntoFrame(frame) {
    var _a;
    return {
      payload: frame.payload,
      userTimestamp: (_a = frame.extensions.userTimestamp) === null || _a === void 0 ? void 0 : _a.timestamp
    };
  }
};const TrackSymbol = Symbol.for('lk.track');
const DataTrackSymbol = Symbol.for('lk.data-track');class RemoteDataTrack {
  /** @internal */
  constructor(info, manager, options) {
    this.trackSymbol = TrackSymbol;
    this.isLocal = false;
    this.typeSymbol = DataTrackSymbol;
    this.info = info;
    this.manager = manager;
    this.publisherIdentity = options.publisherIdentity;
  }
  /** Subscribes to the data track to receive frames.
   *
   * # Returns
   *
   * A stream that yields {@link DataTrackFrame}s as they arrive.
   *
   * # Multiple Subscriptions
   *
   * An application may call `subscribe` more than once to process frames in
   * multiple places. For example, one async task might plot values on a graph
   * while another writes them to a file.
   *
   * Internally, only the first call to `subscribe` communicates with the SFU and
   * allocates the resources required to receive frames. Additional subscriptions
   * reuse the same underlying pipeline and do not trigger additional signaling.
   *
   * Note that newly created subscriptions only receive frames published after
   * the initial subscription is established.
   */
  subscribe(options) {
    try {
      const _this$manager$openSub = this.manager.openSubscriptionStream(this.info.sid, options === null || options === void 0 ? void 0 : options.signal, options === null || options === void 0 ? void 0 : options.bufferSize),
        _this$manager$openSub2 = _slicedToArray(_this$manager$openSub, 2),
        stream = _this$manager$openSub2[0],
        sfuSubscriptionComplete = _this$manager$openSub2[1];
      // Prevent uncaught promise rejections from bubbling up if rejections occur after the
      // readable stream is discarded.
      sfuSubscriptionComplete.catch(() => {});
      return stream;
    } catch (err) {
      // NOTE: Rethrow errors to break Throws<...> type boundary
      throw err;
    }
  }
  /** Configure how incoming frames for this track are processed before they are handed out to
   * subscribers (the "pipeline"). These options apply to all current and future subscriptions
   * of this track, and may be set at any time. */
  setPipelineOptions(options) {
    this.manager.setPipelineOptions(this.info.sid, options);
  }
}/** A class for serializing / deserializing data track packet header sections. */
class DataTrackPacketHeader extends Serializable {
  constructor(opts) {
    var _a;
    super();
    this.marker = opts.marker;
    this.trackHandle = opts.trackHandle;
    this.sequence = opts.sequence;
    this.frameNumber = opts.frameNumber;
    this.timestamp = opts.timestamp;
    this.extensions = (_a = opts.extensions) !== null && _a !== void 0 ? _a : new DataTrackExtensions();
  }
  extensionsMetrics() {
    const lengthBytes = this.extensions.toBinaryLengthBytes();
    const lengthWords = Math.ceil((EXT_WORDS_INDICATOR_SIZE + lengthBytes) / 4);
    const paddingLengthBytes = lengthWords * 4 - EXT_WORDS_INDICATOR_SIZE - lengthBytes;
    return {
      lengthBytes,
      lengthWords,
      paddingLengthBytes
    };
  }
  toBinaryLengthBytes() {
    const _this$extensionsMetri = this.extensionsMetrics(),
      extLengthBytes = _this$extensionsMetri.lengthBytes,
      extPaddingLengthBytes = _this$extensionsMetri.paddingLengthBytes;
    let totalLengthBytes = BASE_HEADER_LEN;
    if (extLengthBytes > 0) {
      totalLengthBytes += EXT_WORDS_INDICATOR_SIZE + extLengthBytes + extPaddingLengthBytes;
    }
    return totalLengthBytes;
  }
  toBinaryInto(dataView) {
    if (dataView.byteLength < this.toBinaryLengthBytes()) {
      throw DataTrackSerializeError.tooSmallForHeader();
    }
    let initial = SUPPORTED_VERSION << VERSION_SHIFT;
    let marker;
    switch (this.marker) {
      case FrameMarker.Inter:
        marker = FRAME_MARKER_INTER;
        break;
      case FrameMarker.Final:
        marker = FRAME_MARKER_FINAL;
        break;
      case FrameMarker.Start:
        marker = FRAME_MARKER_START;
        break;
      case FrameMarker.Single:
        marker = FRAME_MARKER_SINGLE;
        break;
    }
    initial |= marker << FRAME_MARKER_SHIFT;
    const _this$extensionsMetri2 = this.extensionsMetrics(),
      extensionsLengthBytes = _this$extensionsMetri2.lengthBytes,
      extensionsLengthWords = _this$extensionsMetri2.lengthWords,
      extensionsPaddingLengthBytes = _this$extensionsMetri2.paddingLengthBytes;
    if (extensionsLengthBytes > 0) {
      initial |= 1 << EXT_FLAG_SHIFT;
    }
    let byteIndex = 0;
    dataView.setUint8(byteIndex, initial);
    byteIndex += U8_LENGTH_BYTES;
    dataView.setUint8(byteIndex, 0); // Reserved
    byteIndex += U8_LENGTH_BYTES;
    dataView.setUint16(byteIndex, this.trackHandle);
    byteIndex += U16_LENGTH_BYTES;
    dataView.setUint16(byteIndex, this.sequence.value);
    byteIndex += U16_LENGTH_BYTES;
    dataView.setUint16(byteIndex, this.frameNumber.value);
    byteIndex += U16_LENGTH_BYTES;
    dataView.setUint32(byteIndex, this.timestamp.asTicks());
    byteIndex += U32_LENGTH_BYTES;
    if (extensionsLengthBytes > 0) {
      // NOTE: The protocol is implemented in a way where if the extension bit is set, any
      // deserializer assumes the extensions section is at least one byte long, and the "length"
      // field represents the "number of additional bytes" long the extensions section is. This is
      // potentially unintuitive so I wanted to call it out.
      const rtpOrientedExtensionLengthWords = extensionsLengthWords - 1;
      dataView.setUint16(byteIndex, rtpOrientedExtensionLengthWords);
      byteIndex += U16_LENGTH_BYTES;
      const extensionBytes = this.extensions.toBinaryInto(new DataView(dataView.buffer, dataView.byteOffset + byteIndex));
      byteIndex += extensionBytes;
      for (let i = 0; i < extensionsPaddingLengthBytes; i += 1) {
        dataView.setUint8(byteIndex, 0);
        byteIndex += U8_LENGTH_BYTES;
      }
    }
    const totalLengthBytes = this.toBinaryLengthBytes();
    if (byteIndex !== totalLengthBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("DataTrackPacketHeader.toBinaryInto: Wrote ".concat(byteIndex, " bytes but expected length was ").concat(totalLengthBytes, " bytes"));
    }
    return totalLengthBytes;
  }
  static fromBinary(input) {
    const dataView = coerceToDataView(input);
    if (dataView.byteLength < BASE_HEADER_LEN) {
      throw DataTrackDeserializeError.tooShort();
    }
    let byteIndex = 0;
    const initial = dataView.getUint8(byteIndex);
    byteIndex += U8_LENGTH_BYTES;
    const version = initial >> VERSION_SHIFT & VERSION_MASK;
    if (version > SUPPORTED_VERSION) {
      throw DataTrackDeserializeError.unsupportedVersion(version);
    }
    let marker;
    switch (initial >> FRAME_MARKER_SHIFT & FRAME_MARKER_MASK) {
      case FRAME_MARKER_START:
        marker = FrameMarker.Start;
        break;
      case FRAME_MARKER_FINAL:
        marker = FrameMarker.Final;
        break;
      case FRAME_MARKER_SINGLE:
        marker = FrameMarker.Single;
        break;
      case FRAME_MARKER_INTER:
      default:
        marker = FrameMarker.Inter;
        break;
    }
    const extensionsFlag = (initial >> EXT_FLAG_SHIFT & EXT_FLAG_MASK) > 0;
    byteIndex += U8_LENGTH_BYTES; // Reserved
    let trackHandle;
    try {
      trackHandle = DataTrackHandle.fromNumber(dataView.getUint16(byteIndex));
    } catch (e) {
      if (e instanceof DataTrackHandleError && (e.isReason(DataTrackHandleErrorReason.Reserved) || e.isReason(DataTrackHandleErrorReason.TooLarge))) {
        throw DataTrackDeserializeError.invalidHandle(e);
      } else {
        throw e;
      }
    }
    byteIndex += U16_LENGTH_BYTES;
    const sequence = WrapAroundUnsignedInt.u16(dataView.getUint16(byteIndex));
    byteIndex += U16_LENGTH_BYTES;
    const frameNumber = WrapAroundUnsignedInt.u16(dataView.getUint16(byteIndex));
    byteIndex += U16_LENGTH_BYTES;
    const timestamp = DataTrackTimestamp.fromRtpTicks(dataView.getUint32(byteIndex));
    byteIndex += U32_LENGTH_BYTES;
    let extensions = new DataTrackExtensions();
    if (extensionsFlag) {
      if (dataView.byteLength - byteIndex < U16_LENGTH_BYTES) {
        throw DataTrackDeserializeError.missingExtWords();
      }
      let rtpOrientedExtensionWords = dataView.getUint16(byteIndex);
      byteIndex += U16_LENGTH_BYTES;
      // NOTE: The protocol is implemented in a way where if the extension bit is set, any
      // deserializer assumes the extensions section is at least one byte long, and the "length"
      // field represents the "number of additional bytes" long the extensions section is. This is
      // potentially unintuitive so I wanted to call it out.
      const extensionWords = rtpOrientedExtensionWords + 1;
      let extensionLengthBytes = 4 * extensionWords - EXT_WORDS_INDICATOR_SIZE;
      if (byteIndex + extensionLengthBytes > dataView.byteLength) {
        throw DataTrackDeserializeError.headerOverrun();
      }
      let extensionDataView = new DataView(dataView.buffer, dataView.byteOffset + byteIndex, extensionLengthBytes);
      const _DataTrackExtensions$ = DataTrackExtensions.fromBinary(extensionDataView),
        _DataTrackExtensions$2 = _slicedToArray(_DataTrackExtensions$, 2),
        result = _DataTrackExtensions$2[0],
        readBytes = _DataTrackExtensions$2[1];
      extensions = result;
      byteIndex += readBytes;
    }
    return [new DataTrackPacketHeader({
      marker,
      trackHandle: trackHandle,
      sequence,
      frameNumber,
      timestamp,
      extensions
    }), byteIndex];
  }
  toJSON() {
    return {
      marker: this.marker,
      trackHandle: this.trackHandle,
      sequence: this.sequence.value,
      frameNumber: this.frameNumber.value,
      timestamp: this.timestamp.asTicks(),
      extensions: this.extensions.toJSON()
    };
  }
}
/** Marker indicating a packet's position in relation to a frame. */
var FrameMarker;
(function (FrameMarker) {
  /** Packet is the first in a frame. */
  FrameMarker[FrameMarker["Start"] = 0] = "Start";
  /** Packet is within a frame. */
  FrameMarker[FrameMarker["Inter"] = 1] = "Inter";
  /** Packet is the last in a frame. */
  FrameMarker[FrameMarker["Final"] = 2] = "Final";
  /** Packet is the only one in a frame. */
  FrameMarker[FrameMarker["Single"] = 3] = "Single";
})(FrameMarker || (FrameMarker = {}));
/** A class for serializing / deserializing data track packets. */
class DataTrackPacket extends Serializable {
  constructor(header, payload) {
    super();
    this.header = header;
    this.payload = payload;
  }
  toBinaryLengthBytes() {
    return this.header.toBinaryLengthBytes() + this.payload.byteLength;
  }
  toBinaryInto(dataView) {
    let byteIndex = 0;
    const headerLengthBytes = this.header.toBinaryInto(dataView);
    byteIndex += headerLengthBytes;
    if (dataView.byteLength - byteIndex < this.payload.byteLength) {
      throw DataTrackSerializeError.tooSmallForPayload();
    }
    for (let index = 0; index < this.payload.length; index += 1) {
      dataView.setUint8(byteIndex, this.payload[index]);
      byteIndex += U8_LENGTH_BYTES;
    }
    const totalLengthBytes = this.toBinaryLengthBytes();
    if (byteIndex !== totalLengthBytes) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("DataTrackPacket.toBinaryInto: Wrote ".concat(byteIndex, " bytes but expected length was ").concat(totalLengthBytes, " bytes"));
    }
    return totalLengthBytes;
  }
  static fromBinary(input) {
    const dataView = coerceToDataView(input);
    const _DataTrackPacketHeade = DataTrackPacketHeader.fromBinary(dataView),
      _DataTrackPacketHeade2 = _slicedToArray(_DataTrackPacketHeade, 2),
      header = _DataTrackPacketHeade2[0],
      headerByteLength = _DataTrackPacketHeade2[1];
    const payload = dataView.buffer.slice(dataView.byteOffset + headerByteLength, dataView.byteOffset + dataView.byteLength);
    return [new DataTrackPacket(header, new Uint8Array(payload)), dataView.byteLength];
  }
  toJSON() {
    return {
      header: this.header.toJSON(),
      payload: this.payload
    };
  }
}const log$3 = getLogger(LoggerNames.DataTracks);
/** An error indicating a frame was dropped. */
class DataTrackDepacketizerDropError extends LivekitReasonedError {
  constructor(message, reason, frameNumber, options) {
    super(19, "Frame ".concat(frameNumber, " dropped: ").concat(message), options);
    this.name = 'DataTrackDepacketizerDropError';
    this.reason = reason;
    this.reasonName = DataTrackDepacketizerDropReason[reason];
    this.frameNumber = frameNumber;
  }
  static interrupted(frameNumber, newFrameNumber) {
    return new DataTrackDepacketizerDropError("Interrupted by the start of a new frame ".concat(newFrameNumber), DataTrackDepacketizerDropReason.Interrupted, frameNumber);
  }
  static unknownFrame(frameNumber) {
    return new DataTrackDepacketizerDropError('Initial packet was never received.', DataTrackDepacketizerDropReason.UnknownFrame, frameNumber);
  }
  static bufferFull(frameNumber) {
    return new DataTrackDepacketizerDropError('Reorder buffer is full.', DataTrackDepacketizerDropReason.BufferFull, frameNumber);
  }
  static incomplete(frameNumber, receivedPackets, expectedPackets) {
    return new DataTrackDepacketizerDropError("Not all packets received before final packet. Received ".concat(receivedPackets, " packets, expected ").concat(expectedPackets, " packets."), DataTrackDepacketizerDropReason.Incomplete, frameNumber);
  }
}
/** Reason why a frame was dropped. */
var DataTrackDepacketizerDropReason;
(function (DataTrackDepacketizerDropReason) {
  DataTrackDepacketizerDropReason[DataTrackDepacketizerDropReason["Interrupted"] = 0] = "Interrupted";
  DataTrackDepacketizerDropReason[DataTrackDepacketizerDropReason["UnknownFrame"] = 1] = "UnknownFrame";
  DataTrackDepacketizerDropReason[DataTrackDepacketizerDropReason["BufferFull"] = 2] = "BufferFull";
  DataTrackDepacketizerDropReason[DataTrackDepacketizerDropReason["Incomplete"] = 3] = "Incomplete";
})(DataTrackDepacketizerDropReason || (DataTrackDepacketizerDropReason = {}));
class DataTrackDepacketizer {
  constructor() {
    /** Partial frames currently being assembled, keyed by frame number. `Map` preserves insertion
     * order, so the oldest entry is the first key. */
    this.partials = new Map();
  }
  /** Should be repeatedly called with received {@link DataTrackPacket}s - intermediate calls
   * aggregate the packet's state internally, and return null.
   *
   * Once this method is called with the final packet to form a frame, a new {@link DataTrackFrameInternal}
   * is returned.*/
  push(packet, options) {
    switch (packet.header.marker) {
      case FrameMarker.Single:
        return this.frameFromSingle(packet, options);
      case FrameMarker.Start:
        return this.beginPartial(packet, options);
      case FrameMarker.Inter:
      case FrameMarker.Final:
        return this.pushToPartial(packet);
    }
  }
  reset() {
    this.partials.clear();
  }
  peekOldestPartialFrameNumber() {
    const first = this.partials.keys().next();
    return first.done ? null : first.value;
  }
  frameFromSingle(packet, options) {
    var _a;
    if (packet.header.marker !== FrameMarker.Single) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("Depacketizer.frameFromSingle: packet.header.marker was not FrameMarker.Single, found ".concat(packet.header.marker, "."));
    }
    // A `Single` packet is a self-contained frame and doesn't reserve a partials slot, but if
    // the partials map is at capacity, treat it as a signal that the oldest in-flight partial
    // is stale and evict it (matches `main`'s behavior when `maxPartialFrames`
    // defaults to 1).
    const maxPartialFrames = (_a = options === null || options === void 0 ? void 0 : options.maxPartialFrames) !== null && _a !== void 0 ? _a : 1;
    if (this.partials.size >= maxPartialFrames) {
      const oldestPartialFrameNumber = this.peekOldestPartialFrameNumber();
      if (typeof oldestPartialFrameNumber !== 'number') {
        // @throws-transformer ignore - this should be treated as a "panic" and not be caught
        throw new Error("Depacketizer.frameFromSingle: no oldest frame number found, but partials.size is ".concat(this.partials.size, "."));
      }
      this.partials.delete(oldestPartialFrameNumber);
      if (options === null || options === void 0 ? void 0 : options.throwOnInterruption) {
        throw DataTrackDepacketizerDropError.interrupted(oldestPartialFrameNumber, packet.header.frameNumber.value);
      }
      log$3.warn("Data track frame ".concat(oldestPartialFrameNumber, " was interrupted by single-packet frame ").concat(packet.header.frameNumber.value, ", dropping."));
    }
    return {
      payload: packet.payload,
      extensions: packet.header.extensions
    };
  }
  /** Begin assembling a new packet. */
  beginPartial(packet, options) {
    var _a;
    if (packet.header.marker !== FrameMarker.Start) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("Depacketizer.beginPartial: packet.header.marker was not FrameMarker.Start, found ".concat(packet.header.marker, "."));
    }
    const startSequence = packet.header.sequence;
    const frameNumber = packet.header.frameNumber.value;
    const partial = {
      startSequence,
      extensions: packet.header.extensions,
      payloads: new Map([[startSequence.value, packet.payload]])
    };
    // Loop in case `maxPartialFrames` shrunk relative to a previous push call - evict the
    // oldest partials until there is room for the new one. With `throwOnInterruption` set the
    // throw inside the loop short-circuits on the first eviction, matching the single-eviction
    // behavior callers expect when they ask to be told about interruptions.
    const maxPartialFrames = (_a = options === null || options === void 0 ? void 0 : options.maxPartialFrames) !== null && _a !== void 0 ? _a : 1;
    while (this.partials.size >= maxPartialFrames) {
      const oldestPartialFrameNumber = this.peekOldestPartialFrameNumber();
      if (typeof oldestPartialFrameNumber !== 'number') {
        // partials map is empty - nothing more to evict
        break;
      }
      this.partials.delete(oldestPartialFrameNumber);
      if (options === null || options === void 0 ? void 0 : options.throwOnInterruption) {
        throw DataTrackDepacketizerDropError.interrupted(oldestPartialFrameNumber, frameNumber);
      }
      log$3.warn("Data track partials full (max ".concat(maxPartialFrames, "), evicted oldest frame ").concat(oldestPartialFrameNumber, " to make room for new frame ").concat(frameNumber, "."));
    }
    this.partials.set(frameNumber, partial);
    return null;
  }
  /** Push to the existing partial frame. */
  pushToPartial(packet) {
    if (packet.header.marker !== FrameMarker.Inter && packet.header.marker !== FrameMarker.Final) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error("Depacketizer.pushToPartial: packet.header.marker was not FrameMarker.Inter or FrameMarker.Final, found ".concat(packet.header.marker, "."));
    }
    const packetFrameNumber = packet.header.frameNumber.value;
    const matchingPartial = this.partials.get(packetFrameNumber);
    if (!matchingPartial) {
      this.partials.delete(packetFrameNumber);
      throw DataTrackDepacketizerDropError.unknownFrame(packetFrameNumber);
    }
    // NOTE: this check will block reprocessing packets with duplicate sequence values if the
    // buffer is full already, which could maybe be problematic for very large frames.
    if (matchingPartial.payloads.size >= DataTrackDepacketizer.MAX_BUFFER_PACKETS) {
      this.partials.delete(packetFrameNumber);
      throw DataTrackDepacketizerDropError.bufferFull(packetFrameNumber);
    }
    // Note: receiving a packet with a duplicate `sequence` value is something that likely won't
    // happen in actual use, but even if it does (maybe a low level network retransmission?) the
    // last packet with a given sequence received should always win.
    if (matchingPartial.payloads.has(packet.header.sequence.value)) {
      log$3.warn("Data track frame ".concat(packetFrameNumber, " received duplicate packet for sequence ").concat(packet.header.sequence.value, ", so replacing with newly received packet."));
    }
    matchingPartial.payloads.set(packet.header.sequence.value, packet.payload);
    if (packet.header.marker === FrameMarker.Final) {
      return this.finalize(packetFrameNumber, matchingPartial, packet.header.sequence.value);
    }
    return null;
  }
  /** Try to reassemble the complete frame. */
  finalize(partialFrameNumber, partial, endSequence) {
    const received = partial.payloads.size;
    let payloadLengthBytes = 0;
    for (const p of partial.payloads.values()) {
      payloadLengthBytes += p.length;
    }
    const payload = new Uint8Array(payloadLengthBytes);
    let sequencePointer = partial.startSequence.clone();
    let payloadOffsetPointerBytes = 0;
    while (true) {
      const partialPayload = partial.payloads.get(sequencePointer.value);
      if (!partialPayload) {
        break;
      }
      partial.payloads.delete(sequencePointer.value);
      const payloadRemainingBytes = payload.length - payloadOffsetPointerBytes;
      if (partialPayload.length > payloadRemainingBytes) {
        // @throws-transformer ignore - this should be treated as a "panic" and not be caught
        throw new Error("Depacketizer.finalize: Expected at least ".concat(partialPayload.length, " more bytes left in the payload buffer, only got ").concat(payloadRemainingBytes, " bytes."));
      }
      payload.set(partialPayload, payloadOffsetPointerBytes);
      payloadOffsetPointerBytes += partialPayload.length;
      // NOTE: sequencePointer could wrap around, which is why this isn't a "<"
      if (sequencePointer.value != endSequence) {
        sequencePointer.increment();
        continue;
      }
      // The packet is done processing, reset the state so another frame can be processed next.
      this.partials.delete(partialFrameNumber);
      return {
        payload,
        extensions: partial.extensions
      };
    }
    this.partials.delete(partialFrameNumber);
    throw DataTrackDepacketizerDropError.incomplete(partialFrameNumber, received, endSequence - partial.startSequence.value + 1);
  }
}
/** Maximum number of packets to buffer per frame before dropping. */
DataTrackDepacketizer.MAX_BUFFER_PACKETS = 128;var DataTrackSubscribeErrorReason;
(function (DataTrackSubscribeErrorReason) {
  /** The track has been unpublished and is no longer available */
  DataTrackSubscribeErrorReason[DataTrackSubscribeErrorReason["Unpublished"] = 0] = "Unpublished";
  /** Request to subscribe to data track timed-out */
  DataTrackSubscribeErrorReason[DataTrackSubscribeErrorReason["Timeout"] = 1] = "Timeout";
  /** Cannot subscribe to data track when disconnected */
  DataTrackSubscribeErrorReason[DataTrackSubscribeErrorReason["Disconnected"] = 2] = "Disconnected";
  /** Subscription to data track cancelled by caller */
  DataTrackSubscribeErrorReason[DataTrackSubscribeErrorReason["Cancelled"] = 4] = "Cancelled";
})(DataTrackSubscribeErrorReason || (DataTrackSubscribeErrorReason = {}));
class DataTrackSubscribeError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(22, message, options);
    this.name = 'DataTrackSubscribeError';
    this.reason = reason;
    this.reasonName = DataTrackSubscribeErrorReason[reason];
  }
  static unpublished() {
    return new DataTrackSubscribeError('The track has been unpublished and is no longer available', DataTrackSubscribeErrorReason.Unpublished);
  }
  static timeout() {
    return new DataTrackSubscribeError('Request to subscribe to data track timed-out', DataTrackSubscribeErrorReason.Timeout);
  }
  static disconnected() {
    return new DataTrackSubscribeError('Cannot subscribe to data track when disconnected', DataTrackSubscribeErrorReason.Disconnected);
  }
  // NOTE: this was introduced by web / there isn't a corresponding case in the rust version.
  static cancelled() {
    return new DataTrackSubscribeError('Subscription to data track cancelled by caller', DataTrackSubscribeErrorReason.Cancelled);
  }
}const log$2 = getLogger(LoggerNames.DataTracks);
/**
 * Pipeline for an individual data track subscription.
 */
class IncomingDataTrackPipeline {
  /**
   * Creates a new pipeline with the given options.
   */
  constructor(options) {
    var _a, _b;
    const hasProvider = options.e2eeManager !== null;
    if (options.info.usesE2ee !== hasProvider) {
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error('IncomingDataTrackPipeline: DataTrackInfo.usesE2ee must match presence of decryptionProvider');
    }
    const depacketizer = new DataTrackDepacketizer();
    this.publisherIdentity = options.publisherIdentity;
    this.e2eeManager = (_a = options.e2eeManager) !== null && _a !== void 0 ? _a : null;
    this.depacketizer = depacketizer;
    this.options = (_b = options.pipelineOptions) !== null && _b !== void 0 ? _b : {};
  }
  updateE2eeManager(e2eeManager) {
    this.e2eeManager = e2eeManager;
  }
  setOptions(options) {
    this.options = options;
  }
  processPacket(packet) {
    return __awaiter(this, void 0, void 0, function* () {
      const frame = this.depacketize(packet);
      if (!frame) {
        return null;
      }
      const decrypted = yield this.decryptIfNeeded(frame);
      if (!decrypted) {
        return null;
      }
      return decrypted;
    });
  }
  /**
   * Depacketize the given frame, log if a drop occurs.
   */
  depacketize(packet) {
    let frame;
    try {
      frame = this.depacketizer.push(packet, {
        throwOnInterruption: false,
        maxPartialFrames: this.options.maxPartialFrames
      });
    } catch (err) {
      // In a future version, use this to maintain drop statistics.
      // FIXME: is this a good idea?
      log$2.warn("Data frame depacketize error: ".concat(err));
      return null;
    }
    return frame;
  }
  /**
   * Decrypt the frame's payload if E2EE is enabled for this track.
   */
  decryptIfNeeded(frame) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      const e2eeManager = this.e2eeManager;
      if (!e2eeManager) {
        return frame;
      }
      const e2ee = (_b = (_a = frame.extensions) === null || _a === void 0 ? void 0 : _a.e2ee) !== null && _b !== void 0 ? _b : null;
      if (!e2ee) {
        log$2.error('Missing E2EE meta');
        return null;
      }
      let result;
      try {
        result = yield e2eeManager.handleEncryptedData(frame.payload, e2ee.iv, this.publisherIdentity, e2ee.keyIndex);
      } catch (err) {
        log$2.error("Error decrypting packet: ".concat(err));
        return null;
      }
      frame.payload = result.payload;
      return frame;
    });
  }
}const log$1 = getLogger(LoggerNames.DataTracks);
/** How long to wait when attempting to subscribe before timing out. */
const SUBSCRIBE_TIMEOUT_MILLISECONDS = 10000;
/** Maximum number of {@link DataTrackFrame}s that are cached for each ReadableStream subscription.
 * If data comes in too fast and saturates this threshold, backpressure will be applied. */
const READABLE_STREAM_DEFAULT_BUFFER_SIZE = 16;
class IncomingDataTrackManager extends eventsExports.EventEmitter {
  constructor(options) {
    var _a;
    super();
    /** Mapping between track SID and descriptor. */
    this.descriptors = new Map();
    /** Mapping between subscriber handle and track SID.
     *
     * This is an index that allows track descriptors to be looked up
     * by subscriber handle in O(1) time, to make routing incoming packets
     * (a hot code path) faster.
     */
    this.subscriptionHandles = new Map();
    this.e2eeManager = (_a = options === null || options === void 0 ? void 0 : options.e2eeManager) !== null && _a !== void 0 ? _a : null;
  }
  /** @internal */
  updateE2eeManager(e2eeManager) {
    this.e2eeManager = e2eeManager;
    // Propegate downwards to all pre-existing pipelines
    for (const descriptor of this.descriptors.values()) {
      if (descriptor.subscription.type === 'active') {
        descriptor.subscription.pipeline.updateE2eeManager(e2eeManager);
      }
    }
  }
  /** @internal */
  setPipelineOptions(sid, options) {
    const descriptor = this.descriptors.get(sid);
    if (!descriptor) {
      log$1.warn("Unknown track ".concat(sid, ", cannot set pipeline options."));
      return;
    }
    descriptor.pipelineOptions = options;
    if (descriptor.subscription.type === 'active') {
      descriptor.subscription.pipeline.setOptions(options);
    }
  }
  /** Allocates a ReadableStream which emits when a new {@link DataTrackFrame} is received from the
   * SFU. The SFU subscription is initiated lazily when the stream is created.
   *
   * @returns A tuple of the ReadableStream and a Promise that resolves once the SFU subscription
   * is fully established / the stream is ready to receive frames.
   *
   * @internal
   **/
  openSubscriptionStream(sid, signal) {
    let bufferSize = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : READABLE_STREAM_DEFAULT_BUFFER_SIZE;
    let streamController = null;
    const sfuSubscriptionComplete = new Future();
    // Hold the descriptor by reference: SID reassignment re-keys the map, so lookups
    // with the SID captured at subscribe time would fail.
    const descriptor = this.descriptors.get(sid);
    const detachSignal = () => {
      signal === null || signal === void 0 ? void 0 : signal.removeEventListener('abort', onAbort);
    };
    const cleanup = () => {
      detachSignal();
      if (!streamController) {
        log$1.warn("ReadableStream subscribed to ".concat(sid, " was not started."));
        return;
      }
      if (!descriptor || this.descriptors.get(descriptor.info.sid) !== descriptor) {
        log$1.warn("Unknown track ".concat(sid, ", skipping cancel..."));
        return;
      }
      if (descriptor.subscription.type !== 'active') {
        log$1.warn("Subscription for track ".concat(sid, " is not active, skipping cancel..."));
        return;
      }
      descriptor.subscription.streamControllers.delete(streamController);
      // If no active stream controllers are left, also unsubscribe on the SFU end.
      if (descriptor.subscription.streamControllers.size === 0) {
        this.unSubscribeRequest(descriptor.info.sid);
      }
    };
    const onAbort = () => {
      var _a;
      if (!streamController) {
        return;
      }
      if ((descriptor === null || descriptor === void 0 ? void 0 : descriptor.subscription.type) === 'active') {
        descriptor.subscription.streamControllers.delete(streamController);
      }
      streamController.error(DataTrackSubscribeError.cancelled());
      (_a = sfuSubscriptionComplete.reject) === null || _a === void 0 ? void 0 : _a.call(sfuSubscriptionComplete, DataTrackSubscribeError.cancelled());
      cleanup();
    };
    const stream = new ReadableStream({
      start: controller => {
        streamController = controller;
        this.subscribeRequest(sid, signal).then(() => __awaiter(this, void 0, void 0, function* () {
          var _a, _b, _c;
          if (!descriptor || this.descriptors.get(descriptor.info.sid) !== descriptor) {
            log$1.error("Unknown track ".concat(sid));
            const err = DataTrackSubscribeError.disconnected();
            controller.error(err);
            (_a = sfuSubscriptionComplete.reject) === null || _a === void 0 ? void 0 : _a.call(sfuSubscriptionComplete, err);
            return;
          }
          if (descriptor.subscription.type !== 'active') {
            log$1.error("Subscription for track ".concat(sid, " is not active"));
            const err = DataTrackSubscribeError.disconnected();
            controller.error(err);
            (_b = sfuSubscriptionComplete.reject) === null || _b === void 0 ? void 0 : _b.call(sfuSubscriptionComplete, err);
            return;
          }
          // Attach the abort signal, aborting immediately if the abort signal was fired while
          // subscribeRequest was in flight.
          if (signal === null || signal === void 0 ? void 0 : signal.aborted) {
            onAbort();
            return;
          }
          signal === null || signal === void 0 ? void 0 : signal.addEventListener('abort', onAbort);
          descriptor.subscription.streamControllers.set(controller, detachSignal);
          (_c = sfuSubscriptionComplete.resolve) === null || _c === void 0 ? void 0 : _c.call(sfuSubscriptionComplete);
        })).catch(err => {
          var _a;
          // subscribeRequest rejected (cancelled, timed out, disconnected). The signal
          // listener was never attached in this path, so nothing to detach.
          controller.error(err);
          (_a = sfuSubscriptionComplete.reject) === null || _a === void 0 ? void 0 : _a.call(sfuSubscriptionComplete, err);
        });
      },
      cancel: () => {
        cleanup();
      }
    }, new CountQueuingStrategy({
      highWaterMark: bufferSize
    }));
    return [stream, sfuSubscriptionComplete.promise];
  }
  /** Client requested to subscribe to a data track.
   *
   * This is sent when the user calls {@link RemoteDataTrack.subscribe}.
   *
   * Only the first request to subscribe to a given track incurs meaningful overhead; subsequent
   * requests simply attach an additional receiver to the broadcast channel, allowing them to consume
   * frames from the existing subscription pipeline.
   */
  subscribeRequest(sid, signal) {
    return __awaiter(this, void 0, void 0, function* () {
      const descriptor = this.descriptors.get(sid);
      if (!descriptor) {
        // @throws-transformer ignore - this should be treated as a "panic" and not be caught
        throw new Error('Cannot subscribe to unknown track');
      }
      const waitForCompletionFuture = (currentDescriptor, userProvidedSignal, timeoutSignal) => __awaiter(this, void 0, void 0, function* () {
        if (currentDescriptor.subscription.type === 'active') {
          // Subscription has already become active! So bail out early, there is nothing to wait for.
          return;
        }
        if (currentDescriptor.subscription.type !== 'pending') {
          // @throws-transformer ignore - this should be treated as a "panic" and not be caught
          throw new Error("Descriptor for track ".concat(sid, " is not pending, found ").concat(currentDescriptor.subscription.type));
        }
        const combinedSignal = abortSignalAny([userProvidedSignal, timeoutSignal].filter(s => typeof s !== 'undefined'));
        const proxiedCompletionFuture = new Future();
        currentDescriptor.subscription.completionFuture.promise.then(() => {
          var _a;
          return (_a = proxiedCompletionFuture.resolve) === null || _a === void 0 ? void 0 : _a.call(proxiedCompletionFuture);
        }).catch(err => {
          var _a;
          return (_a = proxiedCompletionFuture.reject) === null || _a === void 0 ? void 0 : _a.call(proxiedCompletionFuture, err);
        });
        const onAbort = () => {
          var _a;
          if (currentDescriptor.subscription.type !== 'pending') {
            return;
          }
          currentDescriptor.subscription.pendingRequestCount -= 1;
          if (timeoutSignal === null || timeoutSignal === void 0 ? void 0 : timeoutSignal.aborted) {
            // A timeout should apply to the underlying SFU subscription and cancel all user
            // subscriptions.
            currentDescriptor.subscription.cancel();
            return;
          }
          if (currentDescriptor.subscription.pendingRequestCount <= 0) {
            // No user subscriptions are still pending, so cancel the underlying pending `sfuUpdateSubscription`
            currentDescriptor.subscription.cancel();
            return;
          }
          // Other subscriptions are still pending for this data track, so just cancel this one
          // active user subscription, and leave the rest of the user subscriptions alone.
          (_a = proxiedCompletionFuture.reject) === null || _a === void 0 ? void 0 : _a.call(proxiedCompletionFuture, DataTrackSubscribeError.cancelled());
        };
        if (combinedSignal.aborted) {
          onAbort();
        }
        combinedSignal.addEventListener('abort', onAbort);
        yield proxiedCompletionFuture.promise;
        combinedSignal.removeEventListener('abort', onAbort);
      });
      switch (descriptor.subscription.type) {
        case 'none':
          {
            descriptor.subscription = {
              type: 'pending',
              completionFuture: new Future(),
              pendingRequestCount: 1,
              cancel: () => {
                var _a, _b;
                const previousDescriptorSubscription = descriptor.subscription;
                descriptor.subscription = {
                  type: 'none'
                };
                // Let the SFU know that the subscribe has been cancelled
                this.emit('sfuUpdateSubscription', {
                  sid: descriptor.info.sid,
                  subscribe: false
                });
                if (previousDescriptorSubscription.type === 'pending') {
                  (_b = (_a = previousDescriptorSubscription.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, timeoutSignal.aborted ? DataTrackSubscribeError.timeout() :
                  // NOTE: the below cancelled case was introduced by web / there isn't a corresponding case in the rust version.
                  DataTrackSubscribeError.cancelled());
                }
              }
            };
            this.emit('sfuUpdateSubscription', {
              sid,
              subscribe: true
            });
            const timeoutSignal = abortSignalTimeout(SUBSCRIBE_TIMEOUT_MILLISECONDS);
            // Wait for the subscription to complete, or time out if it takes too long
            yield waitForCompletionFuture(descriptor, signal, timeoutSignal);
            return;
          }
        case 'pending':
          {
            descriptor.subscription.pendingRequestCount += 1;
            // Wait for the subscription to complete
            yield waitForCompletionFuture(descriptor, signal);
            return;
          }
        case 'active':
          {
            return;
          }
      }
    });
  }
  /**
   * Get information about all currently subscribed tracks.
   * @internal */
  querySubscribed() {
    return __awaiter(this, void 0, void 0, function* () {
      const descriptorInfos = Array.from(this.descriptors.values()).filter(descriptor => descriptor.subscription.type === 'active').map(descriptor => [descriptor.info, descriptor.publisherIdentity]);
      return descriptorInfos;
    });
  }
  /** Client requested to unsubscribe from a data track. */
  unSubscribeRequest(sid) {
    var _a;
    const descriptor = this.descriptors.get(sid);
    if (!descriptor) {
      // FIXME: rust implementation returns here, not throws
      // @throws-transformer ignore - this should be treated as a "panic" and not be caught
      throw new Error('Cannot subscribe to unknown track');
    }
    if (descriptor.subscription.type !== 'active') {
      log$1.warn("Unexpected descriptor state in unSubscribeRequest, expected active, found ".concat((_a = descriptor.subscription) === null || _a === void 0 ? void 0 : _a.type));
      return;
    }
    this.closeStreamControllers(descriptor.subscription.streamControllers, sid);
    // FIXME: this might be wrong? Shouldn't this only occur if it is the last subscription to
    // terminate?
    const previousDescriptorSubscription = descriptor.subscription;
    descriptor.subscription = {
      type: 'none'
    };
    this.subscriptionHandles.delete(previousDescriptorSubscription.subcriptionHandle);
    this.emit('sfuUpdateSubscription', {
      sid,
      subscribe: false
    });
  }
  /** Detach abort-signal listeners and close all downstream stream controllers for an active
   * subscription. Used when the subscription is being torn down by the manager (unsubscribe,
   * unpublish, or shutdown). */
  closeStreamControllers(streamControllers, sid) {
    for (const _ref of streamControllers) {
      var _ref2 = _slicedToArray(_ref, 2);
      const controller = _ref2[0];
      const detachSignal = _ref2[1];
      // Detach before close so we don't leak a listener on the user's AbortSignal.
      detachSignal();
      try {
        controller.close();
      } catch (err) {
        // Defensive: if the controller has already been errored (e.g. by a racing abort whose
        // listener removed itself before we got here), close() throws. There's nothing
        // meaningful to do other than log — the stream is already terminal.
        log$1.warn("Failed to close readable stream for track ".concat(sid, ": ").concat(err));
      }
    }
  }
  /** SFU notification that track publications have changed.
   *
   * This event is produced from both {@link JoinResponse} and {@link ParticipantUpdate}
   * to provide a complete view of remote participants' track publications:
   *
   * - From a `JoinResponse`, it captures the initial set of tracks published when a participant joins.
   * - From a `ParticipantUpdate`, it captures subsequent changes (i.e., new tracks being
   *   published and existing tracks unpublished).
   */
  receiveSfuPublicationUpdates(updates) {
    return __awaiter(this, void 0, void 0, function* () {
      if (updates.size === 0) {
        return;
      }
      // Detect published track
      const publisherParticipantToSidsInUpdate = new Map();
      for (const _ref3 of updates.entries()) {
        var _ref4 = _slicedToArray(_ref3, 2);
        const publisherIdentity = _ref4[0];
        const infos = _ref4[1];
        const sidsInUpdate = new Set();
        for (const info of infos) {
          sidsInUpdate.add(info.sid);
          if (this.descriptors.has(info.sid)) {
            continue;
          }
          if (this.handleSidReassigned(publisherIdentity, info)) {
            continue;
          }
          yield this.handleTrackPublished(publisherIdentity, info);
        }
        publisherParticipantToSidsInUpdate.set(publisherIdentity, sidsInUpdate);
      }
      // Detect unpublished tracks
      for (const _ref5 of publisherParticipantToSidsInUpdate.entries()) {
        var _ref6 = _slicedToArray(_ref5, 2);
        const publisherIdentity = _ref6[0];
        const sidsInUpdate = _ref6[1];
        const descriptorsForPublisher = Array.from(this.descriptors.entries()).filter(_ref7 => {
          let _ref8 = _slicedToArray(_ref7, 2);
            _ref8[0];
            let descriptor = _ref8[1];
          return descriptor.publisherIdentity === publisherIdentity;
        }).map(_ref9 => {
          let _ref0 = _slicedToArray(_ref9, 1),
            sid = _ref0[0];
          return sid;
        });
        let unpublishedSids = descriptorsForPublisher.filter(sid => !sidsInUpdate.has(sid));
        for (const sid of unpublishedSids) {
          this.handleTrackUnpublished(sid);
        }
      }
    });
  }
  /**
   * Get information about all currently remotely published tracks which could be subscribed to.
   * @internal */
  queryPublications() {
    return __awaiter(this, void 0, void 0, function* () {
      return Array.from(this.descriptors.values()).map(descriptor => descriptor.info);
    });
  }
  handleTrackPublished(publisherIdentity, info) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.descriptors.has(info.sid)) {
        log$1.error("Existing descriptor for track ".concat(info.sid));
        return;
      }
      let descriptor = {
        info,
        publisherIdentity,
        subscription: {
          type: 'none'
        },
        pipelineOptions: {}
      };
      this.descriptors.set(descriptor.info.sid, descriptor);
      const track = new RemoteDataTrack(descriptor.info, this, {
        publisherIdentity
      });
      this.emit('trackPublished', {
        track
      });
    });
  }
  /**
   * Detects and handles SID reassignment, which occurs when the publisher
   * republishes its tracks after a full reconnect.
   *
   * Returns `true` if an SID reassignment occurred, `false` otherwise.
   */
  handleSidReassigned(publisherIdentity, info) {
    // Publisher identity and pub handle are stable across republications.
    const existingEntry = Array.from(this.descriptors.entries()).find(_ref1 => {
      let _ref10 = _slicedToArray(_ref1, 2);
        _ref10[0];
        let descriptor = _ref10[1];
      return descriptor.publisherIdentity === publisherIdentity && descriptor.info.pubHandle === info.pubHandle;
    });
    if (!existingEntry) {
      return false;
    }
    const _existingEntry = _slicedToArray(existingEntry, 2),
      oldSid = _existingEntry[0],
      descriptor = _existingEntry[1];
    // Invariant: other than SID, info should not have changed.
    // TODO: consider refactoring to move SID out of info to allow for direct comparison.
    const _descriptor$info = descriptor.info,
      name = _descriptor$info.name,
      usesE2ee = _descriptor$info.usesE2ee;
    if (name !== info.name || usesE2ee !== info.usesE2ee) {
      log$1.warn("Info mismatch for ".concat(oldSid, ", treating as new publication"));
      return false;
    }
    const newSid = info.sid;
    log$1.debug("SID reassigned: ".concat(oldSid, " -> ").concat(newSid));
    if (!this.descriptors.delete(oldSid)) {
      return false;
    }
    descriptor.info.sid = newSid;
    switch (descriptor.subscription.type) {
      case 'none':
        break;
      case 'pending':
      case 'active':
        // The SFU does not carry subscriptions across a publisher's full
        // reconnect; re-request the subscription under the new SID.
        this.emit('sfuUpdateSubscription', {
          sid: newSid,
          subscribe: true
        });
        break;
    }
    if (descriptor.subscription.type === 'active') {
      // Keep the routing index consistent until the SFU assigns a new handle
      // (see `registerSubscriberHandle`).
      this.subscriptionHandles.set(descriptor.subscription.subcriptionHandle, newSid);
    }
    this.descriptors.set(newSid, descriptor);
    return true;
  }
  handleTrackUnpublished(sid) {
    const descriptor = this.descriptors.get(sid);
    if (!descriptor) {
      log$1.error("Unknown track ".concat(sid));
      return;
    }
    this.descriptors.delete(sid);
    if (descriptor.subscription.type === 'active') {
      this.closeStreamControllers(descriptor.subscription.streamControllers, sid);
      this.subscriptionHandles.delete(descriptor.subscription.subcriptionHandle);
    }
    this.emit('trackUnpublished', {
      sid,
      publisherIdentity: descriptor.publisherIdentity
    });
  }
  /** SFU notification that handles have been assigned for requested subscriptions. */
  receivedSfuSubscriberHandles(
  /** Mapping between track handles attached to incoming packets to the
   * track SIDs they belong to. */
  mapping) {
    for (const _ref11 of mapping.entries()) {
      var _ref12 = _slicedToArray(_ref11, 2);
      const handle = _ref12[0];
      const sid = _ref12[1];
      this.registerSubscriberHandle(handle, sid);
    }
  }
  registerSubscriberHandle(assignedHandle, sid) {
    var _a, _b;
    const descriptor = this.descriptors.get(sid);
    if (!descriptor) {
      log$1.error("Unknown track ".concat(sid));
      return;
    }
    switch (descriptor.subscription.type) {
      case 'none':
        {
          // Handle assigned when there is no pending or active subscription is unexpected.
          log$1.warn("No subscription for ".concat(sid));
          return;
        }
      case 'active':
        {
          // Update handle for an active subscription. This can occur following a full reconnect.
          this.subscriptionHandles.delete(descriptor.subscription.subcriptionHandle);
          descriptor.subscription.subcriptionHandle = assignedHandle;
          this.subscriptionHandles.set(assignedHandle, sid);
          return;
        }
      case 'pending':
        {
          log$1.debug("data track subscription activated", {
            sid,
            handle: assignedHandle
          });
          const pipeline = new IncomingDataTrackPipeline({
            info: descriptor.info,
            publisherIdentity: descriptor.publisherIdentity,
            e2eeManager: this.e2eeManager,
            pipelineOptions: descriptor.pipelineOptions
          });
          const previousDescriptorSubscription = descriptor.subscription;
          descriptor.subscription = {
            type: 'active',
            subcriptionHandle: assignedHandle,
            pipeline,
            streamControllers: new Map()
          };
          this.subscriptionHandles.set(assignedHandle, sid);
          (_b = (_a = previousDescriptorSubscription.completionFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
        }
    }
  }
  /** Packet has been received over the transport. */
  packetReceived(bytes) {
    return __awaiter(this, void 0, void 0, function* () {
      let packet;
      try {
        var _DataTrackPacket$from = DataTrackPacket.fromBinary(bytes);
        var _DataTrackPacket$from2 = _slicedToArray(_DataTrackPacket$from, 1);
        packet = _DataTrackPacket$from2[0];
      } catch (err) {
        log$1.error("Failed to deserialize packet: ".concat(err));
        return;
      }
      const sid = this.subscriptionHandles.get(packet.header.trackHandle);
      if (!sid) {
        log$1.warn("Unknown subscriber handle ".concat(packet.header.trackHandle));
        return;
      }
      const descriptor = this.descriptors.get(sid);
      if (!descriptor) {
        log$1.error("Missing descriptor for track ".concat(sid));
        return;
      }
      if (descriptor.subscription.type !== 'active') {
        log$1.warn("Received packet for track ".concat(sid, " without active subscription"));
        return;
      }
      const internalFrame = yield descriptor.subscription.pipeline.processPacket(packet);
      if (!internalFrame) {
        // Not all packets have been received yet to form a complete frame
        return;
      }
      // Broadcast to all downstream subscribers
      for (const controller of descriptor.subscription.streamControllers.keys()) {
        if (controller.desiredSize !== null && controller.desiredSize <= 0) {
          log$1.warn("Cannot send frame to subscribers: readable stream is full (desiredSize is ".concat(controller.desiredSize, "). To increase this threshold, set a higher 'options.highWaterMark' when calling .subscribe()."));
          continue;
        }
        const frame = DataTrackFrameInternal.lossyIntoFrame(internalFrame);
        controller.enqueue(frame);
      }
    });
  }
  /** Resend all subscription updates.
   *
   * This must be sent after a full reconnect to ensure the SFU knows which
   * tracks are subscribed to locally.
   */
  resendSubscriptionUpdates() {
    for (const _ref13 of this.descriptors) {
      var _ref14 = _slicedToArray(_ref13, 2);
      const sid = _ref14[0];
      const descriptor = _ref14[1];
      if (descriptor.subscription.type === 'none') {
        continue;
      }
      this.emit('sfuUpdateSubscription', {
        sid,
        subscribe: true
      });
    }
  }
  /** Called when a remote participant is disconnected so that any pending data tracks can be
   * cancelled. */
  handleRemoteParticipantDisconnected(remoteParticipantIdentity) {
    var _a, _b;
    for (const descriptor of this.descriptors.values()) {
      if (descriptor.publisherIdentity !== remoteParticipantIdentity) {
        continue;
      }
      switch (descriptor.subscription.type) {
        case 'none':
          break;
        case 'pending':
          (_b = (_a = descriptor.subscription.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, DataTrackSubscribeError.disconnected());
          break;
        case 'active':
          this.unSubscribeRequest(descriptor.info.sid);
          break;
      }
    }
  }
  /** Resets the manager, ending any subscriptions, and getting it ready for the next room
   * connection.  */
  reset() {
    var _a, _b;
    for (const descriptor of this.descriptors.values()) {
      this.emit('trackUnpublished', {
        sid: descriptor.info.sid,
        publisherIdentity: descriptor.publisherIdentity
      });
      if (descriptor.subscription.type === 'pending') {
        (_b = (_a = descriptor.subscription.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, DataTrackSubscribeError.disconnected());
      }
      if (descriptor.subscription.type === 'active') {
        this.closeStreamControllers(descriptor.subscription.streamControllers, descriptor.info.sid);
      }
    }
    this.descriptors.clear();
    this.subscriptionHandles.clear();
  }
}
export { DataTrackDepacketizer, DataTrackDepacketizerDropError, DataTrackDepacketizerDropReason, DataTrackE2eeExtension, DataTrackExtensions, DataTrackFrameInternal, DataTrackPacket, DataTrackPacketHeader, DataTrackSubscribeError, DataTrackSubscribeErrorReason, DataTrackSymbol, FrameMarker, IncomingDataTrackManager, IncomingDataTrackPipeline, READABLE_STREAM_DEFAULT_BUFFER_SIZE, RemoteDataTrack, SUBSCRIBE_TIMEOUT_MILLISECONDS, TrackSymbol, log$1, log$2, log$3 };
