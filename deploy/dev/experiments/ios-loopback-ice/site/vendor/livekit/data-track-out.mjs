// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 30011-30733 行按职责切出，行为不变。
import { __asyncDelegator, __asyncGenerator, __asyncValues, __await, __awaiter, _slicedToArray } from './tslib.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { LivekitReasonedError } from './errors.mjs';
import { Future } from './utils-misc.mjs';
import { DataTrackClock, DataTrackTimestamp, WrapAroundUnsignedInt } from './device.mjs';
import { DataTrackHandleAllocator, abortSignalAny, abortSignalTimeout } from './data-track-core.mjs';
import { DataTrackE2eeExtension, DataTrackFrameInternal, DataTrackPacket, DataTrackPacketHeader, DataTrackSymbol, FrameMarker, TrackSymbol } from './data-track.mjs';

class DataTrackPacketizerError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(19, message, options);
    this.name = 'DataTrackPacketizerError';
    this.reason = reason;
    this.reasonName = DataTrackPacketizerReason[reason];
  }
  static mtuTooShort() {
    return new DataTrackPacketizerError('MTU is too short to send frame', DataTrackPacketizerReason.MtuTooShort);
  }
}
var DataTrackPacketizerReason;
(function (DataTrackPacketizerReason) {
  DataTrackPacketizerReason[DataTrackPacketizerReason["MtuTooShort"] = 0] = "MtuTooShort";
})(DataTrackPacketizerReason || (DataTrackPacketizerReason = {}));
/** A packetizer takes a {@link DataTrackFrameInternal} as input and generates a series
 * of {@link DataTrackPacket}s for transmission to other clients over webrtc. */
