// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 25154-26901 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { ClientConfigSetting, ConnectionQuality$1, DisconnectReason, ReconnectReason } from './protobuf-messages-enums.mjs';
import { DataPacket, DataPacket_Kind, EncryptedPacket, EncryptedPacketPayload, Encryption_Type } from './protobuf-messages-room.mjs';
import { RpcAck } from './protobuf-messages-chat.mjs';
import { LeaveRequest_Action, PublishDataTrackResponse, SignalTarget, UpdateSubscription } from './protobuf-messages-signal.mjs';
import { DataChannelInfo, DataChannelReceiveState, SyncState } from './protobuf-messages-signal2.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { TypedPromise } from './browser.mjs';
import { ConnectionError, ConnectionErrorReason, CriticalTimers, NegotiationError, PublishDataError, SignalReconnectError, TrackInvalidError, UnexpectedConnectionState } from './errors.mjs';
import { EngineEvent } from './event-enums.mjs';
import { getTrackPublicationInfo } from './util.mjs';
import { isReactNative, isWeb, negotiateDependencyDescriptor, sleep, supportsAddTrack, supportsTransceiver } from './track.mjs';
import { isPublisherOfferWithJoinSupported, isVideoCodec, isVideoTrack, toHttpUrl } from './utils-misc.mjs';
import { asEncryptablePacket, isInsertableStreamSupported } from './e2ee-crypto.mjs';
import { hasFrameMetadataPublishOptions, isFrameMetadataSupported, shouldUseFrameMetadataScriptTransform } from './e2ee-frame.mjs';
import { DataTrackInfo } from './data-track-core.mjs';
import { SignalClient, SignalConnectionState } from './signal-client.mjs';
import { PCEvents, TTLMap, toProtoSessionDescription } from './sdp-parser.mjs';
import { PCTransportManager, PCTransportState, roomConnectOptionDefaults } from './pc-transport.mjs';
import { DataChannelKind, DataChannelManager } from './data-channel.mjs';
import { minReconnectWait } from './local-video-track.mjs';


const leaveReconnect = 'leave-reconnect';
/**
 * How long local connection quality must stay `LOST` while connected and publishing before we
 * force a full reconnect — `LOST` is the server's verdict that it isn't receiving our media.
 */
