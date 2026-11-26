// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 16206-17026 行按职责切出，行为不变。
/**
 * @experimental
 */
import { _ } from './protobuf-helpers.mjs';
import { __awaiter } from './tslib.mjs';
import { Encryption_Type } from './protobuf-messages-room.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger, onWorkerLogLevelChanged, workerLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { DeviceUnsupportedError } from './errors.mjs';
import { EngineEvent, ParticipantEvent, RoomEvent } from './event-enums.mjs';
import { mimeTypeToVideoCodecString } from './util.mjs';
import { isSafariBased, isScriptTransformSupportedForWorker } from './track.mjs';
import { Future, isLocalTrack, isVideoTrack } from './utils-misc.mjs';
import { E2EE_FLAG, E2EE_TRACK_ID, EncryptionEvent, KeyProviderEvent, isE2EESupported } from './e2ee-crypto.mjs';
import { CryptorError, CryptorErrorReason } from './e2ee-keyprovider.mjs';
import { hasFrameMetadataPublishOptions, isFrameMetadataSupported, shouldUseFrameMetadataScriptTransform } from './e2ee-frame.mjs';
import { RemoteVideoTrack } from './remote-track.mjs';
import { ConnectionState } from './room.mjs';

class E2EEManager extends eventsExports.EventEmitter {
  get logContext() {
    var _a, _b;
    return {
      room: (_a = this.room) === null || _a === void 0 ? void 0 : _a.name,
      participant: (_b = this.room) === null || _b === void 0 ? void 0 : _b.localParticipant.identity
    };
  }
  constructor(options, dcEncryptionEnabled) {
    super();
    this.decryptDataRequests = new Map();
    this.encryptDataRequests = new Map();
    this.log = getLogger(LoggerNames.E2EE, () => this.logContext);
    this.onWorkerMessage = ev => {
      var _a, _b;
      const _ev$data = ev.data,
        kind = _ev$data.kind,
        data = _ev$data.data;
      switch (kind) {
        case 'error':
          // If error has uuid, it's from an async operation (encrypt/decrypt).
          // Reject the corresponding future and let the caller decide how to log/handle;
          // logging here would duplicate whatever the caller does.
          if (data.uuid) {
            const decryptFuture = this.decryptDataRequests.get(data.uuid);
            if (decryptFuture === null || decryptFuture === void 0 ? void 0 : decryptFuture.reject) {
              decryptFuture.reject(data.error);
              break;
            }
            const encryptFuture = this.encryptDataRequests.get(data.uuid);
            if (encryptFuture === null || encryptFuture === void 0 ? void 0 : encryptFuture.reject) {
              encryptFuture.reject(data.error);
              break;
            }
          }
          this.log.error(data.error.message);
          this.emit(EncryptionEvent.EncryptionError, data.error, data.participantIdentity);
          break;
        case 'initAck':
          if (data.enabled) {
            this.keyProvider.getKeys().forEach(keyInfo => {
              this.postKey(keyInfo, false);
            });
          }
          break;
        case 'enable':
          if (data.enabled) {
            this.keyProvider.getKeys().forEach(keyInfo => {
              this.postKey(keyInfo, false);
            });
          }
          if (this.encryptionEnabled !== data.enabled && data.participantIdentity === ((_a = this.room) === null || _a === void 0 ? void 0 : _a.localParticipant.identity)) {
            this.emit(EncryptionEvent.ParticipantEncryptionStatusChanged, data.enabled, this.room.localParticipant);
            this.encryptionEnabled = data.enabled;
          } else if (data.participantIdentity) {
            const participant = (_b = this.room) === null || _b === void 0 ? void 0 : _b.getParticipantByIdentity(data.participantIdentity);
            if (!participant) {
              throw TypeError("couldn't set encryption status, participant not found".concat(data.participantIdentity));
            }
            this.emit(EncryptionEvent.ParticipantEncryptionStatusChanged, data.enabled, participant);
          }
          break;
        case 'ratchetKey':
          this.keyProvider.emit(KeyProviderEvent.KeyRatcheted, data.ratchetResult, data.participantIdentity, data.keyIndex);
          break;
        case 'decryptDataResponse':
          const decryptFuture = this.decryptDataRequests.get(data.uuid);
          if (decryptFuture === null || decryptFuture === void 0 ? void 0 : decryptFuture.resolve) {
            decryptFuture.resolve(data);
          }
          break;
        case 'encryptDataResponse':
          const encryptFuture = this.encryptDataRequests.get(data.uuid);
          if (encryptFuture === null || encryptFuture === void 0 ? void 0 : encryptFuture.resolve) {
            encryptFuture.resolve(data);
          }
          break;
        case 'packetTrailerMetadata':
          this.handleFrameMetadata(data.trackId, data.rtpTimestamp, data.ssrc, data.metadata);
          break;
        case 'log':
          workerLogger[data.level](data.msg, data.context);
          break;
      }
    };
    this.onWorkerError = ev => {
      this.log.error('e2ee worker encountered an error:', {
        error: ev.error
      });
      this.emit(EncryptionEvent.EncryptionError, ev.error, undefined);
    };
    this.keyProvider = options.keyProvider;
    this.worker = options.worker;
    this.encryptionEnabled = false;
    this.dataChannelEncryptionEnabled = dcEncryptionEnabled;
  }
  get isEnabled() {
    return this.encryptionEnabled;
  }
  get isDataChannelEncryptionEnabled() {
    return this.isEnabled && this.dataChannelEncryptionEnabled;
  }
  /**
   * @internal
   */
  setup(room) {
    if (!isE2EESupported()) {
      throw new DeviceUnsupportedError('tried to setup end-to-end encryption on an unsupported browser');
    }
    this.log.info('setting up e2ee');
    if (room !== this.room) {
      this.room = room;
      this.setupEventListeners(room, this.keyProvider);
      // this.worker = new Worker('');
      const msg = {
        kind: 'init',
        data: {
          keyProviderOptions: this.keyProvider.getOptions(),
          loglevel: workerLogger.getLevel()
        }
      };
      if (this.worker) {
        this.log.info("initializing worker", {
          worker: this.worker
        });
        this.worker.onmessage = this.onWorkerMessage;
        this.worker.onerror = this.onWorkerError;
        this.worker.postMessage(msg);
        this.subscribeToLogLevelChanges();
      }
    }
  }
  /**
   * Subscribe the current worker to main-thread `workerLogger` level changes,
   * without strongly retaining `this` or `this.worker` from the module-global
   * listener Set on the logger. See {@link disposeRegistry}.
   */
  subscribeToLogLevelChanges() {
    var _a;
    // Guard against duplicate registration on re-setup.
    (_a = this.unsubscribeLogLevel) === null || _a === void 0 ? void 0 : _a.call(this);
    let unsub;
    if (E2EEManager.disposeRegistry) {
      // Modern engines: hold the worker weakly so the module-global listener Set
      // on the logger can't retain this manager, and clean up the entry on GC.
      const workerRef = new WeakRef(this.worker);
      unsub = onWorkerLogLevelChanged(level => {
        const worker = workerRef.deref();
        if (!worker) {
          unsub === null || unsub === void 0 ? void 0 : unsub();
          return;
        }
        worker.postMessage({
          kind: 'setLogLevel',
          data: {
            level
          }
        });
      });
      E2EEManager.disposeRegistry.register(this, unsub, this);
    } else {
      // Safari <14.1 and similar: no WeakRef. Fall back to a strong reference;
      // the leak lives until the consumer calls `dispose()`.
      const worker = this.worker;
      unsub = onWorkerLogLevelChanged(level => {
        worker.postMessage({
          kind: 'setLogLevel',
          data: {
            level
          }
        });
      });
    }
    this.unsubscribeLogLevel = unsub;
  }
  /**
   * @internal
   * Release the log-level subscription, reject any pending encrypt/decrypt
   * futures, and detach the worker message handlers. The worker itself is
   * caller-owned and is not terminated. Idempotent.
   */
  dispose() {
    var _a, _b, _c;
    (_a = this.unsubscribeLogLevel) === null || _a === void 0 ? void 0 : _a.call(this);
    this.unsubscribeLogLevel = undefined;
    if (E2EEManager.disposeRegistry) {
      E2EEManager.disposeRegistry.unregister(this);
    }
    // Reject pending futures BEFORE detaching worker handlers, so any late
    // response can't resolve one after we've cut the pipe. Each future's
    // `onFinally` deletes its own map entry, so both maps drain themselves.
    // Snapshot before iterating in case a rejection handler mutates the map.
    const disposalError = new CryptorError('E2EEManager disposed', CryptorErrorReason.InternalError);
    for (const future of [...this.encryptDataRequests.values()]) {
      (_b = future.reject) === null || _b === void 0 ? void 0 : _b.call(future, disposalError);
    }
    for (const future of [...this.decryptDataRequests.values()]) {
      (_c = future.reject) === null || _c === void 0 ? void 0 : _c.call(future, disposalError);
    }
    if (this.worker) {
      this.worker.onmessage = null;
      this.worker.onerror = null;
    }
    this.removeAllListeners();
  }
  /**
   * @internal
   */
  setParticipantCryptorEnabled(enabled, participantIdentity) {
    this.log.debug("set e2ee to ".concat(enabled, " for participant ").concat(participantIdentity));
    this.postEnable(enabled, participantIdentity);
  }
  /**
   * @internal
   */
  setSifTrailer(trailer) {
    if (!trailer || trailer.length === 0) {
      this.log.warn("ignoring server sent trailer as it's empty");
    } else {
      this.postSifTrailer(trailer);
    }
  }
  handleFrameMetadata(trackId, rtpTimestamp, ssrc, metadata) {
    if (!this.room) {
      return;
    }
    for (const participant of [this.room.localParticipant, ...this.room.remoteParticipants.values()]) {
      for (const pub of participant.trackPublications.values()) {
        if (pub.track && pub.track.mediaStreamID === trackId && pub.track instanceof RemoteVideoTrack && pub.track.frameMetadataExtractor) {
          pub.track.frameMetadataExtractor.storeMetadata(rtpTimestamp, ssrc, metadata);
          return;
        }
      }
    }
  }
  setupEngine(engine) {
    engine.on(EngineEvent.RTPVideoMapUpdate, rtpMap => {
      this.postRTPMap(rtpMap);
    });
  }
  setupEventListeners(room, keyProvider) {
    room.on(RoomEvent.TrackPublished, (pub, participant) => this.setParticipantCryptorEnabledForPublication(pub, participant.identity));
    room.on(RoomEvent.ConnectionStateChanged, state => {
      if (state === ConnectionState.Connected) {
        room.remoteParticipants.forEach(participant => {
          participant.trackPublications.forEach(pub => {
            this.setParticipantCryptorEnabledForPublication(pub, participant.identity);
          });
        });
      }
    }).on(RoomEvent.TrackUnsubscribed, (track, _, participant) => {
      var _a;
      const msg = {
        kind: 'removeTransform',
        data: {
          participantIdentity: participant.identity,
          trackId: track.mediaStreamID
        }
      };
      (_a = this.worker) === null || _a === void 0 ? void 0 : _a.postMessage(msg);
    }).on(RoomEvent.TrackSubscribed, (track, pub, participant) => {
      this.setupE2EEReceiver(track, participant.identity, pub.trackInfo);
    }).on(RoomEvent.SignalConnected, () => {
      if (!this.room) {
        throw new TypeError("expected room to be present on signal connect");
      }
      const latestKeyIndex = keyProvider.getLatestManuallySetKeyIndex();
      keyProvider.getKeys().forEach(keyInfo => {
        var _a;
        this.postKey(keyInfo, latestKeyIndex === ((_a = keyInfo.keyIndex) !== null && _a !== void 0 ? _a : 0));
      });
      this.setParticipantCryptorEnabled(this.room.localParticipant.isE2EEEnabled, this.room.localParticipant.identity);
    });
    room.localParticipant.on(ParticipantEvent.LocalSenderCreated, (sender, track) => __awaiter(this, void 0, void 0, function* () {
      this.setupE2EESender(track, sender);
    }));
    room.localParticipant.on(ParticipantEvent.LocalTrackPublished, publication => {
      // Safari doesn't support retrieving payload information on RTCEncodedVideoFrame, so we need to update the codec manually once we have the trackInfo from the server
      if (!isVideoTrack(publication.track) || !isSafariBased()) {
        return;
      }
      const msg = {
        kind: 'updateCodec',
        data: {
          trackId: publication.track.mediaStreamID,
          codec: mimeTypeToVideoCodecString(publication.trackInfo.codecs[0].mimeType),
          participantIdentity: this.room.localParticipant.identity,
          hasPacketTrailer: false
        }
      };
      this.worker.postMessage(msg);
    });
    keyProvider.on(KeyProviderEvent.SetKey, (keyInfo, updateCurrentKeyIndex) => this.postKey(keyInfo, updateCurrentKeyIndex !== null && updateCurrentKeyIndex !== void 0 ? updateCurrentKeyIndex : true)).on(KeyProviderEvent.RatchetRequest, (participantId, keyIndex) => this.postRatchetRequest(participantId, keyIndex));
  }
  encryptData(data) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.worker) {
        throw Error('could not encrypt data, worker is missing');
      }
      const uuid = crypto.randomUUID();
      const msg = {
        kind: 'encryptDataRequest',
        data: {
          uuid,
          payload: data,
          participantIdentity: this.room.localParticipant.identity
        }
      };
      const future = new Future();
      future.onFinally = () => {
        this.encryptDataRequests.delete(uuid);
      };
      this.encryptDataRequests.set(uuid, future);
      this.worker.postMessage(msg);
      return future.promise;
    });
  }
  handleEncryptedData(payload, iv, participantIdentity, keyIndex) {
    if (!this.worker) {
      throw Error('could not handle encrypted data, worker is missing');
    }
    const uuid = crypto.randomUUID();
    const msg = {
      kind: 'decryptDataRequest',
      data: {
        uuid,
        payload,
        iv,
        participantIdentity,
        keyIndex
      }
    };
    const future = new Future();
    future.onFinally = () => {
      this.decryptDataRequests.delete(uuid);
    };
    this.decryptDataRequests.set(uuid, future);
    this.worker.postMessage(msg);
    return future.promise;
  }
  postRatchetRequest(participantIdentity, keyIndex) {
    if (!this.worker) {
      throw Error('could not ratchet key, worker is missing');
    }
    const msg = {
      kind: 'ratchetRequest',
      data: {
        participantIdentity: participantIdentity,
        keyIndex
      }
    };
    this.worker.postMessage(msg);
  }
  postKey(_ref, updateCurrentKeyIndex) {
    let key = _ref.key,
      participantIdentity = _ref.participantIdentity,
      keyIndex = _ref.keyIndex;
    var _a;
    if (!this.worker) {
      throw Error('could not set key, worker is missing');
    }
    const msg = {
      kind: 'setKey',
      data: {
        participantIdentity: participantIdentity,
        isPublisher: participantIdentity === ((_a = this.room) === null || _a === void 0 ? void 0 : _a.localParticipant.identity),
        key,
        keyIndex,
        updateCurrentKeyIndex
      }
    };
    this.worker.postMessage(msg);
  }
  postEnable(enabled, participantIdentity) {
    if (this.worker) {
      const enableMsg = {
        kind: 'enable',
        data: {
          enabled,
          participantIdentity
        }
      };
      this.worker.postMessage(enableMsg);
    } else {
      throw new ReferenceError('failed to enable e2ee, worker is not ready');
    }
  }
  postRTPMap(map) {
    var _a;
    if (!this.worker) {
      throw TypeError('could not post rtp map, worker is missing');
    }
    if (!((_a = this.room) === null || _a === void 0 ? void 0 : _a.localParticipant.identity)) {
      throw TypeError('could not post rtp map, local participant identity is missing');
    }
    const msg = {
      kind: 'setRTPMap',
      data: {
        map,
        participantIdentity: this.room.localParticipant.identity
      }
    };
    this.worker.postMessage(msg);
  }
  postSifTrailer(trailer) {
    if (!this.worker) {
      throw Error('could not post SIF trailer, worker is missing');
    }
    const msg = {
      kind: 'setSifTrailer',
      data: {
        trailer
      }
    };
    this.worker.postMessage(msg);
  }
  /**
   * Derive the participant's encryption state from one of its publications.
   * Publications without trackInfo are skipped rather than thrown on: throwing
   * inside the forEach above would leave every remaining participant without an
   * encryption state, and a cryptor with unknown state can no longer decrypt.
   */
  setParticipantCryptorEnabledForPublication(pub, participantIdentity) {
    if (!pub.trackInfo) {
      this.log.warn('skipping e2ee enabled update for publication without trackInfo', {
        trackSid: pub.trackSid
      });
      return;
    }
    this.setParticipantCryptorEnabled(pub.trackInfo.encryption !== Encryption_Type.NONE, participantIdentity);
  }
  setupE2EEReceiver(track, remoteId, trackInfo) {
    if (!track.receiver) {
      return;
    }
    if (!(trackInfo === null || trackInfo === void 0 ? void 0 : trackInfo.mimeType) || trackInfo.mimeType === '') {
      throw new TypeError('MimeType missing from trackInfo, cannot set up E2EE cryptor');
    }
    const hasPacketTrailer = track.kind === 'video' && !!trackInfo.packetTrailerFeatures && trackInfo.packetTrailerFeatures.length > 0;
    this.handleReceiver(track.receiver, track.mediaStreamID, remoteId, track.kind === 'video' ? mimeTypeToVideoCodecString(trackInfo.mimeType) : undefined, hasPacketTrailer);
  }
  setupE2EESender(track, sender) {
    var _a, _b, _c, _d;
    if (!isLocalTrack(track) || !sender) {
      if (!sender) this.log.warn('early return because sender is not ready');
      return;
    }
    const isVideo = isVideoTrack(track);
    // Safari exposes no per-frame codec info until trackInfo arrives. Seed only h264/h265:
    // a wrong vp8 seed would skip the NALU path, a wrong NALU seed only costs a logged fallback.
    const publishedCodec = isVideo ? (_a = track.publishOptions) === null || _a === void 0 ? void 0 : _a.videoCodec : undefined;
    this.handleSender(sender, track.mediaStreamID, publishedCodec === 'h264' || publishedCodec === 'h265' ? publishedCodec : undefined, isVideo ? (_c = (_b = track.publishOptions) === null || _b === void 0 ? void 0 : _b.frameMetadata) !== null && _c !== void 0 ? _c : (_d = track.publishOptions) === null || _d === void 0 ? void 0 : _d.packetTrailer : undefined);
  }
  /**
   * Handles the given {@code RTCRtpReceiver} by creating a {@code TransformStream} which will inject
   * a frame decoder.
   *
   */
  handleReceiver(receiver, trackId, participantIdentity, codec, hasPacketTrailer) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.worker) {
        return;
      }
      if (isScriptTransformSupportedForWorker()) {
        const options = {
          kind: 'decode',
          participantIdentity,
          trackId,
          codec,
          hasPacketTrailer
        };
        receiver.transform = new RTCRtpScriptTransform(this.worker, options);
      } else {
        if (E2EE_FLAG in receiver && E2EE_TRACK_ID in receiver) {
          // The transceiver is being reused for a new track. We cannot set up a new
          // pipeline from here: this receiver's encoded streams were transferred to
          // the worker on first setup and a transferred stream can't be transferred
          // again, while createEncodedStreams() may only be called once per receiver.
          // So hand the worker the new track/participant and let it re-point (and if
          // necessary rebuild) the cryptor that still owns those streams.
          const msg = {
            kind: 'updateCodec',
            data: {
              trackId,
              previousTrackId: receiver[E2EE_TRACK_ID],
              codec,
              participantIdentity,
              hasPacketTrailer
            }
          };
          this.worker.postMessage(msg);
          receiver[E2EE_TRACK_ID] = trackId;
          return;
        }
        // @ts-ignore
        let writable = receiver.writableStream;
        // @ts-ignore
        let readable = receiver.readableStream;
        if (!writable || !readable) {
          // @ts-ignore
          const receiverStreams = receiver.createEncodedStreams();
          // @ts-ignore
          receiver.writableStream = receiverStreams.writable;
          writable = receiverStreams.writable;
          // @ts-ignore
          receiver.readableStream = receiverStreams.readable;
          readable = receiverStreams.readable;
        }
        const msg = {
          kind: 'decode',
          data: {
            readableStream: readable,
            writableStream: writable,
            trackId: trackId,
            codec,
            participantIdentity: participantIdentity,
            hasPacketTrailer
          }
        };
        this.worker.postMessage(msg, [readable, writable]);
      }
      // @ts-ignore
      receiver[E2EE_FLAG] = true;
      // @ts-ignore
      receiver[E2EE_TRACK_ID] = trackId;
    });
  }
  /**
   * Handles the given {@code RTCRtpSender} by creating a {@code TransformStream} which will inject
   * a frame encoder.
   *
   */
  handleSender(sender, trackId, codec, frameMetadata) {
    var _a;
    if (E2EE_FLAG in sender || !this.worker) {
      return;
    }
    if (!((_a = this.room) === null || _a === void 0 ? void 0 : _a.localParticipant.identity) || this.room.localParticipant.identity === '') {
      throw TypeError('local identity needs to be known in order to set up encrypted sender');
    }
    if (isScriptTransformSupportedForWorker()) {
      this.log.info('initialize script transform');
      const options = {
        kind: 'encode',
        participantIdentity: this.room.localParticipant.identity,
        trackId,
        codec,
        hasPacketTrailer: hasFrameMetadataPublishOptions(frameMetadata),
        packetTrailer: frameMetadata
      };
      // @ts-ignore
      sender.transform = new RTCRtpScriptTransform(this.worker, options);
    } else {
      this.log.info('initialize encoded streams');
      // @ts-ignore
      const senderStreams = sender.createEncodedStreams();
      const msg = {
        kind: 'encode',
        data: {
          readableStream: senderStreams.readable,
          writableStream: senderStreams.writable,
          codec,
          trackId,
          participantIdentity: this.room.localParticipant.identity,
          hasPacketTrailer: hasFrameMetadataPublishOptions(frameMetadata),
          packetTrailer: frameMetadata
        }
      };
      this.worker.postMessage(msg, [senderStreams.readable, senderStreams.writable]);
    }
    // @ts-ignore
    sender[E2EE_FLAG] = true;
  }
}
/**
 * Runs a cleanup callback once this manager is garbage collected. Lets the
 * log-level listener (held in a module-global Set on the main-thread logger)
 * fall out of scope even when the consumer forgets to call `dispose()`.
 */
