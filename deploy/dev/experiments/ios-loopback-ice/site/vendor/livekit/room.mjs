// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 34662-37107 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { protoInt64 } from './protobuf-int64.mjs';
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { DisconnectReason, TrackSource, TrackType } from './protobuf-messages-enums.mjs';
import { Encryption_Type, ParticipantInfo, ParticipantInfo_Kind, ParticipantInfo_State, Room$1, TrackInfo } from './protobuf-messages-room.mjs';
import { ClientInfo_Capability } from './protobuf-messages-chat.mjs';
import { LeaveRequest, LeaveRequest_Action } from './protobuf-messages-signal.mjs';
import { SimulateScenario } from './protobuf-messages-signal2.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { CLIENT_PROTOCOL_DEFAULT, TypedPromise, getBrowser } from './browser.mjs';
import { ConnectionError, ConnectionErrorReason, CriticalTimers, UnexpectedConnectionState, UnsupportedServer, canFailOverToAnotherRegion } from './errors.mjs';
import { EngineEvent, ParticipantEvent, RoomEvent, TrackEvent } from './event-enums.mjs';
import { getNewAudioContext, kindToSource, sourceToKind, summarizeStatsReport } from './util.mjs';
import { Track, isBrowserSupported, isCloud, isReactNative, isSafariBased, isWeb, sleep, supportsSetSinkId, unpackStreamId } from './track.mjs';
import { Future, createDummyVideoStreamTrack, ensureTrailingSlash, extractChatMessage, extractTrackSid, extractTranscriptionSegments, getDisconnectReasonFromConnectionError, getEmptyAudioStreamTrack, isDeflateRawCompressionSupported, isLocalAudioTrack, isLocalParticipant, isLocalVideoTrack, isRemotePub, numberToBigInt, releaseEmptyAudioStreamTrack, toHttpUrl, unwrapConstraint } from './utils-misc.mjs';
import { EncryptionEvent } from './e2ee-crypto.mjs';
import { isFrameMetadataSupported } from './e2ee-frame.mjs';
import { E2EEManager, FrameMetadataManager } from './e2ee-manager.mjs';
import { BackOffStrategy, DeviceManager } from './device.mjs';
import { DataTrackInfo } from './data-track-core.mjs';
import { audioDefaults, publishDefaults, roomConnectOptionDefaults, roomOptionDefaults, videoDefaults } from './pc-transport.mjs';
import { DataChannelKind } from './data-channel.mjs';
import { LocalAudioTrack } from './local-track.mjs';
import { LocalVideoTrack } from './local-video-track.mjs';
import { DEFAULT_MAX_AGE_MS, RTCEngine } from './rtc-engine.mjs';
import { IncomingDataStreamManager, OutgoingDataStreamManager, RegionUrlProvider } from './region-streams.mjs';
import { IncomingDataTrackManager } from './data-track.mjs';
import { OutgoingDataTrackManager } from './data-track-out.mjs';
import { RPC_REQUEST_DATA_STREAM_TOPIC, RPC_RESPONSE_DATA_STREAM_TOPIC, RpcClientManager, RpcError, RpcServerManager } from './rpc.mjs';
import { LocalTrackPublication } from './publications.mjs';
import { LocalParticipant } from './local-participant.mjs';
import { RemoteParticipant } from './remote-participant.mjs';

var ConnectionState;
(function (ConnectionState) {
  ConnectionState["Disconnected"] = "disconnected";
  ConnectionState["Connecting"] = "connecting";
  ConnectionState["Connected"] = "connected";
  ConnectionState["Reconnecting"] = "reconnecting";
  ConnectionState["SignalReconnecting"] = "signalReconnecting";
})(ConnectionState || (ConnectionState = {}));
const CONNECTION_RECONCILE_FREQUENCY_MS = 4 * 1000;
const STATS_LOG_FREQUENCY_MS = 30 * 1000;
const DUMMY_AUDIO_ELEMENT_ID = 'livekit-dummy-audio-el';
/**
 * One silent audio element serves every room on the page, so it is reference counted: a room
 * disconnecting must not pull it out from under the rooms still playing through it. The element
 * and its track go away once the last room hands its reference back.
 */
let sharedDummyAudioElement;
function createSharedDummyAudioElement() {
  const element = document.createElement('audio');
  element.id = DUMMY_AUDIO_ELEMENT_ID;
  element.autoplay = true;
  element.hidden = true;
  const track = getEmptyAudioStreamTrack();
  track.enabled = true;
  const stream = new MediaStream([track]);
  element.srcObject = stream;
  document.body.append(element);
  return {
    element,
    stream,
    track,
    refCount: 0
  };
}
/**
 * In LiveKit, a room is the logical grouping for a list of participants.
 * Participants in a room can publish tracks, and subscribe to others' tracks.
 *
 * a Room fires [[RoomEvent | RoomEvents]].
 *
 * @noInheritDoc
 */
