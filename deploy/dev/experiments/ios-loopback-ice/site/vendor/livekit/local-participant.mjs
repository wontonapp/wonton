// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 32196-34011 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { protoInt64 } from './protobuf-int64.mjs';
import { __asyncValues, __awaiter, _slicedToArray } from './tslib.mjs';
import { AudioTrackFeature } from './protobuf-messages-enums.mjs';
import { DataPacket, DataPacket_Kind, Encryption_Type, VideoLayer_Mode } from './protobuf-messages-room.mjs';
import { ChatMessage, SipDTMF, UserPacket } from './protobuf-messages-chat.mjs';
import { AddTrackRequest, SimulcastCodec } from './protobuf-messages-signal.mjs';
import { RequestResponse_Reason } from './protobuf-messages-signal2.mjs';
import { TypedPromise } from './browser.mjs';
import { DeviceUnsupportedError, NegotiationError, PublishTrackError, SignalRequestError, TrackInvalidError, UnexpectedConnectionState } from './errors.mjs';
import { EngineEvent, ParticipantEvent, TrackEvent } from './event-enums.mjs';
import { ScreenSharePresets, VideoPresets, getTrackSourceFromProto, isBackupCodec, mergeDefaultOptions, mimeTypeToVideoCodecString, screenCaptureToDisplayMediaStreamOptions, sourceToKind } from './util.mjs';
import { Track, isChromiumBased, isE2EESimulcastSupported, isFireFox, isSVCCodec, isSVCSimulcast, isSVCSimulcastSupportedByServer, isSafari17Based, isWeb, sleep, supportsAV1, supportsVP9, usesLegacySVCEncodings } from './track.mjs';
import { Future, isAudioTrack, isDeviceAcquisitionFailure, isLocalAudioTrack, isLocalTrack, isLocalVideoTrack, isPermissionDeniedError, isVideoCodec, isVideoTrack } from './utils-misc.mjs';
import { getFrameMetadataFeatures, getFrameMetadataPublishOptions, hasFrameMetadataPublishOptions, isFrameMetadataSupported } from './e2ee-frame.mjs';
import { DeviceManager } from './device.mjs';
import { SignalConnectionState } from './signal-client.mjs';
import { PCTransportState, defaultVideoCodec } from './pc-transport.mjs';
import { DataChannelKind } from './data-channel.mjs';
import { LocalAudioTrack, computeStartTargetBitrate, computeTrackBackupEncodings, computeVideoEncodings, getDefaultDegradationPreference } from './local-track.mjs';
import { LocalVideoTrack, videoLayersFromEncodings } from './local-video-track.mjs';
import { DataTrackPublishError, LocalDataTrack } from './data-track-out.mjs';
import { LocalTrackPublication, createLocalTracks } from './publications.mjs';
import { Participant, trackPermissionToProto } from './participant.mjs';