E2EEManager.disposeRegistry = typeof FinalizationRegistry !== 'undefined' && typeof WeakRef !== 'undefined' && new FinalizationRegistry(cleanup => {
  cleanup();
});const MAX_ENTRIES = 300;
/**
 * Caches frame metadata extracted from received video frames,
 * keyed by RTP timestamp so it can be looked up when the frame is displayed.
 *
 * Metadata is populated either by the frame metadata worker managed by
 * `FrameMetadataManager` (non-E2EE) or by the E2EE FrameCryptor worker
 * after decryption (E2EE).
 *
 * @experimental
 */
class FrameMetadataExtractor {
  constructor() {
    this.metadataMap = new Map();
    this.activeSsrc = 0;
  }
  storeMetadata(rtpTimestamp, ssrc, metadata) {
    // Simulcast layer switch: SSRC changed, flush stale entries from old layer.
    if (this.activeSsrc !== 0 && this.activeSsrc !== ssrc) {
      this.metadataMap.clear();
    }
    this.activeSsrc = ssrc;
    while (this.metadataMap.size >= MAX_ENTRIES) {
      const evicted = this.metadataMap.keys().next().value;
      this.metadataMap.delete(evicted);
    }
    this.metadataMap.set(rtpTimestamp, metadata);
  }
  lookupMetadata(rtpTimestamp) {
    return this.metadataMap.get(rtpTimestamp);
  }
  dispose() {
    this.metadataMap.clear();
    this.activeSsrc = 0;
  }
}/**
 * Manages packet trailer extraction for received video tracks.
 *
 * When a track's TrackInfo indicates packet trailer features, the manager
 * wires up an encoded frame transform to strip the trailer from encoded frames
 * and cache the metadata for lookup.
 *
 * Packet trailer extraction is worker-only. If no worker is configured, the
 * SDK does not advertise packet trailer support and skips extraction.
 *
 * When E2EE is active, the E2EE FrameCryptor worker handles trailer
 * extraction directly (before decryption), so this manager only creates
 * the extractor/metadata cache — no separate pipeline is installed.
 *
 * @experimental
 */