const connectionQualityLostTimeout = 10 * 1000;
const reliabeReceiveStateTTL = 30000;
const initialMediaSectionsAudio = 3;
const initialMediaSectionsVideo = 3;
var PCState;
(function (PCState) {
  PCState[PCState["New"] = 0] = "New";
  PCState[PCState["Connected"] = 1] = "Connected";
  PCState[PCState["Disconnected"] = 2] = "Disconnected";
  PCState[PCState["Reconnecting"] = 3] = "Reconnecting";
  PCState[PCState["Closed"] = 4] = "Closed";
})(PCState || (PCState = {}));
// Default data-channel max message size (bytes), used when the remote SDP
// answer does not advertise an `a=max-message-size` attribute (RFC 8841).
// `0` means "no limit".
const DEFAULT_MAX_MESSAGE_SIZE = 64000;
/** @internal */
class RTCEngine extends eventsExports.EventEmitter {
  get isClosed() {
    return this._isClosed;
  }
  get isNewlyCreated() {
    return this._isNewlyCreated;
  }
  get pendingReconnect() {
    return !!this.reconnectTimeout;
  }
  get serverVersion() {
    var _a, _b, _c;
    return ((_b = (_a = this.latestJoinResponse) === null || _a === void 0 ? void 0 : _a.serverInfo) === null || _b === void 0 ? void 0 : _b.version) || ((_c = this.latestJoinResponse) === null || _c === void 0 ? void 0 : _c.serverVersion) || undefined;
  }
  get reliableChannel() {
    return this.dataChannels.reliable;
  }
  get lossyChannel() {
    return this.dataChannels.lossy;
  }
  get dataTrackChannel() {
    return this.dataChannels.dataTrack;
  }
  constructor(options) {
    var _a;
    super();
    this.options = options;
    this.rtcConfig = {};
    this.peerConnectionTimeout = roomConnectOptionDefaults.peerConnectionTimeout;
    this.fullReconnectOnNext = false;
    /**
     * @internal
     */
    this.latestRemoteOfferId = 0;
    this.subscriberPrimary = false;
    this.pcState = PCState.New;
    this._isClosed = true;
    this._isNewlyCreated = true;
    this.pendingTrackResolvers = {};
    this.reconnectAttempts = 0;
    this.reconnectStart = 0;
    this.attemptingReconnect = false;
    /** keeps track of how often an initial join connection has been tried */
    this.joinAttempts = 0;
    /** specifies how often an initial join connection is allowed to retry */
    this.maxJoinAttempts = 1;
    this.shouldFailNext = false;
    this.shouldFailOnV1Path = false;
    this.log = livekitLogger;
    this.reliableReceivedState = new TTLMap(reliabeReceiveStateTTL);
    this.midToTrackId = {};
    /** used to indicate whether the browser is currently waiting to reconnect */
    this.isWaitingForNetworkReconnect = false;
    /**
     * Abort handlers for the in-flight `negotiate()` calls, which a single central
     * Closing/Restarting listener pair fans out to. Registering a listener pair per call instead
     * accumulates them on the engine emitter and trips its max-listener warning under
     * renegotiation bursts.
     */
    this.pendingNegotiationAborts = new Set();
    this.handleDataChannel = _a => __awaiter(this, [_a], void 0, function (_ref) {
      var _this = this;
      let channel = _ref.channel;
      return function* () {
        if (!channel) {
          return;
        }
        if (_this.dataChannels.adoptSubscriberChannel(channel)) {
          _this.log.debug("on data channel ".concat(channel.id, ", ").concat(channel.label));
        }
      }();
    });
    this.handleDataMessage = message => __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d;
      // make sure to respect incoming data message order by processing message events one after the other
      const unlock = yield this.dataProcessLock.lock();
      try {
        const bytes = yield this.decodeDataMessage(message);
        if (!bytes) {
          return;
        }
        const dp = DataPacket.fromBinary(bytes);
        if (dp.sequence > 0 && dp.participantSid !== '') {
          const lastSeq = this.reliableReceivedState.get(dp.participantSid);
          if (lastSeq && dp.sequence <= lastSeq) {
            // ignore duplicate or out-of-order packets in reliable channel
            return;
          }
          this.reliableReceivedState.set(dp.participantSid, dp.sequence);
        }
        if (((_a = dp.value) === null || _a === void 0 ? void 0 : _a.case) === 'speaker') {
          // dispatch speaker updates
          this.emit(EngineEvent.ActiveSpeakersUpdate, dp.value.value.speakers);
        } else if (((_b = dp.value) === null || _b === void 0 ? void 0 : _b.case) === 'encryptedPacket') {
          if (!this.e2eeManager) {
            this.log.error('Received encrypted packet but E2EE not set up');
            return;
          }
          let decryptedData;
          try {
            decryptedData = yield this.e2eeManager.handleEncryptedData(dp.value.value.encryptedValue, dp.value.value.iv, dp.participantIdentity, dp.value.value.keyIndex);
          } catch (err) {
            this.log.debug('failed to decrypt data packet', {
              error: err,
              participantIdentity: dp.participantIdentity
            });
            return;
          }
          const decryptedPacket = EncryptedPacketPayload.fromBinary(decryptedData.payload);
          const newDp = new DataPacket({
            value: decryptedPacket.value,
            participantIdentity: dp.participantIdentity,
            participantSid: dp.participantSid
          });
          if (((_c = newDp.value) === null || _c === void 0 ? void 0 : _c.case) === 'user') {
            // compatibility
            applyUserDataCompat(newDp, newDp.value.value);
          }
          this.emit(EngineEvent.DataPacketReceived, newDp, dp.value.value.encryptionType);
        } else {
          if (((_d = dp.value) === null || _d === void 0 ? void 0 : _d.case) === 'user') {
            // compatibility
            applyUserDataCompat(dp, dp.value.value);
          }
          this.emit(EngineEvent.DataPacketReceived, dp, Encryption_Type.NONE);
        }
      } finally {
        unlock();
      }
    });
    this.handleDataTrackMessage = message => __awaiter(this, void 0, void 0, function* () {
      const bytes = yield this.decodeDataMessage(message);
      if (!bytes) {
        return;
      }
      this.emit('dataTrackPacketReceived', bytes);
    });
    this.handleDataError = event => {
      // Errors fired while we're tearing the connection down (e.g. the SCTP transport aborting as
      // the peer connection closes) carry no actionable information — the channel is going away
      // regardless. Suppress them so a graceful disconnect doesn't surface spurious errors.
      // See livekit/client-sdk-js#1953.
      if (this._isClosed) {
        return;
      }
      const channel = event.currentTarget;
      const channelKind = channel.maxRetransmits === 0 ? 'lossy' : 'reliable';
      if (typeof RTCErrorEvent !== 'undefined' && event instanceof RTCErrorEvent && event.error) {
        const error = event.error;
        this.log.error("DataChannel error on ".concat(channelKind, ": ").concat(error.message), {
          error,
          errorDetail: error.errorDetail,
          sctpCauseCode: error.sctpCauseCode
        });
      } else {
        this.log.error("Unknown DataChannel error on ".concat(channelKind), {
          event
        });
      }
    };
    this.handleDataChannelClose = kind => () => {
      var _a;
      // A publisher DC closing while the session is up and the publisher PC is still
      // connected is the signature of an oversized message having aborted the channel
      // (see livekit/rust-sdks#1137). Surface it; do not attempt renegotiation.
      if (!this._isClosed && ((_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.publisher.getConnectionState()) === 'connected') {
        this.log.error("publisher data channel '".concat(DataChannelKind[kind], "' closed unexpectedly"), this.logContext);
      }
    };
    // websocket reconnect behavior. if websocket is interrupted, and the PeerConnection
    // continues to work, we can reconnect to websocket to continue the session
    // after a number of retries, we'll close and give up permanently
    this.handleDisconnect = (connection, disconnectReason) => {
      if (this._isClosed) {
        return;
      }
      this.log.warn("".concat(connection, " disconnected"));
      if (this.reconnectAttempts === 0) {
        // only reset start time on the first try
        this.reconnectStart = Date.now();
      }
      const disconnect = duration => {
        this.log.warn("could not recover connection after ".concat(this.reconnectAttempts, " attempts, ").concat(duration, "ms. giving up"));
        this.emit(EngineEvent.Disconnected);
        this.close("gave up reconnecting after ".concat(this.reconnectAttempts, " attempts, ").concat(duration, "ms"));
      };
      const duration = Date.now() - this.reconnectStart;
      let delay = this.getNextRetryDelay({
        elapsedMs: duration,
        retryCount: this.reconnectAttempts
      });
      if (delay === null) {
        disconnect(duration);
        return;
      }
      if (connection === leaveReconnect) {
        // The server asked for a reconnect, so skip any remaining backoff. Use the policy's first
        // retry delay instead of 0 so that clients told to reconnect together don't all reconnect
        // at the same instant.
        delay = this.getNextRetryDelay({
          elapsedMs: duration,
          retryCount: 0
        });
        if (delay === null) {
          disconnect(duration);
          return;
        }
      }
      this.log.debug("reconnecting in ".concat(delay, "ms"));
      this.clearReconnectTimeout();
      if (this.token) {
        // token may have been refreshed, we do not want to recreate the regionUrlProvider
        // since the current engine may have inherited a regional url
        this.emit(EngineEvent.TokenRefreshed, this.token);
      }
      this.reconnectTimeout = CriticalTimers.setTimeout(() => this.attemptReconnect(disconnectReason).finally(() => this.reconnectTimeout = undefined), delay);
    };
    this.waitForRestarted = () => {
      return new Promise((resolve, reject) => {
        if (this.pcState === PCState.Connected) {
          resolve();
        }
        const onRestarted = () => {
          this.off(EngineEvent.Disconnected, onDisconnected);
          resolve();
        };
        const onDisconnected = () => {
          this.off(EngineEvent.Restarted, onRestarted);
          reject();
        };
        this.once(EngineEvent.Restarted, onRestarted);
        this.once(EngineEvent.Disconnected, onDisconnected);
      });
    };
    this.onRtpMapAvailable = rtpTypes => {
      const rtpMap = new Map();
      rtpTypes.forEach(rtp => {
        const codec = rtp.codec.toLowerCase();
        if (isVideoCodec(codec)) {
          rtpMap.set(rtp.payload, codec);
        }
      });
      this.emit(EngineEvent.RTPVideoMapUpdate, rtpMap);
    };
    this.handleBrowserOnLine = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.url) {
        return;
      }
      const hasNetworkConnection = yield fetch(toHttpUrl(this.url), {
        method: 'HEAD'
      }).then(resp => resp.ok).catch(() => false);
      if (!hasNetworkConnection) {
        return;
      }
      this.log.info('detected network reconnected');
      if (
      // in case the engine is currently reconnecting, attempt a reconnect immediately after the browser state has changed to 'onLine'
      this.client.currentState === SignalConnectionState.RECONNECTING ||
      // also if the browser went offline before and the engine still thinks it's in a connected state, treat it as a network interruption that we haven't noticed yet
      this.isWaitingForNetworkReconnect && this.client.currentState === SignalConnectionState.CONNECTED) {
        this.clearReconnectTimeout();
        this.attemptReconnect(ReconnectReason.RR_SIGNAL_DISCONNECTED);
        this.isWaitingForNetworkReconnect = false;
      }
    });
    this.handleBrowserOffline = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.url) {
        return;
      }
      try {
        yield Promise.race([fetch(toHttpUrl(this.url), {
          method: 'HEAD'
        }),
        // if there's no internet connection the fetch rejects immediately, so we only use a short timeout here
        sleep(4000).then(() => Promise.reject())]);
      } catch (e) {
        // only set if the browser still thinks it's offline after the request failed
        if (window.navigator.onLine === false) {
          this.log.info('detected network interruption');
          this.isWaitingForNetworkReconnect = true;
        }
      }
    });
    this.log = getLogger((_a = options.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Engine, () => this.logContext);
    this.loggerOptions = {
      loggerName: options.loggerName,
      loggerContextCb: () => this.logContext
    };
    this.client = new SignalClient(undefined, this.loggerOptions);
    this.client.signalLatency = this.options.expSignalLatency;
    this.reconnectPolicy = this.options.reconnectPolicy;
    this.closingLock = new _();
    this.dataProcessLock = new _();
    this.dataChannels = new DataChannelManager({
      isEngineClosed: () => this.isClosed,
      isReconnecting: () => this.attemptingReconnect,
      onDataMessage: message => this.handleDataMessage(message),
      onDataTrackMessage: message => this.handleDataTrackMessage(message),
      onDataError: event => this.handleDataError(event),
      onChannelClose: kind => this.handleDataChannelClose(kind)(),
      onBufferStatusChanged: (kind, isLow) => this.emit(EngineEvent.DCBufferStatusChanged, isLow, kind)
    });
    this.client.onParticipantUpdate = updates => this.emit(EngineEvent.ParticipantUpdate, updates);
    this.client.onConnectionQuality = update => {
      this.handleLocalConnectionQuality(update);
      this.emit(EngineEvent.ConnectionQualityUpdate, update);
    };
    this.client.onRoomUpdate = update => this.emit(EngineEvent.RoomUpdate, update);
    this.client.onSubscriptionError = resp => this.emit(EngineEvent.SubscriptionError, resp);
    this.client.onSubscriptionPermissionUpdate = update => this.emit(EngineEvent.SubscriptionPermissionUpdate, update);
    this.client.onSpeakersChanged = update => this.emit(EngineEvent.SpeakersChanged, update);
    this.client.onStreamStateUpdate = update => this.emit(EngineEvent.StreamStateChanged, update);
    this.client.onRequestResponse = response => this.emit(EngineEvent.SignalRequestResponse, response);
    this.client.onParticipantUpdate = updates => this.emit(EngineEvent.ParticipantUpdate, updates);
    this.client.onJoined = joinResponse => this.emit(EngineEvent.Joined, joinResponse);
    const abortPendingNegotiations = () => {
      // Iterate a copy: each handler removes itself from the set as its negotiation settles.
      for (const abort of Array.from(this.pendingNegotiationAborts)) {
        abort();
      }
    };
    this.on(EngineEvent.Closing, abortPendingNegotiations);
    this.on(EngineEvent.Restarting, abortPendingNegotiations);
  }
  /** @internal */
  get logContext() {
    var _a, _b, _c, _d, _e, _f;
    return {
      room: (_b = (_a = this.latestJoinResponse) === null || _a === void 0 ? void 0 : _a.room) === null || _b === void 0 ? void 0 : _b.name,
      roomID: (_d = (_c = this.latestJoinResponse) === null || _c === void 0 ? void 0 : _c.room) === null || _d === void 0 ? void 0 : _d.sid,
      participant: (_f = (_e = this.latestJoinResponse) === null || _e === void 0 ? void 0 : _e.participant) === null || _f === void 0 ? void 0 : _f.identity,
      participantID: this.participantSid
    };
  }
  join(url_1, token_1, opts_1, abortSignal_1) {
    return __awaiter(this, arguments, void 0, function (url, token, opts, abortSignal) {
      var _this2 = this;
      let useV0Path = arguments.length > 4 && arguments[4] !== undefined ? arguments[4] : false;
      return function* () {
        var _a, _b, _c, _d;
        _this2._isNewlyCreated = false;
        _this2.url = url;
        _this2.token = token;
        _this2.signalOpts = opts;
        _this2.maxJoinAttempts = opts.maxRetries;
        try {
          _this2.joinAttempts += 1;
          _this2.joinStartedAt = performance.now();
          _this2.setupSignalClientCallbacks();
          // Whether the initial publisher offer is bundled with the join request. Computed once and
          // reused after the join below. Only the (non-Firefox) offer-with-join path does this.
          const sendOfferWithJoin = !useV0Path && isPublisherOfferWithJoinSupported();
          let offerProto;
          if (sendOfferWithJoin) {
            if (!_this2.pcManager) {
              // Firefox is excluded from offer-with-join (see isPublisherOfferWithJoinSupported):
              // customers reported ICE connectivity problems for FF on this path (#1919) that we were
              // never able to reproduce, so out of caution FF stays on the deferred path below. The
              // exact cause is unknown — note that ICE gathering does not actually start here, since
              // createInitialOffer() defers setLocalDescription (via pendingInitialOffer) until the
              // answer is applied, after updateConfiguration() has set the server's TURN servers.
              yield _this2.configure();
              _this2.applyInitialPublisherLayout();
            }
            const offer = yield (_a = _this2.pcManager) === null || _a === void 0 ? void 0 : _a.publisher.createInitialOffer();
            if (offer) {
              offerProto = toProtoSessionDescription(offer.offer, offer.offerId);
            }
          }
          if (abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.aborted) {
            throw ConnectionError.cancelled('Connection aborted');
          }
          if (!useV0Path && _this2.shouldFailOnV1Path) {
            _this2.shouldFailOnV1Path = false;
            throw ConnectionError.serviceNotFound('Simulated v1 path failure', 'v0-rtc');
          }
          const joinResponse = yield _this2.client.join(url, token, opts, abortSignal, useV0Path, offerProto);
          if (abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.aborted) {
            _this2.log.info('join cancelled after the signal join response');
            throw ConnectionError.cancelled('Connection aborted');
          }
          _this2._isClosed = false;
          _this2.latestJoinResponse = joinResponse;
          _this2.participantSid = (_b = joinResponse.participant) === null || _b === void 0 ? void 0 : _b.sid;
          _this2.subscriberPrimary = joinResponse.subscriberPrimary;
          if (sendOfferWithJoin) {
            (_c = _this2.pcManager) === null || _c === void 0 ? void 0 : _c.updateConfiguration(_this2.makeRTCConfiguration(joinResponse));
          } else {
            if (!_this2.pcManager) {
              // Deferred path (Firefox, and V0): configure with the join response so the PC picks up
              // the server's ICE servers and topology, then negotiate separately rather than bundling
              // the offer with the join.
              yield _this2.configure(joinResponse, !useV0Path);
              if (abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.aborted) {
                _this2.log.info('join cancelled after configuring peer connections');
                throw ConnectionError.cancelled('Connection aborted');
              }
              if (!useV0Path) {
                // The V1 first offer must carry the media layout so Firefox binds receive decoders for
                // subscribed tracks — without it, subscribed audio/video arrive as RTP but
                // never decode. V0 (legacy dual-PC) keeps its original lazy behavior.
                _this2.applyInitialPublisherLayout();
              }
            }
            // create offer
            if (!_this2.subscriberPrimary || joinResponse.fastPublish) {
              _this2.negotiate().catch(err => {
                _this2.log.error(err);
              });
            }
          }
          if (_this2.joinStartedAt !== undefined) {
            // Lets a video published before the transports connect be seeded from the time elapsed
            // so far, rather than delaying its offer until the setup time is measured.
            (_d = _this2.pcManager) === null || _d === void 0 ? void 0 : _d.publisher.setConnectionStartedAt(_this2.joinStartedAt);
          }
          _this2.registerOnLineListener();
          _this2.clientConfiguration = joinResponse.clientConfiguration;
          _this2.emit(EngineEvent.SignalConnected, joinResponse);
          let serverInfo = joinResponse.serverInfo;
          if (!serverInfo) {
            serverInfo = {
              version: joinResponse.serverVersion,
              region: joinResponse.serverRegion
            };
          }
          _this2.log.info("connected to Livekit Server ".concat(Object.entries(serverInfo).map(_ref2 => {
            let _ref3 = _slicedToArray(_ref2, 2),
              key = _ref3[0],
              value = _ref3[1];
            return "".concat(key, ": ").concat(value);
          }).join(', ')));
          return {
            joinResponse,
            serverInfo
          };
        } catch (e) {
          if (e instanceof ConnectionError) {
            if (e.reason === ConnectionErrorReason.ServerUnreachable) {
              _this2.log.warn("Couldn't connect to server, attempt ".concat(_this2.joinAttempts, " of ").concat(_this2.maxJoinAttempts));
              if (_this2.joinAttempts < _this2.maxJoinAttempts) {
                return _this2.join(url, token, opts, abortSignal, useV0Path);
              }
            } else if (e.reason === ConnectionErrorReason.ServiceNotFound) {
              _this2.log.warn("Initial connection failed: ".concat(e.message, " \u2013 Retrying"));
              if (_this2.pcManager) {
                _this2.pcManager.onStateChange = undefined;
                yield _this2.cleanupPeerConnections();
              }
              return _this2.join(url, token, opts, abortSignal, true);
            }
          }
          throw e;
        }
      }();
    });
  }
  /**
   * @param reason why the session is ending, recorded by the signal lifecycle. Worth passing
   * wherever the caller knows more than "someone called close" — the server's leave reason, or
   * having given up on reconnecting.
   */
  close(reason) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      (_a = this.reconnectAbortController) === null || _a === void 0 ? void 0 : _a.abort();
      this.reconnectAbortController = undefined;
      const unlock = yield this.closingLock.lock();
      if (this.isClosed) {
        unlock();
        return;
      }
      try {
        this._isClosed = true;
        this.joinAttempts = 0;
        this.joinStartedAt = undefined;
        this.emit(EngineEvent.Closing);
        this.removeAllListeners();
        this.deregisterOnLineListener();
        this.clearPendingReconnect();
        this.clearLostQualityTimeout();
        this.cleanupLossyDataStats();
        yield this.cleanupPeerConnections();
        yield this.cleanupClient(reason);
      } finally {
        unlock();
      }
    });
  }
  cleanupPeerConnections() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      this.dataChannels.teardown();
      yield (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.close();
      this.pcManager = undefined;
      // the connecting timestamp belongs to the transports we just tore down
      this.transportConnectingSince = undefined;
      this.reliableReceivedState.clear();
    });
  }
  cleanupLossyDataStats() {
    this.lossyChannel.stopThresholdTuning();
  }
  cleanupClient(reason) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.client.close(true, reason);
      this.client.resetCallbacks();
      // Any in-flight addTrack requests are orphaned by the signal reconnect — the new session
      // won't deliver `trackPublishedResponse` for them, so reject the pending resolvers and
      // clear the map. Otherwise a subsequent `addTrack` call with the same client id (e.g. a
      // publish retry after a `NegotiationError`) throws `TrackInvalidError`.
      for (const cid of Object.keys(this.pendingTrackResolvers)) {
        this.pendingTrackResolvers[cid].reject();
      }
      this.pendingTrackResolvers = {};
    });
  }
  addTrack(req) {
    if (this.pendingTrackResolvers[req.cid]) {
      throw new TrackInvalidError('a track with the same ID has already been published');
    }
    return new Promise((resolve, reject) => {
      const publicationTimeout = CriticalTimers.setTimeout(() => {
        delete this.pendingTrackResolvers[req.cid];
        reject(ConnectionError.timeout('publication of local track timed out, no response from server'));
      }, 10000);
      this.pendingTrackResolvers[req.cid] = {
        resolve: info => {
          CriticalTimers.clearTimeout(publicationTimeout);
          resolve(info);
        },
        reject: () => {
          CriticalTimers.clearTimeout(publicationTimeout);
          reject(new Error('Cancelled publication by calling unpublish'));
        }
      };
      this.client.sendAddTrack(req);
    });
  }
  /**
   * Removes sender from PeerConnection, returning true if it was removed successfully
   * and a negotiation is necessary
   * @param sender
   * @returns
   */
  removeTrack(sender) {
    if (sender.track && this.pendingTrackResolvers[sender.track.id]) {
      const reject = this.pendingTrackResolvers[sender.track.id].reject;
      if (reject) {
        reject();
      }
      delete this.pendingTrackResolvers[sender.track.id];
    }
    try {
      this.pcManager.removeTrack(sender);
      return true;
    } catch (e) {
      this.log.warn('failed to remove track', {
        error: e
      });
    }
    return false;
  }
  updateMuteStatus(trackSid, muted) {
    this.client.sendMuteTrack(trackSid, muted);
  }
  get dataSubscriberReadyState() {
    var _a;
    return (_a = this.dataChannelForKind(DataChannelKind.RELIABLE, true)) === null || _a === void 0 ? void 0 : _a.readyState;
  }
  getConnectedServerAddress() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      return (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.getConnectedAddress();
    });
  }
  /* @internal */
  setRegionStrategy(strategy) {
    this.regionStrategy = strategy;
  }
  configure(joinResponse, useSinglePeerConnection) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      // already configured
      if (this.pcManager && this.pcManager.currentState !== PCTransportState.NEW) {
        return;
      }
      if (!joinResponse) {
        const rtcConfig = this.makeRTCConfiguration();
        this.pcManager = new PCTransportManager('publisher-only', this.loggerOptions, rtcConfig);
      } else {
        this.participantSid = (_a = joinResponse.participant) === null || _a === void 0 ? void 0 : _a.sid;
        const rtcConfig = this.makeRTCConfiguration(joinResponse);
        this.pcManager = new PCTransportManager(useSinglePeerConnection ? 'publisher-only' : joinResponse.subscriberPrimary ? 'subscriber-primary' : 'publisher-primary', this.loggerOptions, rtcConfig);
      }
      this.emit(EngineEvent.TransportsCreated, this.pcManager.publisher, this.pcManager.subscriber);
      this.pcManager.onIceCandidate = (candidate, target) => {
        this.client.sendIceCandidate(candidate, target);
      };
      this.pcManager.onPublisherOffer = (offer, offerId) => {
        this.client.sendOffer(offer, offerId);
      };
      this.pcManager.onDataChannel = this.handleDataChannel;
      this.pcManager.onStateChange = (connectionState, publisherState, subscriberState) => __awaiter(this, void 0, void 0, function* () {
        var _a;
        this.log.debug("primary PC state changed ".concat(connectionState));
        // Record when the primary transport actually entered CONNECTING so
        // verifyTransport() can bound how long we tolerate it. Deriving it from the
        // real transition (this handler only fires on state changes) rather than from
        // observation time keeps it from going stale across peer-connection rebuilds.
        if (connectionState === PCTransportState.CONNECTING) {
          this.transportConnectingSince = Date.now();
        } else {
          this.transportConnectingSince = undefined;
        }
        if (['closed', 'disconnected', 'failed'].includes(publisherState)) {
          // reset publisher connection promise
          this.publisherConnectionPromise = undefined;
        }
        if (connectionState === PCTransportState.CONNECTED) {
          if (this.joinStartedAt !== undefined) {
            // Connection setup time is the only network signal there is before the first video
            // offer. Set synchronously, so it is in place before `room.connect()` resolves and the
            // app can publish. A full restart joins again and builds new transports, so it is
            // measured again; a resume keeps its estimator and never restarts the timer.
            const setupTimeMs = Math.round(performance.now() - this.joinStartedAt);
            this.joinStartedAt = undefined;
            this.log.info("connection setup took ".concat(setupTimeMs, " ms"));
            (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.publisher.setConnectionSetupTime(setupTimeMs);
          }
          const shouldEmit = this.pcState === PCState.New;
          this.pcState = PCState.Connected;
          if (shouldEmit) {
            this.emit(EngineEvent.Connected, this.latestJoinResponse);
          }
        } else if (connectionState === PCTransportState.FAILED) {
          // on Safari, PeerConnection will switch to 'disconnected' during renegotiation
          if (this.pcState === PCState.Connected || this.pcState === PCState.Reconnecting) {
            this.pcState = PCState.Disconnected;
            this.handleDisconnect('peerconnection failed', subscriberState === 'failed' ? ReconnectReason.RR_SUBSCRIBER_FAILED : ReconnectReason.RR_PUBLISHER_FAILED);
          }
        }
        // detect cases where both signal client and peer connection are severed and assume that user has lost network connection
        const isSignalSevered = this.client.isDisconnected || this.client.currentState === SignalConnectionState.RECONNECTING;
        const isPCSevered = [PCTransportState.FAILED, PCTransportState.CLOSING, PCTransportState.CLOSED].includes(connectionState);
        if (isSignalSevered && isPCSevered && !this._isClosed) {
          this.emit(EngineEvent.Offline);
        }
      });
      this.pcManager.onTrack = ev => {
        // this fires after the underlying transceiver is stopped and potentially
        // peer connection closed, so do not bubble up if there are no streams
        if (ev.streams.length === 0) return;
        this.emit(EngineEvent.MediaTrackAdded, ev.track, ev.streams[0], ev.receiver);
      };
    });
  }
  setupSignalClientCallbacks() {
    // configure signaling client
    this.client.onAnswer = (sd, offerId, midToTrackId) => __awaiter(this, void 0, void 0, function* () {
      if (!this.pcManager) {
        return;
      }
      this.log.debug('received server answer', {
        RTCSdpType: sd.type,
        sdp: sd.sdp,
        midToTrackId
      });
      // in dual PC mode the publisher answer carries no mapping (the server's publisher
      // transport has no sending tracks) and must not clobber the subscriber offer mapping
      if (this.pcManager.mode === 'publisher-only') {
        this.midToTrackId = midToTrackId;
      }
      yield this.pcManager.setPublisherAnswer(sd, offerId);
      this.emit(EngineEvent.RequestSubscribedCodecRefresh);
    });
    // add candidate on trickle
    this.client.onTrickle = (candidate, target) => {
      if (!this.pcManager) {
        return;
      }
      this.log.debug('got ICE candidate from peer', {
        candidate,
        target
      });
      this.pcManager.addIceCandidate(candidate, target);
    };
    // when server creates an offer for the client
    this.client.onOffer = (sd, offerId, midToTrackId) => __awaiter(this, void 0, void 0, function* () {
      this.latestRemoteOfferId = offerId;
      if (!this.pcManager) {
        return;
      }
      this.midToTrackId = midToTrackId;
      const answer = yield this.pcManager.createSubscriberAnswerFromOffer(sd, offerId);
      if (answer) {
        this.client.sendAnswer(answer, offerId);
      }
    });
    this.client.onLocalTrackPublished = res => {
      var _a;
      this.log.debug('received trackPublishedResponse', {
        cid: res.cid,
        track: (_a = res.track) === null || _a === void 0 ? void 0 : _a.sid
      });
      if (!this.pendingTrackResolvers[res.cid]) {
        this.log.error("missing track resolver for ".concat(res.cid), {
          cid: res.cid
        });
        return;
      }
      const resolve = this.pendingTrackResolvers[res.cid].resolve;
      delete this.pendingTrackResolvers[res.cid];
      resolve(res.track);
    };
    this.client.onLocalTrackUnpublished = response => {
      this.emit(EngineEvent.LocalTrackUnpublished, response);
    };
    this.client.onLocalTrackSubscribed = trackSid => {
      this.emit(EngineEvent.LocalTrackSubscribed, trackSid);
    };
    this.client.onTokenRefresh = token => {
      this.token = token;
      this.emit(EngineEvent.TokenRefreshed, token);
    };
    this.client.onRemoteMuteChanged = (trackSid, muted) => {
      this.emit(EngineEvent.RemoteMute, trackSid, muted);
    };
    this.client.onSubscribedQualityUpdate = update => {
      this.emit(EngineEvent.SubscribedQualityUpdate, update);
    };
    this.client.onRoomMoved = res => {
      var _a;
      this.participantSid = (_a = res.participant) === null || _a === void 0 ? void 0 : _a.sid;
      if (this.latestJoinResponse) {
        this.latestJoinResponse.room = res.room;
      }
      this.emit(EngineEvent.RoomMoved, res);
    };
    this.client.onMediaSectionsRequirement = requirement => {
      this.addMediaSections(requirement.numAudios, requirement.numVideos);
      this.negotiate().catch(err => {
        this.log.error(err);
      });
    };
    this.client.onPublishDataTrackResponse = event => {
      this.emit(EngineEvent.PublishDataTrackResponse, event);
    };
    this.client.onUnPublishDataTrackResponse = event => {
      this.emit(EngineEvent.UnPublishDataTrackResponse, event);
    };
    this.client.onDataTrackSubscriberHandles = event => {
      this.emit(EngineEvent.DataTrackSubscriberHandles, event);
    };
    this.client.onClose = () => {
      this.handleDisconnect('signal', ReconnectReason.RR_SIGNAL_DISCONNECTED);
    };
    this.client.onLeave = leave => {
      var _a;
      this.log.info("client leave request received (action=".concat(leave === null || leave === void 0 ? void 0 : leave.action, ")"), {
        reason: leave === null || leave === void 0 ? void 0 : leave.reason
      });
      if (leave.regions) {
        this.log.debug('updating regions');
        this.emit(EngineEvent.ServerRegionsReported, leave.regions);
      }
      switch (leave.action) {
        case LeaveRequest_Action.DISCONNECT:
          this.emit(EngineEvent.Disconnected, leave === null || leave === void 0 ? void 0 : leave.reason);
          this.close("server leave: ".concat((_a = DisconnectReason[leave.reason]) !== null && _a !== void 0 ? _a : leave.reason));
          break;
        case LeaveRequest_Action.RECONNECT:
          this.fullReconnectOnNext = true;
          // reconnect immediately instead of waiting for next attempt
          this.handleDisconnect(leaveReconnect);
          break;
        case LeaveRequest_Action.RESUME:
          // reconnect immediately instead of waiting for next attempt
          this.handleDisconnect(leaveReconnect);
      }
    };
  }
  makeRTCConfiguration(serverResponse) {
    var _a;
    const rtcConfig = Object.assign({}, this.rtcConfig);
    // E2EE and packet trailer extraction both rely on encoded frame transforms.
    // Only opt into the createEncodedStreams flavor when that path will be
    // used; RTCRtpScriptTransform does not need the PeerConnection flag.
    const needsInsertableStreams = ((_a = this.signalOpts) === null || _a === void 0 ? void 0 : _a.e2eeEnabled) || this.frameMetadataWorker && !shouldUseFrameMetadataScriptTransform();
    if (needsInsertableStreams && isInsertableStreamSupported()) {
      this.log.debug('E2EE - setting up transports with insertable streams');
      //  this makes sure that no data is sent before the transforms are ready
      // @ts-ignore
      rtcConfig.encodedInsertableStreams = true;
    }
    // @ts-ignore
    rtcConfig.sdpSemantics = 'unified-plan';
    // @ts-ignore
    rtcConfig.continualGatheringPolicy = 'gather_continually';
    if (!serverResponse) {
      return rtcConfig;
    }
    // update ICE servers before creating PeerConnection
    if (serverResponse.iceServers && !rtcConfig.iceServers) {
      const rtcIceServers = [];
      serverResponse.iceServers.forEach(iceServer => {
        const rtcIceServer = {
          urls: iceServer.urls
        };
        if (iceServer.username) rtcIceServer.username = iceServer.username;
        if (iceServer.credential) {
          rtcIceServer.credential = iceServer.credential;
        }
        rtcIceServers.push(rtcIceServer);
      });
      rtcConfig.iceServers = rtcIceServers;
    }
    if (serverResponse.clientConfiguration && serverResponse.clientConfiguration.forceRelay === ClientConfigSetting.ENABLED) {
      rtcConfig.iceTransportPolicy = 'relay';
    }
    return rtcConfig;
  }
  /**
   * Populate the publisher PC so its first offer carries the data channels + recvonly media
   * sections. Required for every V1 connection: Firefox only binds receive decoders for media
   * present in that first offer, and the offer-with-join path needs the sections to
   * build a meaningful initial offer. Must be called on a configured pcManager.
   */
  applyInitialPublisherLayout() {
    this.createDataChannels();
    /**
     * Native libwebrtc does not support pre-populating the media sections,
     * so we skip it for React Native.
     *
     * Related: https://github.com/livekit/rust-sdks/pull/1151
     */
    if (!isReactNative()) {
      this.addMediaSections(initialMediaSectionsAudio, initialMediaSectionsVideo);
    }
  }
  addMediaSections(numAudios, numVideos) {
    var _a, _b, _c;
    const transceiverInit = {
      direction: 'recvonly'
    };
    for (let i = 0; i < numAudios; i++) {
      (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.addPublisherTransceiverOfKind('audio', transceiverInit);
    }
    // media only arrives on these sections when there is no subscriber connection to arrive on
    const receivesMedia = ((_b = this.pcManager) === null || _b === void 0 ? void 0 : _b.mode) === 'publisher-only';
    for (let i = 0; i < numVideos; i++) {
      const transceiver = (_c = this.pcManager) === null || _c === void 0 ? void 0 : _c.addPublisherTransceiverOfKind('video', transceiverInit);
      if (receivesMedia && transceiver) {
        const negotiated = negotiateDependencyDescriptor(transceiver);
        this.log.debug('dependency descriptor negotiated for received video', {
          negotiated
        });
      }
    }
  }
  createDataChannels() {
    if (!this.pcManager) {
      return;
    }
    if (this.dataChannels.hasPublisherChannels) {
      // Replacing live channels (the Safari null-id path in resumeConnection): the new ones start
      // out connecting, so drop the memoized readiness check and make the next sender wait for
      // them to open.
      this.publisherConnectionPromise = undefined;
    }
    this.dataChannels.createPublisherChannels(this.pcManager);
  }
  /** Normalizes an incoming data-channel message into bytes, or logs and returns undefined. */
  decodeDataMessage(message) {
    return __awaiter(this, void 0, void 0, function* () {
      if (message.data instanceof ArrayBuffer) {
        return new Uint8Array(message.data);
      }
      if (message.data instanceof Blob) {
        return new Uint8Array(yield message.data.arrayBuffer());
      }
      this.log.error('unsupported data type', {
        data: message.data
      });
      return undefined;
    });
  }
  createSender(track, opts, encodings) {
    return __awaiter(this, void 0, void 0, function* () {
      let sender;
      if (supportsTransceiver()) {
        sender = yield this.createTransceiverRTCRtpSender(track, opts, encodings);
      } else if (supportsAddTrack()) {
        this.log.warn('using add-track fallback');
        sender = yield this.createRTCRtpSender(track.mediaStreamTrack);
      } else {
        throw new UnexpectedConnectionState('Required webRTC APIs not supported on this device');
      }
      this.setupFrameMetadataSender(sender, opts);
      return sender;
    });
  }
  createSimulcastSender(track, simulcastTrack, opts, encodings) {
    return __awaiter(this, void 0, void 0, function* () {
      let sender;
      if (supportsTransceiver()) {
        sender = yield this.createSimulcastTransceiverSender(track, simulcastTrack, opts, encodings);
      } else if (supportsAddTrack()) {
        this.log.debug('using add-track fallback');
        sender = yield this.createRTCRtpSender(track.mediaStreamTrack);
      } else {
        throw new UnexpectedConnectionState('Cannot stream on this device');
      }
      if (sender) {
        this.setupFrameMetadataSender(sender, opts);
      }
      return sender;
    });
  }
  get frameMetadataWorker() {
    var _a, _b;
    return (_b = (_a = this.options.frameMetadata) !== null && _a !== void 0 ? _a : this.options.packetTrailer) === null || _b === void 0 ? void 0 : _b.worker;
  }
  setupFrameMetadataSender(sender) {
    let opts = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    var _a, _b, _c;
    const worker = this.frameMetadataWorker;
    if (!worker || ((_a = this.signalOpts) === null || _a === void 0 ? void 0 : _a.e2eeEnabled)) {
      return;
    }
    const frameMetadata = (_b = opts.frameMetadata) !== null && _b !== void 0 ? _b : opts.packetTrailer;
    const hasMetadata = hasFrameMetadataPublishOptions(frameMetadata);
    if (shouldUseFrameMetadataScriptTransform()) {
      if (hasMetadata) {
        // @ts-ignore
        sender.transform = new RTCRtpScriptTransform(worker, {
          kind: 'encode',
          packetTrailer: frameMetadata
        });
      }
      return;
    }
    if (!isFrameMetadataSupported((_c = this.options.frameMetadata) !== null && _c !== void 0 ? _c : this.options.packetTrailer) || !('createEncodedStreams' in sender)) {
      if (hasMetadata) {
        this.log.warn('frame metadata transform not supported; skipping write', this.logContext);
      }
      return;
    }
    // @ts-ignore
    const _sender$createEncoded = sender.createEncodedStreams(),
      readable = _sender$createEncoded.readable,
      writable = _sender$createEncoded.writable;
    if (hasMetadata) {
      worker.postMessage({
        kind: 'encode',
        data: {
          readableStream: readable,
          writableStream: writable,
          packetTrailer: frameMetadata
        }
      }, [readable, writable]);
    } else {
      readable.pipeTo(writable);
    }
  }
  createTransceiverRTCRtpSender(track, opts, encodings) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.pcManager) {
        throw new UnexpectedConnectionState('publisher is closed');
      }
      const streams = [];
      if (track.mediaStream) {
        streams.push(track.mediaStream);
      }
      if (isVideoTrack(track)) {
        track.codec = opts.videoCodec;
      }
      const transceiverInit = {
        direction: 'sendonly',
        streams
      };
      if (encodings) {
        transceiverInit.sendEncodings = encodings;
      }
      // addTransceiver for react-native is async. web is synchronous, but await won't effect it.
      const transceiver = yield this.pcManager.addPublisherTransceiver(track.mediaStreamTrack, transceiverInit);
      return transceiver.sender;
    });
  }
  createSimulcastTransceiverSender(track, simulcastTrack, opts, encodings) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.pcManager) {
        throw new UnexpectedConnectionState('publisher is closed');
      }
      const transceiverInit = {
        direction: 'sendonly'
      };
      if (encodings) {
        transceiverInit.sendEncodings = encodings;
      }
      // addTransceiver for react-native is async. web is synchronous, but await won't effect it.
      const transceiver = yield this.pcManager.addPublisherTransceiver(simulcastTrack.mediaStreamTrack, transceiverInit);
      if (!opts.videoCodec) {
        return;
      }
      yield track.setSimulcastTrackSender(opts.videoCodec, transceiver.sender);
      return transceiver.sender;
    });
  }
  createRTCRtpSender(track) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.pcManager) {
        throw new UnexpectedConnectionState('publisher is closed');
      }
      return this.pcManager.addPublisherTrack(track);
    });
  }
  /**
   * A sustained local `LOST` while connected and publishing means the server isn't receiving
   * our media, so force a full reconnect; any non-`LOST` value cancels a pending trigger.
   */
  handleLocalConnectionQuality(update) {
    if (!this.participantSid) {
      return;
    }
    const localUpdate = update.updates.find(u => u.participantSid === this.participantSid);
    if (!localUpdate) {
      return;
    }
    if (localUpdate.quality === ConnectionQuality$1.LOST) {
      this.scheduleLostQualityReconnect();
    } else {
      this.clearLostQualityTimeout();
    }
  }
  scheduleLostQualityReconnect() {
    if (this.lostQualityTimeout) {
      // already counting down towards a reconnect
      return;
    }
    this.lostQualityTimeout = CriticalTimers.setTimeout(() => {
      this.lostQualityTimeout = undefined;
      if (this._isClosed || this.pcState !== PCState.Connected || this.attemptingReconnect) {
        return;
      }
      if (!this.hasActivePublisherSenders()) {
        return;
      }
      this.log.warn('local connection quality lost while publishing, triggering full reconnect', this.logContext);
      this.fullReconnectOnNext = true;
      this.handleDisconnect('connection quality lost', ReconnectReason.RR_PUBLISHER_FAILED);
    }, connectionQualityLostTimeout);
  }
  clearLostQualityTimeout() {
    if (this.lostQualityTimeout) {
      CriticalTimers.clearTimeout(this.lostQualityTimeout);
      this.lostQualityTimeout = undefined;
    }
  }
  /** Whether the publisher currently has any sender with a live track. */
  hasActivePublisherSenders() {
    var _a, _b;
    return (_b = (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.publisher.getSenders().some(sender => !!sender.track && sender.track.readyState === 'live')) !== null && _b !== void 0 ? _b : false;
  }
  /**
   * Forces a full reconnect while keeping the engine (and its saved credentials) alive. Used by
   * Room's connection-reconcile safety net when the transport silently died but we looked connected.
   * @internal
   */
  reconnect() {
    let reason = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : ReconnectReason.RR_UNKNOWN;
    this.fullReconnectOnNext = true;
    this.handleDisconnect('reconcile', reason);
  }
  attemptReconnect(reason) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      if (this._isClosed) {
        return;
      }
      // guard for attempting reconnection multiple times while one attempt is still not finished
      if (this.attemptingReconnect) {
        this.log.warn('already attempting reconnect, returning early');
        return;
      }
      // A pending Lost-quality countdown belongs to the session we're now leaving; cancel it so
      // it can't fire against the reconnected session before the server has evaluated it. (A resume
      // keeps the peer connections, so cleanupPeerConnections wouldn't cover this path.)
      this.clearLostQualityTimeout();
      if (((_a = this.clientConfiguration) === null || _a === void 0 ? void 0 : _a.resumeConnection) === ClientConfigSetting.DISABLED ||
      // signaling state could change to closed due to hardware sleep
      // those connections cannot be resumed
      ((_c = (_b = this.pcManager) === null || _b === void 0 ? void 0 : _b.currentState) !== null && _c !== void 0 ? _c : PCTransportState.NEW) === PCTransportState.NEW) {
        this.fullReconnectOnNext = true;
      }
      // Consume the flag up front: capture whether this attempt is a full reconnect, then reset
      // it. From here on a `true` value unambiguously represents a *new* full-reconnect request
      // that arrived while this attempt was running (e.g. a server RECONNECT leave), which the
      // finally block dispatches — for both the resume and full-reconnect paths.
      const fullReconnect = this.fullReconnectOnNext;
      this.fullReconnectOnNext = false;
      let succeeded = false;
      const abortController = new AbortController();
      this.reconnectAbortController = abortController;
      try {
        this.attemptingReconnect = true;
        if (fullReconnect) {
          yield this.restartConnection(abortController.signal);
        } else {
          yield this.resumeConnection(reason);
        }
        if (abortController.signal.aborted) {
          this.log.info('reconnect attempt cancelled, not marking it as succeeded');
          return;
        }
        this.clearPendingReconnect();
        succeeded = true;
      } catch (e) {
        if (abortController.signal.aborted) {
          this.log.info('reconnect attempt cancelled, not retrying', {
            error: e
          });
          return;
        }
        this.reconnectAttempts += 1;
        let recoverable = true;
        if (e instanceof UnexpectedConnectionState) {
          this.log.debug('received unrecoverable error', {
            error: e
          });
          // unrecoverable
          recoverable = false;
        } else if (fullReconnect || !(e instanceof SignalReconnectError)) {
          // a failed full reconnect stays a full reconnect; a failed resume can only be
          // resumed again for a signal-level error, otherwise it escalates
          this.fullReconnectOnNext = true;
        }
        if (recoverable) {
          this.handleDisconnect('reconnect', ReconnectReason.RR_UNKNOWN);
        } else {
          this.log.info("could not recover connection after ".concat(this.reconnectAttempts, " attempts, ").concat(Date.now() - this.reconnectStart, "ms. giving up"));
          this.emit(EngineEvent.Disconnected);
          yield this.close("gave up reconnecting after ".concat(this.reconnectAttempts, " attempts, ").concat(Date.now() - this.reconnectStart, "ms"));
        }
      } finally {
        // close() may already have cleared this (or a later attempt replaced it); only release our own
        if (this.reconnectAbortController === abortController) {
          this.reconnectAbortController = undefined;
        }
        this.attemptingReconnect = false;
        // A full reconnect requested while this attempt was running (e.g. a `RECONNECT` leave
        // during a resume or a restart) that a successful attempt didn't act on; dispatch it now
        // (the failure path already retries).
        if (succeeded && this.fullReconnectOnNext && !this._isClosed) {
          this.log.debug('full reconnect requested during in-progress attempt, dispatching');
          this.handleDisconnect('reconnect');
        }
      }
    });
  }
  getNextRetryDelay(context) {
    try {
      return this.reconnectPolicy.nextRetryDelayInMs(context);
    } catch (e) {
      this.log.warn('encountered error in reconnect policy', {
        error: e
      });
    }
    // error in user code with provided reconnect policy, stop reconnecting
    return null;
  }
  restartConnection(abortSignal, regionUrl) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      try {
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled before starting', {
            regionUrl
          });
          return;
        }
        if (!this.url || !this.token) {
          // permanent failure, don't attempt reconnection
          throw new UnexpectedConnectionState('could not reconnect, url or token not saved');
        }
        this.log.info("reconnecting, attempt: ".concat(this.reconnectAttempts));
        this.emit(EngineEvent.Restarting);
        if (!this.client.isDisconnected) {
          yield this.client.sendLeave();
        }
        yield this.cleanupPeerConnections();
        yield this.cleanupClient();
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled after cleaning up the previous session', {
            regionUrl
          });
          return;
        }
        let joinResponse;
        try {
          if (!this.signalOpts) {
            this.log.warn('attempted connection restart, without signal options present');
            throw new SignalReconnectError();
          }
          // in case a regionUrl is passed, the region URL takes precedence
          joinResponse = (yield this.join(regionUrl !== null && regionUrl !== void 0 ? regionUrl : this.url, this.token, this.signalOpts, abortSignal, !this.options.singlePeerConnection)).joinResponse;
        } catch (e) {
          if (e instanceof ConnectionError && e.reason === ConnectionErrorReason.NotAllowed) {
            throw new UnexpectedConnectionState('could not reconnect, token might be expired');
          }
          throw new SignalReconnectError();
        }
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled after the signal join response', {
            regionUrl
          });
          return;
        }
        if (this.shouldFailNext) {
          this.shouldFailNext = false;
          throw new Error('simulated failure');
        }
        this.client.setReconnected();
        this.emit(EngineEvent.SignalRestarted, joinResponse);
        yield this.waitForPCReconnected();
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled after peer connections reconnected', {
            regionUrl
          });
          return;
        }
        // re-check signal connection state before setting engine as resumed
        if (this.client.currentState !== SignalConnectionState.CONNECTED) {
          throw new SignalReconnectError('Signal connection got severed during reconnect');
        }
        (_a = this.regionStrategy) === null || _a === void 0 ? void 0 : _a.resetAttempts();
        // reconnect success
        this.emit(EngineEvent.Restarted);
      } catch (error) {
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled after a failed attempt', {
            regionUrl
          });
          return;
        }
        const nextRegionUrl = yield (_b = this.regionStrategy) === null || _b === void 0 ? void 0 : _b.getNextUrl(abortSignal);
        if (abortSignal.aborted) {
          this.log.info('full reconnect cancelled during region selection', {
            regionUrl
          });
          return;
        }
        if (nextRegionUrl) {
          yield this.restartConnection(abortSignal, nextRegionUrl);
          return;
        } else {
          // no more regions to try (or we're not on cloud)
          (_c = this.regionStrategy) === null || _c === void 0 ? void 0 : _c.resetAttempts();
          throw error;
        }
      }
    });
  }
  resumeConnection(reason) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.url || !this.token) {
        // permanent failure, don't attempt reconnection
        throw new UnexpectedConnectionState('could not reconnect, url or token not saved');
      }
      // trigger publisher reconnect
      if (!this.pcManager) {
        throw new UnexpectedConnectionState('publisher and subscriber connections unset');
      }
      this.log.info("resuming signal connection, attempt ".concat(this.reconnectAttempts));
      this.emit(EngineEvent.Resuming);
      let res;
      try {
        this.setupSignalClientCallbacks();
        res = yield this.client.reconnect(this.url, this.token, this.participantSid, reason);
      } catch (error) {
        let message = '';
        if (error instanceof Error) {
          message = error.message;
          this.log.error(error.message, {
            error
          });
        }
        if (error instanceof ConnectionError && error.reason === ConnectionErrorReason.NotAllowed) {
          throw new UnexpectedConnectionState('could not reconnect, token might be expired');
        }
        if (error instanceof ConnectionError && error.reason === ConnectionErrorReason.LeaveRequest) {
          throw error;
        }
        throw new SignalReconnectError(message);
      }
      this.emit(EngineEvent.SignalResumed);
      if (res) {
        const rtcConfig = this.makeRTCConfiguration(res);
        this.pcManager.updateConfiguration(rtcConfig);
        if (this.latestJoinResponse) {
          this.latestJoinResponse.serverInfo = res.serverInfo;
        }
      } else {
        this.log.warn('Did not receive reconnect response');
      }
      if (this.shouldFailNext) {
        this.shouldFailNext = false;
        throw new Error('simulated failure');
      }
      yield this.pcManager.triggerIceRestart();
      yield this.waitForPCReconnected();
      // re-check signal connection state before setting engine as resumed
      if (this.client.currentState !== SignalConnectionState.CONNECTED) {
        throw new SignalReconnectError('Signal connection got severed during reconnect');
      }
      this.client.setReconnected();
      // recreate publish datachannel if it's id is null
      // (for safari https://bugs.webkit.org/show_bug.cgi?id=184688)
      const reliableDC = this.dataChannelForKind(DataChannelKind.RELIABLE);
      if ((reliableDC === null || reliableDC === void 0 ? void 0 : reliableDC.readyState) === 'open' && reliableDC.id === null) {
        this.createDataChannels();
      }
      if (res === null || res === void 0 ? void 0 : res.lastMessageSeq) {
        this.resendReliableMessagesForResume(res.lastMessageSeq).catch(error => {
          this.log.warn('failed to resend reliable messages after resume', Object.assign(Object.assign({}, this.logContext), {
            error
          }));
        });
      }
      // resume success
      this.emit(EngineEvent.Resumed);
    });
  }
  waitForPCInitialConnection(timeout, abortController) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.pcManager) {
        throw new UnexpectedConnectionState('PC manager is closed');
      }
      yield this.pcManager.ensurePCTransportConnection(abortController, timeout);
    });
  }
  waitForPCReconnected() {
    return __awaiter(this, void 0, void 0, function* () {
      this.pcState = PCState.Reconnecting;
      this.log.debug('waiting for peer connection to reconnect');
      try {
        yield sleep(minReconnectWait); // FIXME setTimeout again not ideal for a connection critical path
        if (!this.pcManager) {
          throw new UnexpectedConnectionState('PC manager is closed');
        }
        yield this.pcManager.ensurePCTransportConnection(undefined, this.peerConnectionTimeout);
        this.pcState = PCState.Connected;
      } catch (e) {
        // TODO do we need a `failed` state here for the PC?
        this.pcState = PCState.Disconnected;
        throw ConnectionError.internal("could not establish PC connection, ".concat(e.message));
      }
    });
  }
  /** @internal */
  publishRpcAck(destinationIdentity, requestId) {
    return __awaiter(this, void 0, void 0, function* () {
      const packet = new DataPacket({
        destinationIdentities: [destinationIdentity],
        kind: DataPacket_Kind.RELIABLE,
        value: {
          case: 'rpcAck',
          value: new RpcAck({
            requestId
          })
        }
      });
      yield this.sendDataPacket(packet, DataChannelKind.RELIABLE);
    });
  }
  /* @internal */
  sendDataPacket(packet, /**  Data-track frames don't come through here — they're sent pre-serialized via {@link sendDataTrackFrame }. */
  kind) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      // make sure we do have a data connection
      yield this.ensurePublisherConnected(kind);
      if (this.e2eeManager && this.e2eeManager.isDataChannelEncryptionEnabled) {
        const encryptablePacket = asEncryptablePacket(packet);
        if (encryptablePacket) {
          const encryptedData = yield this.e2eeManager.encryptData(encryptablePacket.toBinary());
          packet.value = {
            case: 'encryptedPacket',
            value: new EncryptedPacket({
              encryptedValue: encryptedData.payload,
              iv: encryptedData.iv,
              keyIndex: encryptedData.keyIndex
            })
          };
        }
      }
      if (kind === DataChannelKind.RELIABLE) {
        packet.sequence = this.reliableChannel.nextSequence();
      }
      const msg = packet.toBinary();
      // Clamp to the SDK default - libwebrtc advertises larger (~256 KiB)
      // than LiveKit/pion can deliver end-to-end (~64 KiB), so we trust
      // the answer up untilthe built in ceiling.
      const maxPublisherMessageSizeBytes = Math.min((_b = (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.getMaxPublisherMessageSize()) !== null && _b !== void 0 ? _b : DEFAULT_MAX_MESSAGE_SIZE, DEFAULT_MAX_MESSAGE_SIZE);
      if (typeof maxPublisherMessageSizeBytes !== 'undefined' && maxPublisherMessageSizeBytes !== 0 /* 0 means "no limit" */ && msg.byteLength > maxPublisherMessageSizeBytes) {
        throw new PublishDataError("cannot publish data packet larger than ".concat(maxPublisherMessageSizeBytes, " bytes (got ").concat(msg.byteLength, ")"));
      }
      // The full-buffer policy (drop for lossy, wait/replay for reliable) is the channel's own, as
      // is emitting the buffer-status change once the send settles.
      if (kind === DataChannelKind.RELIABLE) {
        yield this.reliableChannel.send(msg, packet.sequence);
      } else {
        yield this.lossyChannel.send(msg);
      }
    });
  }
  /**
   * Sends pre-serialized bytes on the data-track channel. This is the one send path that doesn't
   * go through {@link sendDataPacket} — Room's `packetAvailable` handler calls it directly with
   * bytes the data-track pipeline already serialized.
   *
   * @internal
   */
  sendDataTrackFrame(bytes) {
    return __awaiter(this, void 0, void 0, function* () {
      // Make sure we do have a data connection: on lazily negotiated publisher connections this is
      // what kicks negotiation off, and it waits for the channel to open. Memoized, so the
      // steady-state cost is one await on an already-resolved promise.
      yield this.ensurePublisherConnected(DataChannelKind.DATA_TRACK_LOSSY);
      yield this.dataTrackChannel.send(bytes);
    });
  }
  resendReliableMessagesForResume(lastMessageSeq) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.ensurePublisherConnected(DataChannelKind.RELIABLE);
      yield this.reliableChannel.replay(lastMessageSeq);
    });
  }
  /** The flow-control gate for `kind` — see {@link FlowControlledDataChannel}. */
  flowControlFor(kind) {
    return this.dataChannels.channelFor(kind);
  }
  /**
   * Resolves once the caller may send on the `kind` channel — see
   * {@link FlowControlledDataChannel.waitForHeadroomWithLock}.
   */
  waitForBufferHeadroom(kind) {
    return __awaiter(this, void 0, void 0, function* () {
      const dc = this.dataChannelForKind(kind);
      if (!dc) {
        throw new UnexpectedConnectionState("DataChannel not found, kind: ".concat(kind));
      }
      return this.flowControlFor(kind).waitForHeadroomWithLock(dc);
    });
  }
  /**
   * @internal
   */
  ensureDataTransportConnected(kind_1) {
    return __awaiter(this, arguments, void 0, function (kind) {
      var _this3 = this;
      let subscriber = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : this.subscriberPrimary;
      return function* () {
        var _a;
        if (!_this3.pcManager) {
          throw new UnexpectedConnectionState('PC manager is closed');
        }
        const transport = subscriber ? _this3.pcManager.subscriber : _this3.pcManager.publisher;
        const transportName = subscriber ? 'Subscriber' : 'Publisher';
        if (!transport) {
          throw ConnectionError.internal("".concat(transportName, " connection not set"));
        }
        let needNegotiation = false;
        if (!subscriber && !_this3.dataChannelForKind(kind, subscriber)) {
          _this3.createDataChannels();
          needNegotiation = true;
        }
        if (!needNegotiation && !subscriber && !_this3.pcManager.publisher.isICEConnected && _this3.pcManager.publisher.getICEConnectionState() !== 'checking') {
          needNegotiation = true;
        }
        if (needNegotiation) {
          // start negotiation
          _this3.negotiate().catch(err => {
            _this3.log.error(err);
          });
        }
        const targetChannel = _this3.dataChannelForKind(kind, subscriber);
        if ((targetChannel === null || targetChannel === void 0 ? void 0 : targetChannel.readyState) === 'open') {
          return;
        }
        // wait until ICE connected
        const endTime = new Date().getTime() + _this3.peerConnectionTimeout;
        while (new Date().getTime() < endTime) {
          if (transport.isICEConnected && ((_a = _this3.dataChannelForKind(kind, subscriber)) === null || _a === void 0 ? void 0 : _a.readyState) === 'open') {
            return;
          }
          yield sleep(50);
        }
        throw ConnectionError.internal("could not establish ".concat(transportName, " connection, state: ").concat(transport.getICEConnectionState()));
      }();
    });
  }
  ensurePublisherConnected(kind) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.publisherConnectionPromise) {
        this.publisherConnectionPromise = this.ensureDataTransportConnected(kind, false);
      }
      yield this.publisherConnectionPromise;
    });
  }
  /* @internal */
  verifyTransport() {
    if (!this.pcManager) {
      return false;
    }
    const state = this.pcManager.currentState;
    const allowedConnectionStates = [PCTransportState.CONNECTING, PCTransportState.CONNECTED];
    if (!allowedConnectionStates.includes(state)) {
      return false;
    }
    // ensure signal is connected
    if (!this.client.ws || this.client.ws.readyState === WebSocket.CLOSED) {
      return false;
    }
    // A transport stuck in CONNECTING never reaches CONNECTED nor reports FAILED, so it would
    // otherwise look healthy forever; bound how long we tolerate it. The entry time is recorded
    // in the pcManager state-change handler (see configure()), so this is a pure read — an
    // unrecorded CONNECTING fails open rather than measuring against a stale timestamp.
    if (state === PCTransportState.CONNECTING && this.transportConnectingSince !== undefined && Date.now() - this.transportConnectingSince > this.peerConnectionTimeout) {
      this.log.warn('transport stuck in connecting state', this.logContext);
      return false;
    }
    return true;
  }
  /** @internal */
  negotiate() {
    return __awaiter(this, void 0, void 0, function* () {
      // observe signal state
      return new TypedPromise((resolve, reject) => __awaiter(this, void 0, void 0, function* () {
        if (!this.pcManager) {
          reject(new NegotiationError('PC manager is closed'));
          return;
        }
        this.pcManager.requirePublisher();
        // don't negotiate without any transceivers or data channel, it will generate sdp without ice frag then negotiate failed
        if (this.pcManager.publisher.getTransceivers().length == 0 && !this.dataChannels.hasPublisherChannels) {
          this.createDataChannels();
        }
        const abortController = new AbortController();
        const handleClosed = () => {
          abortController.abort();
          this.log.debug('engine disconnected while negotiation was ongoing');
          resolve();
          return;
        };
        if (this.isClosed) {
          reject(new NegotiationError('cannot negotiate on closed engine'));
        }
        this.pendingNegotiationAborts.add(handleClosed);
        this.pcManager.publisher.off(PCEvents.RTPVideoPayloadTypes, this.onRtpMapAvailable);
        this.pcManager.publisher.once(PCEvents.RTPVideoPayloadTypes, this.onRtpMapAvailable);
        try {
          yield this.pcManager.negotiate(abortController);
          resolve();
        } catch (e) {
          if (abortController.signal.aborted) {
            // negotiation was aborted due to engine close or restart, resolve
            // cleanly to avoid triggering a cascading reconnect loop
            resolve();
            return;
          }
          if (e instanceof NegotiationError) {
            this.fullReconnectOnNext = true;
          }
          this.handleDisconnect('negotiation', ReconnectReason.RR_UNKNOWN);
          if (e instanceof Error) {
            reject(e);
          } else {
            reject(new Error(String(e)));
          }
        } finally {
          this.pendingNegotiationAborts.delete(handleClosed);
        }
      }));
    });
  }
  dataChannelForKind(kind, sub) {
    return this.dataChannels.getHandle(kind, sub);
  }
  /** @internal */
  sendSyncState(remoteTracks, localTracks, localDataTrackInfos) {
    var _a, _b, _c, _d;
    if (!this.pcManager) {
      this.log.warn('sync state cannot be sent without peer connection setup');
      return;
    }
    const previousPublisherOffer = this.pcManager.publisher.getLocalDescription();
    const previousPublisherAnswer = this.pcManager.publisher.getRemoteDescription();
    const previousSubscriberOffer = (_a = this.pcManager.subscriber) === null || _a === void 0 ? void 0 : _a.getRemoteDescription();
    const previousSubscriberAnswer = (_b = this.pcManager.subscriber) === null || _b === void 0 ? void 0 : _b.getLocalDescription();
    /* 1. autosubscribe on, so subscribed tracks = all tracks - unsub tracks,
          in this case, we send unsub tracks, so server add all tracks to this
          subscribe pc and unsub special tracks from it.
       2. autosubscribe off, we send subscribed tracks.
    */
    const autoSubscribe = (_d = (_c = this.signalOpts) === null || _c === void 0 ? void 0 : _c.autoSubscribe) !== null && _d !== void 0 ? _d : true;
    const trackSids = new Array();
    const trackSidsDisabled = new Array();
    remoteTracks.forEach(track => {
      if (track.isDesired !== autoSubscribe) {
        trackSids.push(track.trackSid);
      }
      if (!track.isEnabled) {
        trackSidsDisabled.push(track.trackSid);
      }
    });
    this.client.sendSyncState(new SyncState({
      answer: this.pcManager.mode === 'publisher-only' ? previousPublisherAnswer ? toProtoSessionDescription({
        sdp: previousPublisherAnswer.sdp,
        type: previousPublisherAnswer.type
      }) : undefined : previousSubscriberAnswer ? toProtoSessionDescription({
        sdp: previousSubscriberAnswer.sdp,
        type: previousSubscriberAnswer.type
      }) : undefined,
      offer: this.pcManager.mode === 'publisher-only' ? previousPublisherOffer ? toProtoSessionDescription({
        sdp: previousPublisherOffer.sdp,
        type: previousPublisherOffer.type
      }) : undefined : previousSubscriberOffer ? toProtoSessionDescription({
        sdp: previousSubscriberOffer.sdp,
        type: previousSubscriberOffer.type
      }) : undefined,
      subscription: new UpdateSubscription({
        trackSids,
        subscribe: !autoSubscribe,
        participantTracks: []
      }),
      publishTracks: getTrackPublicationInfo(localTracks),
      dataChannels: this.dataChannelsInfo(),
      trackSidsDisabled,
      datachannelReceiveStates: this.reliableReceivedState.map((seq, sid) => {
        return new DataChannelReceiveState({
          publisherSid: sid,
          lastSeq: seq
        });
      }),
      publishDataTracks: localDataTrackInfos.map(info => {
        return new PublishDataTrackResponse({
          info: DataTrackInfo.toProtobuf(info)
        });
      })
    }));
  }
  /* @internal */
  failNext() {
    // debugging method to fail the next reconnect/resume attempt
    this.shouldFailNext = true;
  }
  /* @internal */
  failNextV1Path() {
    // debugging method to fail the next connection attempt for /rtc/v1 to trigger the fallback version
    this.shouldFailOnV1Path = true;
  }
  dataChannelsInfo() {
    const infos = [];
    const getInfo = (dc, target) => {
      if ((dc === null || dc === void 0 ? void 0 : dc.id) !== undefined && dc.id !== null) {
        infos.push(new DataChannelInfo({
          label: dc.label,
          id: dc.id,
          target
        }));
      }
    };
    getInfo(this.dataChannelForKind(DataChannelKind.LOSSY), SignalTarget.PUBLISHER);
    getInfo(this.dataChannelForKind(DataChannelKind.RELIABLE), SignalTarget.PUBLISHER);
    getInfo(this.dataChannelForKind(DataChannelKind.LOSSY, true), SignalTarget.SUBSCRIBER);
    getInfo(this.dataChannelForKind(DataChannelKind.RELIABLE, true), SignalTarget.SUBSCRIBER);
    return infos;
  }
  clearReconnectTimeout() {
    if (this.reconnectTimeout) {
      CriticalTimers.clearTimeout(this.reconnectTimeout);
    }
  }
  clearPendingReconnect() {
    this.clearReconnectTimeout();
    this.reconnectAttempts = 0;
  }
  registerOnLineListener() {
    if (isWeb()) {
      window.addEventListener('online', this.handleBrowserOnLine);
      window.addEventListener('offline', this.handleBrowserOffline);
    }
  }
  deregisterOnLineListener() {
    if (isWeb()) {
      window.removeEventListener('online', this.handleBrowserOnLine);
      window.removeEventListener('offline', this.handleBrowserOffline);
    }
  }
  getTrackIdForReceiver(receiver) {
    var _a;
    const mid = (_a = this.pcManager) === null || _a === void 0 ? void 0 : _a.getMidForReceiver(receiver);
    if (mid) {
      const match = Object.entries(this.midToTrackId).find(_ref4 => {
        let _ref5 = _slicedToArray(_ref4, 1),
          key = _ref5[0];
        return key === mid;
      });
      if (match) {
        return match[1];
      }
    }
  }
}
function applyUserDataCompat(newObj, oldObj) {
  const participantIdentity = newObj.participantIdentity ? newObj.participantIdentity : oldObj.participantIdentity;
  newObj.participantIdentity = participantIdentity;
  oldObj.participantIdentity = participantIdentity;
  const destinationIdentities = newObj.destinationIdentities.length !== 0 ? newObj.destinationIdentities : oldObj.destinationIdentities;
  newObj.destinationIdentities = destinationIdentities;
  oldObj.destinationIdentities = destinationIdentities;
}const log$4 = getLogger(LoggerNames.Region);
const DEFAULT_MAX_AGE_MS = 5000;
const STOP_REFETCH_DELAY_MS = 30000;
export { DEFAULT_MAX_AGE_MS, DEFAULT_MAX_MESSAGE_SIZE, PCState, RTCEngine, STOP_REFETCH_DELAY_MS, applyUserDataCompat, connectionQualityLostTimeout, initialMediaSectionsAudio, initialMediaSectionsVideo, leaveReconnect, log$4, reliabeReceiveStateTTL };