class DataTrackPacketizer {
  constructor(trackHandle, mtuSizeBytes) {
    this.sequence = WrapAroundUnsignedInt.u16(0);
    this.frameNumber = WrapAroundUnsignedInt.u16(0);
    this.clock = DataTrackClock.rtpStartingNow(DataTrackTimestamp.rtpRandom());
    this.handle = trackHandle;
    this.mtuSizeBytes = mtuSizeBytes;
  }
  /** @internal */
  static computeFrameMarker(index, packetCount) {
    if (packetCount <= 1) {
      return FrameMarker.Single;
    }
    if (index === 0) {
      return FrameMarker.Start;
    } else if (index === packetCount - 1) {
      return FrameMarker.Final;
    } else {
      return FrameMarker.Inter;
    }
  }
  /** Generates a series of packets for the specified {@link DataTrackFrameInternal}.
   *
   * NOTE: The return value of this function is a generator, so it can be lazily ran if desired,
   * or converted to an array with {@link Array.from}.
   */
  *packetize(frame, options) {
    var _a;
    const frameNumber = this.frameNumber.getThenIncrement();
    const headerParams = {
      marker: FrameMarker.Inter,
      trackHandle: this.handle,
      sequence: WrapAroundUnsignedInt.u16(0),
      frameNumber,
      timestamp: (_a = options === null || options === void 0 ? void 0 : options.now) !== null && _a !== void 0 ? _a : this.clock.now(),
      extensions: frame.extensions
    };
    const headerSerializedLengthBytes = new DataTrackPacketHeader(headerParams).toBinaryLengthBytes();
    if (headerSerializedLengthBytes >= this.mtuSizeBytes) {
      throw DataTrackPacketizerError.mtuTooShort();
    }
    const maxPayloadSizeBytes = this.mtuSizeBytes - headerSerializedLengthBytes;
    const packetCount = Math.ceil(frame.payload.byteLength / maxPayloadSizeBytes);
    for (let index = 0, indexBytes = 0; indexBytes < frame.payload.byteLength; _ref = [index + 1, indexBytes + maxPayloadSizeBytes], index = _ref[0], indexBytes = _ref[1], _ref) {
      var _ref;
      const sequence = this.sequence.getThenIncrement();
      const packetHeader = new DataTrackPacketHeader(Object.assign(Object.assign({}, headerParams), {
        marker: DataTrackPacketizer.computeFrameMarker(index, packetCount),
        sequence
      }));
      const packetPayloadLengthBytes = Math.min(
      // All but the last packet will be max length ...
      maxPayloadSizeBytes,
      // ... and the last packet will be as long as it needs to be to finish out the buffer.
      frame.payload.byteLength - indexBytes);
      const packetPayload = new Uint8Array(frame.payload.buffer, frame.payload.byteOffset + indexBytes, packetPayloadLengthBytes);
      yield new DataTrackPacket(packetHeader, packetPayload);
    }
  }
}var DataTrackPublishErrorReason;
(function (DataTrackPublishErrorReason) {
  /**
   * Local participant does not have permission to publish data tracks.
   *
   * Ensure the participant's token contains the `canPublishData` grant.
   */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["NotAllowed"] = 0] = "NotAllowed";
  /** A track with the same name is already published by the local participant. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["DuplicateName"] = 1] = "DuplicateName";
  /** Request to publish the track took long to complete. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["Timeout"] = 2] = "Timeout";
  /** No additional data tracks can be published by the local participant. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["LimitReached"] = 3] = "LimitReached";
  /** Cannot publish data track when the room is disconnected. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["Disconnected"] = 4] = "Disconnected";
  // NOTE: this was introduced by web / there isn't a corresponding case in the rust version.
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["Cancelled"] = 5] = "Cancelled";
  /** The name requested is not able to be used when creating the data track. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["InvalidName"] = 6] = "InvalidName";
  /** There was an error publishing, but it was not something that could be sorted into a known
   * category. */
  DataTrackPublishErrorReason[DataTrackPublishErrorReason["Unknown"] = 7] = "Unknown";
})(DataTrackPublishErrorReason || (DataTrackPublishErrorReason = {}));
class DataTrackPublishError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(21, message, options);
    this.name = 'DataTrackPublishError';
    this.reason = reason;
    this.reasonName = DataTrackPublishErrorReason[reason];
    this.rawMessage = options === null || options === void 0 ? void 0 : options.rawMessage;
  }
  static notAllowed(rawMessage) {
    return new DataTrackPublishError('Data track publishing unauthorized', DataTrackPublishErrorReason.NotAllowed, {
      rawMessage
    });
  }
  static duplicateName(rawMessage) {
    return new DataTrackPublishError('Track name already taken', DataTrackPublishErrorReason.DuplicateName, {
      rawMessage
    });
  }
  static invalidName(rawMessage) {
    return new DataTrackPublishError('Track name is invalid', DataTrackPublishErrorReason.InvalidName, {
      rawMessage
    });
  }
  static timeout() {
    return new DataTrackPublishError('Publish data track timed-out. Does the LiveKit server support data tracks?', DataTrackPublishErrorReason.Timeout);
  }
  static limitReached(rawMessage) {
    return new DataTrackPublishError('Data track publication limit reached', DataTrackPublishErrorReason.LimitReached, {
      rawMessage
    });
  }
  static unknown(reason, message) {
    return new DataTrackPublishError("Received RequestResponse for publishDataTrack, but reason was unrecognised (".concat(reason, ", ").concat(message, ")"), DataTrackPublishErrorReason.Unknown);
  }
  static disconnected() {
    return new DataTrackPublishError('Room disconnected', DataTrackPublishErrorReason.Disconnected);
  }
  // NOTE: this was introduced by web / there isn't a corresponding case in the rust version.
  static cancelled() {
    return new DataTrackPublishError('Publish data track cancelled by caller', DataTrackPublishErrorReason.Cancelled);
  }
}
var DataTrackPushFrameErrorReason;
(function (DataTrackPushFrameErrorReason) {
  /** Track is no longer published. */
  DataTrackPushFrameErrorReason[DataTrackPushFrameErrorReason["TrackUnpublished"] = 0] = "TrackUnpublished";
  /** Frame was dropped. */
  // NOTE: this should become a web specific error, the rust version of this "dropped" error means
  // something different and will be renamed to "QueueFull".
  DataTrackPushFrameErrorReason[DataTrackPushFrameErrorReason["Dropped"] = 1] = "Dropped";
})(DataTrackPushFrameErrorReason || (DataTrackPushFrameErrorReason = {}));
class DataTrackPushFrameError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(22, message, options);
    this.name = 'DataTrackPushFrameError';
    this.reason = reason;
    this.reasonName = DataTrackPushFrameErrorReason[reason];
  }
  static trackUnpublished() {
    return new DataTrackPushFrameError('Track is no longer published', DataTrackPushFrameErrorReason.TrackUnpublished);
  }
  static dropped(cause) {
    return new DataTrackPushFrameError('Frame was dropped', DataTrackPushFrameErrorReason.Dropped, {
      cause
    });
  }
}
var DataTrackOutgoingPipelineErrorReason;
(function (DataTrackOutgoingPipelineErrorReason) {
  DataTrackOutgoingPipelineErrorReason[DataTrackOutgoingPipelineErrorReason["Packetizer"] = 0] = "Packetizer";
  DataTrackOutgoingPipelineErrorReason[DataTrackOutgoingPipelineErrorReason["Encryption"] = 1] = "Encryption";
})(DataTrackOutgoingPipelineErrorReason || (DataTrackOutgoingPipelineErrorReason = {}));
class DataTrackOutgoingPipelineError extends LivekitReasonedError {
  constructor(message, reason, options) {
    super(21, message, options);
    this.name = 'DataTrackOutgoingPipelineError';
    this.reason = reason;
    this.reasonName = DataTrackOutgoingPipelineErrorReason[reason];
  }
  static packetizer(cause) {
    return new DataTrackOutgoingPipelineError('Error packetizing frame', DataTrackOutgoingPipelineErrorReason.Packetizer, {
      cause
    });
  }
  static encryption(cause) {
    return new DataTrackOutgoingPipelineError('Error encrypting frame', DataTrackOutgoingPipelineErrorReason.Encryption, {
      cause
    });
  }
}class LocalDataTrack {
  /** @internal */
  constructor(options, manager) {
    this.trackSymbol = TrackSymbol;
    this.isLocal = true;
    this.typeSymbol = DataTrackSymbol;
    /** Represents the currently active {@link DataTrackHandle} for the publication. */
    this.handle = null;
    this.log = livekitLogger;
    /** Resolves once the data track has sent all pending packets the rtc data channel buffer. */
    this.flushedFuture = new Future();
    this.isFlushed = true;
    this.handleManagerReset = () => {
      var _a, _b;
      // When the associated manager resets, mark any in flight flushes as complete
      // There's nothing actionable a user can do to get these to complete so no
      // error is being thrown.
      (_b = (_a = this.flushedFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
      this.manager.off('packetsFlushedChange', this.handleManagerPacketsFlushedChange);
      this.manager.off('reset', this.handleManagerReset);
    };
    this.handleManagerPacketsFlushedChange = event => {
      var _a, _b;
      this.isFlushed = event.isFlushed;
      if (event.isFlushed) {
        (_b = (_a = this.flushedFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
        this.flushedFuture = new Future();
      }
    };
    this.options = options;
    this.manager = manager;
    this.log = getLogger(LoggerNames.DataTracks);
    this.manager.on('packetsFlushedChange', this.handleManagerPacketsFlushedChange);
    this.manager.on('reset', this.handleManagerReset);
  }
  /** @internal */
  static withExplicitHandle(options, manager, handle) {
    const track = new LocalDataTrack(options, manager);
    track.handle = handle;
    return track;
  }
  /** Metrics about the data track publication. */
  get info() {
    const descriptor = this.descriptor;
    if ((descriptor === null || descriptor === void 0 ? void 0 : descriptor.type) === 'active') {
      return descriptor.info;
    } else {
      return undefined;
    }
  }
  /** The raw descriptor from the manager containing the internal state for this local track. */
  get descriptor() {
    return this.handle ? this.manager.getDescriptor(this.handle) : null;
  }
  /**
   * Publish the track to the SFU. This must be done before calling {@link tryPush} for the first time.
   * @internal
   * */
  publish(signal) {
    return __awaiter(this, void 0, void 0, function* () {
      try {
        this.handle = yield this.manager.publishRequest(this.options, signal);
      } catch (err) {
        // NOTE: Rethrow errors to break Throws<...> type boundary
        throw err;
      }
    });
  }
  isPublished() {
    var _a;
    // NOTE: a track which is internally in the "resubscribing" state is still considered
    // published from the public API perspective.
    return ((_a = this.descriptor) === null || _a === void 0 ? void 0 : _a.type) === 'active' && this.descriptor.publishState !== 'unpublished';
  }
  /** Try pushing a frame to subscribers of the track.
   *
   * Pushing a frame can fail for several reasons:
   *
   * - The track has been unpublished by the local participant or SFU
   * - The room is no longer connected
   */
  tryPush(frame) {
    if (!this.handle) {
      throw DataTrackPushFrameError.trackUnpublished();
    }
    const internalFrame = DataTrackFrameInternal.from(frame);
    try {
      return this.manager.tryProcessAndSend(this.handle, internalFrame);
    } catch (err) {
      // NOTE: wrapping in the bare try/catch like this means that the Throws<...> type doesn't
      // propagate upwards into the public interface.
      throw err;
    }
  }
  /**
   * When called, waits for all in flight packets to be sent before resolving.
   *
   * Use this to:
   *
   * 1. Send frames exactly in order:
   * ```ts
   * await track.tryPush(/* ... *\/);
   * await track.flush();
   * await track.tryPush(/* ... *\/);
   * await track.flush();
   * // ... etc ...
   * ```
   *
   * 2. Wait for frames to all be delivered before unpublishing a local data track:
   *
   * ```ts
   * await track.tryPush(/* ... *\/);
   * await track.tryPush(/* ... *\/);
   * await track.tryPush(/* ... *\/);
   * // ... etc ...
   * await track.flush();
   * await track.unpublish();
   * ```
   **/
  flush() {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.isFlushed) {
        return;
      }
      return this.flushedFuture.promise;
    });
  }
  /**
   * Unpublish the track from the SFU. Once this is called, any further calls to {@link tryPush}
   * will fail.
   * */
  unpublish() {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.handle) {
        livekitLogger.warn("Data track \"".concat(this.options.name, "\" is not published, so unpublishing has no effect."));
        return;
      }
      try {
        yield this.manager.unpublishRequest(this.handle);
      } catch (err) {
        // NOTE: Rethrow errors to break Throws<...> type boundary
        throw err;
      }
    });
  }
}/** Processes outgoing frames into final packets for distribution to the SFU. */
class DataTrackOutgoingPipeline {
  constructor(options) {
    this.e2eeManager = options.e2eeManager;
    this.packetizer = new DataTrackPacketizer(options.info.pubHandle, DataTrackOutgoingPipeline.TRANSPORT_MTU_BYTES);
  }
  updateE2eeManager(e2eeManager) {
    this.e2eeManager = e2eeManager;
  }
  processFrame(frame) {
    return __asyncGenerator(this, arguments, function* processFrame_1() {
      const encryptedFrame = yield __await(this.encryptIfNeeded(frame));
      try {
        yield __await(yield* __asyncDelegator(__asyncValues(this.packetizer.packetize(encryptedFrame))));
      } catch (error) {
        if (error instanceof DataTrackPacketizerError) {
          throw DataTrackOutgoingPipelineError.packetizer(error);
        }
        throw error;
      }
    });
  }
  encryptIfNeeded(frame) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.e2eeManager) {
        return frame;
      }
      let encryptedResult;
      try {
        encryptedResult = yield this.e2eeManager.encryptData(frame.payload);
      } catch (err) {
        throw DataTrackOutgoingPipelineError.encryption(err);
      }
      frame.payload = encryptedResult.payload;
      frame.extensions.e2ee = new DataTrackE2eeExtension(encryptedResult.keyIndex, encryptedResult.iv);
      return frame;
    });
  }
}
/** Maximum transmission unit (MTU) of the transport. */
DataTrackOutgoingPipeline.TRANSPORT_MTU_BYTES = 16000;const log = getLogger(LoggerNames.DataTracks);
const Descriptor = {
  pending() {
    return {
      type: 'pending',
      completionFuture: new Future()
    };
  },
  active(info, e2eeManager) {
    return {
      type: 'active',
      info,
      publishState: 'published',
      pipeline: new DataTrackOutgoingPipeline({
        info,
        e2eeManager
      }),
      unpublishingFuture: new Future()
    };
  }
};
/** How long to wait when attempting to publish before timing out. */
const PUBLISH_TIMEOUT_MILLISECONDS = 10000;
class OutgoingDataTrackManager extends eventsExports.EventEmitter {
  constructor(options) {
    var _a;
    super();
    this.handleAllocator = new DataTrackHandleAllocator();
    this.descriptors = new Map();
    /** Number of packets for each data track which have been emitted via the `packetAvailable` event
     * and which have not yet been sent via the rtc data channel yet. Once this goes to 0, then
     * all in flight packets have been delivered, and the data tracks is "flushed". */
    this.inFlightPacketCounter = new Map();
    this.e2eeManager = (_a = options === null || options === void 0 ? void 0 : options.e2eeManager) !== null && _a !== void 0 ? _a : null;
  }
  static withDescriptors(descriptors) {
    const manager = new OutgoingDataTrackManager();
    manager.descriptors = descriptors;
    return manager;
  }
  /** @internal */
  updateE2eeManager(e2eeManager) {
    this.e2eeManager = e2eeManager;
    // Propegate downwards to all pre-existing pipelines
    for (const descriptor of this.descriptors.values()) {
      if (descriptor.type === 'active') {
        descriptor.pipeline.updateE2eeManager(e2eeManager);
      }
    }
  }
  /**
   * Used by attached {@link LocalDataTrack} instances to query their associated descriptor info.
   * @internal
   */
  getDescriptor(handle) {
    var _a;
    return (_a = this.descriptors.get(handle)) !== null && _a !== void 0 ? _a : null;
  }
  /** Used by attached {@link LocalDataTrack} instances to broadcast data track packets to other
   * subscribers.
   * @internal
   */
  tryProcessAndSend(handle, frame) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, e_1, _b, _c;
      var _d;
      const descriptor = this.getDescriptor(handle);
      if ((descriptor === null || descriptor === void 0 ? void 0 : descriptor.type) !== 'active') {
        throw DataTrackPushFrameError.trackUnpublished();
      }
      if (descriptor.publishState === 'unpublished') {
        throw DataTrackPushFrameError.trackUnpublished();
      }
      if (descriptor.publishState === 'republishing') {
        throw DataTrackPushFrameError.dropped('Data track republishing');
      }
      try {
        try {
          for (var _e = true, _f = __asyncValues(descriptor.pipeline.processFrame(frame)), _g; _g = yield _f.next(), _a = _g.done, !_a; _e = true) {
            _c = _g.value;
            _e = false;
            const packet = _c;
            const prev = (_d = this.inFlightPacketCounter.get(handle)) !== null && _d !== void 0 ? _d : 0;
            this.inFlightPacketCounter.set(handle, prev + 1);
            if (prev === 0) {
              // A new packet has been sent, so there are now packets in the rtc data channel buffer for
              // this data track frame
              this.emit('packetsFlushedChange', {
                handle,
                isFlushed: false
              });
            }
            this.emit('packetAvailable', {
              handle,
              bytes: packet.toBinary()
            });
          }
        } catch (e_1_1) {
          e_1 = {
            error: e_1_1
          };
        } finally {
          try {
            if (!_e && !_a && (_b = _f.return)) yield _b.call(_f);
          } finally {
            if (e_1) throw e_1.error;
          }
        }
      } catch (err) {
        // NOTE: In the rust implementation this "dropped" error means something different (not enough room
        // in the track mpsc channel)
        throw DataTrackPushFrameError.dropped(err);
      }
    });
  }
  /** The client has sent a packet over the rtc data channel. This signal is used for determining
   * once all packets are sent and a data track has been "flushed".
   *
   * @internal */
  handlePacketSendComplete(handle) {
    var _a;
    const prev = (_a = this.inFlightPacketCounter.get(handle)) !== null && _a !== void 0 ? _a : 0;
    let counter = prev - 1;
    if (counter < 0) {
      log.warn("OutgoingDataTrackManager.handlePacketSendComplete: inFlightPacketCounter was decremented below 0 (got ".concat(this.inFlightPacketCounter, " - resetting to 0. Were more packets send than were emitted?"));
      counter = 0;
    }
    this.inFlightPacketCounter.set(handle, counter);
    if (counter === 0) {
      this.emit('packetsFlushedChange', {
        handle,
        isFlushed: true
      });
    }
  }
  /**
   * Client requested to publish a track.
   *
   * If the LiveKit server is too old and doesn't support data tracks, a
   * {@link DataTrackPublishError#timeout} will be thrown.
   *
   * @internal
   **/
  publishRequest(options, signal) {
    return __awaiter(this, void 0, void 0, function* () {
      const handle = this.handleAllocator.get();
      if (!handle) {
        throw DataTrackPublishError.limitReached();
      }
      const timeoutSignal = abortSignalTimeout(PUBLISH_TIMEOUT_MILLISECONDS);
      const combinedSignal = signal ? abortSignalAny([signal, timeoutSignal]) : timeoutSignal;
      if (this.descriptors.has(handle)) {
        // @throws-transformer ignore - this should be treated as a "panic" and not be caught
        throw new Error('Descriptor for handle already exists');
      }
      const descriptor = Descriptor.pending();
      this.descriptors.set(handle, descriptor);
      const onAbort = () => {
        var _a, _b;
        const existingDescriptor = this.descriptors.get(handle);
        if (!existingDescriptor) {
          log.warn("No descriptor for ".concat(handle));
          return;
        }
        this.descriptors.delete(handle);
        // Let the SFU know that the publish has been cancelled
        this.emit('sfuUnpublishRequest', {
          handle
        });
        if (existingDescriptor.type === 'pending') {
          (_b = (_a = existingDescriptor.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, timeoutSignal.aborted ? DataTrackPublishError.timeout() :
          // NOTE: the below cancelled case was introduced by web / there isn't a corresponding case in the rust version.
          DataTrackPublishError.cancelled());
        }
      };
      if (combinedSignal.aborted) {
        onAbort(); // NOTE: this rejects `completionFuture`; the next line just returns the rejection
        return descriptor.completionFuture.promise.then(() => handle /* no-op, makes typescript happy */);
      }
      combinedSignal.addEventListener('abort', onAbort);
      this.emit('sfuPublishRequest', {
        handle,
        name: options.name,
        usesE2ee: this.e2eeManager !== null
      });
      yield descriptor.completionFuture.promise;
      combinedSignal.removeEventListener('abort', onAbort);
      this.emit('trackPublished', {
        track: LocalDataTrack.withExplicitHandle(options, this, handle)
      });
      return handle;
    });
  }
  /**
   * Get information about all currently published tracks.
   * @internal
   **/
  queryPublished() {
    const descriptorInfos = Array.from(this.descriptors.values()).filter(descriptor => descriptor.type === 'active').map(descriptor => descriptor.info);
    return descriptorInfos;
  }
  /**
   * Client request to unpublish a track.
   * @internal
   **/
  unpublishRequest(handle) {
    return __awaiter(this, void 0, void 0, function* () {
      const descriptor = this.descriptors.get(handle);
      if (!descriptor) {
        log.warn("No descriptor for ".concat(handle));
        return;
      }
      if (descriptor.type !== 'active') {
        log.warn("Track ".concat(handle, " not active"));
        return;
      }
      this.emit('sfuUnpublishRequest', {
        handle
      });
      yield descriptor.unpublishingFuture.promise;
      this.inFlightPacketCounter.delete(handle);
      this.emit('trackUnpublished', {
        sid: descriptor.info.sid
      });
    });
  }
  /**
   * SFU responded to a request to publish a data track.
   * @internal
   **/
  receivedSfuPublishResponse(handle, result) {
    var _a, _b, _c, _d;
    const descriptor = this.descriptors.get(handle);
    if (!descriptor) {
      log.warn("No descriptor for ".concat(handle));
      return;
    }
    this.descriptors.delete(handle);
    switch (descriptor.type) {
      case 'pending':
        {
          if (result.type === 'ok') {
            const info = result.data;
            log.debug("SFU accepted publish request for handle ".concat(handle), {
              sid: info.sid
            });
            const e2eeManager = info.usesE2ee ? this.e2eeManager : null;
            this.descriptors.set(info.pubHandle, Descriptor.active(info, e2eeManager));
            (_b = (_a = descriptor.completionFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
          } else {
            log.debug("SFU rejected publish request for handle ".concat(handle), {
              error: result.error
            });
            (_d = (_c = descriptor.completionFuture).reject) === null || _d === void 0 ? void 0 : _d.call(_c, result.error);
          }
          return;
        }
      case 'active':
        {
          if (descriptor.publishState !== 'republishing') {
            log.warn("Track ".concat(handle, " already active"));
            return;
          }
          if (result.type === 'error') {
            log.warn("Republish failed for track ".concat(handle));
            return;
          }
          log.debug("Track ".concat(handle, " republished"));
          descriptor.info.sid = result.data.sid;
          descriptor.publishState = 'published';
          this.descriptors.set(descriptor.info.pubHandle, descriptor);
        }
    }
  }
  /**
   * SFU notification that a track has been unpublished.
   * @internal
   **/
  receivedSfuUnpublishResponse(handle) {
    var _a, _b;
    const descriptor = this.descriptors.get(handle);
    if (!descriptor) {
      log.warn("No descriptor for ".concat(handle));
      return;
    }
    this.descriptors.delete(handle);
    if (descriptor.type !== 'active') {
      log.warn("Track ".concat(handle, " not active"));
      return;
    }
    descriptor.publishState = 'unpublished';
    (_b = (_a = descriptor.unpublishingFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
  }
  /** Republish all tracks.
   *
   * This must be sent after a full reconnect in order for existing publications
   * to be recognized by the SFU. Each republished track will be assigned a new SID.
   * @internal
   */
  sfuWillRepublishTracks() {
    var _a, _b;
    for (const _ref of this.descriptors.entries()) {
      var _ref2 = _slicedToArray(_ref, 2);
      const handle = _ref2[0];
      const descriptor = _ref2[1];
      switch (descriptor.type) {
        case 'pending':
          // TODO: support republish for pending publications
          this.descriptors.delete(handle);
          (_b = (_a = descriptor.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, DataTrackPublishError.disconnected());
          break;
        case 'active':
          descriptor.publishState = 'republishing';
          this.emit('sfuPublishRequest', {
            handle: descriptor.info.pubHandle,
            name: descriptor.info.name,
            usesE2ee: descriptor.info.usesE2ee
          });
      }
    }
  }
  /**
   * Reset's the state of the manager and all associated tracks. Run on room disconnect to get
   * the manager ready for the next room connection.
   * @internal
   **/
  reset() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d;
      this.handleAllocator.reset();
      for (const descriptor of this.descriptors.values()) {
        switch (descriptor.type) {
          case 'pending':
            (_b = (_a = descriptor.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, DataTrackPublishError.disconnected());
            break;
          case 'active':
            // Abandon any unpublishing descriptors that were in flight and assume they will get
            // cleaned up automatically with the connection shutdown.
            (_d = (_c = descriptor.unpublishingFuture).resolve) === null || _d === void 0 ? void 0 : _d.call(_c);
            yield this.unpublishRequest(descriptor.info.pubHandle);
            break;
        }
      }
      this.descriptors.clear();
      this.inFlightPacketCounter.clear();
      this.emit('reset');
    });
  }
}
export { DataTrackOutgoingPipeline, DataTrackOutgoingPipelineError, DataTrackOutgoingPipelineErrorReason, DataTrackPacketizer, DataTrackPacketizerError, DataTrackPacketizerReason, DataTrackPublishError, DataTrackPublishErrorReason, DataTrackPushFrameError, DataTrackPushFrameErrorReason, Descriptor, LocalDataTrack, OutgoingDataTrackManager, PUBLISH_TIMEOUT_MILLISECONDS, log };