class FrameMetadataManager {
  constructor(options) {
    this.extractors = new Map();
    /**
     * Tracks the trackId associated with each receiver that has had its
     * encoded streams handed off to the worker. Used to detect receiver
     * reuse (transceiver recycling) so we can remap trackIds instead of
     * re-transferring already-consumed streams.
     */
    this.workerPipelines = new Map();
    this.onWorkerMessage = ev => {
      const msg = ev.data;
      if (msg.kind === 'metadata') {
        const extractor = this.extractors.get(msg.data.trackId);
        if (extractor) {
          extractor.storeMetadata(msg.data.rtpTimestamp, msg.data.ssrc, msg.data.metadata);
        }
      }
    };
    this.onWorkerError = ev => {
      livekitLogger.error('frame metadata worker encountered an error:', {
        error: ev.error
      });
    };
    this.worker = options === null || options === void 0 ? void 0 : options.worker;
  }
  /** @internal */
  setup(room) {
    if (room === this.room) {
      return;
    }
    this.room = room;
    if (this.worker) {
      this.worker.onmessage = this.onWorkerMessage;
      this.worker.onerror = this.onWorkerError;
      this.worker.postMessage({
        kind: 'init'
      });
    }
    room.on(RoomEvent.TrackSubscribed, (track, pub, _participant) => {
      if (track.kind !== 'video') {
        return;
      }
      this.setupReceiver(track, pub.trackInfo);
    }).on(RoomEvent.TrackUnsubscribed, track => {
      this.teardownTrack(track);
    }).on(RoomEvent.Disconnected, () => {
      this.cleanup();
    });
  }
  setupReceiver(track, trackInfo) {
    var _a, _b, _c;
    const receiver = track.receiver;
    if (!receiver) {
      return;
    }
    // Only install a pipeline for tracks that actually advertise packet
    // trailer features. This keeps us out of the way for tracks published by
    // clients on older protocols or that don't opt into the feature.
    const hasFeatures = !!(trackInfo === null || trackInfo === void 0 ? void 0 : trackInfo.packetTrailerFeatures) && trackInfo.packetTrailerFeatures.length > 0;
    if (!hasFeatures) {
      if (!((_a = this.room) === null || _a === void 0 ? void 0 : _a.hasE2EESetup)) {
        this.setupPassthroughReceiver(receiver, track.mediaStreamID);
      }
      return;
    }
    if (!isFrameMetadataSupported(this.worker ? {
      worker: this.worker
    } : undefined) && !((_b = this.room) === null || _b === void 0 ? void 0 : _b.hasE2EESetup)) {
      livekitLogger.warn('frame metadata transform not supported; skipping extraction');
      return;
    }
    const extractor = new FrameMetadataExtractor();
    const trackId = track.mediaStreamID;
    this.extractors.set(trackId, extractor);
    track.frameMetadataExtractor = extractor;
    if ((_c = this.room) === null || _c === void 0 ? void 0 : _c.hasE2EESetup) {
      // E2EE worker strips the trailer and injects metadata directly into
      // the extractor via E2eeManager; no pipeline is needed here.
      return;
    }
    this.setupWorkerReceiver(receiver, trackId, true);
  }
  setupPassthroughReceiver(receiver, trackId) {
    if (shouldUseFrameMetadataScriptTransform()) {
      if ('transform' in receiver) {
        // @ts-ignore
        receiver.transform = null;
      }
      return;
    }
    if (this.worker && isFrameMetadataSupported({
      worker: this.worker
    }) && !this.workerPipelines.has(receiver)) {
      this.setupWorkerReceiver(receiver, trackId, false);
      return;
    }
    if (this.worker && this.workerPipelines.has(receiver)) {
      this.setupWorkerReceiver(receiver, trackId, false);
    }
  }
  setupWorkerReceiver(receiver, newTrackId) {
    let hasPacketTrailer = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : true;
    const worker = this.worker;
    if (!worker) {
      return;
    }
    if (shouldUseFrameMetadataScriptTransform()) {
      // @ts-ignore
      receiver.transform = new RTCRtpScriptTransform(worker, {
        kind: 'decode',
        trackId: newTrackId
      });
      return;
    }
    const existingTrackId = this.workerPipelines.get(receiver);
    if (existingTrackId) {
      // Receiver is reused (transceiver recycled). The worker already owns
      // the encoded streams — just remap the trackId so metadata is keyed
      // correctly and re-activate processing.
      const msg = {
        kind: 'updateTrackId',
        data: {
          oldTrackId: existingTrackId,
          newTrackId,
          hasPacketTrailer
        }
      };
      worker.postMessage(msg);
      this.workerPipelines.set(receiver, newTrackId);
      return;
    }
    if (!('createEncodedStreams' in receiver)) {
      livekitLogger.warn('createEncodedStreams not supported');
      return;
    }
    let streams;
    try {
      // @ts-ignore — createEncodedStreams is not in standard typings
      streams = receiver.createEncodedStreams();
    } catch (err) {
      livekitLogger.warn('failed to create encoded streams', {
        error: err
      });
      return;
    }
    const msg = {
      kind: 'decode',
      data: {
        readableStream: streams.readable,
        writableStream: streams.writable,
        trackId: newTrackId,
        hasPacketTrailer
      }
    };
    worker.postMessage(msg, [streams.readable, streams.writable]);
    this.workerPipelines.set(receiver, newTrackId);
  }
  teardownTrack(track) {
    const trackId = track.mediaStreamID;
    const extractor = this.extractors.get(trackId);
    if (extractor) {
      extractor.dispose();
      this.extractors.delete(trackId);
    }
    if (track instanceof RemoteVideoTrack) {
      track.frameMetadataExtractor = undefined;
    }
    // The receiver pipeline is intentionally left running. If the receiver is
    // reused for a new track, `setupReceiver` will remap it. If the room
    // disconnects, `cleanup` drops all state. Any metadata produced in the
    // meantime is harmless — the extractor above has already been disposed and
    // is no longer reachable from any track.
  }
  cleanup() {
    var _a;
    for (const extractor of this.extractors.values()) {
      extractor.dispose();
    }
    this.extractors.clear();
    this.workerPipelines.clear();
    (_a = this.worker) === null || _a === void 0 ? void 0 : _a.terminate();
  }
}
/** @deprecated Use {@link FrameMetadataManager} instead. */
const PacketTrailerManager = FrameMetadataManager;
export { E2EEManager, FrameMetadataExtractor, FrameMetadataManager, MAX_ENTRIES, PacketTrailerManager };