class LocalParticipant extends Participant {
  /** @internal */
  constructor(sid, identity, engine, options, roomOutgoingDataStreamManager, roomOutgoingDataTrackManager, rpcClientManager, rpcServerManager) {
    super(sid, identity, undefined, undefined, undefined, {
      loggerName: options.loggerName,
      loggerContextCb: () => this.engine.logContext
    });
    this.pendingPublishing = new Set();
    this.pendingPublishPromises = new Map();
    this.participantTrackPermissions = [];
    this.allParticipantsAllowedToSubscribe = true;
    this.encryptionType = Encryption_Type.NONE;
    this.e2eeStateMutex = new _();
    this.enabledPublishVideoCodecs = [];
    this.handleReconnecting = () => {
      if (!this.reconnectFuture) {
        this.reconnectFuture = new Future();
      }
    };
    this.handleReconnected = () => {
      var _a, _b;
      (_b = (_a = this.reconnectFuture) === null || _a === void 0 ? void 0 : _a.resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
      this.reconnectFuture = undefined;
      this.updateTrackSubscriptionPermissions();
    };
    this.handleClosing = () => {
      var _a, _b, _c, _d, _e, _f;
      if (this.reconnectFuture) {
        // @throws-transformer ignore - introduced due to adding Throws into Future, investigate this
        // further
        this.reconnectFuture.promise.catch(e => this.log.warn(e.message));
        (_b = (_a = this.reconnectFuture) === null || _a === void 0 ? void 0 : _a.reject) === null || _b === void 0 ? void 0 : _b.call(_a, new Error('Got disconnected during reconnection attempt'));
        this.reconnectFuture = undefined;
      }
      if (this.signalConnectedFuture) {
        (_d = (_c = this.signalConnectedFuture).reject) === null || _d === void 0 ? void 0 : _d.call(_c, new Error('Got disconnected without signal connected'));
        this.signalConnectedFuture = undefined;
      }
      (_f = (_e = this.activeAgentFuture) === null || _e === void 0 ? void 0 : _e.reject) === null || _f === void 0 ? void 0 : _f.call(_e, new Error('Got disconnected without active agent present'));
      this.activeAgentFuture = undefined;
      this.firstActiveAgent = undefined;
    };
    this.handleSignalConnected = joinResponse => {
      var _a, _b;
      if (joinResponse.participant) {
        this.updateInfo(joinResponse.participant);
      }
      if (!this.signalConnectedFuture) {
        this.signalConnectedFuture = new Future();
      }
      (_b = (_a = this.signalConnectedFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
    };
    this.handleSignalRequestResponse = response => {
      const requestId = response.requestId,
        reason = response.reason,
        message = response.message;
      const targetRequest = this.pendingSignalRequests.get(requestId);
      if (targetRequest) {
        if (reason !== RequestResponse_Reason.OK) {
          targetRequest.reject(new SignalRequestError(message, reason));
        }
        this.pendingSignalRequests.delete(requestId);
      }
      switch (response.request.case) {
        case 'publishDataTrack':
          {
            let error;
            switch (response.reason) {
              case RequestResponse_Reason.NOT_ALLOWED:
                error = DataTrackPublishError.notAllowed(response.message);
                break;
              case RequestResponse_Reason.DUPLICATE_NAME:
                error = DataTrackPublishError.duplicateName(response.message);
                break;
              case RequestResponse_Reason.INVALID_NAME:
                error = DataTrackPublishError.invalidName(response.message);
                break;
              case RequestResponse_Reason.LIMIT_EXCEEDED:
                error = DataTrackPublishError.limitReached(response.message);
                break;
              default:
                error = DataTrackPublishError.unknown(response.reason, response.message);
                break;
            }
            this.roomOutgoingDataTrackManager.receivedSfuPublishResponse(response.request.value.pubHandle, {
              type: 'error',
              error
            });
            break;
          }
      }
    };
    this.updateTrackSubscriptionPermissions = () => {
      this.log.debug('updating track subscription permissions', {
        allParticipantsAllowed: this.allParticipantsAllowedToSubscribe,
        participantTrackPermissions: this.participantTrackPermissions
      });
      this.engine.client.sendUpdateSubscriptionPermissions(this.allParticipantsAllowedToSubscribe, this.participantTrackPermissions.map(p => trackPermissionToProto(p)));
    };
    /** @internal */
    this.onTrackUnmuted = track => {
      this.onTrackMuted(track, track.isUpstreamPaused);
    };
    // when the local track changes in mute status, we'll notify server as such
    /** @internal */
    this.onTrackMuted = (track, muted) => {
      if (muted === undefined) {
        muted = true;
      }
      if (!track.sid) {
        this.log.error('could not update mute status for unpublished track', getLogContextFromTrack(track));
        return;
      }
      this.engine.updateMuteStatus(track.sid, muted);
    };
    this.onTrackUpstreamPaused = track => {
      this.log.debug('upstream paused', getLogContextFromTrack(track));
      this.onTrackMuted(track, true);
    };
    this.onTrackUpstreamResumed = track => {
      this.log.debug('upstream resumed', getLogContextFromTrack(track));
      this.onTrackMuted(track, track.isMuted);
    };
    this.onTrackFeatureUpdate = track => {
      const pub = this.audioTrackPublications.get(track.sid);
      if (!pub) {
        this.log.warn("Could not update local audio track settings, missing publication for track ".concat(track.sid));
        return;
      }
      this.engine.client.sendUpdateLocalAudioTrack(pub.trackSid, pub.getTrackFeatures());
    };
    this.onTrackCpuConstrained = (track, publication) => {
      this.log.debug('track cpu constrained', getLogContextFromTrack(publication));
      this.emit(ParticipantEvent.LocalTrackCpuConstrained, track, publication);
    };
    this.handleSubscribedQualityUpdate = update => __awaiter(this, void 0, void 0, function* () {
      var _a, e_1, _b, _c;
      var _d;
      if (!((_d = this.roomOptions) === null || _d === void 0 ? void 0 : _d.dynacast)) {
        return;
      }
      const pub = this.videoTrackPublications.get(update.trackSid);
      if (!pub) {
        this.log.warn('received subscribed quality update for unknown track', {
          trackSid: update.trackSid
        });
        return;
      }
      if (!pub.videoTrack) {
        return;
      }
      const newCodecs = yield pub.videoTrack.setPublishingCodecs(update.subscribedCodecs);
      try {
        for (var _e = true, newCodecs_1 = __asyncValues(newCodecs), newCodecs_1_1; newCodecs_1_1 = yield newCodecs_1.next(), _a = newCodecs_1_1.done, !_a; _e = true) {
          _c = newCodecs_1_1.value;
          _e = false;
          const codec = _c;
          if (isBackupCodec(codec)) {
            this.log.debug("publish ".concat(codec, " for ").concat(pub.videoTrack.sid), getLogContextFromTrack(pub));
            yield this.publishAdditionalCodecForTrack(pub.videoTrack, codec, pub.options);
          }
        }
      } catch (e_1_1) {
        e_1 = {
          error: e_1_1
        };
      } finally {
        try {
          if (!_e && !_a && (_b = newCodecs_1.return)) yield _b.call(newCodecs_1);
        } finally {
          if (e_1) throw e_1.error;
        }
      }
    });
    this.handleLocalTrackUnpublished = unpublished => {
      const track = this.trackPublications.get(unpublished.trackSid);
      if (!track) {
        this.log.warn('received unpublished event for unknown track', {
          trackSid: unpublished.trackSid
        });
        return;
      }
      this.unpublishTrack(track.track);
    };
    this.handleTrackEnded = track => __awaiter(this, void 0, void 0, function* () {
      if (track.source === Track.Source.ScreenShare || track.source === Track.Source.ScreenShareAudio) {
        this.log.debug('unpublishing local track due to TrackEnded', getLogContextFromTrack(track));
        this.unpublishTrack(track);
      } else if (track.isUserProvided) {
        yield track.mute();
      } else if (isLocalAudioTrack(track) || isLocalVideoTrack(track)) {
        try {
          if (isWeb()) {
            try {
              const currentPermissions = yield navigator === null || navigator === void 0 ? void 0 : navigator.permissions.query({
                // the permission query for camera and microphone currently not supported in Safari and Firefox
                // @ts-ignore
                name: track.source === Track.Source.Camera ? 'camera' : 'microphone'
              });
              if (currentPermissions && currentPermissions.state === 'denied') {
                this.log.warn("user has revoked access to ".concat(track.source), getLogContextFromTrack(track));
                // detect granted change after permissions were denied to try and resume then
                currentPermissions.onchange = () => {
                  if (currentPermissions.state !== 'denied') {
                    if (!track.isMuted) {
                      track.restartTrack();
                    }
                    currentPermissions.onchange = null;
                  }
                };
                throw new Error('GetUserMedia Permission denied');
              }
            } catch (e) {
              // permissions query fails for firefox, we continue and try to restart the track
            }
          }
          if (!track.isMuted) {
            this.log.debug('track ended, attempting to use a different device', getLogContextFromTrack(track));
            if (isLocalAudioTrack(track)) {
              yield this.restartOnDefaultAudioDevice(track);
            } else {
              yield track.restartTrack();
            }
          }
        } catch (e) {
          this.log.warn("could not restart track, muting instead", getLogContextFromTrack(track));
          yield track.mute();
        }
      }
    });
    this.audioTrackPublications = new Map();
    this.videoTrackPublications = new Map();
    this.trackPublications = new Map();
    this.engine = engine;
    this.roomOptions = options;
    this.setupEngine(engine);
    this.activeDeviceMap = new Map([['audioinput', 'default'], ['videoinput', 'default'], ['audiooutput', 'default']]);
    this.pendingSignalRequests = new Map();
    this.roomOutgoingDataStreamManager = roomOutgoingDataStreamManager;
    this.roomOutgoingDataTrackManager = roomOutgoingDataTrackManager;
    this.rpcClientManager = rpcClientManager;
    this.rpcServerManager = rpcServerManager;
  }
  get lastCameraError() {
    return this.cameraError;
  }
  get lastMicrophoneError() {
    return this.microphoneError;
  }
  get isE2EEEnabled() {
    return this.encryptionType !== Encryption_Type.NONE;
  }
  getTrackPublication(source) {
    const track = super.getTrackPublication(source);
    if (track) {
      return track;
    }
  }
  getTrackPublicationByName(name) {
    const track = super.getTrackPublicationByName(name);
    if (track) {
      return track;
    }
  }
  /**
   * @internal
   */
  setupEngine(engine) {
    var _a;
    this.engine = engine;
    this.engine.on(EngineEvent.RemoteMute, (trackSid, muted) => {
      const pub = this.trackPublications.get(trackSid);
      if (!pub || !pub.track) {
        return;
      }
      if (muted) {
        pub.mute();
      } else {
        pub.unmute();
      }
    });
    if ((_a = this.signalConnectedFuture) === null || _a === void 0 ? void 0 : _a.isResolved) {
      this.signalConnectedFuture = undefined;
    }
    this.engine.on(EngineEvent.Connected, this.handleReconnected).on(EngineEvent.SignalConnected, this.handleSignalConnected).on(EngineEvent.SignalRestarted, this.handleReconnected).on(EngineEvent.SignalResumed, this.handleReconnected).on(EngineEvent.Restarting, this.handleReconnecting).on(EngineEvent.Resuming, this.handleReconnecting).on(EngineEvent.LocalTrackUnpublished, this.handleLocalTrackUnpublished).on(EngineEvent.SubscribedQualityUpdate, this.handleSubscribedQualityUpdate).on(EngineEvent.Closing, this.handleClosing).on(EngineEvent.SignalRequestResponse, this.handleSignalRequestResponse);
  }
  /**
   * Sets and updates the metadata of the local participant.
   * Note: this requires `canUpdateOwnMetadata` permission.
   * method will throw if the user doesn't have the required permissions
   * @param metadata
   */
  setMetadata(metadata) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.requestMetadataUpdate({
        metadata
      });
    });
  }
  /**
   * Sets and updates the name of the local participant.
   * Note: this requires `canUpdateOwnMetadata` permission.
   * method will throw if the user doesn't have the required permissions
   * @param metadata
   */
  setName(name) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.requestMetadataUpdate({
        name
      });
    });
  }
  /**
   * Set or update participant attributes. It will make updates only to keys that
   * are present in `attributes`, and will not override others.
   * Note: this requires `canUpdateOwnMetadata` permission.
   * @param attributes attributes to update
   */
  setAttributes(attributes) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.requestMetadataUpdate({
        attributes
      });
    });
  }
  requestMetadataUpdate(_a) {
    return __awaiter(this, arguments, void 0, function (_ref) {
      var _this = this;
      let metadata = _ref.metadata,
        name = _ref.name,
        attributes = _ref.attributes;
      return function* () {
        return new TypedPromise((resolve, reject) => __awaiter(_this, void 0, void 0, function* () {
          var _a, _b;
          try {
            let isRejected = false;
            const requestId = yield this.engine.client.sendUpdateLocalMetadata((_a = metadata !== null && metadata !== void 0 ? metadata : this.metadata) !== null && _a !== void 0 ? _a : '', (_b = name !== null && name !== void 0 ? name : this.name) !== null && _b !== void 0 ? _b : '', attributes);
            const startTime = performance.now();
            this.pendingSignalRequests.set(requestId, {
              resolve,
              reject: error => {
                reject(error);
                isRejected = true;
              },
              values: {
                name,
                metadata,
                attributes
              }
            });
            while (performance.now() - startTime < 5000 && !isRejected) {
              if ((!name || this.name === name) && (!metadata || this.metadata === metadata) && (!attributes || Object.entries(attributes).every(_ref2 => {
                let _ref3 = _slicedToArray(_ref2, 2),
                  key = _ref3[0],
                  value = _ref3[1];
                return this.attributes[key] === value || value === '' && !this.attributes[key];
              }))) {
                this.pendingSignalRequests.delete(requestId);
                resolve();
                return;
              }
              yield sleep(50);
            }
            reject(new SignalRequestError('Request to update local metadata timed out', 'TimeoutError'));
          } catch (e) {
            if (e instanceof Error) {
              reject(e);
            } else {
              reject(new Error(String(e)));
            }
          }
        }));
      }();
    });
  }
  /**
   * Enable or disable a participant's camera track.
   *
   * If a track has already published, it'll mute or unmute the track.
   * Resolves with a `LocalTrackPublication` instance if successful and `undefined` otherwise
   */
  setCameraEnabled(enabled, options, publishOptions) {
    return this.setTrackEnabled(Track.Source.Camera, enabled, options, publishOptions);
  }
  /**
   * Enable or disable a participant's microphone track.
   *
   * If a track has already published, it'll mute or unmute the track.
   * Resolves with a `LocalTrackPublication` instance if successful and `undefined` otherwise
   */
  setMicrophoneEnabled(enabled, options, publishOptions) {
    return this.setTrackEnabled(Track.Source.Microphone, enabled, options, publishOptions);
  }
  /**
   * Start or stop sharing a participant's screen
   * Resolves with a `LocalTrackPublication` instance if successful and `undefined` otherwise
   */
  setScreenShareEnabled(enabled, options, publishOptions) {
    return this.setTrackEnabled(Track.Source.ScreenShare, enabled, options, publishOptions);
  }
  /** @internal */
  setE2EEEnabled(enabled) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.e2eeStateMutex.lock();
      try {
        this.encryptionType = enabled ? Encryption_Type.GCM : Encryption_Type.NONE;
        yield Promise.all(this.pendingPublishPromises.values());
        if (this.trackPublications.size === 0 || Array.from(this.trackPublications.values()).every(pub => pub.isEncrypted === enabled)) {
          return;
        }
        yield this.republishAllTracks(undefined, false);
      } finally {
        unlock();
      }
    });
  }
  setTrackEnabled(source, enabled, options, publishOptions) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      this.log.debug('setTrackEnabled', {
        source,
        enabled
      });
      if (this.republishPromise) {
        yield this.republishPromise;
      }
      let track = this.getTrackPublication(source);
      if (enabled) {
        if (track) {
          yield track.unmute();
        } else {
          let localTracks;
          if (this.pendingPublishing.has(source)) {
            const pendingTrack = yield this.waitForPendingPublicationOfSource(source);
            if (!pendingTrack) {
              this.log.info('waiting for pending publication promise timed out', {
                source
              });
            }
            yield pendingTrack === null || pendingTrack === void 0 ? void 0 : pendingTrack.unmute();
            return pendingTrack;
          }
          this.pendingPublishing.add(source);
          try {
            switch (source) {
              case Track.Source.Camera:
                localTracks = yield this.createTracks({
                  video: (_a = options) !== null && _a !== void 0 ? _a : true
                });
                break;
              case Track.Source.Microphone:
                localTracks = yield this.createTracks({
                  audio: (_b = options) !== null && _b !== void 0 ? _b : true
                });
                break;
              case Track.Source.ScreenShare:
                localTracks = yield this.createScreenTracks(Object.assign({}, options));
                break;
              default:
                throw new TrackInvalidError(source);
            }
          } catch (e) {
            localTracks === null || localTracks === void 0 ? void 0 : localTracks.forEach(tr => {
              tr.stop();
            });
            if (e instanceof Error) {
              this.emit(ParticipantEvent.MediaDevicesError, e, sourceToKind(source));
            }
            this.pendingPublishing.delete(source);
            throw e;
          }
          for (const localTrack of localTracks) {
            const opts = Object.assign(Object.assign({}, this.roomOptions.publishDefaults), options);
            if (source === Track.Source.Microphone && isAudioTrack(localTrack) && opts.preConnectBuffer) {
              this.log.info('starting preconnect buffer for microphone');
              localTrack.startPreConnectBuffer();
            }
          }
          try {
            const publishPromises = [];
            for (const localTrack of localTracks) {
              this.log.info('publishing track', getLogContextFromTrack(localTrack));
              publishPromises.push(this.publishTrack(localTrack, publishOptions));
            }
            const publishedTracks = yield Promise.all(publishPromises);
            // for screen share publications including audio, this will only return the screen share publication, not the screen share audio one
            // revisit if we want to return an array of tracks instead for v2
            var _publishedTracks = _slicedToArray(publishedTracks, 1);
            track = _publishedTracks[0];
          } catch (e) {
            localTracks === null || localTracks === void 0 ? void 0 : localTracks.forEach(tr => {
              tr.stop();
            });
            throw e;
          } finally {
            this.pendingPublishing.delete(source);
          }
        }
      } else {
        if (!(track === null || track === void 0 ? void 0 : track.track) && this.pendingPublishing.has(source)) {
          // if there's no track available yet first wait for pending publishing promises of that source to see if it becomes available
          track = yield this.waitForPendingPublicationOfSource(source);
          if (!track) {
            this.log.info('waiting for pending publication promise timed out', {
              source
            });
          }
        }
        if (track && track.track) {
          // screenshare cannot be muted, unpublish instead
          if (source === Track.Source.ScreenShare) {
            const unpublishPromises = [this.unpublishTrack(track.track)];
            const screenAudioTrack = this.getTrackPublication(Track.Source.ScreenShareAudio);
            if (screenAudioTrack && screenAudioTrack.track) {
              unpublishPromises.push(this.unpublishTrack(screenAudioTrack.track));
            }
            var _yield$Promise$all = yield Promise.all(unpublishPromises);
            var _yield$Promise$all2 = _slicedToArray(_yield$Promise$all, 1);
            track = _yield$Promise$all2[0];
          } else {
            yield track.mute();
          }
        }
      }
      return track;
    });
  }
  /**
   * Publish both camera and microphone at the same time. This is useful for
   * displaying a single Permission Dialog box to the end user.
   */
  enableCameraAndMicrophone() {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.pendingPublishing.has(Track.Source.Camera) || this.pendingPublishing.has(Track.Source.Microphone)) {
        // no-op it's already been requested
        return;
      }
      this.pendingPublishing.add(Track.Source.Camera);
      this.pendingPublishing.add(Track.Source.Microphone);
      try {
        const tracks = yield this.createTracks({
          audio: true,
          video: true
        });
        yield Promise.all(tracks.map(track => this.publishTrack(track)));
      } finally {
        this.pendingPublishing.delete(Track.Source.Camera);
        this.pendingPublishing.delete(Track.Source.Microphone);
      }
    });
  }
  /**
   * Create local camera and/or microphone tracks
   * @param options
   * @returns
   */
  createTracks(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      options !== null && options !== void 0 ? options : options = {};
      const mergedOptionsWithProcessors = mergeDefaultOptions(options, (_a = this.roomOptions) === null || _a === void 0 ? void 0 : _a.audioCaptureDefaults, (_b = this.roomOptions) === null || _b === void 0 ? void 0 : _b.videoCaptureDefaults);
      try {
        const tracks = yield createLocalTracks(mergedOptionsWithProcessors, {
          loggerName: this.roomOptions.loggerName,
          loggerContextCb: () => this.logContext
        });
        const localTracks = yield Promise.all(tracks.map(track => __awaiter(this, void 0, void 0, function* () {
          if (isAudioTrack(track)) {
            this.microphoneError = undefined;
            yield track.setAudioContext(this.audioContext);
            track.source = Track.Source.Microphone;
            this.emit(ParticipantEvent.AudioStreamAcquired);
          }
          if (isVideoTrack(track)) {
            this.cameraError = undefined;
            track.source = Track.Source.Camera;
          }
          return track;
        })));
        return localTracks;
      } catch (err) {
        if (err instanceof Error) {
          if (options.audio) {
            this.microphoneError = err;
          }
          if (options.video) {
            this.cameraError = err;
          }
        }
        throw err;
      }
    });
  }
  /**
   * Creates a screen capture tracks with getDisplayMedia().
   * A LocalVideoTrack is always created and returned.
   * If { audio: true }, and the browser supports audio capture, a LocalAudioTrack is also created.
   */
  createScreenTracks(options) {
    return __awaiter(this, void 0, void 0, function* () {
      if (options === undefined) {
        options = {};
      }
      if (navigator.mediaDevices.getDisplayMedia === undefined) {
        throw new DeviceUnsupportedError('getDisplayMedia not supported');
      }
      if (options.resolution === undefined && !isSafari17Based()) {
        // we need to constrain the dimensions, otherwise it could lead to low bitrate
        // due to encoding a huge video. Encoding such large surfaces is really expensive
        // unfortunately Safari 17 has a but and cannot be constrained by default
        options.resolution = ScreenSharePresets.h1080fps30.resolution;
      }
      const constraints = screenCaptureToDisplayMediaStreamOptions(options);
      const stream = yield navigator.mediaDevices.getDisplayMedia(constraints);
      const tracks = stream.getVideoTracks();
      if (tracks.length === 0) {
        throw new TrackInvalidError('no video track found');
      }
      const screenVideo = new LocalVideoTrack(tracks[0], undefined, false, {
        loggerName: this.roomOptions.loggerName,
        loggerContextCb: () => this.logContext
      });
      screenVideo.source = Track.Source.ScreenShare;
      if (options.contentHint) {
        screenVideo.mediaStreamTrack.contentHint = options.contentHint;
      }
      const localTracks = [screenVideo];
      if (stream.getAudioTracks().length > 0) {
        this.emit(ParticipantEvent.AudioStreamAcquired);
        const screenAudio = new LocalAudioTrack(stream.getAudioTracks()[0], undefined, false, this.audioContext, {
          loggerName: this.roomOptions.loggerName,
          loggerContextCb: () => this.logContext
        });
        screenAudio.source = Track.Source.ScreenShareAudio;
        localTracks.push(screenAudio);
      }
      return localTracks;
    });
  }
  /**
   * Publish a new track to the room
   * @param track
   * @param options
   */
  publishTrack(track, options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.publishOrRepublishTrack(track, options);
    });
  }
  /**
   * Waits for the engine's next `Restarted` event. Unlike `engine.waitForRestarted`, this does
   * not short-circuit when `pcState === Connected` — at the point this is called (right after a
   * `NegotiationError`) the PC transport is still connected, but `fullReconnectOnNext` has been
   * set and `attemptReconnect` is queued via setTimeout. We need to wait for that restart to
   * actually complete (which clears `pendingTrackResolvers` via `cleanupClient`) before retrying.
   */
  waitForNextEngineRestart() {
    let timeoutMs = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 15000;
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        this.engine.off(EngineEvent.Restarted, onRestarted);
        this.engine.off(EngineEvent.Closing, onClosing);
      };
      const onRestarted = () => {
        cleanup();
        resolve();
      };
      const onClosing = () => {
        cleanup();
        reject(new Error('engine closed before restart completed'));
      };
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error('timed out waiting for engine restart'));
      }, timeoutMs);
      this.engine.once(EngineEvent.Restarted, onRestarted);
      this.engine.once(EngineEvent.Closing, onClosing);
    });
  }
  publishOrRepublishTrack(track_1, options_1) {
    return __awaiter(this, arguments, void 0, function (track, options) {
      var _this2 = this;
      let isRepublish = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : false;
      let hasRetriedAfterNegotiationError = arguments.length > 3 && arguments[3] !== undefined ? arguments[3] : false;
      return function* () {
        var _a, _b, _c, _d;
        if (isLocalAudioTrack(track)) {
          yield track.setAudioContext(_this2.audioContext);
        }
        yield (_a = _this2.reconnectFuture) === null || _a === void 0 ? void 0 : _a.promise;
        if (_this2.republishPromise && !isRepublish) {
          yield _this2.republishPromise;
        }
        if (isLocalTrack(track) && _this2.pendingPublishPromises.has(track)) {
          yield _this2.pendingPublishPromises.get(track);
        }
        let defaultConstraints;
        if (track instanceof MediaStreamTrack) {
          defaultConstraints = track.getConstraints();
        } else {
          // we want to access constraints directly as `track.mediaStreamTrack`
          // might be pointing to a non-device track (e.g. processed track) already
          defaultConstraints = track.constraints;
          let deviceKind = undefined;
          switch (track.source) {
            case Track.Source.Microphone:
              deviceKind = 'audioinput';
              break;
            case Track.Source.Camera:
              deviceKind = 'videoinput';
          }
          if (deviceKind && _this2.activeDeviceMap.has(deviceKind)) {
            defaultConstraints = Object.assign(Object.assign({}, defaultConstraints), {
              deviceId: _this2.activeDeviceMap.get(deviceKind)
            });
          }
        }
        // convert raw media track into audio or video track
        if (track instanceof MediaStreamTrack) {
          switch (track.kind) {
            case 'audio':
              track = new LocalAudioTrack(track, defaultConstraints, true, _this2.audioContext, {
                loggerName: _this2.roomOptions.loggerName,
                loggerContextCb: () => _this2.logContext
              });
              break;
            case 'video':
              track = new LocalVideoTrack(track, defaultConstraints, true, {
                loggerName: _this2.roomOptions.loggerName,
                loggerContextCb: () => _this2.logContext
              });
              break;
            default:
              throw new TrackInvalidError("unsupported MediaStreamTrack kind ".concat(track.kind));
          }
        } else {
          track.updateLoggerOptions({
            loggerName: _this2.roomOptions.loggerName,
            loggerContextCb: () => _this2.logContext
          });
        }
        // is it already published? if so skip
        let existingPublication;
        _this2.trackPublications.forEach(publication => {
          if (!publication.track) {
            return;
          }
          if (publication.track === track) {
            existingPublication = publication;
          }
        });
        if (existingPublication) {
          _this2.log.warn('track has already been published, skipping', getLogContextFromTrack(existingPublication));
          return existingPublication;
        }
        const opts = Object.assign(Object.assign({}, _this2.roomOptions.publishDefaults), options);
        const isStereoInput = 'channelCount' in track.mediaStreamTrack.getSettings() &&
        // @ts-ignore `channelCount` on getSettings() is currently only available for Safari, but is generally the best way to determine a stereo track https://developer.mozilla.org/en-US/docs/Web/API/MediaTrackSettings/channelCount
        track.mediaStreamTrack.getSettings().channelCount === 2 || track.mediaStreamTrack.getConstraints().channelCount === 2;
        const isStereo = (_b = opts.forceStereo) !== null && _b !== void 0 ? _b : isStereoInput;
        // disable dtx for stereo track if not enabled explicitly
        if (isStereo) {
          if (opts.dtx === undefined) {
            _this2.log.debug("Opus DTX will be disabled for stereo tracks by default. Enable them explicitly to make it work.", getLogContextFromTrack(track));
          }
          if (opts.red === undefined) {
            _this2.log.debug("Opus RED will be disabled for stereo tracks by default. Enable them explicitly to make it work.");
          }
          (_c = opts.dtx) !== null && _c !== void 0 ? _c : opts.dtx = false;
          (_d = opts.red) !== null && _d !== void 0 ? _d : opts.red = false;
        }
        if (!isE2EESimulcastSupported() && _this2.roomOptions.e2ee) {
          _this2.log.info("End-to-end encryption is set up, simulcast publishing will be disabled on Safari versions and iOS browsers running iOS < v17.2");
          opts.simulcast = false;
        }
        if (opts.source) {
          track.source = opts.source;
        }
        const publishPromise = new Promise((resolve, reject) => __awaiter(_this2, void 0, void 0, function* () {
          try {
            if (this.engine.client.currentState !== SignalConnectionState.CONNECTED) {
              this.log.debug('deferring track publication until signal is connected', {
                track: getLogContextFromTrack(track)
              });
              let publicationTimedOut = false;
              const timeout = setTimeout(() => {
                publicationTimedOut = true;
                track.stop();
                reject(new PublishTrackError('publishing rejected as engine not connected within timeout', 408));
              }, 15000);
              yield this.waitUntilEngineConnected();
              clearTimeout(timeout);
              if (publicationTimedOut) {
                return;
              }
              const publication = yield this.publish(track, opts, isStereo);
              resolve(publication);
            } else {
              try {
                const publication = yield this.publish(track, opts, isStereo);
                resolve(publication);
              } catch (e) {
                reject(e);
              }
            }
          } catch (e) {
            reject(e);
          }
        }));
        _this2.pendingPublishPromises.set(track, publishPromise);
        try {
          const publication = yield publishPromise;
          return publication;
        } catch (e) {
          if (!hasRetriedAfterNegotiationError && e instanceof NegotiationError) {
            _this2.log.warn('negotiation due to track publish failed, retrying after reconnect', {
              error: e
            });
            _this2.pendingPublishPromises.delete(track);
            yield _this2.waitForNextEngineRestart();
            return yield _this2.publishOrRepublishTrack(track, options, isRepublish, true);
          }
          throw e;
        } finally {
          _this2.pendingPublishPromises.delete(track);
        }
      }();
    });
  }
  waitUntilEngineConnected() {
    if (!this.signalConnectedFuture) {
      this.signalConnectedFuture = new Future();
    }
    return this.signalConnectedFuture.promise;
  }
  hasPermissionsToPublish(track) {
    if (!this.permissions) {
      this.log.warn('no permissions present for publishing track', getLogContextFromTrack(track));
      return false;
    }
    const _this$permissions = this.permissions,
      canPublish = _this$permissions.canPublish,
      canPublishSources = _this$permissions.canPublishSources;
    if (canPublish && (canPublishSources.length === 0 || canPublishSources.map(source => getTrackSourceFromProto(source)).includes(track.source))) {
      return true;
    }
    this.log.warn('insufficient permissions to publish', getLogContextFromTrack(track));
    return false;
  }
  publish(track, opts, isStereo) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e, _f, _g, _h, _j, _k, _l;
      if (!this.hasPermissionsToPublish(track)) {
        throw new PublishTrackError('failed to publish track, insufficient permissions', 403);
      }
      const existingTrackOfSource = Array.from(this.trackPublications.values()).find(publishedTrack => isLocalTrack(track) && publishedTrack.source === track.source);
      if (existingTrackOfSource && track.source !== Track.Source.Unknown) {
        this.log.info("publishing a second track with the same source: ".concat(track.source), getLogContextFromTrack(track));
      }
      if (opts.stopMicTrackOnMute && isAudioTrack(track)) {
        track.stopOnMute = true;
      }
      if (track.source === Track.Source.ScreenShare && isFireFox()) {
        // Firefox does not work well with simulcasted screen share
        // we frequently get no data on layer 0 when enabled
        opts.simulcast = false;
      }
      // require full AV1/VP9 SVC support prior to using it
      if (opts.videoCodec === 'av1' && !supportsAV1()) {
        opts.videoCodec = undefined;
      }
      if (opts.videoCodec === 'vp9' && !supportsVP9()) {
        opts.videoCodec = undefined;
      }
      if (opts.videoCodec === undefined) {
        opts.videoCodec = defaultVideoCodec;
      }
      if (this.enabledPublishVideoCodecs.length > 0) {
        // fallback to a supported codec if it is not supported
        if (!this.enabledPublishVideoCodecs.some(c => opts.videoCodec === mimeTypeToVideoCodecString(c.mime))) {
          opts.videoCodec = mimeTypeToVideoCodecString(this.enabledPublishVideoCodecs[0].mime);
        }
      }
      const videoCodec = opts.videoCodec;
      // handle track actions
      track.on(TrackEvent.Muted, this.onTrackMuted);
      track.on(TrackEvent.Unmuted, this.onTrackUnmuted);
      track.on(TrackEvent.Ended, this.handleTrackEnded);
      track.on(TrackEvent.UpstreamPaused, this.onTrackUpstreamPaused);
      track.on(TrackEvent.UpstreamResumed, this.onTrackUpstreamResumed);
      track.on(TrackEvent.AudioTrackFeatureUpdate, this.onTrackFeatureUpdate);
      const audioFeatures = [];
      const disableDtx = !((_a = opts.dtx) !== null && _a !== void 0 ? _a : true);
      const settings = track.getSourceTrackSettings();
      if (settings.autoGainControl) {
        audioFeatures.push(AudioTrackFeature.TF_AUTO_GAIN_CONTROL);
      }
      if (settings.echoCancellation) {
        audioFeatures.push(AudioTrackFeature.TF_ECHO_CANCELLATION);
      }
      if (settings.noiseSuppression) {
        audioFeatures.push(AudioTrackFeature.TF_NOISE_SUPPRESSION);
      }
      if (settings.channelCount && settings.channelCount > 1) {
        audioFeatures.push(AudioTrackFeature.TF_STEREO);
      }
      if (disableDtx) {
        audioFeatures.push(AudioTrackFeature.TF_NO_DTX);
      }
      if (isLocalAudioTrack(track) && track.hasPreConnectBuffer) {
        audioFeatures.push(AudioTrackFeature.TF_PRECONNECT_BUFFER);
      }
      const packetTrailerFeatures = this.normalizeRequestedFrameMetadataOptions(track, opts);
      // create track publication from track
      const req = new AddTrackRequest({
        // get local track id for use during publishing
        cid: track.mediaStreamTrack.id,
        name: opts.name,
        type: Track.kindToProto(track.kind),
        muted: track.isMuted,
        source: Track.sourceToProto(track.source),
        disableDtx,
        encryption: this.encryptionType,
        stereo: isStereo,
        disableRed: this.isE2EEEnabled || !((_b = opts.red) !== null && _b !== void 0 ? _b : true),
        stream: opts === null || opts === void 0 ? void 0 : opts.stream,
        backupCodecPolicy: opts === null || opts === void 0 ? void 0 : opts.backupCodecPolicy,
        audioFeatures,
        packetTrailerFeatures
      });
      // compute encodings and layers for video
      let encodings;
      if (track.kind === Track.Kind.Video) {
        let dims;
        try {
          dims = yield track.waitForDimensions();
        } catch (e) {
          // use defaults, it's quite painful for congestion control without simulcast
          // so using default dims according to publish settings
          const defaultRes = (_d = (_c = this.roomOptions.videoCaptureDefaults) === null || _c === void 0 ? void 0 : _c.resolution) !== null && _d !== void 0 ? _d : VideoPresets.h720.resolution;
          dims = {
            width: defaultRes.width,
            height: defaultRes.height
          };
          // log failure
          this.log.error('could not determine track dimensions, using defaults', Object.assign(Object.assign({}, getLogContextFromTrack(track)), {
            dims
          }));
        }
        // width and height should be defined for video
        req.width = dims.width;
        req.height = dims.height;
        // for svc codecs, disable simulcast and use vp8 for backup codec
        if (isLocalVideoTrack(track)) {
          if (isSVCSimulcast(videoCodec, opts) && (usesLegacySVCEncodings() || !isSVCSimulcastSupportedByServer((_e = this.engine) === null || _e === void 0 ? void 0 : _e.serverVersion))) {
            opts.simulcast = false;
            this.log.info('SVC simulcast is not supported, disabling simulcast.', getLogContextFromTrack(track));
          }
          const svcSimulcast = isSVCSimulcast(videoCodec, opts);
          if (isSVCCodec(videoCodec) && !svcSimulcast) {
            if (track.source === Track.Source.ScreenShare) {
              // vp9 svc with screenshare cannot encode multiple spatial layers
              // doing so reduces publish resolution to minimal resolution
              opts.scalabilityMode = 'L1T3';
              // Chrome does not allow more than 5 fps with L1T3, and it has encoding bugs with L3T3
              // It has a different path for screenshare handling and it seems to be untested/buggy
              // As a workaround, we are setting contentHint to force it to go through the same
              // path as regular camera video. While this is not optimal, it delivers the performance
              // that we need
              if ('contentHint' in track.mediaStreamTrack) {
                track.mediaStreamTrack.contentHint = 'motion';
                this.log.debug('forcing contentHint to motion for screenshare with SVC codecs', getLogContextFromTrack(track));
              }
            }
            // set scalabilityMode to 'L3T3_KEY' by default
            opts.scalabilityMode = (_f = opts.scalabilityMode) !== null && _f !== void 0 ? _f : 'L3T3_KEY';
          }
          const primaryCodec = new SimulcastCodec({
            codec: videoCodec,
            cid: track.mediaStreamTrack.id
          });
          if (svcSimulcast) {
            primaryCodec.videoLayerMode = VideoLayer_Mode.ONE_SPATIAL_LAYER_PER_STREAM;
          }
          req.simulcastCodecs = [primaryCodec];
          // set up backup
          if (opts.backupCodec === true) {
            opts.backupCodec = {
              codec: defaultVideoCodec
            };
          }
          if (opts.backupCodec && videoCodec !== opts.backupCodec.codec &&
          // TODO remove this once e2ee is supported for backup codecs
          req.encryption === Encryption_Type.NONE) {
            // multi-codec simulcast requires dynacast
            if (!this.roomOptions.dynacast) {
              this.roomOptions.dynacast = true;
            }
            req.simulcastCodecs.push(new SimulcastCodec({
              codec: opts.backupCodec.codec,
              cid: ''
            }));
          }
        }
        encodings = computeVideoEncodings(track.source === Track.Source.ScreenShare, req.width, req.height, opts);
        req.layers = videoLayersFromEncodings(req.width, req.height, encodings, isSVCCodec(opts.videoCodec) && !isSVCSimulcast(opts.videoCodec, opts));
      } else if (track.kind === Track.Kind.Audio) {
        encodings = [{
          maxBitrate: (_g = opts.audioPreset) === null || _g === void 0 ? void 0 : _g.maxBitrate,
          priority: (_j = (_h = opts.audioPreset) === null || _h === void 0 ? void 0 : _h.priority) !== null && _j !== void 0 ? _j : 'high',
          networkPriority: (_l = (_k = opts.audioPreset) === null || _k === void 0 ? void 0 : _k.priority) !== null && _l !== void 0 ? _l : 'high'
        }];
      }
      if (!this.engine || this.engine.isClosed) {
        throw new UnexpectedConnectionState('cannot publish track when not connected');
      }
      const negotiate = () => __awaiter(this, void 0, void 0, function* () {
        var _a, _b;
        if (!this.engine.pcManager) {
          throw new UnexpectedConnectionState('pcManager is not ready');
        }
        track.sender = yield this.engine.createSender(track, opts, encodings);
        if (isLocalVideoTrack(track)) {
          track.publishOptions = opts;
        }
        this.emit(ParticipantEvent.LocalSenderCreated, track.sender, track);
        if (isLocalVideoTrack(track)) {
          (_a = opts.degradationPreference) !== null && _a !== void 0 ? _a : opts.degradationPreference = getDefaultDegradationPreference(track);
          track.setDegradationPreference(opts.degradationPreference);
        }
        if (encodings) {
          if (isFireFox() && track.kind === Track.Kind.Audio) {
            /* Refer to RFC https://datatracker.ietf.org/doc/html/rfc7587#section-6.1,
               livekit-server uses maxaveragebitrate=510000 in the answer sdp to permit client to
               publish high quality audio track. But firefox always uses this value as the actual
               bitrates, causing the audio bitrates to rise to 510Kbps in any stereo case unexpectedly.
               So the client need to modify maxaverragebitrates in answer sdp to user provided value to
               fix the issue.
             */
            let trackTransceiver = undefined;
            for (const transceiver of this.engine.pcManager.publisher.getTransceivers()) {
              if (transceiver.sender === track.sender) {
                trackTransceiver = transceiver;
                break;
              }
            }
            if (trackTransceiver) {
              this.engine.pcManager.publisher.setTrackCodecBitrate({
                transceiver: trackTransceiver,
                codec: 'opus',
                maxbr: ((_b = encodings[0]) === null || _b === void 0 ? void 0 : _b.maxBitrate) ? encodings[0].maxBitrate / 1000 : 0
              });
            }
          } else if (track.codec && isVideoCodec(track.codec)) {
            // Apply start bitrate for all video codecs to prevent initial blurriness,
            // see computeStartTargetBitrate
            const targetBitrate = computeStartTargetBitrate(track.codec, opts, encodings);
            if (targetBitrate > 0) {
              this.engine.pcManager.publisher.setTrackCodecBitrate({
                cid: req.cid,
                codec: track.codec,
                maxbr: targetBitrate / 1000,
                isScreenShare: track.source === Track.Source.ScreenShare
              });
            }
          }
        }
        yield this.engine.negotiate();
      });
      let ti;
      const addTrackPromise = new Promise((resolve, reject) => __awaiter(this, void 0, void 0, function* () {
        var _a;
        try {
          ti = yield this.engine.addTrack(req);
          resolve(ti);
        } catch (err) {
          if (track.sender && ((_a = this.engine.pcManager) === null || _a === void 0 ? void 0 : _a.publisher)) {
            try {
              this.engine.pcManager.publisher.removeTrack(track.sender);
            } catch (e) {
              this.log.error(e);
            }
            yield this.engine.negotiate().catch(negotiateErr => {
              this.log.error('failed to negotiate after removing track due to failed add track request', Object.assign(Object.assign({}, getLogContextFromTrack(track)), {
                error: negotiateErr
              }));
            });
          }
          reject(err);
        }
      }));
      if (this.enabledPublishVideoCodecs.length > 0 && packetTrailerFeatures.length === 0) {
        const rets = yield Promise.all([addTrackPromise, negotiate()]);
        ti = rets[0];
      } else {
        ti = yield addTrackPromise;
        // server might not support the codec the client has requested, in that case, fallback
        // to a supported codec
        let primaryCodecMime;
        ti.codecs.forEach(codec => {
          if (primaryCodecMime === undefined) {
            primaryCodecMime = codec.mimeType;
          }
        });
        if (primaryCodecMime && track.kind === Track.Kind.Video) {
          const updatedCodec = mimeTypeToVideoCodecString(primaryCodecMime);
          if (updatedCodec !== videoCodec) {
            this.log.debug('falling back to server selected codec', Object.assign(Object.assign({}, getLogContextFromTrack(track)), {
              codec: updatedCodec
            }));
            opts.videoCodec = updatedCodec;
            // recompute encodings since bitrates/etc could have changed
            encodings = computeVideoEncodings(track.source === Track.Source.ScreenShare, req.width, req.height, opts);
          }
        }
        yield negotiate();
      }
      const publication = new LocalTrackPublication(track.kind, ti, track, {
        loggerName: this.roomOptions.loggerName,
        loggerContextCb: () => this.logContext
      });
      publication.on(TrackEvent.CpuConstrained, constrainedTrack => this.onTrackCpuConstrained(constrainedTrack, publication));
      // save options for when it needs to be republished again
      publication.options = opts;
      track.sid = ti.sid;
      // keep publish options on the video track so that it can recompute encoding
      // parameters when the MediaStreamTrack is restarted (e.g. after switching cameras).
      // Seed the dimensions we encoded at publish time so the first no-op restart
      // (e.g. unmute with unchanged constraints) can skip the recompute.
      if (isLocalVideoTrack(track)) {
        track.publishOptions = opts;
        if (req.width && req.height) {
          track.lastEncodedDimensions = {
            width: req.width,
            height: req.height
          };
        }
      }
      this.log.debug("publishing ".concat(track.kind, " with encodings"), {
        encodings,
        trackInfo: ti
      });
      if (isLocalVideoTrack(track)) {
        track.startMonitor(this.engine.client);
      } else if (isLocalAudioTrack(track)) {
        track.startMonitor();
      }
      this.addTrackPublication(publication);
      // send event for publication
      this.emit(ParticipantEvent.LocalTrackPublished, publication);
      if (isLocalAudioTrack(track) && ti.audioFeatures.includes(AudioTrackFeature.TF_PRECONNECT_BUFFER)) {
        const stream = track.getPreConnectBuffer();
        const mimeType = track.getPreConnectBufferMimeType();
        // TODO: we're registering the listener after negotiation, so there might be a race
        this.on(ParticipantEvent.LocalTrackSubscribed, pub => {
          if (pub.trackSid === ti.sid) {
            if (!track.hasPreConnectBuffer) {
              this.log.warn('subscribe event came to late, buffer already closed');
              return;
            }
            this.log.debug('finished recording preconnect buffer', getLogContextFromTrack(track));
            track.stopPreConnectBuffer();
          }
        });
        if (stream) {
          const bufferStreamPromise = new Promise((resolve, reject) => __awaiter(this, void 0, void 0, function* () {
            var _a, e_2, _b, _c;
            var _d, _e;
            try {
              this.log.debug('waiting for agent', getLogContextFromTrack(track));
              const agentActiveTimeout = setTimeout(() => {
                reject(new Error('agent not active within 10 seconds'));
              }, 10000);
              const agent = yield this.waitUntilActiveAgentPresent();
              clearTimeout(agentActiveTimeout);
              this.log.debug('sending preconnect buffer', getLogContextFromTrack(track));
              const writer = yield this.streamBytes({
                name: 'preconnect-buffer',
                mimeType,
                topic: 'lk.agent.pre-connect-audio-buffer',
                destinationIdentities: [agent.identity],
                attributes: {
                  trackId: publication.trackSid,
                  sampleRate: String((_d = settings.sampleRate) !== null && _d !== void 0 ? _d : '48000'),
                  channels: String((_e = settings.channelCount) !== null && _e !== void 0 ? _e : '1')
                }
              });
              try {
                for (var _f = true, stream_1 = __asyncValues(stream), stream_1_1; stream_1_1 = yield stream_1.next(), _a = stream_1_1.done, !_a; _f = true) {
                  _c = stream_1_1.value;
                  _f = false;
                  const chunk = _c;
                  yield writer.write(chunk);
                }
              } catch (e_2_1) {
                e_2 = {
                  error: e_2_1
                };
              } finally {
                try {
                  if (!_f && !_a && (_b = stream_1.return)) yield _b.call(stream_1);
                } finally {
                  if (e_2) throw e_2.error;
                }
              }
              yield writer.close();
              resolve();
            } catch (e) {
              reject(e);
            }
          }));
          bufferStreamPromise.then(() => {
            this.log.debug('preconnect buffer sent successfully', getLogContextFromTrack(track));
          }).catch(e => {
            this.log.error('error sending preconnect buffer', Object.assign(Object.assign({}, getLogContextFromTrack(track)), {
              error: e
            }));
          });
        }
      }
      return publication;
    });
  }
  canPublishFrameMetadata() {
    var _a;
    return !!(this.roomOptions.e2ee || this.roomOptions.encryption || isFrameMetadataSupported((_a = this.roomOptions.frameMetadata) !== null && _a !== void 0 ? _a : this.roomOptions.packetTrailer));
  }
  normalizeRequestedFrameMetadataOptions(track, opts) {
    var _a;
    const fmOpts = (_a = opts.frameMetadata) !== null && _a !== void 0 ? _a : opts.packetTrailer;
    if (track.kind !== Track.Kind.Video || !hasFrameMetadataPublishOptions(fmOpts)) {
      opts.frameMetadata = undefined;
      opts.packetTrailer = undefined;
      return [];
    }
    if (!this.canPublishFrameMetadata()) {
      this.log.warn('frame metadata transform not supported; not advertising features', Object.assign(Object.assign({}, this.logContext), getLogContextFromTrack(track)));
      opts.frameMetadata = undefined;
      opts.packetTrailer = undefined;
      return [];
    }
    const features = getFrameMetadataFeatures(fmOpts);
    const normalized = getFrameMetadataPublishOptions(features);
    opts.frameMetadata = normalized;
    opts.packetTrailer = normalized;
    return features;
  }
  get isLocal() {
    return true;
  }
  /** @internal
   * publish additional codec to existing track
   */
  publishAdditionalCodecForTrack(track, videoCodec, options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      // TODO remove once e2ee is supported for backup tracks
      if (this.encryptionType !== Encryption_Type.NONE) {
        return;
      }
      // is it not published? if so skip
      let existingPublication;
      this.trackPublications.forEach(publication => {
        if (!publication.track) {
          return;
        }
        if (publication.track === track) {
          existingPublication = publication;
        }
      });
      if (!existingPublication) {
        throw new TrackInvalidError('track is not published');
      }
      if (!isLocalVideoTrack(track)) {
        throw new TrackInvalidError('track is not a video track');
      }
      const opts = Object.assign(Object.assign({}, (_a = this.roomOptions) === null || _a === void 0 ? void 0 : _a.publishDefaults), options);
      const encodings = computeTrackBackupEncodings(track, videoCodec, opts);
      if (!encodings) {
        this.log.info("backup codec has been disabled, ignoring request to add additional codec for track", getLogContextFromTrack(track));
        return;
      }
      const simulcastTrack = track.addSimulcastTrack(videoCodec, encodings);
      if (!simulcastTrack) {
        return;
      }
      const packetTrailerFeatures = this.normalizeRequestedFrameMetadataOptions(track, opts);
      // the backup sender encodes the processed track, not the raw capture track.dimensions reports
      const _track$mediaStreamTra = track.mediaStreamTrack.getSettings(),
        width = _track$mediaStreamTra.width,
        height = _track$mediaStreamTra.height;
      const dims = width && height ? {
        width,
        height
      } : undefined;
      if (!dims) {
        this.log.warn('could not determine track dimensions, publishing backup codec without layers', getLogContextFromTrack(track));
      }
      // the server reads the layers off the codec entry for a request that carries a sid, and
      // falls back to the top level ones when it predates per codec layers
      const layers = dims ? videoLayersFromEncodings(dims.width, dims.height, encodings) : [];
      const req = new AddTrackRequest({
        cid: simulcastTrack.mediaStreamTrack.id,
        type: Track.kindToProto(track.kind),
        muted: track.isMuted,
        source: Track.sourceToProto(track.source),
        sid: track.sid,
        width: dims === null || dims === void 0 ? void 0 : dims.width,
        height: dims === null || dims === void 0 ? void 0 : dims.height,
        packetTrailerFeatures,
        layers,
        simulcastCodecs: [{
          codec: opts.videoCodec,
          cid: simulcastTrack.mediaStreamTrack.id,
          layers
        }]
      });
      if (!this.engine || this.engine.isClosed) {
        throw new UnexpectedConnectionState('cannot publish track when not connected');
      }
      const negotiate = () => __awaiter(this, void 0, void 0, function* () {
        yield this.engine.createSimulcastSender(track, simulcastTrack, opts, encodings);
        yield this.engine.negotiate();
      });
      const rets = yield Promise.all([this.engine.addTrack(req), negotiate()]);
      const ti = rets[0];
      this.log.debug("published ".concat(videoCodec, " for track ").concat(track.sid), {
        encodings,
        trackInfo: ti
      });
    });
  }
  unpublishTrack(track, stopOnUnpublish) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      if (isLocalTrack(track)) {
        const publishPromise = this.pendingPublishPromises.get(track);
        if (publishPromise) {
          this.log.debug('awaiting publish promise before attempting to unpublish', getLogContextFromTrack(track));
          yield publishPromise;
        }
      }
      // look through all published tracks to find the right ones
      const publication = this.getPublicationForTrack(track);
      const pubLogContext = publication ? getLogContextFromTrack(publication) : undefined;
      this.log.info('unpublishing track', pubLogContext);
      if (!publication || !publication.track) {
        this.log.warn('track was not unpublished because no publication was found', pubLogContext);
        return undefined;
      }
      track = publication.track;
      track.off(TrackEvent.Muted, this.onTrackMuted);
      track.off(TrackEvent.Unmuted, this.onTrackUnmuted);
      track.off(TrackEvent.Ended, this.handleTrackEnded);
      track.off(TrackEvent.UpstreamPaused, this.onTrackUpstreamPaused);
      track.off(TrackEvent.UpstreamResumed, this.onTrackUpstreamResumed);
      track.off(TrackEvent.AudioTrackFeatureUpdate, this.onTrackFeatureUpdate);
      if (stopOnUnpublish === undefined) {
        stopOnUnpublish = (_b = (_a = this.roomOptions) === null || _a === void 0 ? void 0 : _a.stopLocalTrackOnUnpublish) !== null && _b !== void 0 ? _b : true;
      }
      if (stopOnUnpublish) {
        track.stop();
      } else {
        track.stopMonitor();
      }
      let negotiationNeeded = false;
      const trackSender = track.sender;
      track.sender = undefined;
      if (this.engine.pcManager && this.engine.pcManager.currentState < PCTransportState.FAILED && trackSender) {
        try {
          for (const transceiver of this.engine.pcManager.publisher.getTransceivers()) {
            // if sender is not currently sending (after replaceTrack(null))
            // removeTrack would have no effect.
            // to ensure we end up successfully removing the track, manually set
            // the transceiver to inactive
            if (transceiver.sender === trackSender) {
              transceiver.direction = 'inactive';
              negotiationNeeded = true;
            }
          }
          try {
            negotiationNeeded = this.engine.removeTrack(trackSender);
          } catch (e) {
            this.log.warn(e);
            negotiationNeeded = true;
          }
          if (isLocalVideoTrack(track)) {
            for (const _ref4 of track.simulcastCodecs) {
              var _ref5 = _slicedToArray(_ref4, 2);
              const trackInfo = _ref5[1];
              if (trackInfo.sender) {
                try {
                  negotiationNeeded = this.engine.removeTrack(trackInfo.sender);
                } catch (e) {
                  this.log.warn(e);
                  negotiationNeeded = true;
                }
                trackInfo.sender = undefined;
              }
            }
            track.simulcastCodecs.clear();
          }
        } catch (e) {
          this.log.warn('failed to unpublish track', Object.assign(Object.assign({}, pubLogContext), {
            error: e
          }));
        }
      }
      // remove from our maps
      this.trackPublications.delete(publication.trackSid);
      switch (publication.kind) {
        case Track.Kind.Audio:
          this.audioTrackPublications.delete(publication.trackSid);
          break;
        case Track.Kind.Video:
          this.videoTrackPublications.delete(publication.trackSid);
          break;
      }
      this.emit(ParticipantEvent.LocalTrackUnpublished, publication);
      publication.setTrack(undefined);
      if (negotiationNeeded) {
        yield this.engine.negotiate();
      }
      return publication;
    });
  }
  unpublishTracks(tracks) {
    return __awaiter(this, void 0, void 0, function* () {
      const results = yield Promise.all(tracks.map(track => this.unpublishTrack(track)));
      return results.filter(track => !!track);
    });
  }
  republishAllTracks(options_1) {
    return __awaiter(this, arguments, void 0, function (options) {
      var _this3 = this;
      let restartTracks = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
      return function* () {
        if (_this3.republishPromise) {
          yield _this3.republishPromise;
        }
        _this3.republishPromise = new TypedPromise((resolve, reject) => __awaiter(_this3, void 0, void 0, function* () {
          try {
            const localPubs = [];
            this.trackPublications.forEach(pub => {
              if (pub.track) {
                if (options) {
                  pub.options = Object.assign(Object.assign({}, pub.options), options);
                }
                localPubs.push(pub);
              }
            });
            yield Promise.all(localPubs.map(pub => __awaiter(this, void 0, void 0, function* () {
              const track = pub.track;
              yield this.unpublishTrack(track, false);
              if (restartTracks && !track.isMuted && track.source !== Track.Source.ScreenShare && track.source !== Track.Source.ScreenShareAudio && (isLocalAudioTrack(track) || isLocalVideoTrack(track)) && !track.isUserProvided) {
                // generally we need to restart the track before publishing, often a full reconnect
                // is necessary because computer had gone to sleep.
                this.log.debug('restarting existing track', {
                  track: pub.trackSid
                });
                yield track.restartTrack();
              }
              yield this.publishOrRepublishTrack(track, pub.options, true);
            })));
            resolve();
          } catch (error) {
            if (error instanceof Error) {
              reject(error);
            } else {
              reject(new Error(String(error)));
            }
          } finally {
            this.republishPromise = undefined;
          }
        }));
        yield _this3.republishPromise;
      }();
    });
  }
  /**
   * Publish a new data payload to the room. Data will be forwarded to each
   * participant in the room if the destination field in publishOptions is empty
   *
   * @param data Uint8Array of the payload. To send string data, use TextEncoder.encode
   * @param options optionally specify a `reliable`, `topic` and `destination`
   */
  publishData(data_1) {
    return __awaiter(this, arguments, void 0, function (data) {
      var _this4 = this;
      let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
      return function* () {
        const kind = options.reliable ? DataChannelKind.RELIABLE : DataChannelKind.LOSSY;
        const dataPacketKind = options.reliable ? DataPacket_Kind.RELIABLE : DataPacket_Kind.LOSSY;
        const destinationIdentities = options.destinationIdentities;
        const topic = options.topic;
        let userPacket = new UserPacket({
          participantIdentity: _this4.identity,
          payload: data,
          destinationIdentities,
          topic
        });
        const packet = new DataPacket({
          kind: dataPacketKind,
          value: {
            case: 'user',
            value: userPacket
          }
        });
        yield _this4.engine.sendDataPacket(packet, kind);
      }();
    });
  }
  /**
   * Publish SIP DTMF message to the room.
   *
   * @param code DTMF code
   * @param digit DTMF digit
   */
  publishDtmf(code, digit) {
    return __awaiter(this, void 0, void 0, function* () {
      const packet = new DataPacket({
        kind: DataPacket_Kind.RELIABLE,
        value: {
          case: 'sipDtmf',
          value: new SipDTMF({
            code: code,
            digit: digit
          })
        }
      });
      yield this.engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
    });
  }
  /** @deprecated Consider migrating to {@link sendText} */
  sendChatMessage(text, options) {
    return __awaiter(this, void 0, void 0, function* () {
      const msg = {
        id: crypto.randomUUID(),
        message: text,
        timestamp: Date.now(),
        attachedFiles: options === null || options === void 0 ? void 0 : options.attachments
      };
      const packet = new DataPacket({
        value: {
          case: 'chatMessage',
          value: new ChatMessage(Object.assign(Object.assign({}, msg), {
            timestamp: protoInt64.parse(msg.timestamp)
          }))
        }
      });
      yield this.engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
      this.emit(ParticipantEvent.ChatMessage, msg);
      return msg;
    });
  }
  /** @deprecated Consider migrating to {@link sendText} */
  editChatMessage(editText, originalMessage) {
    return __awaiter(this, void 0, void 0, function* () {
      const msg = Object.assign(Object.assign({}, originalMessage), {
        message: editText,
        editTimestamp: Date.now()
      });
      const packet = new DataPacket({
        value: {
          case: 'chatMessage',
          value: new ChatMessage(Object.assign(Object.assign({}, msg), {
            timestamp: protoInt64.parse(msg.timestamp),
            editTimestamp: protoInt64.parse(msg.editTimestamp)
          }))
        }
      });
      yield this.engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
      this.emit(ParticipantEvent.ChatMessage, msg);
      return msg;
    });
  }
  /**
   * Sends the given string to participants in the room via the data channel.
   * For longer messages, consider using {@link streamText} instead.
   *
   * @param text The text payload
   * @param options.topic Topic identifier used to route the stream to appropriate handlers.
   */
  sendText(text, options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.roomOutgoingDataStreamManager.sendText(text, options);
    });
  }
  /**
   * Creates a new TextStreamWriter which can be used to stream text incrementally
   * to participants in the room via the data channel.
   *
   * @param options.topic Topic identifier used to route the stream to appropriate handlers.
   *
   * @internal
   * @experimental CAUTION, might get removed in a minor release
   */
  streamText(options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.roomOutgoingDataStreamManager.streamText(options);
    });
  }
  /** Send a File to all participants in the room via the data channel.
   * @param file The File object payload
   * @param options.topic Topic identifier used to route the stream to appropriate handlers.
   * @param options.onProgress A callback function used to monitor the upload progress percentage.
   */
  sendFile(file, options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.roomOutgoingDataStreamManager.sendFile(file, options);
    });
  }
  /**
   * Sends the given bytes to participants in the room via the data channel.
   * For files, consider using {@link sendFile}; for longer/incremental payloads, {@link streamBytes}.
   *
   * @param bytes The byte payload
   * @param options.topic Topic identifier used to route the stream to appropriate handlers.
   */
  sendBytes(bytes, options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.roomOutgoingDataStreamManager.sendBytes(bytes, options);
    });
  }
  /**
   * Stream bytes incrementally to participants in the room via the data channel.
   * For sending files, consider using {@link sendFile} instead.
   *
   * @param options.topic Topic identifier used to route the stream to appropriate handlers.
   */
  streamBytes(options) {
    return __awaiter(this, void 0, void 0, function* () {
      return this.roomOutgoingDataStreamManager.streamBytes(options);
    });
  }
  /**
   * Initiate an RPC call to a remote participant
   * @param params - Parameters for initiating the RPC call, see {@link PerformRpcParams}
   * @returns A promise that resolves with the response payload or rejects with an error.
   * @throws Error on failure. Details in `message`.
   */
  performRpc(params) {
    return this.rpcClientManager.performRpc(params).then(_ref6 => {
      let _ref7 = _slicedToArray(_ref6, 2);
        _ref7[0];
        let completionPromise = _ref7[1];
      return completionPromise;
    });
  }
  /**
   * @deprecated use `room.registerRpcMethod` instead
   */
  registerRpcMethod(method, handler) {
    this.rpcServerManager.registerRpcMethod(method, handler);
  }
  /**
   * @deprecated use `room.unregisterRpcMethod` instead
   */
  unregisterRpcMethod(method) {
    this.rpcServerManager.unregisterRpcMethod(method);
  }
  /**
   * Control who can subscribe to LocalParticipant's published tracks.
   *
   * By default, all participants can subscribe. This allows fine-grained control over
   * who is able to subscribe at a participant and track level.
   *
   * Note: if access is given at a track-level (i.e. both [allParticipantsAllowed] and
   * [ParticipantTrackPermission.allTracksAllowed] are false), any newer published tracks
   * will not grant permissions to any participants and will require a subsequent
   * permissions update to allow subscription.
   *
   * @param allParticipantsAllowed Allows all participants to subscribe all tracks.
   *  Takes precedence over [[participantTrackPermissions]] if set to true.
   *  By default this is set to true.
   * @param participantTrackPermissions Full list of individual permissions per
   *  participant/track. Any omitted participants will not receive any permissions.
   */
  setTrackSubscriptionPermissions(allParticipantsAllowed) {
    let participantTrackPermissions = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : [];
    this.participantTrackPermissions = participantTrackPermissions;
    this.allParticipantsAllowedToSubscribe = allParticipantsAllowed;
    if (!this.engine.client.isDisconnected) {
      this.updateTrackSubscriptionPermissions();
    }
  }
  /** @internal */
  setEnabledPublishCodecs(codecs) {
    this.enabledPublishVideoCodecs = codecs.filter(c => c.mime.split('/')[0].toLowerCase() === 'video');
  }
  /** @internal */
  updateInfo(info) {
    if (!super.updateInfo(info)) {
      return false;
    }
    // reconcile track mute status.
    // if server's track mute status doesn't match actual, we'll have to update
    // the server's copy
    info.tracks.forEach(ti => {
      var _a, _b;
      const pub = this.trackPublications.get(ti.sid);
      if (pub) {
        const mutedOnServer = pub.isMuted || ((_b = (_a = pub.track) === null || _a === void 0 ? void 0 : _a.isUpstreamPaused) !== null && _b !== void 0 ? _b : false);
        if (mutedOnServer !== ti.muted) {
          this.log.debug('updating server mute state after reconcile', Object.assign(Object.assign({}, getLogContextFromTrack(pub)), {
            mutedOnServer
          }));
          this.engine.client.sendMuteTrack(ti.sid, mutedOnServer);
        }
      }
    });
    return true;
  }
  /** @internal */
  setActiveAgent(agent) {
    var _a, _b, _c, _d;
    this.firstActiveAgent = agent;
    if (agent && !this.firstActiveAgent) {
      this.firstActiveAgent = agent;
    }
    if (agent) {
      (_b = (_a = this.activeAgentFuture) === null || _a === void 0 ? void 0 : _a.resolve) === null || _b === void 0 ? void 0 : _b.call(_a, agent);
    } else {
      (_d = (_c = this.activeAgentFuture) === null || _c === void 0 ? void 0 : _c.reject) === null || _d === void 0 ? void 0 : _d.call(_c, new Error('Agent disconnected'));
    }
    this.activeAgentFuture = undefined;
  }
  waitUntilActiveAgentPresent() {
    if (this.firstActiveAgent) {
      return Promise.resolve(this.firstActiveAgent);
    }
    if (!this.activeAgentFuture) {
      this.activeAgentFuture = new Future();
    }
    return this.activeAgentFuture.promise;
  }
  /**
   * @internal
   */
  restartOnDefaultAudioDevice(track) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      try {
        if (isChromiumBased()) {
          // only chrome has the notion of a literal `default` device
          yield track.setDeviceId({
            exact: 'default'
          });
        } else {
          // just try to restart for other browsers and try to change the device id as a fallback in the catch clause
          yield track.restartTrack();
        }
      } catch (e) {
        // restart() clears the manually-stopped flag on entry, so retrying a track the user stopped
        // while the first attempt was in flight would re-open the capture device behind their back
        if (!isDeviceAcquisitionFailure(e) || isPermissionDeniedError(e) || track.isStopped) {
          throw e;
        }
        this.log.debug('could not reacquire the audio device, retrying with an enumerated device', getLogContextFromTrack(track));
        // permission is already granted at this point, so skip the probe that would re-acquire media
        const allDevices = yield DeviceManager.getInstance().getDevices('audioinput', false);
        yield track.setDeviceId((_b = (_a = allDevices === null || allDevices === void 0 ? void 0 : allDevices[0]) === null || _a === void 0 ? void 0 : _a.deviceId) !== null && _b !== void 0 ? _b : 'default');
      }
    });
  }
  getPublicationForTrack(track) {
    let publication;
    this.trackPublications.forEach(pub => {
      const localTrack = pub.track;
      if (!localTrack) {
        return;
      }
      // this looks overly complicated due to this object tree
      if (track instanceof MediaStreamTrack) {
        if (isLocalAudioTrack(localTrack) || isLocalVideoTrack(localTrack)) {
          if (localTrack.mediaStreamTrack === track) {
            publication = pub;
          }
        }
      } else if (track === localTrack) {
        publication = pub;
      }
    });
    return publication;
  }
  waitForPendingPublicationOfSource(source) {
    return __awaiter(this, void 0, void 0, function* () {
      const waitForPendingTimeout = 10000;
      const startTime = Date.now();
      while (Date.now() < startTime + waitForPendingTimeout) {
        const publishPromiseEntry = Array.from(this.pendingPublishPromises.entries()).find(_ref8 => {
          let _ref9 = _slicedToArray(_ref8, 1),
            pendingTrack = _ref9[0];
          return pendingTrack.source === source;
        });
        if (publishPromiseEntry) {
          return publishPromiseEntry[1];
        }
        yield sleep(20);
      }
    });
  }
  /** Publishes a data track.
   *
   * Returns the published data track if successful. Use {@link LocalDataTrack#tryPush}
   * to send data frames on the track.
   */
  publishDataTrack(options) {
    return __awaiter(this, void 0, void 0, function* () {
      const track = new LocalDataTrack(options, this.roomOutgoingDataTrackManager);
      yield track.publish();
      return track;
    });
  }
}
export { LocalParticipant };