class Room extends eventsExports.EventEmitter {
  get hasE2EESetup() {
    return this.e2eeManager !== undefined;
  }
  /**
   * Creates a new Room, the primary construct for a LiveKit session.
   * @param options
   */
  constructor(options) {
    var _this;
    var _a, _b, _c, _d, _e, _f, _g;
    super();
    _this = this;
    this.state = ConnectionState.Disconnected;
    /**
     * list of participants that are actively speaking. when this changes
     * a [[RoomEvent.ActiveSpeakersChanged]] event is fired
     */
    this.activeSpeakers = [];
    /** reflects the sender encryption status of the local participant */
    this.isE2EEEnabled = false;
    this.audioEnabled = true;
    /**
     * whether `audioContext` was created by the SDK and therefore has to be closed by it.
     * A context handed in through `webAudioMix` is owned by the application instead.
     */
    this.ownsAudioContext = false;
    /**
     * set once the audio context has been released as part of a disconnect. Guards against
     * creating a new context outside of a connection lifecycle, where nothing would close it again.
     */
    this.audioContextReleased = false;
    /**
     * serialises `acquireAudioContext` against `releaseAudioContext`. Both await per-track graph
     * rebuilds, so without it a reconnect landing inside a disconnect's teardown leaves the two
     * racing to set the participants' context, and the loser's value is the one that sticks.
     */
    this.audioContextMutex = new _();
    /** silent audio tracks this room acquired, handed back to the shared source on disconnect */
    this.acquiredEmptyAudioTracks = new Set();
    this.e2eeStateMutex = new _();
    this.isVideoPlaybackBlocked = false;
    this.log = livekitLogger;
    this.statsLog = livekitLogger;
    this.bufferedEvents = [];
    this.isResuming = false;
    this.pendingTrackAddedCallbacks = new Map();
    this.connect = (url, token, opts) => __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!isBrowserSupported()) {
        if (isReactNative()) {
          throw Error("WebRTC isn't detected, have you called registerGlobals?");
        } else {
          throw Error("LiveKit doesn't seem to be supported on this browser. Try to update your browser and make sure no browser extensions are disabling webRTC.");
        }
      }
      // In case a disconnect called happened right before the connect call, make sure the disconnect is completed first by awaiting its lock
      const unlockDisconnect = yield this.disconnectLock.lock();
      if (this.state === ConnectionState.Connected) {
        // when the state is reconnecting or connected, this function returns immediately
        this.log.info("already connected to room ".concat(this.name));
        unlockDisconnect();
        return Promise.resolve();
      }
      if (this.connectFuture) {
        unlockDisconnect();
        return this.connectFuture.promise;
      }
      this.setAndEmitConnectionState(ConnectionState.Connecting);
      if (((_a = this.regionUrlProvider) === null || _a === void 0 ? void 0 : _a.getServerUrl().toString()) !== ensureTrailingSlash(url)) {
        this.regionUrl = undefined;
        this.regionUrlProvider = undefined;
      }
      if (isCloud(new URL(url))) {
        if (this.regionUrlProvider === undefined) {
          this.regionUrlProvider = new RegionUrlProvider(url, token);
        } else {
          this.regionUrlProvider.updateToken(token);
        }
        // trigger the first fetch without waiting for a response
        // if initial connection fails, this will speed up picking regional url
        // on subsequent runs
        this.regionUrlProvider.fetchRegionSettings().then(settings => {
          var _a;
          (_a = this.regionUrlProvider) === null || _a === void 0 ? void 0 : _a.setServerReportedRegions(settings);
        }).catch(e => {
          this.log.warn('could not fetch region settings', {
            error: e
          });
        });
      }
      const connectFn = (resolve, reject, regionUrl) => __awaiter(this, void 0, void 0, function* () {
        var _a, _b;
        if (this.abortController) {
          this.abortController.abort();
        }
        // explicit creation as local var needed to satisfy TS compiler when passing it to `attemptConnection` further down
        const abortController = new AbortController();
        this.abortController = abortController;
        // at this point the intention to connect has been signalled so we can allow cancelling of the connection via disconnect() again
        unlockDisconnect === null || unlockDisconnect === void 0 ? void 0 : unlockDisconnect();
        try {
          yield BackOffStrategy.getInstance().getBackOffPromise(url);
          if (abortController.signal.aborted) {
            throw ConnectionError.cancelled('Connection attempt aborted');
          }
          yield this.attemptConnection(regionUrl !== null && regionUrl !== void 0 ? regionUrl : url, token, opts, abortController);
          this.abortController = undefined;
          resolve();
        } catch (error) {
          if (this.regionUrlProvider && error instanceof ConnectionError && canFailOverToAnotherRegion(error)) {
            let nextUrl = null;
            try {
              this.log.debug('Fetching next region');
              nextUrl = yield this.regionUrlProvider.getNextBestRegionUrl((_a = this.abortController) === null || _a === void 0 ? void 0 : _a.signal);
            } catch (regionFetchError) {
              if (regionFetchError instanceof ConnectionError && (regionFetchError.status === 401 || regionFetchError.reason === ConnectionErrorReason.Cancelled)) {
                this.handleDisconnect(this.options.stopLocalTrackOnUnpublish);
                reject(regionFetchError);
                return;
              }
            }
            if (
            // making sure we only register failed attempts on things we actually care about
            [ConnectionErrorReason.InternalError, ConnectionErrorReason.ServerUnreachable, ConnectionErrorReason.Timeout].includes(error.reason)) {
              this.log.debug('Adding failed connection attempt to back off');
              BackOffStrategy.getInstance().addFailedConnectionAttempt(url);
            }
            if (nextUrl && !((_b = this.abortController) === null || _b === void 0 ? void 0 : _b.signal.aborted)) {
              this.log.info("Initial connection failed with ConnectionError: ".concat(error.message, ". Retrying with another region: ").concat(nextUrl));
              this.recreateEngine(true);
              yield connectFn(resolve, reject, nextUrl);
            } else {
              this.handleDisconnect(this.options.stopLocalTrackOnUnpublish, getDisconnectReasonFromConnectionError(error));
              reject(error);
            }
          } else {
            let disconnectReason = DisconnectReason.UNKNOWN_REASON;
            if (error instanceof ConnectionError) {
              disconnectReason = getDisconnectReasonFromConnectionError(error);
            }
            this.handleDisconnect(this.options.stopLocalTrackOnUnpublish, disconnectReason);
            reject(error);
          }
        }
      });
      const regionUrl = this.regionUrl;
      this.regionUrl = undefined;
      this.connectFuture = new Future((resolve, reject) => {
        connectFn(resolve, reject, regionUrl);
      }, () => {
        this.clearConnectionFutures();
      });
      return this.connectFuture.promise;
    });
    this.connectSignal = (url, token, engine, connectOptions, roomOptions, abortController) => __awaiter(this, void 0, void 0, function* () {
      const _yield$engine$join = yield engine.join(url, token, {
          autoSubscribe: connectOptions.autoSubscribe,
          adaptiveStream: typeof roomOptions.adaptiveStream === 'object' ? true : roomOptions.adaptiveStream,
          disableIceLite: connectOptions.disableIceLite,
          subscriberAllowPause: connectOptions.subscriberAllowPause,
          clientInfoCapabilities: this.getClientInfoCapabilities(roomOptions),
          maxRetries: connectOptions.maxRetries,
          e2eeEnabled: !!this.e2eeManager,
          websocketTimeout: connectOptions.websocketTimeout
        }, abortController.signal, !roomOptions.singlePeerConnection),
        joinResponse = _yield$engine$join.joinResponse,
        serverInfo = _yield$engine$join.serverInfo;
      this.serverInfo = serverInfo;
      if (!serverInfo.version) {
        throw new UnsupportedServer('unknown server version');
      }
      if (serverInfo.version === '0.15.1' && this.options.dynacast) {
        this.log.debug('disabling dynacast due to server version');
        // dynacast has a bug in 0.15.1, so we cannot use it then
        roomOptions.dynacast = false;
      }
      return joinResponse;
    });
    this.applyJoinResponse = joinResponse => {
      const pi = joinResponse.participant;
      this.localParticipant.sid = pi.sid;
      this.localParticipant.identity = pi.identity;
      this.localParticipant.setEnabledPublishCodecs(joinResponse.enabledPublishCodecs);
      if (this.e2eeManager) {
        try {
          this.e2eeManager.setSifTrailer(joinResponse.sifTrailer);
        } catch (e) {
          this.log.error(e instanceof Error ? e.message : 'Could not set SifTrailer', {
            error: e
          });
        }
      }
      // populate remote participants, these should not trigger new events
      this.handleParticipantUpdates([pi, ...joinResponse.otherParticipants]);
      if (joinResponse.room) {
        this.handleRoomUpdate(joinResponse.room);
      }
    };
    this.attemptConnection = (url, token, opts, abortController) => __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      if (this.state === ConnectionState.Reconnecting || this.isResuming || ((_a = this.engine) === null || _a === void 0 ? void 0 : _a.pendingReconnect)) {
        this.log.info('Reconnection attempt replaced by new connection attempt');
        // make sure we close and recreate the existing engine in order to get rid of any potentially ongoing reconnection attempts
        this.recreateEngine(true);
      } else {
        // create engine if previously disconnected
        this.maybeCreateEngine();
      }
      if ((_b = this.regionUrlProvider) === null || _b === void 0 ? void 0 : _b.isCloud()) {
        this.engine.setRegionStrategy(this.createRegionStrategy());
      }
      this.acquireAudioContext(true).catch(error => this.log.warn('Could not acquire audio context', Object.assign(Object.assign({}, this.logContext), {
        error
      })));
      this.connOptions = Object.assign(Object.assign({}, roomConnectOptionDefaults), opts);
      if (this.connOptions.rtcConfig) {
        this.engine.rtcConfig = this.connOptions.rtcConfig;
      }
      if (this.connOptions.peerConnectionTimeout) {
        this.engine.peerConnectionTimeout = this.connOptions.peerConnectionTimeout;
      }
      try {
        const joinResponse = yield this.connectSignal(url, token, this.engine, this.connOptions, this.options, abortController);
        this.applyJoinResponse(joinResponse);
        // forward metadata changed for the local participant
        this.setupLocalParticipantEvents();
        this.emit(RoomEvent.SignalConnected);
      } catch (err) {
        yield this.engine.close();
        this.recreateEngine();
        const resultingError = abortController.signal.aborted ? ConnectionError.cancelled('Signal connection aborted') : ConnectionError.serverUnreachable('could not establish signal connection');
        if (err instanceof Error) {
          resultingError.message = "".concat(resultingError.message, ": ").concat(err.message);
        }
        if (err instanceof ConnectionError) {
          resultingError.reason = err.reason;
          resultingError.status = err.status;
        }
        this.log.debug("error trying to establish signal connection", {
          error: err
        });
        throw resultingError;
      }
      if (abortController.signal.aborted) {
        yield this.engine.close();
        this.recreateEngine();
        throw ConnectionError.cancelled("Connection attempt aborted");
      }
      try {
        yield this.engine.waitForPCInitialConnection(this.connOptions.peerConnectionTimeout, abortController);
      } catch (e) {
        yield this.engine.close();
        this.recreateEngine();
        throw e;
      }
      // also hook unload event
      if (isWeb() && this.options.disconnectOnPageLeave) {
        // capturing both 'pagehide' and 'beforeunload' to capture broadest set of browser behaviors
        window.addEventListener('pagehide', this.onPageLeave);
        window.addEventListener('beforeunload', this.onPageLeave);
      }
      if (isWeb()) {
        window.addEventListener('freeze', this.onPageLeave);
      }
      this.setAndEmitConnectionState(ConnectionState.Connected);
      this.emit(RoomEvent.Connected);
      BackOffStrategy.getInstance().resetFailedConnectionAttempts(url);
      this.registerConnectionReconcile();
      // Notify region provider about successful connection
      if (this.regionUrlProvider) {
        this.regionUrlProvider.notifyConnected();
      }
    });
    /**
     * disconnects the room, emits [[RoomEvent.Disconnected]]
     */
    this.disconnect = function () {
      for (var _len = arguments.length, args_1 = new Array(_len), _key = 0; _key < _len; _key++) {
        args_1[_key] = arguments[_key];
      }
      return __awaiter(_this, [...args_1], void 0, function () {
        var _this2 = this;
        let stopTracks = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
        return function* () {
          var _a, _b, _c;
          const unlock = yield _this2.disconnectLock.lock();
          try {
            if (_this2.state === ConnectionState.Disconnected) {
              _this2.log.debug('already disconnected');
              return;
            }
            _this2.log.info('disconnect from room');
            if (_this2.state === ConnectionState.Connecting || _this2.state === ConnectionState.Reconnecting || _this2.isResuming) {
              // try aborting pending connection attempt
              const msg = 'Abort connection attempt due to user initiated disconnect';
              _this2.log.warn(msg);
              (_a = _this2.abortController) === null || _a === void 0 ? void 0 : _a.abort(msg);
              // in case the abort controller didn't manage to cancel the connection attempt, reject the connect promise explicitly
              (_c = (_b = _this2.connectFuture) === null || _b === void 0 ? void 0 : _b.reject) === null || _c === void 0 ? void 0 : _c.call(_b, ConnectionError.cancelled('Client initiated disconnect'));
              _this2.connectFuture = undefined;
            }
            // close engine (also closes client)
            if (_this2.engine) {
              // send leave
              if (!_this2.engine.client.isDisconnected) {
                yield _this2.engine.client.sendLeave();
              }
              yield _this2.engine.close();
            }
            _this2.handleDisconnect(stopTracks, DisconnectReason.CLIENT_INITIATED);
            /* @ts-ignore */
            _this2.engine = undefined;
          } finally {
            unlock();
          }
        }();
      });
    };
    this.onPageLeave = () => __awaiter(this, void 0, void 0, function* () {
      this.log.info('Page leave detected, disconnecting');
      yield this.disconnect();
    });
    /**
     * Browsers have different policies regarding audio playback. Most requiring
     * some form of user interaction (click/tap/etc).
     * In those cases, audio will be silent until a click/tap triggering one of the following
     * - `startAudio`
     * - `getUserMedia`
     */
    this.startAudio = () => __awaiter(this, void 0, void 0, function* () {
      const elements = [];
      const browser = getBrowser();
      if (browser && browser.os === 'iOS' && !this.audioContextReleased) {
        elements.push(this.acquireDummyAudioElement());
      }
      this.remoteParticipants.forEach(p => {
        p.audioTrackPublications.forEach(t => {
          if (t.track) {
            t.track.attachedElements.forEach(e => {
              elements.push(e);
            });
          }
        });
      });
      try {
        yield Promise.all([
        // once the audio context has been released as part of a disconnect there is nothing left
        // to unlock, and creating a new context here would leak it as no disconnect will follow
        ...(this.audioContextReleased ? [] : [this.acquireAudioContext()]), ...elements.map(e => {
          // when webAudioMix is enabled, attached elements are deliberately kept muted by
          // RemoteAudioTrack.attach() and audio is routed through the web audio graph instead.
          // unmuting them here would cause double playback on platforms where element.volume
          // has no effect (e.g. iOS Safari)
          if (!this.options.webAudioMix) {
            e.muted = false;
          }
          return e.play();
        })]);
        this.handleAudioPlaybackStarted();
      } catch (err) {
        this.handleAudioPlaybackFailed(err);
        throw err;
      }
    });
    this.startVideo = () => __awaiter(this, void 0, void 0, function* () {
      const elements = [];
      for (const p of this.remoteParticipants.values()) {
        p.videoTrackPublications.forEach(tr => {
          var _a;
          (_a = tr.track) === null || _a === void 0 ? void 0 : _a.attachedElements.forEach(el => {
            if (!elements.includes(el)) {
              elements.push(el);
            }
          });
        });
      }
      yield Promise.all(elements.map(el => el.play())).then(() => {
        this.handleVideoPlaybackStarted();
      }).catch(e => {
        if (e.name === 'NotAllowedError') {
          this.handleVideoPlaybackFailed();
        } else {
          this.log.warn('Resuming video playback failed, make sure you call `startVideo` directly in a user gesture handler');
        }
      });
    });
    this.handleRestarting = () => {
      this.clearConnectionReconcile();
      // in case we went from resuming to full-reconnect, make sure to reflect it on the isResuming flag
      this.isResuming = false;
      // also unwind existing participants & existing subscriptions
      for (const p of this.remoteParticipants.values()) {
        this.handleParticipantDisconnected(p.identity, p);
      }
      if (this.setAndEmitConnectionState(ConnectionState.Reconnecting)) {
        this.emit(RoomEvent.Reconnecting);
      }
    };
    this.handleRestarted = () => {
      this.outgoingDataTrackManager.sfuWillRepublishTracks();
      this.incomingDataTrackManager.resendSubscriptionUpdates();
    };
    this.handleSignalRestarted = joinResponse => __awaiter(this, void 0, void 0, function* () {
      this.log.debug("signal reconnected to server, region ".concat(joinResponse.serverRegion), {
        region: joinResponse.serverRegion
      });
      this.bufferedEvents = [];
      this.applyJoinResponse(joinResponse);
      try {
        // unpublish & republish tracks
        yield this.localParticipant.republishAllTracks(undefined, true);
      } catch (error) {
        this.log.error('error trying to re-publish tracks after reconnection', {
          error
        });
      }
      try {
        yield this.engine.waitForRestarted();
        this.log.debug("fully reconnected to server", {
          region: joinResponse.serverRegion
        });
      } catch (_a) {
        // reconnection failed, handleDisconnect is being invoked already, just return here
        return;
      }
      this.setAndEmitConnectionState(ConnectionState.Connected);
      this.emit(RoomEvent.Reconnected);
      this.registerConnectionReconcile();
      this.emitBufferedEvents();
    });
    this.handleParticipantUpdates = participantInfos => {
      var _a;
      // handle changes to participant state, and send events
      for (const info of participantInfos) {
        if (info.identity === this.localParticipant.identity) {
          this.localParticipant.updateInfo(info);
          continue;
        }
        // LiveKit server doesn't send identity info prior to version 1.5.2 in disconnect updates
        // so we try to map an empty identity to an already known sID manually
        if (info.identity === '') {
          info.identity = (_a = this.sidToIdentity.get(info.sid)) !== null && _a !== void 0 ? _a : '';
        }
        let remoteParticipant = this.remoteParticipants.get(info.identity);
        // when it's disconnected, send updates
        if (info.state === ParticipantInfo_State.DISCONNECTED) {
          this.handleParticipantDisconnected(info.identity, remoteParticipant, info.disconnectReason === DisconnectReason.UNKNOWN_REASON ? undefined : info.disconnectReason);
        } else {
          // create participant if doesn't exist
          this.getOrCreateParticipant(info.identity, info);
        }
      }
      // Ingest data track publication updates into data tracks infrastructure
      const mapped = new Map(participantInfos.filter(p => p.identity !== this.localParticipant.identity).map(info => {
        return [info.identity, info.dataTracks.map(dataTrack => DataTrackInfo.from(dataTrack))];
      }));
      this.incomingDataTrackManager.receiveSfuPublicationUpdates(mapped);
    };
    // updates are sent only when there's a change to speaker ordering
    this.handleActiveSpeakersUpdate = speakers => {
      const activeSpeakers = [];
      const seenSids = {};
      speakers.forEach(speaker => {
        seenSids[speaker.sid] = true;
        if (speaker.sid === this.localParticipant.sid) {
          this.localParticipant.audioLevel = speaker.level;
          this.localParticipant.setIsSpeaking(true);
          activeSpeakers.push(this.localParticipant);
        } else {
          const p = this.getRemoteParticipantBySid(speaker.sid);
          if (p) {
            p.audioLevel = speaker.level;
            p.setIsSpeaking(true);
            activeSpeakers.push(p);
          }
        }
      });
      if (!seenSids[this.localParticipant.sid]) {
        this.localParticipant.audioLevel = 0;
        this.localParticipant.setIsSpeaking(false);
      }
      this.remoteParticipants.forEach(p => {
        if (!seenSids[p.sid]) {
          p.audioLevel = 0;
          p.setIsSpeaking(false);
        }
      });
      this.activeSpeakers = activeSpeakers;
      this.emitWhenConnected(RoomEvent.ActiveSpeakersChanged, activeSpeakers);
    };
    // process list of changed speakers
    this.handleSpeakersChanged = speakerUpdates => {
      const lastSpeakers = new Map();
      this.activeSpeakers.forEach(p => {
        const remoteParticipant = this.remoteParticipants.get(p.identity);
        if (remoteParticipant && remoteParticipant.sid !== p.sid) {
          return;
        }
        lastSpeakers.set(p.sid, p);
      });
      speakerUpdates.forEach(speaker => {
        let p = this.getRemoteParticipantBySid(speaker.sid);
        if (speaker.sid === this.localParticipant.sid) {
          p = this.localParticipant;
        }
        if (!p) {
          return;
        }
        p.audioLevel = speaker.level;
        p.setIsSpeaking(speaker.active);
        if (speaker.active) {
          lastSpeakers.set(speaker.sid, p);
        } else {
          lastSpeakers.delete(speaker.sid);
        }
      });
      const activeSpeakers = Array.from(lastSpeakers.values());
      activeSpeakers.sort((a, b) => b.audioLevel - a.audioLevel);
      this.activeSpeakers = activeSpeakers;
      this.emitWhenConnected(RoomEvent.ActiveSpeakersChanged, activeSpeakers);
    };
    this.handleStreamStateUpdate = streamStateUpdate => {
      streamStateUpdate.streamStates.forEach(streamState => {
        const participant = this.getRemoteParticipantBySid(streamState.participantSid);
        if (!participant) {
          return;
        }
        const pub = participant.getTrackPublicationBySid(streamState.trackSid);
        if (!pub || !pub.track) {
          return;
        }
        const newStreamState = Track.streamStateFromProto(streamState.state);
        const prevStreamState = pub.track.streamState;
        pub.track.setStreamState(newStreamState);
        if (newStreamState !== prevStreamState) {
          participant.emit(ParticipantEvent.TrackStreamStateChanged, pub, pub.track.streamState);
          this.emitWhenConnected(RoomEvent.TrackStreamStateChanged, pub, pub.track.streamState, participant);
        }
      });
    };
    this.handleSubscriptionPermissionUpdate = update => {
      const participant = this.getRemoteParticipantBySid(update.participantSid);
      if (!participant) {
        return;
      }
      const pub = participant.getTrackPublicationBySid(update.trackSid);
      if (!pub) {
        return;
      }
      pub.setAllowed(update.allowed);
    };
    this.handleSubscriptionError = update => {
      this.cancelPendingTrackAdded(update.trackSid);
      const participant = Array.from(this.remoteParticipants.values()).find(p => p.trackPublications.has(update.trackSid));
      if (!participant) {
        return;
      }
      const pub = participant.getTrackPublicationBySid(update.trackSid);
      if (!pub) {
        return;
      }
      pub.setSubscriptionError(update.err);
    };
    this.handleDataPacket = (packet, encryptionType) => {
      // find the participant
      const participant = this.remoteParticipants.get(packet.participantIdentity);
      if (packet.value.case === 'user') {
        this.handleUserPacket(participant, packet.value.value, packet.kind, encryptionType, packet.participantIdentity);
      } else if (packet.value.case === 'transcription') {
        this.handleTranscription(participant, packet.value.value);
      } else if (packet.value.case === 'sipDtmf') {
        this.handleSipDtmf(participant, packet.value.value);
      } else if (packet.value.case === 'chatMessage') {
        this.handleChatMessage(participant, packet.value.value);
      } else if (packet.value.case === 'metrics') {
        this.handleMetrics(packet.value.value, participant);
      } else if (packet.value.case === 'streamHeader' || packet.value.case === 'streamChunk' || packet.value.case === 'streamTrailer') {
        this.handleDataStream(packet, encryptionType);
      } else if (packet.value.case === 'rpcRequest') {
        const rpc = packet.value.value;
        this.rpcServerManager.handleIncomingRpcRequest(packet.participantIdentity, rpc);
      } else if (packet.value.case === 'rpcResponse') {
        const rpcResponse = packet.value.value;
        switch (rpcResponse.value.case) {
          case 'payload':
            this.rpcClientManager.handleIncomingRpcResponseSuccess(rpcResponse.requestId, rpcResponse.value.value);
            break;
          case 'error':
            this.rpcClientManager.handleIncomingRpcResponseFailure(rpcResponse.requestId, RpcError.fromProto(rpcResponse.value.value));
            break;
          default:
            this.log.warn("Unknown rpcResponse.value.case: ".concat(rpcResponse.value.case), this.logContext);
            break;
        }
      } else if (packet.value.case === 'rpcAck') {
        this.rpcClientManager.handleIncomingRpcAck(packet.value.value.requestId);
      }
    };
    this.handleUserPacket = (participant, userPacket, kind, encryptionType, participantIdentity) => {
      this.emit(RoomEvent.DataReceived, userPacket.payload, participant, kind, userPacket.topic, encryptionType, participantIdentity);
      // also emit on the participant
      participant === null || participant === void 0 ? void 0 : participant.emit(ParticipantEvent.DataReceived, userPacket.payload, kind, encryptionType);
    };
    this.handleSipDtmf = (participant, dtmf) => {
      this.emit(RoomEvent.SipDTMFReceived, dtmf, participant);
      // also emit on the participant
      participant === null || participant === void 0 ? void 0 : participant.emit(ParticipantEvent.SipDTMFReceived, dtmf);
    };
    this.handleTranscription = (_remoteParticipant, transcription) => {
      // find the participant
      const participant = transcription.transcribedParticipantIdentity === this.localParticipant.identity ? this.localParticipant : this.getParticipantByIdentity(transcription.transcribedParticipantIdentity);
      const publication = participant === null || participant === void 0 ? void 0 : participant.trackPublications.get(transcription.trackId);
      const segments = extractTranscriptionSegments(transcription, this.transcriptionReceivedTimes);
      publication === null || publication === void 0 ? void 0 : publication.emit(TrackEvent.TranscriptionReceived, segments);
      participant === null || participant === void 0 ? void 0 : participant.emit(ParticipantEvent.TranscriptionReceived, segments, publication);
      this.emit(RoomEvent.TranscriptionReceived, segments, participant, publication);
    };
    this.handleChatMessage = (participant, chatMessage) => {
      const msg = extractChatMessage(chatMessage);
      this.emit(RoomEvent.ChatMessage, msg, participant);
    };
    this.handleMetrics = (metrics, participant) => {
      this.emit(RoomEvent.MetricsReceived, metrics, participant);
    };
    this.handleDataStream = (packet, encryptionType) => {
      this.incomingDataStreamManager.handleDataStreamPacket(packet, encryptionType);
    };
    this.bufferedSegments = new Map();
    this.handleAudioPlaybackStarted = () => {
      if (this.canPlaybackAudio) {
        return;
      }
      this.audioEnabled = true;
      this.emit(RoomEvent.AudioPlaybackStatusChanged, true);
    };
    this.handleAudioPlaybackFailed = e => {
      this.log.warn('could not playback audio', {
        error: e
      });
      if (!this.canPlaybackAudio) {
        return;
      }
      this.audioEnabled = false;
      this.emit(RoomEvent.AudioPlaybackStatusChanged, false);
    };
    this.handleVideoPlaybackStarted = () => {
      if (this.isVideoPlaybackBlocked) {
        this.isVideoPlaybackBlocked = false;
        this.emit(RoomEvent.VideoPlaybackStatusChanged, true);
      }
    };
    this.handleVideoPlaybackFailed = () => {
      if (!this.isVideoPlaybackBlocked) {
        this.isVideoPlaybackBlocked = true;
        this.emit(RoomEvent.VideoPlaybackStatusChanged, false);
      }
    };
    this.handleDeviceChange = () => __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (((_a = getBrowser()) === null || _a === void 0 ? void 0 : _a.os) !== 'iOS') {
        // default devices are non deterministic on iOS, so we don't attempt to select them here
        yield this.selectDefaultDevices();
      }
      this.emit(RoomEvent.MediaDevicesChanged);
    });
    this.handleRoomUpdate = room => {
      const oldRoom = this.roomInfo;
      this.roomInfo = room;
      if (oldRoom && oldRoom.metadata !== room.metadata) {
        this.emitWhenConnected(RoomEvent.RoomMetadataChanged, room.metadata);
      }
      if ((oldRoom === null || oldRoom === void 0 ? void 0 : oldRoom.activeRecording) !== room.activeRecording) {
        this.emitWhenConnected(RoomEvent.RecordingStatusChanged, room.activeRecording);
      }
    };
    this.handleConnectionQualityUpdate = update => {
      update.updates.forEach(info => {
        if (info.participantSid === this.localParticipant.sid) {
          this.localParticipant.setConnectionQuality(info.quality);
          return;
        }
        const participant = this.getRemoteParticipantBySid(info.participantSid);
        if (participant) {
          participant.setConnectionQuality(info.quality);
        }
      });
    };
    this.getRemoteParticipantClientProtocol = identity => {
      var _a, _b;
      return (_b = (_a = this.remoteParticipants.get(identity)) === null || _a === void 0 ? void 0 : _a.clientProtocol) !== null && _b !== void 0 ? _b : CLIENT_PROTOCOL_DEFAULT;
    };
    this.getRemoteParticipantCapabilities = identity => {
      var _a, _b;
      return (_b = (_a = this.remoteParticipants.get(identity)) === null || _a === void 0 ? void 0 : _a.capabilities) !== null && _b !== void 0 ? _b : [];
    };
    this.getAllRemoteParticipantIdentities = () => {
      return Array.from(this.remoteParticipants.keys());
    };
    /**
     * Dumps stats of both peer connections.
     */
    this.logWebRTCStats = () => __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d;
      const pcManager = (_a = this.engine) === null || _a === void 0 ? void 0 : _a.pcManager;
      if (!pcManager) {
        return;
      }
      try {
        const _yield$Promise$all = yield Promise.all([pcManager.publisher.getStats(), (_b = pcManager.subscriber) === null || _b === void 0 ? void 0 : _b.getStats()]),
          _yield$Promise$all2 = _slicedToArray(_yield$Promise$all, 2),
          publisher = _yield$Promise$all2[0],
          subscriber = _yield$Promise$all2[1];
        const publisherStats = publisher && summarizeStatsReport(publisher);
        const subscriberStats = subscriber && summarizeStatsReport(subscriber);
        this.statsLog.info("webrtc stats", {
          publisher: publisherStats === null || publisherStats === void 0 ? void 0 : publisherStats.connection,
          subscriber: subscriberStats === null || subscriberStats === void 0 ? void 0 : subscriberStats.connection,
          inbound: [...((_c = publisherStats === null || publisherStats === void 0 ? void 0 : publisherStats.inbound) !== null && _c !== void 0 ? _c : []), ...((_d = subscriberStats === null || subscriberStats === void 0 ? void 0 : subscriberStats.inbound) !== null && _d !== void 0 ? _d : [])],
          outbound: publisherStats === null || publisherStats === void 0 ? void 0 : publisherStats.outbound
        });
      } catch (error) {
        this.statsLog.debug('could not collect webrtc stats', {
          error
        });
      }
    });
    this.onLocalParticipantMetadataChanged = metadata => {
      this.emit(RoomEvent.ParticipantMetadataChanged, metadata, this.localParticipant);
    };
    this.onLocalParticipantNameChanged = name => {
      this.emit(RoomEvent.ParticipantNameChanged, name, this.localParticipant);
    };
    this.onLocalAttributesChanged = changedAttributes => {
      this.emit(RoomEvent.ParticipantAttributesChanged, changedAttributes, this.localParticipant);
    };
    this.onLocalTrackMuted = pub => {
      this.emit(RoomEvent.TrackMuted, pub, this.localParticipant);
    };
    this.onLocalTrackUnmuted = pub => {
      this.emit(RoomEvent.TrackUnmuted, pub, this.localParticipant);
    };
    this.onTrackProcessorUpdate = processor => {
      var _a;
      (_a = processor === null || processor === void 0 ? void 0 : processor.onPublish) === null || _a === void 0 ? void 0 : _a.call(processor, this);
    };
    this.onLocalTrackPublished = pub => __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e, _f;
      (_a = pub.track) === null || _a === void 0 ? void 0 : _a.on(TrackEvent.TrackProcessorUpdate, this.onTrackProcessorUpdate);
      (_b = pub.track) === null || _b === void 0 ? void 0 : _b.on(TrackEvent.Restarted, this.onLocalTrackRestarted);
      (_e = (_d = (_c = pub.track) === null || _c === void 0 ? void 0 : _c.getProcessor()) === null || _d === void 0 ? void 0 : _d.onPublish) === null || _e === void 0 ? void 0 : _e.call(_d, this);
      this.emit(RoomEvent.LocalTrackPublished, pub, this.localParticipant);
      if (isLocalAudioTrack(pub.track)) {
        const trackIsSilent = yield pub.track.checkForSilence();
        if (trackIsSilent) {
          this.emit(RoomEvent.LocalAudioSilenceDetected, pub);
        }
      }
      const deviceId = yield (_f = pub.track) === null || _f === void 0 ? void 0 : _f.getDeviceId(false);
      const deviceKind = sourceToKind(pub.source);
      if (deviceKind && deviceId && deviceId !== this.localParticipant.activeDeviceMap.get(deviceKind)) {
        this.localParticipant.activeDeviceMap.set(deviceKind, deviceId);
        this.emit(RoomEvent.ActiveDeviceChanged, deviceKind, deviceId);
      }
    });
    this.onLocalTrackUnpublished = pub => {
      var _a, _b;
      (_a = pub.track) === null || _a === void 0 ? void 0 : _a.off(TrackEvent.TrackProcessorUpdate, this.onTrackProcessorUpdate);
      (_b = pub.track) === null || _b === void 0 ? void 0 : _b.off(TrackEvent.Restarted, this.onLocalTrackRestarted);
      this.emit(RoomEvent.LocalTrackUnpublished, pub, this.localParticipant);
    };
    this.onLocalTrackRestarted = track => __awaiter(this, void 0, void 0, function* () {
      const deviceId = yield track.getDeviceId(false);
      const deviceKind = sourceToKind(track.source);
      if (deviceKind && deviceId && deviceId !== this.localParticipant.activeDeviceMap.get(deviceKind)) {
        this.log.debug("local track restarted, setting ".concat(deviceKind, " ").concat(deviceId, " active"));
        this.localParticipant.activeDeviceMap.set(deviceKind, deviceId);
        this.emit(RoomEvent.ActiveDeviceChanged, deviceKind, deviceId);
      }
    });
    this.onLocalConnectionQualityChanged = quality => {
      this.emit(RoomEvent.ConnectionQualityChanged, quality, this.localParticipant);
    };
    this.onMediaDevicesError = (e, kind) => {
      this.emit(RoomEvent.MediaDevicesError, e, kind);
    };
    this.onLocalParticipantPermissionsChanged = prevPermissions => {
      this.emit(RoomEvent.ParticipantPermissionsChanged, prevPermissions, this.localParticipant);
    };
    this.onLocalChatMessageSent = msg => {
      this.emit(RoomEvent.ChatMessage, msg, this.localParticipant);
    };
    this.setMaxListeners(100);
    this.remoteParticipants = new Map();
    this.sidToIdentity = new Map();
    this.options = Object.assign(Object.assign({}, roomOptionDefaults), options);
    this.log = getLogger((_a = this.options.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Room, () => this.logContext);
    // its own logger name, so the stats dumps can be silenced or routed
    // separately from the rest of the room's logs
    this.statsLog = getLogger(LoggerNames.Stats, () => this.logContext);
    this.transcriptionReceivedTimes = new Map();
    this.options.audioCaptureDefaults = Object.assign(Object.assign({}, audioDefaults), options === null || options === void 0 ? void 0 : options.audioCaptureDefaults);
    this.options.videoCaptureDefaults = Object.assign(Object.assign({}, videoDefaults), options === null || options === void 0 ? void 0 : options.videoCaptureDefaults);
    this.options.publishDefaults = Object.assign(Object.assign({}, publishDefaults), options === null || options === void 0 ? void 0 : options.publishDefaults);
    this.maybeCreateEngine();
    this.incomingDataStreamManager = new IncomingDataStreamManager((_b = this.options.dataStream) === null || _b === void 0 ? void 0 : _b.maxPayloadByteLength);
    this.outgoingDataStreamManager = new OutgoingDataStreamManager(this.engine, this.log, this.getRemoteParticipantClientProtocol, this.getRemoteParticipantCapabilities, this.getAllRemoteParticipantIdentities);
    this.incomingDataTrackManager = new IncomingDataTrackManager({
      e2eeManager: this.e2eeManager
    });
    this.incomingDataTrackManager.on('sfuUpdateSubscription', event => {
      this.engine.client.sendUpdateDataSubscription(event.sid, event.subscribe);
    }).on('trackPublished', event => {
      var _a;
      if (event.track.publisherIdentity === this.localParticipant.identity) {
        // Only advertize tracks from other participants
        return;
      }
      this.emit(RoomEvent.DataTrackPublished, event.track);
      (_a = this.remoteParticipants.get(event.track.publisherIdentity)) === null || _a === void 0 ? void 0 : _a.addRemoteDataTrack(event.track);
    }).on('trackUnpublished', event => {
      var _a;
      if (event.publisherIdentity === this.localParticipant.identity) {
        // Only advertize tracks from other participants
        return;
      }
      this.emit(RoomEvent.DataTrackUnpublished, event.sid);
      (_a = this.remoteParticipants.get(event.publisherIdentity)) === null || _a === void 0 ? void 0 : _a.removeRemoteDataTrack(event.sid);
    });
    this.outgoingDataTrackManager = new OutgoingDataTrackManager({
      e2eeManager: this.e2eeManager
    });
    this.outgoingDataTrackManager.on('sfuPublishRequest', event => {
      this.engine.client.sendPublishDataTrackRequest(event.handle, event.name, event.usesE2ee);
    }).on('sfuUnpublishRequest', event => {
      this.engine.client.sendUnPublishDataTrackRequest(event.handle);
    }).on('trackPublished', event => {
      this.emit(RoomEvent.LocalDataTrackPublished, event.track);
    }).on('trackUnpublished', event => {
      this.emit(RoomEvent.LocalDataTrackUnpublished, event.sid);
    }).on('packetAvailable', _ref => {
      let handle = _ref.handle,
        bytes = _ref.bytes;
      this.engine.sendDataTrackFrame(bytes).finally(() => this.outgoingDataTrackManager.handlePacketSendComplete(handle));
    });
    this.registerRpcDataStreamHandler();
    this.rpcClientManager = new RpcClientManager(this.log, this.outgoingDataStreamManager, this.getRemoteParticipantClientProtocol, () => {
      var _a;
      return (_a = this.engine) === null || _a === void 0 ? void 0 : _a.serverVersion;
    });
    this.rpcClientManager.on('sendDataPacket', _ref2 => {
      let packet = _ref2.packet;
      var _a;
      (_a = this.engine) === null || _a === void 0 ? void 0 : _a.sendDataPacket(packet, DataChannelKind.RELIABLE);
    });
    this.rpcServerManager = new RpcServerManager(this.log, this.outgoingDataStreamManager, this.getRemoteParticipantClientProtocol);
    this.rpcServerManager.on('sendDataPacket', _ref3 => {
      let packet = _ref3.packet;
      var _a;
      (_a = this.engine) === null || _a === void 0 ? void 0 : _a.sendDataPacket(packet, DataChannelKind.RELIABLE);
    });
    this.disconnectLock = new _();
    this.localParticipant = new LocalParticipant('', '', this.engine, this.options, this.outgoingDataStreamManager, this.outgoingDataTrackManager, this.rpcClientManager, this.rpcServerManager);
    this.setupFrameMetadata();
    if (this.options.e2ee || this.options.encryption) {
      this.setupE2EE();
    }
    this.engine.e2eeManager = this.e2eeManager;
    this.incomingDataTrackManager.updateE2eeManager((_c = this.e2eeManager) !== null && _c !== void 0 ? _c : null);
    this.outgoingDataTrackManager.updateE2eeManager((_d = this.e2eeManager) !== null && _d !== void 0 ? _d : null);
    if (this.options.videoCaptureDefaults.deviceId) {
      this.localParticipant.activeDeviceMap.set('videoinput', unwrapConstraint(this.options.videoCaptureDefaults.deviceId));
    }
    if (this.options.audioCaptureDefaults.deviceId) {
      this.localParticipant.activeDeviceMap.set('audioinput', unwrapConstraint(this.options.audioCaptureDefaults.deviceId));
    }
    if ((_e = this.options.audioOutput) === null || _e === void 0 ? void 0 : _e.deviceId) {
      this.switchActiveDevice('audiooutput', unwrapConstraint(this.options.audioOutput.deviceId)).catch(e => this.log.warn("Could not set audio output: ".concat(e.message)));
    }
    if (isWeb()) {
      const cleanupController = new AbortController();
      let onDeviceChange;
      if (Room.cleanupRegistry) {
        // Wrap the listener in a WeakRef closure so navigator.mediaDevices does not
        // strongly retain the Room. When the user drops their Room ref, the
        // FinalizationRegistry callback aborts the controller and removes the listener.
        const roomRef = new WeakRef(this);
        onDeviceChange = () => {
          const self = roomRef.deref();
          if (!self) {
            return;
          }
          self.handleDeviceChange();
        };
        Room.cleanupRegistry.register(this, () => {
          cleanupController.abort();
        });
      } else {
        // Legacy browsers without WeakRef/FinalizationRegistry: fall back to a
        // direct listener (matches pre-#1944 behavior).
        onDeviceChange = this.handleDeviceChange;
      }
      // in order to catch device changes prior to room connection we need to register the event in the constructor
      (_g = (_f = navigator.mediaDevices) === null || _f === void 0 ? void 0 : _f.addEventListener) === null || _g === void 0 ? void 0 : _g.call(_f, 'devicechange', onDeviceChange, {
        signal: cleanupController.signal
      });
    }
  }
  registerTextStreamHandler(topic, callback) {
    return this.incomingDataStreamManager.registerTextStreamHandler(topic, callback);
  }
  unregisterTextStreamHandler(topic) {
    return this.incomingDataStreamManager.unregisterTextStreamHandler(topic);
  }
  registerByteStreamHandler(topic, callback) {
    return this.incomingDataStreamManager.registerByteStreamHandler(topic, callback);
  }
  unregisterByteStreamHandler(topic) {
    return this.incomingDataStreamManager.unregisterByteStreamHandler(topic);
  }
  /**
   * Establishes the participant as a receiver for calls of the specified RPC method.
   *
   * @param method - The name of the indicated RPC method
   * @param handler - Will be invoked when an RPC request for this method is received
   * @returns A promise that resolves when the method is successfully registered
   * @throws {Error} If a handler for this method is already registered (must call unregisterRpcMethod first)
   *
   * @example
   * ```typescript
   * room.localParticipant?.registerRpcMethod(
   *   'greet',
   *   async (data: RpcInvocationData) => {
   *     console.log(`Received greeting from ${data.callerIdentity}: ${data.payload}`);
   *     return `Hello, ${data.callerIdentity}!`;
   *   }
   * );
   * ```
   *
   * The handler should return a Promise that resolves to a string.
   * If unable to respond within `responseTimeout`, the request will result in an error on the caller's side.
   *
   * You may throw errors of type `RpcError` with a string `message` in the handler,
   * and they will be received on the caller's side with the message intact.
   * Other errors thrown in your handler will not be transmitted as-is, and will instead arrive to the caller as `1500` ("Application Error").
   */
  registerRpcMethod(method, handler) {
    this.rpcServerManager.registerRpcMethod(method, handler);
  }
  /**
   * Unregisters a previously registered RPC method.
   *
   * @param method - The name of the RPC method to unregister
   */
  unregisterRpcMethod(method) {
    this.rpcServerManager.unregisterRpcMethod(method);
  }
  /**
   * @experimental
   */
  setE2EEEnabled(enabled) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.e2eeStateMutex.lock();
      try {
        if (this.e2eeManager) {
          if (this.isE2EEEnabled !== enabled) {
            yield this.localParticipant.setE2EEEnabled(enabled);
            if (this.localParticipant.identity !== '') {
              this.e2eeManager.setParticipantCryptorEnabled(enabled, this.localParticipant.identity);
            }
          }
        } else {
          throw Error('e2ee not configured, please set e2ee settings within the room options');
        }
      } finally {
        unlock();
      }
    });
  }
  setupE2EE() {
    // when encryption is enabled via `options.encryption`, we enable data channel encryption
    var _a, _b;
    const dcEncryptionEnabled = !!this.options.encryption;
    const e2eeOptions = this.options.encryption || this.options.e2ee;
    if (e2eeOptions) {
      if ('e2eeManager' in e2eeOptions) {
        this.e2eeManager = e2eeOptions.e2eeManager;
        this.e2eeManager.isDataChannelEncryptionEnabled = dcEncryptionEnabled;
      } else {
        this.e2eeManager = new E2EEManager(e2eeOptions, dcEncryptionEnabled);
      }
      this.e2eeManager.on(EncryptionEvent.ParticipantEncryptionStatusChanged, (enabled, participant) => {
        if (isLocalParticipant(participant)) {
          this.isE2EEEnabled = enabled;
        }
        this.emit(RoomEvent.ParticipantEncryptionStatusChanged, enabled, participant);
      });
      this.e2eeManager.on(EncryptionEvent.EncryptionError, (error, participantIdentity) => {
        const participant = participantIdentity ? this.getParticipantByIdentity(participantIdentity) : undefined;
        this.emit(RoomEvent.EncryptionError, error, participant);
      });
      (_a = this.e2eeManager) === null || _a === void 0 ? void 0 : _a.setup(this);
      (_b = this.e2eeManager) === null || _b === void 0 ? void 0 : _b.setupEngine(this.engine);
    }
  }
  setupFrameMetadata() {
    var _a;
    const opts = (_a = this.options.frameMetadata) !== null && _a !== void 0 ? _a : this.options.packetTrailer;
    this.frameMetadataManager = new FrameMetadataManager(opts);
    this.frameMetadataManager.setup(this);
  }
  get logContext() {
    var _a, _b, _c;
    return {
      room: this.name,
      roomID: (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.sid,
      participant: (_b = this.localParticipant) === null || _b === void 0 ? void 0 : _b.identity,
      participantID: (_c = this.localParticipant) === null || _c === void 0 ? void 0 : _c.sid
    };
  }
  /**
   * if the current room has a participant with `recorder: true` in its JWT grant
   **/
  get isRecording() {
    var _a, _b;
    return (_b = (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.activeRecording) !== null && _b !== void 0 ? _b : false;
  }
  /**
   * server assigned unique room id.
   * returns once a sid has been issued by the server.
   */
  getSid() {
    if (this.state === ConnectionState.Disconnected) {
      return TypedPromise.resolve('');
    }
    if (this.roomInfo && this.roomInfo.sid !== '') {
      return TypedPromise.resolve(this.roomInfo.sid);
    }
    return new TypedPromise((resolve, reject) => {
      const handleRoomUpdate = roomInfo => {
        if (roomInfo.sid !== '') {
          this.engine.off(EngineEvent.RoomUpdate, handleRoomUpdate);
          resolve(roomInfo.sid);
        }
      };
      this.engine.on(EngineEvent.RoomUpdate, handleRoomUpdate);
      this.once(RoomEvent.Disconnected, () => {
        this.engine.off(EngineEvent.RoomUpdate, handleRoomUpdate);
        reject(new UnexpectedConnectionState('Room disconnected before room server id was available'));
      });
    });
  }
  /** user assigned name, derived from JWT token */
  get name() {
    var _a, _b;
    return (_b = (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.name) !== null && _b !== void 0 ? _b : '';
  }
  /** room metadata */
  get metadata() {
    var _a;
    return (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.metadata;
  }
  get numParticipants() {
    var _a, _b;
    return (_b = (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.numParticipants) !== null && _b !== void 0 ? _b : 0;
  }
  get numPublishers() {
    var _a, _b;
    return (_b = (_a = this.roomInfo) === null || _a === void 0 ? void 0 : _a.numPublishers) !== null && _b !== void 0 ? _b : 0;
  }
  maybeCreateEngine() {
    if (this.engine && (this.engine.isNewlyCreated || !this.engine.isClosed)) {
      return;
    }
    this.engine = new RTCEngine(this.options);
    this.engine.e2eeManager = this.e2eeManager;
    this.engine.on(EngineEvent.ParticipantUpdate, this.handleParticipantUpdates).on(EngineEvent.RoomUpdate, this.handleRoomUpdate).on(EngineEvent.SpeakersChanged, this.handleSpeakersChanged).on(EngineEvent.StreamStateChanged, this.handleStreamStateUpdate).on(EngineEvent.ConnectionQualityUpdate, this.handleConnectionQualityUpdate).on(EngineEvent.SubscriptionError, this.handleSubscriptionError).on(EngineEvent.SubscriptionPermissionUpdate, this.handleSubscriptionPermissionUpdate).on(EngineEvent.MediaTrackAdded, (mediaTrack, stream, receiver) => {
      this.onTrackAdded(mediaTrack, stream, receiver);
    }).on(EngineEvent.Disconnected, reason => {
      this.handleDisconnect(this.options.stopLocalTrackOnUnpublish, reason);
    }).on(EngineEvent.ActiveSpeakersUpdate, this.handleActiveSpeakersUpdate).on(EngineEvent.DataPacketReceived, this.handleDataPacket).on(EngineEvent.Resuming, () => {
      this.clearConnectionReconcile();
      this.isResuming = true;
      this.log.debug('Resuming signal connection');
      if (this.setAndEmitConnectionState(ConnectionState.SignalReconnecting)) {
        this.emit(RoomEvent.SignalReconnecting);
      }
    }).on(EngineEvent.Resumed, () => {
      this.registerConnectionReconcile();
      this.isResuming = false;
      this.log.debug('Resumed signal connection');
      this.updateSubscriptions();
      if (this.setAndEmitConnectionState(ConnectionState.Connected)) {
        this.emit(RoomEvent.Reconnected);
      }
      this.emitBufferedEvents();
    }).on(EngineEvent.SignalResumed, () => {
      if (this.state === ConnectionState.Reconnecting || this.isResuming) {
        this.sendSyncState();
      }
      this.emitBufferedEvents();
    }).on(EngineEvent.Restarting, this.handleRestarting).on(EngineEvent.Restarted, this.handleRestarted).on(EngineEvent.SignalRestarted, this.handleSignalRestarted).on(EngineEvent.Offline, () => {
      if (this.setAndEmitConnectionState(ConnectionState.Reconnecting)) {
        this.emit(RoomEvent.Reconnecting);
      }
    }).on(EngineEvent.DCBufferStatusChanged, (status, kind) => {
      this.emit(RoomEvent.DCBufferStatusChanged, status, kind);
    }).on(EngineEvent.LocalTrackSubscribed, subscribedSid => {
      this.handleLocalTrackSubscribed(subscribedSid);
    }).on(EngineEvent.RoomMoved, roomMoved => {
      this.log.debug('room moved', roomMoved);
      if (roomMoved.room) {
        this.handleRoomUpdate(roomMoved.room);
      }
      this.remoteParticipants.forEach((participant, identity) => {
        this.handleParticipantDisconnected(identity, participant);
      });
      this.emit(RoomEvent.Moved, roomMoved.room.name);
      if (roomMoved.participant) {
        this.handleParticipantUpdates([roomMoved.participant, ...roomMoved.otherParticipants]);
      } else {
        this.handleParticipantUpdates(roomMoved.otherParticipants);
      }
    }).on(EngineEvent.PublishDataTrackResponse, event => {
      if (!event.info) {
        this.log.warn("received PublishDataTrackResponse, but event.info was ".concat(event.info, ", so skipping."));
        return;
      }
      this.outgoingDataTrackManager.receivedSfuPublishResponse(event.info.pubHandle, {
        type: 'ok',
        data: {
          sid: event.info.sid,
          pubHandle: event.info.pubHandle,
          name: event.info.name,
          usesE2ee: event.info.encryption !== Encryption_Type.NONE
        }
      });
    }).on(EngineEvent.UnPublishDataTrackResponse, event => {
      if (!event.info) {
        this.log.warn("received UnPublishDataTrackResponse, but event.info was ".concat(event.info, ", so skipping."));
        return;
      }
      this.outgoingDataTrackManager.receivedSfuUnpublishResponse(event.info.pubHandle);
    }).on(EngineEvent.DataTrackSubscriberHandles, event => {
      const handleToSidMapping = new Map(Object.entries(event.subHandles).map(_ref4 => {
        let _ref5 = _slicedToArray(_ref4, 2),
          key = _ref5[0],
          value = _ref5[1];
        return [parseInt(key, 10), value.trackSid];
      }));
      this.incomingDataTrackManager.receivedSfuSubscriberHandles(handleToSidMapping);
    }).on(EngineEvent.DataTrackPacketReceived, packetBytes => {
      try {
        this.incomingDataTrackManager.packetReceived(packetBytes);
      } catch (err) {
        // NOTE: wrapping in the bare try/catch like this means that the Throws<...> type doesn't
        // propagate upwards into the public interface.
        throw err;
      }
    }).on(EngineEvent.Joined, joinResponse => {
      // Ingest data track publication updates into data tracks infrastructure
      const mapped = new Map(joinResponse.otherParticipants.map(participant => {
        return [participant.identity, participant.dataTracks.map(info => DataTrackInfo.from(info))];
      }));
      this.incomingDataTrackManager.receiveSfuPublicationUpdates(mapped);
    }).on(EngineEvent.TokenRefreshed, token => {
      var _a;
      (_a = this.regionUrlProvider) === null || _a === void 0 ? void 0 : _a.updateToken(token);
    }).on(EngineEvent.ServerRegionsReported, regions => {
      var _a;
      (_a = this.regionUrlProvider) === null || _a === void 0 ? void 0 : _a.setServerReportedRegions({
        regionSettings: regions,
        updatedAtInMs: Date.now(),
        maxAgeInMs: DEFAULT_MAX_AGE_MS
      });
    }).on(EngineEvent.RequestSubscribedCodecRefresh, () => {
      for (const videoPub of this.localParticipant.videoTrackPublications.values()) {
        if (isLocalVideoTrack(videoPub.track)) {
          videoPub.track.refreshSubscribedCodecs();
        }
      }
    });
    if (this.localParticipant) {
      this.localParticipant.setupEngine(this.engine);
    }
    if (this.e2eeManager) {
      this.e2eeManager.setupEngine(this.engine);
    }
    if (this.outgoingDataStreamManager) {
      this.outgoingDataStreamManager.setupEngine(this.engine);
    }
  }
  createRegionStrategy() {
    return {
      getNextUrl: signal => __awaiter(this, void 0, void 0, function* () {
        return this.regionUrlProvider ? this.regionUrlProvider.getNextBestRegionUrl(signal) : null;
      }),
      resetAttempts: () => {
        var _a;
        return (_a = this.regionUrlProvider) === null || _a === void 0 ? void 0 : _a.resetAttempts();
      }
    };
  }
  /**
   * getLocalDevices abstracts navigator.mediaDevices.enumerateDevices.
   * In particular, it requests device permissions by default if needed
   * and makes sure the returned device does not consist of dummy devices
   * @param kind
   * @returns a list of available local devices
   */
  static getLocalDevices(kind) {
    let requestPermissions = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
    return DeviceManager.getInstance().getDevices(kind, requestPermissions);
  }
  /**
   * prepareConnection should be called as soon as the page is loaded, in order
   * to speed up the connection attempt. This function will
   * - perform DNS resolution and pre-warm the DNS cache
   * - establish TLS connection and cache TLS keys
   *
   * With LiveKit Cloud, it will also determine the best edge data center for
   * the current client to connect to if a token is provided.
   */
  prepareConnection(url, token) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.state !== ConnectionState.Disconnected) {
        return;
      }
      this.log.debug("prepareConnection to ".concat(url));
      try {
        if (isCloud(new URL(url)) && token) {
          this.regionUrlProvider = new RegionUrlProvider(url, token);
          const regionUrl = yield this.regionUrlProvider.getNextBestRegionUrl();
          // we will not replace the regionUrl if an attempt had already started
          // to avoid overriding regionUrl after a new connection attempt had started
          if (regionUrl && this.state === ConnectionState.Disconnected) {
            this.regionUrl = regionUrl;
            yield fetch(toHttpUrl(regionUrl), {
              method: 'HEAD'
            });
            this.log.debug("prepared connection to ".concat(regionUrl));
          }
        } else {
          yield fetch(toHttpUrl(url), {
            method: 'HEAD'
          });
        }
      } catch (e) {
        this.log.warn('could not prepare connection', {
          error: e
        });
      }
    });
  }
  /**
   * retrieves a participant by identity
   * @param identity
   * @returns
   */
  getParticipantByIdentity(identity) {
    if (this.localParticipant.identity === identity) {
      return this.localParticipant;
    }
    return this.remoteParticipants.get(identity);
  }
  clearConnectionFutures() {
    this.connectFuture = undefined;
  }
  /**
   * @internal for testing
   */
  simulateScenario(scenario, arg) {
    return __awaiter(this, void 0, void 0, function* () {
      let postAction = () => __awaiter(this, void 0, void 0, function* () {});
      let req;
      switch (scenario) {
        case 'signal-reconnect':
          const reconnectDelay = typeof arg === 'number' ? arg : 0;
          // @ts-expect-error function is private
          yield this.engine.client.handleOnClose('simulate disconnect', undefined, reconnectDelay);
          break;
        case 'fail-on-v1-path':
          this.engine.failNextV1Path();
          break;
        case 'speaker':
          req = new SimulateScenario({
            scenario: {
              case: 'speakerUpdate',
              value: 3
            }
          });
          break;
        case 'node-failure':
          req = new SimulateScenario({
            scenario: {
              case: 'nodeFailure',
              value: true
            }
          });
          break;
        case 'server-leave':
          req = new SimulateScenario({
            scenario: {
              case: 'serverLeave',
              value: true
            }
          });
          break;
        case 'migration':
          req = new SimulateScenario({
            scenario: {
              case: 'migration',
              value: true
            }
          });
          break;
        case 'resume-reconnect':
          this.engine.failNext();
          // @ts-expect-error function is private
          yield this.engine.client.handleOnClose('simulate resume-disconnect');
          break;
        case 'disconnect-signal-on-resume':
          postAction = () => __awaiter(this, void 0, void 0, function* () {
            // @ts-expect-error function is private
            yield this.engine.client.handleOnClose('simulate resume-disconnect');
          });
          req = new SimulateScenario({
            scenario: {
              case: 'disconnectSignalOnResume',
              value: true
            }
          });
          break;
        case 'disconnect-signal-on-resume-no-messages':
          postAction = () => __awaiter(this, void 0, void 0, function* () {
            // @ts-expect-error function is private
            yield this.engine.client.handleOnClose('simulate resume-disconnect');
          });
          req = new SimulateScenario({
            scenario: {
              case: 'disconnectSignalOnResumeNoMessages',
              value: true
            }
          });
          break;
        case 'full-reconnect':
          this.engine.fullReconnectOnNext = true;
          // @ts-expect-error function is private
          yield this.engine.client.handleOnClose('simulate full-reconnect');
          break;
        case 'force-tcp':
        case 'force-tls':
          req = new SimulateScenario({
            scenario: {
              case: 'switchCandidateProtocol',
              value: scenario === 'force-tls' ? 2 : 1
            }
          });
          postAction = () => __awaiter(this, void 0, void 0, function* () {
            const onLeave = this.engine.client.onLeave;
            if (onLeave) {
              onLeave(new LeaveRequest({
                reason: DisconnectReason.CLIENT_INITIATED,
                action: LeaveRequest_Action.RECONNECT
              }));
            }
          });
          break;
        case 'subscriber-bandwidth':
          if (arg === undefined || typeof arg !== 'number') {
            throw new Error('subscriber-bandwidth requires a number as argument');
          }
          req = new SimulateScenario({
            scenario: {
              case: 'subscriberBandwidth',
              value: numberToBigInt(arg)
            }
          });
          break;
        case 'leave-full-reconnect':
          req = new SimulateScenario({
            scenario: {
              case: 'leaveRequestFullReconnect',
              value: true
            }
          });
      }
      if (req) {
        yield this.engine.client.sendSimulateScenario(req);
        yield postAction();
      }
    });
  }
  /**
   * iOS blocks audio element playback if
   * - user is not publishing audio themselves and
   * - no other audio source is playing
   *
   * as a workaround, we create an audio element with an empty track, so that
   * silent audio is always playing
   *
   * The element is shared with any other room on the page and handed back on disconnect, so it
   * outlives whichever room happened to create it.
   */
  acquireDummyAudioElement() {
    if (!sharedDummyAudioElement) {
      sharedDummyAudioElement = createSharedDummyAudioElement();
    }
    const shared = sharedDummyAudioElement;
    if (this.releaseDummyAudioElement) {
      // this room already holds a reference, a second one would never be handed back
      return shared.element;
    }
    shared.refCount += 1;
    // the listener is per room rather than shared, so that the room it resumes playback for is
    // still connected, and so that it goes away with the room that installed it
    const handleVisibilityChange = () => {
      // set the srcObject to null on page hide in order to prevent lock screen controls to show up for it
      shared.element.srcObject = document.hidden ? null : shared.stream;
      if (!document.hidden) {
        this.log.debug('page visible again, triggering startAudio to resume playback and update playback status');
        this.startAudio().catch(error => this.log.warn('Could not restart audio on page becoming visible', Object.assign(Object.assign({}, this.logContext), {
          error
        })));
      }
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);
    this.releaseDummyAudioElement = () => {
      this.releaseDummyAudioElement = undefined;
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      shared.refCount -= 1;
      if (shared.refCount > 0) {
        return;
      }
      shared.element.srcObject = null;
      shared.element.remove();
      sharedDummyAudioElement = undefined;
      releaseEmptyAudioStreamTrack(shared.track).catch(error => this.log.warn('Could not release the dummy audio element track', Object.assign(Object.assign({}, this.logContext), {
        error
      })));
    };
    return shared.element;
  }
  /**
   * Obtains a silent audio track from the shared source and keeps track of it, so that it can be
   * handed back on disconnect and the shared `AudioContext` behind it can be closed again.
   */
  acquireEmptyAudioStreamTrack() {
    const track = getEmptyAudioStreamTrack();
    this.acquiredEmptyAudioTracks.add(track);
    return track;
  }
  releaseAcquiredEmptyAudioTrack(track) {
    if (!this.acquiredEmptyAudioTracks.delete(track)) {
      return;
    }
    releaseEmptyAudioStreamTrack(track).catch(error => this.log.warn('Could not release empty audio stream track', Object.assign(Object.assign({}, this.logContext), {
      error
    })));
  }
  releaseAcquiredEmptyAudioTracks() {
    for (const track of this.acquiredEmptyAudioTracks) {
      this.releaseAcquiredEmptyAudioTrack(track);
    }
  }
  /**
   * Returns true if audio playback is enabled
   */
  get canPlaybackAudio() {
    return this.audioEnabled;
  }
  /**
   * Returns true if video playback is enabled
   */
  get canPlaybackVideo() {
    return !this.isVideoPlaybackBlocked;
  }
  getActiveDevice(kind) {
    return this.localParticipant.activeDeviceMap.get(kind);
  }
  /**
   * Switches all active devices used in this room to the given device.
   *
   * Note: setting AudioOutput is not supported on some browsers. See [setSinkId](https://developer.mozilla.org/en-US/docs/Web/API/HTMLMediaElement/setSinkId#browser_compatibility)
   *
   * @param kind use `videoinput` for camera track,
   *  `audioinput` for microphone track,
   *  `audiooutput` to set speaker for all incoming audio tracks
   * @param deviceId
   */
  switchActiveDevice(kind_1, deviceId_1) {
    return __awaiter(this, arguments, void 0, function (kind, deviceId) {
      var _this3 = this;
      let exact = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : true;
      return function* () {
        var _a, _b, _c, _d, _e, _f;
        var _g;
        let success = true;
        let shouldTriggerImmediateDeviceChange = false;
        const deviceConstraint = exact ? {
          exact: deviceId
        } : deviceId;
        if (kind === 'audioinput') {
          shouldTriggerImmediateDeviceChange = _this3.localParticipant.audioTrackPublications.size === 0;
          const prevDeviceId = (_a = _this3.getActiveDevice(kind)) !== null && _a !== void 0 ? _a : _this3.options.audioCaptureDefaults.deviceId;
          _this3.options.audioCaptureDefaults.deviceId = deviceConstraint;
          const tracks = Array.from(_this3.localParticipant.audioTrackPublications.values()).filter(track => track.source === Track.Source.Microphone);
          try {
            success = (yield Promise.all(tracks.map(t => {
              var _a;
              return (_a = t.audioTrack) === null || _a === void 0 ? void 0 : _a.setDeviceId(deviceConstraint);
            }))).every(val => val === true);
          } catch (e) {
            _this3.options.audioCaptureDefaults.deviceId = prevDeviceId;
            throw e;
          }
          const isMuted = tracks.some(t => {
            var _a, _b;
            return (_b = (_a = t.track) === null || _a === void 0 ? void 0 : _a.isMuted) !== null && _b !== void 0 ? _b : false;
          });
          if (success && isMuted) shouldTriggerImmediateDeviceChange = true;
        } else if (kind === 'videoinput') {
          shouldTriggerImmediateDeviceChange = _this3.localParticipant.videoTrackPublications.size === 0;
          const prevDeviceId = (_b = _this3.getActiveDevice(kind)) !== null && _b !== void 0 ? _b : _this3.options.videoCaptureDefaults.deviceId;
          _this3.options.videoCaptureDefaults.deviceId = deviceConstraint;
          const tracks = Array.from(_this3.localParticipant.videoTrackPublications.values()).filter(track => track.source === Track.Source.Camera);
          try {
            success = (yield Promise.all(tracks.map(t => {
              var _a;
              return (_a = t.videoTrack) === null || _a === void 0 ? void 0 : _a.setDeviceId(deviceConstraint);
            }))).every(val => val === true);
          } catch (e) {
            _this3.options.videoCaptureDefaults.deviceId = prevDeviceId;
            throw e;
          }
          const isMuted = tracks.some(t => {
            var _a, _b;
            return (_b = (_a = t.track) === null || _a === void 0 ? void 0 : _a.isMuted) !== null && _b !== void 0 ? _b : false;
          });
          if (success && isMuted) shouldTriggerImmediateDeviceChange = true;
        } else if (kind === 'audiooutput') {
          shouldTriggerImmediateDeviceChange = true;
          if (!supportsSetSinkId() && !_this3.options.webAudioMix || _this3.options.webAudioMix && _this3.audioContext && !('setSinkId' in _this3.audioContext)) {
            throw new Error('cannot switch audio output, the current browser does not support it');
          }
          if (_this3.options.webAudioMix) {
            // setting `default` for web audio output doesn't work, so we need to normalize the id before
            deviceId = (_c = yield DeviceManager.getInstance().normalizeDeviceId('audiooutput', deviceId)) !== null && _c !== void 0 ? _c : '';
          }
          (_d = (_g = _this3.options).audioOutput) !== null && _d !== void 0 ? _d : _g.audioOutput = {};
          const prevDeviceId = (_e = _this3.getActiveDevice(kind)) !== null && _e !== void 0 ? _e : _this3.options.audioOutput.deviceId;
          _this3.options.audioOutput.deviceId = deviceId;
          try {
            if (_this3.options.webAudioMix) {
              // @ts-expect-error setSinkId is not yet in the typescript type of AudioContext
              (_f = _this3.audioContext) === null || _f === void 0 ? void 0 : _f.setSinkId(deviceId);
            }
            // also set audio output on all audio elements, even if webAudioMix is enabled in order to workaround echo cancellation not working on chrome with non-default output devices
            // see https://issues.chromium.org/issues/40252911#comment7
            yield Promise.all(Array.from(_this3.remoteParticipants.values()).map(p => p.setAudioOutput({
              deviceId
            })));
          } catch (e) {
            _this3.options.audioOutput.deviceId = prevDeviceId;
            throw e;
          }
        }
        if (shouldTriggerImmediateDeviceChange) {
          _this3.localParticipant.activeDeviceMap.set(kind, deviceId);
          _this3.emit(RoomEvent.ActiveDeviceChanged, kind, deviceId);
        }
        return success;
      }();
    });
  }
  setupLocalParticipantEvents() {
    this.localParticipant.on(ParticipantEvent.ParticipantMetadataChanged, this.onLocalParticipantMetadataChanged).on(ParticipantEvent.ParticipantNameChanged, this.onLocalParticipantNameChanged).on(ParticipantEvent.AttributesChanged, this.onLocalAttributesChanged).on(ParticipantEvent.TrackMuted, this.onLocalTrackMuted).on(ParticipantEvent.TrackUnmuted, this.onLocalTrackUnmuted).on(ParticipantEvent.LocalTrackPublished, this.onLocalTrackPublished).on(ParticipantEvent.LocalTrackUnpublished, this.onLocalTrackUnpublished).on(ParticipantEvent.ConnectionQualityChanged, this.onLocalConnectionQualityChanged).on(ParticipantEvent.MediaDevicesError, this.onMediaDevicesError).on(ParticipantEvent.AudioStreamAcquired, this.startAudio).on(ParticipantEvent.ChatMessage, this.onLocalChatMessageSent).on(ParticipantEvent.ParticipantPermissionsChanged, this.onLocalParticipantPermissionsChanged);
  }
  recreateEngine(sendLeave) {
    const oldEngine = this.engine;
    if (sendLeave && oldEngine && !oldEngine.client.isDisconnected) {
      oldEngine.client.sendLeave().finally(() => oldEngine.close());
    } else {
      oldEngine === null || oldEngine === void 0 ? void 0 : oldEngine.close();
    }
    /* @ts-ignore */
    this.engine = undefined;
    this.isResuming = false;
    // clear out existing remote participants, since they may have attached
    // the old engine
    this.remoteParticipants.clear();
    this.sidToIdentity.clear();
    this.bufferedEvents = [];
    this.maybeCreateEngine();
  }
  onTrackAdded(mediaTrack, stream, receiver) {
    var _a, _b;
    // don't fire onSubscribed when connecting
    // WebRTC fires onTrack as soon as setRemoteDescription is called on the offer
    // at that time, ICE connectivity has not been established so the track is not
    // technically subscribed.
    // We'll defer these events until when the room is connected or eventually disconnected.
    if ([ConnectionState.Connecting, ConnectionState.Reconnecting].includes(this.state)) {
      const pendingTrackSid = extractTrackSid(mediaTrack, stream);
      this.log.debug('deferring on track for later', {
        mediaTrackId: mediaTrack.id,
        mediaStreamId: stream.id,
        tracksInStream: stream.getTracks().map(track => track.id)
      });
      const reconnectedHandler = () => {
        cleanup();
        this.onTrackAdded(mediaTrack, stream, receiver);
      };
      const cleanup = () => {
        this.off(RoomEvent.Reconnected, reconnectedHandler);
        this.off(RoomEvent.Connected, reconnectedHandler);
        this.off(RoomEvent.Disconnected, cleanup);
        if (pendingTrackSid) {
          const pendingCallbacks = this.pendingTrackAddedCallbacks.get(pendingTrackSid);
          pendingCallbacks === null || pendingCallbacks === void 0 ? void 0 : pendingCallbacks.delete(cleanup);
          if ((pendingCallbacks === null || pendingCallbacks === void 0 ? void 0 : pendingCallbacks.size) === 0) {
            this.pendingTrackAddedCallbacks.delete(pendingTrackSid);
          }
        }
      };
      this.once(RoomEvent.Reconnected, reconnectedHandler);
      this.once(RoomEvent.Connected, reconnectedHandler);
      this.once(RoomEvent.Disconnected, cleanup);
      if (pendingTrackSid) {
        const pendingCallbacks = (_a = this.pendingTrackAddedCallbacks.get(pendingTrackSid)) !== null && _a !== void 0 ? _a : new Set();
        pendingCallbacks.add(cleanup);
        this.pendingTrackAddedCallbacks.set(pendingTrackSid, pendingCallbacks);
      }
      return;
    }
    if (this.state === ConnectionState.Disconnected) {
      this.log.warn('skipping incoming track after Room disconnected');
      return;
    }
    if (mediaTrack.readyState === 'ended') {
      this.log.debug('skipping incoming track as it already ended');
      return;
    }
    const parts = unpackStreamId(stream.id);
    const participantSid = parts[0];
    const streamId = parts[1];
    // firefox will get streamId (pID|trackId) instead of (pID|streamId) as it doesn't support sync tracks by stream
    // and generates its own track id instead of infer from sdp track id.
    let trackId = (_b = extractTrackSid(mediaTrack, stream)) !== null && _b !== void 0 ? _b : mediaTrack.id;
    if (participantSid === this.localParticipant.sid) {
      this.log.warn('tried to create RemoteParticipant for local participant');
      return;
    }
    const participant = Array.from(this.remoteParticipants.values()).find(p => p.sid === participantSid);
    if (!participant) {
      // server could require extra media sections to accelerate subscription.
      if (participantSid.startsWith('PA')) {
        this.log.error("Tried to add a track for a participant, that's not present. Sid: ".concat(participantSid));
      }
      return;
    }
    // in single peer connection case, the trackID is locally generated,
    // not the TR_ prefixed one generated by the server,
    // use `mid` to find the appropriate track.
    if (!trackId.startsWith('TR')) {
      const id = this.engine.getTrackIdForReceiver(receiver);
      if (!id) {
        this.log.error("Tried to add a track whose 'sid' could not be found for a participant, that's not present. Sid: ".concat(participantSid));
        return;
      }
      trackId = id;
    }
    if (!trackId.startsWith('TR')) {
      this.log.warn("Tried to add a track whose 'sid' could not be determined for a participant, that's not present. Sid: ".concat(participantSid, ", streamId: ").concat(streamId, ", trackId: ").concat(trackId), {
        remoteParticipantID: participantSid,
        streamId,
        trackId
      });
    }
    let adaptiveStreamSettings;
    if (this.options.adaptiveStream) {
      if (typeof this.options.adaptiveStream === 'object') {
        adaptiveStreamSettings = this.options.adaptiveStream;
      } else {
        adaptiveStreamSettings = {};
      }
    }
    const publication = participant.addSubscribedMediaTrack(mediaTrack, trackId, stream, receiver, adaptiveStreamSettings);
    if ((publication === null || publication === void 0 ? void 0 : publication.isEncrypted) && !this.e2eeManager) {
      this.emit(RoomEvent.EncryptionError, new Error("Encrypted ".concat(publication.source, " track received from participant ").concat(participant.sid, ", but room does not have encryption enabled!")));
    }
  }
  cancelPendingTrackAdded(trackSid) {
    var _a;
    (_a = this.pendingTrackAddedCallbacks.get(trackSid)) === null || _a === void 0 ? void 0 : _a.forEach(cleanup => cleanup());
  }
  handleLocalTrackSubscribed(subscribedSid) {
    const findPublication = () => this.localParticipant.getTrackPublications().find(_ref6 => {
      let trackSid = _ref6.trackSid;
      return trackSid === subscribedSid;
    });
    const trackPublication = findPublication();
    if (trackPublication) {
      this.emitLocalTrackSubscribed(trackPublication);
      return;
    }
    // the track publication may not be registered yet if the server signals
    // the subscription before publishTrack has finished adding the publication.
    // defer with a timeout until LocalTrackPublished fires for the matching trackSid
    this.log.debug('deferring LocalTrackSubscribed, publication not yet available', {
      subscribedSid
    });
    const TIMEOUT_MS = 10000;
    let timer;
    const onPublished = pub => {
      if (pub.trackSid === subscribedSid) {
        cleanup();
        this.emitLocalTrackSubscribed(pub);
      }
    };
    const cleanup = () => {
      clearTimeout(timer);
      this.localParticipant.off(ParticipantEvent.LocalTrackPublished, onPublished);
      this.off(RoomEvent.Disconnected, cleanup);
    };
    this.localParticipant.on(ParticipantEvent.LocalTrackPublished, onPublished);
    this.once(RoomEvent.Disconnected, cleanup);
    timer = setTimeout(() => {
      cleanup();
      // final attempt in case the publication was added without emitting the event
      const pub = findPublication();
      if (pub) {
        this.emitLocalTrackSubscribed(pub);
      } else {
        this.log.warn('could not find local track publication for LocalTrackSubscribed event after timeout', {
          subscribedSid
        });
      }
    }, TIMEOUT_MS);
  }
  emitLocalTrackSubscribed(trackPublication) {
    this.localParticipant.emit(ParticipantEvent.LocalTrackSubscribed, trackPublication);
    this.emitWhenConnected(RoomEvent.LocalTrackSubscribed, trackPublication, this.localParticipant);
  }
  handleDisconnect() {
    let shouldStopTracks = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
    let reason = arguments.length > 1 ? arguments[1] : undefined;
    var _a, _b, _c;
    this.clearConnectionReconcile();
    this.isResuming = false;
    this.bufferedEvents = [];
    this.transcriptionReceivedTimes.clear();
    this.incomingDataStreamManager.clearControllers();
    this.incomingDataTrackManager.reset();
    this.outgoingDataTrackManager.reset();
    if (this.state === ConnectionState.Disconnected) {
      return;
    }
    this.regionUrl = undefined;
    // Notify region provider about disconnect to potentially stop auto-refetch
    if (this.regionUrlProvider) {
      this.regionUrlProvider.notifyDisconnected();
    }
    try {
      // audio tracks that outlive this room are about to become unreachable through the local
      // participant, collect them while we can so they can be detached from the context below
      const retainedAudioTracks = shouldStopTracks ? [] : Array.from(this.localParticipant.audioTrackPublications.values()).map(pub => pub.track).filter(isLocalAudioTrack);
      this.remoteParticipants.forEach(p => {
        p.trackPublications.forEach(pub => {
          p.unpublishTrack(pub.trackSid);
        });
      });
      this.localParticipant.trackPublications.forEach(pub => {
        var _a, _b, _c;
        if (pub.track) {
          this.localParticipant.unpublishTrack(pub.track, shouldStopTracks);
        }
        if (shouldStopTracks) {
          (_a = pub.track) === null || _a === void 0 ? void 0 : _a.detach();
          (_b = pub.track) === null || _b === void 0 ? void 0 : _b.stop();
        } else {
          (_c = pub.track) === null || _c === void 0 ? void 0 : _c.stopMonitor();
        }
      });
      this.localParticipant.off(ParticipantEvent.ParticipantMetadataChanged, this.onLocalParticipantMetadataChanged).off(ParticipantEvent.ParticipantNameChanged, this.onLocalParticipantNameChanged).off(ParticipantEvent.AttributesChanged, this.onLocalAttributesChanged).off(ParticipantEvent.TrackMuted, this.onLocalTrackMuted).off(ParticipantEvent.TrackUnmuted, this.onLocalTrackUnmuted).off(ParticipantEvent.LocalTrackPublished, this.onLocalTrackPublished).off(ParticipantEvent.LocalTrackUnpublished, this.onLocalTrackUnpublished).off(ParticipantEvent.ConnectionQualityChanged, this.onLocalConnectionQualityChanged).off(ParticipantEvent.MediaDevicesError, this.onMediaDevicesError).off(ParticipantEvent.AudioStreamAcquired, this.startAudio).off(ParticipantEvent.ChatMessage, this.onLocalChatMessageSent).off(ParticipantEvent.ParticipantPermissionsChanged, this.onLocalParticipantPermissionsChanged);
      this.localParticipant.trackPublications.clear();
      this.localParticipant.videoTrackPublications.clear();
      this.localParticipant.audioTrackPublications.clear();
      this.remoteParticipants.clear();
      this.sidToIdentity.clear();
      this.activeSpeakers = [];
      (_a = this.releaseDummyAudioElement) === null || _a === void 0 ? void 0 : _a.call(this);
      this.releaseAcquiredEmptyAudioTracks();
      // teardown is asynchronous (processors need to release their web audio nodes), but
      // `handleDisconnect` is not, so let it finish in the background
      this.releaseAudioContext(retainedAudioTracks).catch(error => this.log.warn('Could not release audio context', Object.assign(Object.assign({}, this.logContext), {
        error
      })));
      if (isWeb()) {
        window.removeEventListener('beforeunload', this.onPageLeave);
        window.removeEventListener('pagehide', this.onPageLeave);
        window.removeEventListener('freeze', this.onPageLeave);
        (_c = (_b = navigator.mediaDevices) === null || _b === void 0 ? void 0 : _b.removeEventListener) === null || _c === void 0 ? void 0 : _c.call(_b, 'devicechange', this.handleDeviceChange);
      }
    } finally {
      this.setAndEmitConnectionState(ConnectionState.Disconnected);
      this.emit(RoomEvent.Disconnected, reason);
    }
  }
  handleParticipantDisconnected(identity, participant, disconnectReason) {
    // remove and send event
    this.remoteParticipants.delete(identity);
    if (!participant) {
      return;
    }
    this.incomingDataStreamManager.validateParticipantHasNoActiveDataStreams(identity);
    this.incomingDataTrackManager.handleRemoteParticipantDisconnected(identity);
    participant.trackPublications.forEach(publication => {
      participant.unpublishTrack(publication.trackSid, true);
    });
    this.emit(RoomEvent.ParticipantDisconnected, participant, disconnectReason);
    participant.setDisconnected();
    this.rpcClientManager.handleParticipantDisconnected(participant.identity);
  }
  /**
   * attempt to select the default devices if the previously selected devices are no longer available after a device change event
   */
  selectDefaultDevices() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      const previousDevices = DeviceManager.getInstance().previousDevices;
      // check for available devices, but don't request permissions in order to avoid prompts for kinds that haven't been used before
      const availableDevices = yield DeviceManager.getInstance().getDevices(undefined, false);
      const browser = getBrowser();
      if ((browser === null || browser === void 0 ? void 0 : browser.name) === 'Chrome' && browser.os !== 'iOS') {
        for (let availableDevice of availableDevices) {
          const previousDevice = previousDevices.find(info => info.deviceId === availableDevice.deviceId);
          if (previousDevice && previousDevice.label !== '' && previousDevice.kind === availableDevice.kind && previousDevice.label !== availableDevice.label) {
            // label has changed on device the same deviceId, indicating that the default device has changed on the OS level
            if (this.getActiveDevice(availableDevice.kind) === 'default') {
              // emit an active device change event only if the selected output device is actually on `default`
              this.emit(RoomEvent.ActiveDeviceChanged, availableDevice.kind, availableDevice.deviceId);
            }
          }
        }
      }
      const kinds = ['audiooutput', 'audioinput', 'videoinput'];
      for (let kind of kinds) {
        const targetSource = kindToSource(kind);
        const targetPublication = this.localParticipant.getTrackPublication(targetSource);
        if (targetPublication && ((_a = targetPublication.track) === null || _a === void 0 ? void 0 : _a.isUserProvided)) {
          // if the track is user provided, we don't want to switch devices on behalf of the user
          continue;
        }
        const devicesOfKind = availableDevices.filter(d => d.kind === kind);
        const activeDevice = this.getActiveDevice(kind);
        if (activeDevice === ((_b = previousDevices.filter(info => info.kind === kind)[0]) === null || _b === void 0 ? void 0 : _b.deviceId)) {
          // in  Safari the first device is always the default, so we assume a user on the default device would like to switch to the default once it changes
          // FF doesn't emit an event when the default device changes, so we perform the same best effort and switch to the new device once connected and if it's the first in the array
          if (devicesOfKind.length > 0 && ((_c = devicesOfKind[0]) === null || _c === void 0 ? void 0 : _c.deviceId) !== activeDevice) {
            yield this.switchActiveDevice(kind, devicesOfKind[0].deviceId);
            continue;
          }
        }
        if (kind === 'audioinput' && !isSafariBased() || kind === 'videoinput') {
          // airpods on Safari need special handling for audioinput as the track doesn't end as soon as you take them out
          continue;
        }
        // switch to first available device if previously active device is not available any more
        if (devicesOfKind.length > 0 && !devicesOfKind.find(deviceInfo => deviceInfo.deviceId === this.getActiveDevice(kind)) && (
        // avoid switching audio output on safari without explicit user action as it leads to slowed down audio playback
        kind !== 'audiooutput' || !isSafariBased())) {
          yield this.switchActiveDevice(kind, devicesOfKind[0].deviceId);
        }
      }
    });
  }
  /**
   * @param forConnect whether this acquisition belongs to a connect, which is the only caller
   * allowed to put a released room back on an audio context.
   */
  acquireAudioContext() {
    return __awaiter(this, arguments, void 0, function () {
      var _this4 = this;
      let forConnect = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : false;
      return function* () {
        var _a, _b;
        const unlock = yield _this4.audioContextMutex.lock();
        try {
          if (_this4.audioContextReleased && !forConnect) {
            // a disconnect set the flag and took the lock while this call was queued behind it.
            // Creating a context now would leave one open on a room nothing tears down again
            return;
          }
          _this4.audioContextReleased = false;
          const providedAudioContext = typeof _this4.options.webAudioMix !== 'boolean' ? _this4.options.webAudioMix.audioContext : undefined;
          if (providedAudioContext) {
            // override audio context with custom audio context if supplied by user
            yield _this4.setRoomAudioContext(providedAudioContext, false);
          } else if (!_this4.audioContext || _this4.audioContext.state === 'closed') {
            // by using an AudioContext, it reduces lag on audio elements
            // https://stackoverflow.com/questions/9811429/html5-audio-tag-on-safari-has-a-delay/54119854#54119854
            yield _this4.setRoomAudioContext((_a = getNewAudioContext()) !== null && _a !== void 0 ? _a : undefined, true);
          }
          if (_this4.audioContext && _this4.audioContext.state === 'suspended') {
            // for iOS a newly created AudioContext is always in `suspended` state.
            // we try our best to resume the context here, if that doesn't work, we just continue with regular processing
            try {
              yield Promise.race([_this4.audioContext.resume(), sleep(200)]);
            } catch (e) {
              _this4.log.warn('Could not resume audio context', {
                error: e
              });
            }
          }
          const newContextIsRunning = ((_b = _this4.audioContext) === null || _b === void 0 ? void 0 : _b.state) === 'running';
          if (newContextIsRunning !== _this4.canPlaybackAudio) {
            _this4.audioEnabled = newContextIsRunning;
            _this4.emit(RoomEvent.AudioPlaybackStatusChanged, newContextIsRunning);
          }
        } finally {
          unlock();
        }
      }();
    });
  }
  /**
   * Points the room and its participants at `audioContext`, rebuilding the web audio graph of
   * every audio track in it. Closes the previous context if the SDK `owned` it.
   */
  setRoomAudioContext(audioContext, owned) {
    return __awaiter(this, void 0, void 0, function* () {
      const ownsAudioContext = !!audioContext && owned;
      if (this.audioContext === audioContext) {
        this.ownsAudioContext = ownsAudioContext;
        return;
      }
      const contextToClose = this.ownsAudioContext ? this.audioContext : undefined;
      this.audioContext = audioContext;
      this.ownsAudioContext = ownsAudioContext;
      try {
        // every audio track rebuilds its nodes in the new context, which has to happen before the
        // previous one is closed so that processors can move off it while it is still alive
        if (this.options.webAudioMix) {
          yield Promise.all(Array.from(this.remoteParticipants.values()).map(participant => participant.setAudioContext(audioContext)));
        }
        yield this.localParticipant.setAudioContext(audioContext);
      } catch (error) {
        // the context is being abandoned either way, so a track that failed to move off it must
        // not stop us closing it — that would leak the context this whole path exists to release
        this.log.warn('Could not move audio tracks to the new audio context', Object.assign(Object.assign({}, this.logContext), {
          error
        }));
      } finally {
        if (contextToClose && contextToClose.state !== 'closed') {
          try {
            yield contextToClose.close();
          } catch (error) {
            this.log.warn('Could not close audio context', Object.assign(Object.assign({}, this.logContext), {
              error
            }));
          }
        }
      }
    });
  }
  /**
   * Detaches the web audio graph the SDK built from the current audio context and closes that
   * context if the SDK created it. A context provided through `webAudioMix` is left untouched.
   */
  releaseAudioContext() {
    return __awaiter(this, arguments, void 0, function () {
      var _this5 = this;
      let retainedAudioTracks = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : [];
      return function* () {
        // both reads happen before the first await, so they describe the room as it was when the
        // disconnect started rather than after a reconnect has had a chance to run
        _this5.audioContextReleased = true;
        const contextToRelease = _this5.audioContext;
        const unlock = yield _this5.audioContextMutex.lock();
        try {
          if (!_this5.audioContextReleased || _this5.audioContext !== contextToRelease) {
            // a reconnect acquired a new context while we were waiting, closing it would leave the
            // reconnected room without one
            return;
          }
          if (!_this5.ownsAudioContext) {
            // a provided context stays open, so tracks that outlive the room stay functional on it.
            // The tracks this room detached have released their nodes during track teardown already.
            // read under the lock: an acquire queued behind an in-flight release flips this flag
            return;
          }
          try {
            // tracks that outlive the room aren't reachable through the local participant anymore,
            // but still hold web audio nodes in the context we're about to close
            yield Promise.all(retainedAudioTracks.map(track => track.setAudioContext(undefined)));
          } catch (error) {
            _this5.log.warn('Could not detach retained audio tracks from the audio context', Object.assign(Object.assign({}, _this5.logContext), {
              error
            }));
          }
          // the participant maps have already been cleared by `handleDisconnect`, so this reaches no
          // tracks of its own — `retainedAudioTracks` above is what detaches them
          yield _this5.setRoomAudioContext(undefined, false);
        } finally {
          unlock();
        }
      }();
    });
  }
  createParticipant(identity, info) {
    var _a;
    let participant;
    if (info) {
      participant = RemoteParticipant.fromParticipantInfo(this.engine.client, info, {
        loggerContextCb: () => this.logContext,
        loggerName: this.options.loggerName
      }, this.incomingDataTrackManager);
    } else {
      participant = new RemoteParticipant(this.engine.client, '', identity, undefined, undefined, undefined, {
        loggerContextCb: () => this.logContext,
        loggerName: this.options.loggerName
      });
    }
    if (this.options.webAudioMix) {
      participant.setAudioContext(this.audioContext).catch(error => this.log.warn('Could not set audio context', Object.assign(Object.assign({}, this.logContext), {
        error
      })));
    }
    if ((_a = this.options.audioOutput) === null || _a === void 0 ? void 0 : _a.deviceId) {
      participant.setAudioOutput(this.options.audioOutput).catch(e => this.log.warn("Could not set audio output: ".concat(e.message)));
    }
    return participant;
  }
  getOrCreateParticipant(identity, info) {
    if (this.remoteParticipants.has(identity)) {
      const existingParticipant = this.remoteParticipants.get(identity);
      if (info) {
        const wasUpdated = existingParticipant.updateInfo(info);
        if (wasUpdated) {
          this.sidToIdentity.set(info.sid, info.identity);
        }
      }
      return existingParticipant;
    }
    const participant = this.createParticipant(identity, info);
    this.remoteParticipants.set(identity, participant);
    this.sidToIdentity.set(info.sid, info.identity);
    // if we have valid info and the participant wasn't in the map before, we can assume the participant is new
    // firing here to make sure that `ParticipantConnected` fires before the initial track events
    this.emitWhenConnected(RoomEvent.ParticipantConnected, participant);
    // also forward events
    // trackPublished is only fired for tracks added after both local participant
    // and remote participant joined the room
    participant.on(ParticipantEvent.TrackPublished, trackPublication => {
      this.emitWhenConnected(RoomEvent.TrackPublished, trackPublication, participant);
    }).on(ParticipantEvent.TrackSubscribed, (track, publication) => {
      // monitor playback status
      if (track.kind === Track.Kind.Audio) {
        track.on(TrackEvent.AudioPlaybackStarted, this.handleAudioPlaybackStarted);
        track.on(TrackEvent.AudioPlaybackFailed, this.handleAudioPlaybackFailed);
      } else if (track.kind === Track.Kind.Video) {
        track.on(TrackEvent.VideoPlaybackFailed, this.handleVideoPlaybackFailed);
        track.on(TrackEvent.VideoPlaybackStarted, this.handleVideoPlaybackStarted);
      }
      this.emitWhenConnected(RoomEvent.TrackSubscribed, track, publication, participant);
    }).on(ParticipantEvent.TrackUnpublished, publication => {
      this.cancelPendingTrackAdded(publication.trackSid);
      this.emit(RoomEvent.TrackUnpublished, publication, participant);
    }).on(ParticipantEvent.TrackUnsubscribed, (track, publication) => {
      this.emit(RoomEvent.TrackUnsubscribed, track, publication, participant);
    }).on(ParticipantEvent.TrackMuted, pub => {
      this.emitWhenConnected(RoomEvent.TrackMuted, pub, participant);
    }).on(ParticipantEvent.TrackUnmuted, pub => {
      this.emitWhenConnected(RoomEvent.TrackUnmuted, pub, participant);
    }).on(ParticipantEvent.ParticipantMetadataChanged, metadata => {
      this.emitWhenConnected(RoomEvent.ParticipantMetadataChanged, metadata, participant);
    }).on(ParticipantEvent.ParticipantNameChanged, name => {
      this.emitWhenConnected(RoomEvent.ParticipantNameChanged, name, participant);
    }).on(ParticipantEvent.AttributesChanged, changedAttributes => {
      this.emitWhenConnected(RoomEvent.ParticipantAttributesChanged, changedAttributes, participant);
    }).on(ParticipantEvent.ConnectionQualityChanged, quality => {
      this.emitWhenConnected(RoomEvent.ConnectionQualityChanged, quality, participant);
    }).on(ParticipantEvent.ParticipantPermissionsChanged, prevPermissions => {
      this.emitWhenConnected(RoomEvent.ParticipantPermissionsChanged, prevPermissions, participant);
    }).on(ParticipantEvent.TrackSubscriptionStatusChanged, (pub, status) => {
      this.emitWhenConnected(RoomEvent.TrackSubscriptionStatusChanged, pub, status, participant);
    }).on(ParticipantEvent.TrackSubscriptionFailed, (trackSid, error) => {
      this.emit(RoomEvent.TrackSubscriptionFailed, trackSid, participant, error);
    }).on(ParticipantEvent.TrackSubscriptionPermissionChanged, (pub, status) => {
      this.emitWhenConnected(RoomEvent.TrackSubscriptionPermissionChanged, pub, status, participant);
    }).on(ParticipantEvent.Active, () => {
      this.emitWhenConnected(RoomEvent.ParticipantActive, participant);
      if (participant.kind === ParticipantInfo_Kind.AGENT) {
        this.localParticipant.setActiveAgent(participant);
      }
    });
    // update info at the end after callbacks have been set up
    if (info) {
      participant.updateInfo(info);
    }
    return participant;
  }
  sendSyncState() {
    const remoteTracks = Array.from(this.remoteParticipants.values()).reduce((acc, participant) => {
      acc.push(...participant.getTrackPublications()); // FIXME would be nice to have this return RemoteTrackPublications directly instead of the type cast
      return acc;
    }, []);
    const localTracks = this.localParticipant.getTrackPublications(); // FIXME would be nice to have this return LocalTrackPublications directly instead of the type cast
    const localDataTrackInfos = this.outgoingDataTrackManager.queryPublished();
    this.engine.sendSyncState(remoteTracks, localTracks, localDataTrackInfos);
  }
  /**
   * After resuming, we'll need to notify the server of the current
   * subscription settings.
   */
  updateSubscriptions() {
    for (const p of this.remoteParticipants.values()) {
      for (const pub of p.videoTrackPublications.values()) {
        if (pub.isSubscribed && isRemotePub(pub)) {
          pub.emitTrackUpdate();
        }
      }
    }
  }
  getRemoteParticipantBySid(sid) {
    const identity = this.sidToIdentity.get(sid);
    if (identity) {
      return this.remoteParticipants.get(identity);
    }
  }
  /** The client capabilities this SDK advertises to other participants in its `ClientInfo`. */
  getClientInfoCapabilities(roomOptions) {
    var _a;
    const capabilities = [];
    if (isFrameMetadataSupported((_a = roomOptions.frameMetadata) !== null && _a !== void 0 ? _a : roomOptions.packetTrailer) || !!this.e2eeManager) {
      capabilities.push(ClientInfo_Capability.CAP_PACKET_TRAILER);
    }
    // Advertise deflate-raw decompression support so peers know they can send us compressed data
    // streams (gated separately from clientProtocol — see the data streams v2 spec).
    if (isDeflateRawCompressionSupported()) {
      capabilities.push(ClientInfo_Capability.CAP_COMPRESSION_DEFLATE_RAW);
    }
    return capabilities;
  }
  registerRpcDataStreamHandler() {
    this.incomingDataStreamManager.registerTextStreamHandler(RPC_REQUEST_DATA_STREAM_TOPIC, (reader_1, _a) => __awaiter(this, [reader_1, _a], void 0, function (reader, _ref7) {
      var _this6 = this;
      let identity = _ref7.identity;
      return function* () {
        var _b;
        const attributes = (_b = reader.info.attributes) !== null && _b !== void 0 ? _b : {};
        yield _this6.rpcServerManager.handleIncomingDataStream(reader, identity, attributes);
      }();
    }));
    this.incomingDataStreamManager.registerTextStreamHandler(RPC_RESPONSE_DATA_STREAM_TOPIC, (reader_1, _a) => __awaiter(this, [reader_1, _a], void 0, function (reader, _ref8) {
      var _this7 = this;
      let identity = _ref8.identity;
      return function* () {
        var _b;
        const attributes = (_b = reader.info.attributes) !== null && _b !== void 0 ? _b : {};
        yield _this7.rpcClientManager.handleIncomingDataStream(reader, identity, attributes);
      }();
    }));
  }
  setStatsLogging(enabled) {
    if (enabled) {
      if (!this.statsLogInterval) {
        this.statsLogInterval = CriticalTimers.setInterval(() => {
          // logWebRTCStats handles its own errors, nothing to await here
          this.logWebRTCStats();
        }, STATS_LOG_FREQUENCY_MS);
      }
    } else if (this.statsLogInterval) {
      CriticalTimers.clearInterval(this.statsLogInterval);
      this.statsLogInterval = undefined;
    }
  }
  registerConnectionReconcile() {
    this.clearConnectionReconcile();
    let consecutiveFailures = 0;
    this.connectionReconcileInterval = CriticalTimers.setInterval(() => {
      if (
      // ensure we didn't tear it down
      !this.engine ||
      // engine detected close, but Room missed it
      this.engine.isClosed ||
      // transports failed without notifying engine
      !this.engine.verifyTransport()) {
        consecutiveFailures++;
        this.log.warn('detected connection state mismatch', {
          numFailures: consecutiveFailures,
          engine: this.engine ? {
            closed: this.engine.isClosed,
            transportsConnectedOrConnecting: this.engine.verifyTransport()
          } : undefined
        });
        if (consecutiveFailures >= 3) {
          this.clearConnectionReconcile();
          if (this.engine && !this.engine.isClosed) {
            // The transport silently died while we still looked connected. Try a full reconnect
            // (keeps the room alive; the engine falls back to Disconnected if it ultimately fails).
            this.log.warn('detected connection state mismatch, attempting full reconnect');
            this.engine.reconnect();
          } else {
            // No usable engine to reconnect with; tear down.
            this.recreateEngine();
            this.handleDisconnect(this.options.stopLocalTrackOnUnpublish, DisconnectReason.STATE_MISMATCH);
          }
        }
      } else {
        consecutiveFailures = 0;
      }
    }, CONNECTION_RECONCILE_FREQUENCY_MS);
  }
  clearConnectionReconcile() {
    if (this.connectionReconcileInterval) {
      CriticalTimers.clearInterval(this.connectionReconcileInterval);
    }
  }
  setAndEmitConnectionState(state) {
    if (state === this.state) {
      // unchanged
      return false;
    }
    this.log.info("connection state changed: ".concat(this.state, " -> ").concat(state));
    this.state = state;
    this.incomingDataStreamManager.setConnected(state === ConnectionState.Connected);
    this.setStatsLogging(state === ConnectionState.Connected);
    this.emit(RoomEvent.ConnectionStateChanged, this.state);
    return true;
  }
  emitBufferedEvents() {
    this.bufferedEvents.forEach(_ref9 => {
      let _ref0 = _slicedToArray(_ref9, 2),
        ev = _ref0[0],
        args = _ref0[1];
      this.emit(ev, ...args);
    });
    this.bufferedEvents = [];
  }
  emitWhenConnected(event) {
    for (var _len2 = arguments.length, args = new Array(_len2 > 1 ? _len2 - 1 : 0), _key2 = 1; _key2 < _len2; _key2++) {
      args[_key2 - 1] = arguments[_key2];
    }
    if (this.state === ConnectionState.Reconnecting || this.isResuming || !this.engine || this.engine.pendingReconnect) {
      // in case the room is reconnecting, buffer the events by firing them later after emitting RoomEvent.Reconnected
      this.bufferedEvents.push([event, args]);
    } else if (this.state === ConnectionState.Connected) {
      return this.emit(event, ...args);
    }
    return false;
  }
  /**
   * Allows to populate a room with simulated participants.
   * No actual connection to a server will be established, all state is
   * @experimental
   */
  simulateParticipants(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d;
      const publishOptions = Object.assign({
        audio: true,
        video: true,
        useRealTracks: false
      }, options.publish);
      const participantOptions = Object.assign({
        count: 9,
        audio: false,
        video: true,
        aspectRatios: [1.66, 1.7, 1.3]
      }, options.participants);
      this.handleDisconnect();
      this.roomInfo = new Room$1({
        sid: 'RM_SIMULATED',
        name: 'simulated-room',
        emptyTimeout: 0,
        maxParticipants: 0,
        creationTime: protoInt64.parse(new Date().getTime()),
        metadata: '',
        numParticipants: 1,
        numPublishers: 1,
        turnPassword: '',
        enabledCodecs: [],
        activeRecording: false
      });
      this.localParticipant.updateInfo(new ParticipantInfo({
        identity: 'simulated-local',
        name: 'local-name'
      }));
      this.setupLocalParticipantEvents();
      this.emit(RoomEvent.SignalConnected);
      this.emit(RoomEvent.Connected);
      this.setAndEmitConnectionState(ConnectionState.Connected);
      if (publishOptions.video) {
        const camPub = new LocalTrackPublication(Track.Kind.Video, new TrackInfo({
          source: TrackSource.CAMERA,
          sid: Math.floor(Math.random() * 10000).toString(),
          type: TrackType.AUDIO,
          name: 'video-dummy'
        }), new LocalVideoTrack(publishOptions.useRealTracks && ((_a = window.navigator.mediaDevices) === null || _a === void 0 ? void 0 : _a.getUserMedia) ? (yield window.navigator.mediaDevices.getUserMedia({
          video: true
        })).getVideoTracks()[0] : createDummyVideoStreamTrack(160 * ((_b = participantOptions.aspectRatios[0]) !== null && _b !== void 0 ? _b : 1), 160, true, true), undefined, false, {
          loggerName: this.options.loggerName,
          loggerContextCb: () => this.logContext
        }), {
          loggerName: this.options.loggerName,
          loggerContextCb: () => this.logContext
        });
        // @ts-ignore
        this.localParticipant.addTrackPublication(camPub);
        this.localParticipant.emit(ParticipantEvent.LocalTrackPublished, camPub);
      }
      if (publishOptions.audio) {
        const audioPub = new LocalTrackPublication(Track.Kind.Audio, new TrackInfo({
          source: TrackSource.MICROPHONE,
          sid: Math.floor(Math.random() * 10000).toString(),
          type: TrackType.AUDIO
        }), new LocalAudioTrack(publishOptions.useRealTracks && ((_c = navigator.mediaDevices) === null || _c === void 0 ? void 0 : _c.getUserMedia) ? (yield navigator.mediaDevices.getUserMedia({
          audio: true
        })).getAudioTracks()[0] : this.acquireEmptyAudioStreamTrack(), undefined, false, this.audioContext, {
          loggerName: this.options.loggerName,
          loggerContextCb: () => this.logContext
        }), {
          loggerName: this.options.loggerName,
          loggerContextCb: () => this.logContext
        });
        // @ts-ignore
        this.localParticipant.addTrackPublication(audioPub);
        this.localParticipant.emit(ParticipantEvent.LocalTrackPublished, audioPub);
      }
      for (let i = 0; i < participantOptions.count - 1; i += 1) {
        let info = new ParticipantInfo({
          sid: Math.floor(Math.random() * 10000).toString(),
          identity: "simulated-".concat(i),
          state: ParticipantInfo_State.ACTIVE,
          tracks: [],
          joinedAt: protoInt64.parse(Date.now())
        });
        const p = this.getOrCreateParticipant(info.identity, info);
        if (participantOptions.video) {
          const dummyVideo = createDummyVideoStreamTrack(160 * ((_d = participantOptions.aspectRatios[i % participantOptions.aspectRatios.length]) !== null && _d !== void 0 ? _d : 1), 160, false, true);
          const videoTrack = new TrackInfo({
            source: TrackSource.CAMERA,
            sid: Math.floor(Math.random() * 10000).toString(),
            type: TrackType.AUDIO
          });
          p.addSubscribedMediaTrack(dummyVideo, videoTrack.sid, new MediaStream([dummyVideo]), new RTCRtpReceiver());
          info.tracks = [...info.tracks, videoTrack];
        }
        if (participantOptions.audio) {
          const dummyTrack = this.acquireEmptyAudioStreamTrack();
          const audioTrack = new TrackInfo({
            source: TrackSource.MICROPHONE,
            sid: Math.floor(Math.random() * 10000).toString(),
            type: TrackType.AUDIO
          });
          p.addSubscribedMediaTrack(dummyTrack, audioTrack.sid, new MediaStream([dummyTrack]), new RTCRtpReceiver());
          info.tracks = [...info.tracks, audioTrack];
        }
        p.updateInfo(info);
      }
    });
  }
  // /** @internal */
  emit(event) {
    for (var _len3 = arguments.length, args = new Array(_len3 > 1 ? _len3 - 1 : 0), _key3 = 1; _key3 < _len3; _key3++) {
      args[_key3 - 1] = arguments[_key3];
    }
    // active speaker updates are too spammy
    if (event !== RoomEvent.ActiveSpeakersChanged && event !== RoomEvent.TranscriptionReceived) {
      // only extract logContext from arguments in order to avoid logging the whole object tree
      const minimizedArgs = mapArgs(args).filter(arg => arg !== undefined);
      if (event === RoomEvent.TrackSubscribed || event === RoomEvent.TrackUnsubscribed) {
        this.log.trace("subscribe trace: ".concat(event), {
          event,
          args: minimizedArgs
        });
      }
      this.log.debug("room event ".concat(event), {
        event,
        args: minimizedArgs
      });
    }
    return super.emit(event, ...args);
  }
}
Room.cleanupRegistry = typeof FinalizationRegistry !== 'undefined' && typeof WeakRef !== 'undefined' && new FinalizationRegistry(cleanup => {
  cleanup();
});
function mapArgs(args) {
  return args.map(arg => {
    if (!arg) {
      return;
    }
    if (Array.isArray(arg)) {
      return mapArgs(arg);
    }
    if (typeof arg === 'object') {
      return 'logContext' in arg ? arg.logContext : undefined;
    }
    return arg;
  });
}
export { CONNECTION_RECONCILE_FREQUENCY_MS, ConnectionState, DUMMY_AUDIO_ELEMENT_ID, Room, STATS_LOG_FREQUENCY_MS, createSharedDummyAudioElement, mapArgs, sharedDummyAudioElement };
