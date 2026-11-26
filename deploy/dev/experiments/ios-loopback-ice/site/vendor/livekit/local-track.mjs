// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 22968-24345 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { __awaiter, __rest, _slicedToArray } from './tslib.mjs';
import { AudioTrackFeature } from './protobuf-messages-enums.mjs';
import { livekitLogger } from './loglevel.mjs';
import { getBrowser } from './browser.mjs';
import { DeviceUnsupportedError, TrackInvalidError } from './errors.mjs';
import { TrackEvent } from './event-enums.mjs';
import { ScreenSharePresets, VideoPreset, VideoPresets, VideoPresets43, constraintsForOptions, detectSilence, waitForFirstVideoFrame } from './util.mjs';
import { Track, attachToElement, detachTrack, isAppleMobile, isMobile, isReactNative, isSVCCodec, isSVCSimulcast, isSafariSvcApi, isWeb, sleep, usesLegacySVCEncodings } from './track.mjs';
import { compareVersions, getReactNativeOs, markDeviceAcquisitionFailure, unwrapConstraint } from './utils-misc.mjs';
import { computeBitrate, debounce, monitorFrequency } from './e2ee-frame.mjs';
import { DeviceManager } from './device.mjs';
import { DEFAULT_DIMENSIONS_TIMEOUT, LocalTrackRecorder, PRE_CONNECT_BUFFER_TIMEOUT, isRecordingSupported } from './recorder.mjs';


class LocalTrack extends Track {
  /** @internal */
  get sender() {
    return this._sender;
  }
  /** @internal */
  set sender(sender) {
    this._sender = sender;
  }
  get constraints() {
    return this._constraints;
  }
  get hasPreConnectBuffer() {
    return !!this.localTrackRecorder;
  }
  /**
   *
   * @param mediaTrack
   * @param kind
   * @param constraints MediaTrackConstraints that are being used when restarting or reacquiring tracks
   * @param userProvidedTrack Signals to the SDK whether or not the mediaTrack should be managed (i.e. released and reacquired) internally by the SDK
   */
  constructor(mediaTrack, kind, constraints) {
    let userProvidedTrack = arguments.length > 3 && arguments[3] !== undefined ? arguments[3] : false;
    let loggerOptions = arguments.length > 4 ? arguments[4] : undefined;
    super(mediaTrack, kind, loggerOptions);
    this.manuallyStopped = false;
    this.pendingDeviceChange = false;
    this._isUpstreamPaused = false;
    this.handleTrackMuteEvent = () => this.debouncedTrackMuteHandler().catch(() => this.log.debug('track mute bounce got cancelled by an unmute event', this.logContext));
    this.debouncedTrackMuteHandler = debounce(() => __awaiter(this, void 0, void 0, function* () {
      yield this.pauseUpstream();
    }), 5000);
    this.handleTrackUnmuteEvent = () => __awaiter(this, void 0, void 0, function* () {
      this.debouncedTrackMuteHandler.cancel('unmute');
      yield this.resumeUpstream();
    });
    this.handleEnded = () => {
      if (this.isInBackground) {
        this.reacquireTrack = true;
      }
      this._mediaStreamTrack.removeEventListener('mute', this.handleTrackMuteEvent);
      this._mediaStreamTrack.removeEventListener('unmute', this.handleTrackUnmuteEvent);
      this.emit(TrackEvent.Ended, this);
    };
    this.reacquireTrack = false;
    this.providedByUser = userProvidedTrack;
    this.muteLock = new _();
    this.pauseUpstreamLock = new _();
    this.trackChangeLock = new _();
    this.trackChangeLock.lock().then(unlock => __awaiter(this, void 0, void 0, function* () {
      try {
        yield this.setMediaStreamTrack(mediaTrack, true);
      } finally {
        unlock();
      }
    }));
    // added to satisfy TS compiler, constraints are synced with MediaStreamTrack
    this._constraints = mediaTrack.getConstraints();
    if (constraints) {
      this._constraints = constraints;
    }
  }
  get id() {
    return this._mediaStreamTrack.id;
  }
  get dimensions() {
    if (this.kind !== Track.Kind.Video) {
      return undefined;
    }
    const _this$_mediaStreamTra = this._mediaStreamTrack.getSettings(),
      width = _this$_mediaStreamTra.width,
      height = _this$_mediaStreamTra.height;
    if (width && height) {
      return {
        width,
        height
      };
    }
    return undefined;
  }
  get isUpstreamPaused() {
    return this._isUpstreamPaused;
  }
  get isUserProvided() {
    return this.providedByUser;
  }
  get isStopped() {
    return this.manuallyStopped;
  }
  get mediaStreamTrack() {
    var _a, _b;
    return (_b = (_a = this.processor) === null || _a === void 0 ? void 0 : _a.processedTrack) !== null && _b !== void 0 ? _b : this._mediaStreamTrack;
  }
  get isLocal() {
    return true;
  }
  /**
   * @internal
   * returns mediaStreamTrack settings of the capturing mediastreamtrack source - ignoring processors
   */
  getSourceTrackSettings() {
    return this._mediaStreamTrack.getSettings();
  }
  setMediaStreamTrack(newTrack, force, isUnmuting) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (newTrack === this._mediaStreamTrack && !force) {
        return;
      }
      if (this._mediaStreamTrack) {
        // detach
        this.attachedElements.forEach(el => {
          detachTrack(this._mediaStreamTrack, el);
        });
        this.debouncedTrackMuteHandler.cancel('new-track');
        this._mediaStreamTrack.removeEventListener('ended', this.handleEnded);
        this._mediaStreamTrack.removeEventListener('mute', this.handleTrackMuteEvent);
        this._mediaStreamTrack.removeEventListener('unmute', this.handleTrackUnmuteEvent);
      }
      this.mediaStream = new MediaStream([newTrack]);
      if (newTrack) {
        newTrack.addEventListener('ended', this.handleEnded);
        // when underlying track emits mute, it indicates that the device is unable
        // to produce media. In this case we'll need to signal with remote that
        // the track is "muted"
        // note this is different from LocalTrack.mute because we do not want to
        // touch MediaStreamTrack.enabled
        newTrack.addEventListener('mute', this.handleTrackMuteEvent);
        newTrack.addEventListener('unmute', this.handleTrackUnmuteEvent);
        this._constraints = newTrack.getConstraints();
      }
      let processedTrack;
      if (this.processor && newTrack) {
        this.log.debug('restarting processor', this.logContext);
        if (this.kind === 'unknown') {
          throw TypeError('cannot set processor on track of unknown kind');
        }
        if (this.processorElement) {
          attachToElement(newTrack, this.processorElement);
          // ensure the processorElement itself stays muted
          this.processorElement.muted = true;
        }
        yield this.processor.restart({
          track: newTrack,
          kind: this.kind,
          element: this.processorElement,
          // audio processors need the context they were initialised with in order to rebuild
          // their graph, omitting it here would leave them without one
          audioContext: this.audioContext,
          localTrack: this
        });
        processedTrack = this.processor.processedTrack;
      }
      if (this.sender && ((_a = this.sender.transport) === null || _a === void 0 ? void 0 : _a.state) !== 'closed') {
        yield this.sender.replaceTrack(processedTrack !== null && processedTrack !== void 0 ? processedTrack : newTrack);
      }
      // if `newTrack` is different from the existing track, stop the
      // older track just before replacing it
      if (!this.providedByUser && this._mediaStreamTrack !== newTrack) {
        this._mediaStreamTrack.stop();
      }
      this._mediaStreamTrack = newTrack;
      if (newTrack) {
        // sync muted state with the enabled state of the newly provided track
        // if restarting as part of an unmute, set enabled to true directly to avoid mute cycling
        this._mediaStreamTrack.enabled = isUnmuting ? true : !this.isMuted;
        // when a valid track is replace, we'd want to start producing
        yield this.resumeUpstream();
        this.attachedElements.forEach(el => {
          attachToElement(processedTrack !== null && processedTrack !== void 0 ? processedTrack : newTrack, el);
        });
      }
      yield this.onMediaStreamTrackChanged();
    });
  }
  /**
   * Hook invoked after the underlying MediaStreamTrack has been swapped and the processor has
   * been restarted around it, so `processor.processedTrack` already holds the new track.
   * Unlike `onSenderTrackSwapped` this also fires on the restart and unmute paths.
   */
  onMediaStreamTrackChanged() {
    return __awaiter(this, void 0, void 0, function* () {
      // base implementation is a no-op; LocalAudioTrack overrides this to move the krisp
      // feature listeners onto the new processed track.
    });
  }
  waitForDimensions() {
    return __awaiter(this, arguments, void 0, function () {
      var _this = this;
      let timeout = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : DEFAULT_DIMENSIONS_TIMEOUT;
      return function* () {
        if (_this.kind === Track.Kind.Audio) {
          throw new Error('cannot get dimensions for audio tracks');
        }
        const started = Date.now();
        if (isAppleMobile()) {
          // iOS and iPadOS keep reporting the camera's sensor frame from getSettings() until the
          // camera has delivered its first picture, so a portrait capture initially reads as
          // landscape. Wait for that first frame instead of guessing at a delay.
          // Gated to the Apple capture pipeline rather than run everywhere: elsewhere getSettings() is
          // already right and the first frame can be hundreds of milliseconds out (measured ~500ms on
          // macOS Safari), which would be pure added publish latency.
          // https://github.com/livekit/client-sdk-js/issues/2099
          // https://github.com/livekit/client-sdk-js/issues/2107
          yield waitForFirstVideoFrame(_this._mediaStreamTrack, timeout, _this.attachedElements);
        }
        while (true) {
          const dims = _this.dimensions;
          if (dims) {
            return dims;
          }
          if (Date.now() - started >= timeout) {
            break;
          }
          yield sleep(50);
        }
        throw new TrackInvalidError('unable to get track dimensions after timeout');
      }();
    });
  }
  setDeviceId(deviceId) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this._constraints.deviceId === deviceId && this._mediaStreamTrack.getSettings().deviceId === unwrapConstraint(deviceId) && this._mediaStreamTrack.readyState === 'live') {
        return true;
      }
      this._constraints.deviceId = deviceId;
      // when track is muted, underlying media stream track is stopped and
      // will be restarted later
      if (this.isMuted) {
        this.pendingDeviceChange = true;
        return true;
      }
      yield this.restartTrack();
      return unwrapConstraint(deviceId) === this._mediaStreamTrack.getSettings().deviceId;
    });
  }
  /**
   * @returns DeviceID of the device that is currently being used for this track
   */
  getDeviceId() {
    return __awaiter(this, arguments, void 0, function () {
      var _this2 = this;
      let normalize = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
      return function* () {
        // screen share doesn't have a usable device id
        if (_this2.source === Track.Source.ScreenShare) {
          return;
        }
        const _this2$_mediaStreamTr = _this2._mediaStreamTrack.getSettings(),
          deviceId = _this2$_mediaStreamTr.deviceId,
          groupId = _this2$_mediaStreamTr.groupId;
        const kind = _this2.kind === Track.Kind.Audio ? 'audioinput' : 'videoinput';
        return normalize ? DeviceManager.getInstance().normalizeDeviceId(kind, deviceId, groupId) : deviceId;
      }();
    });
  }
  mute() {
    return __awaiter(this, void 0, void 0, function* () {
      this.setTrackMuted(true);
      return this;
    });
  }
  unmute() {
    return __awaiter(this, void 0, void 0, function* () {
      this.setTrackMuted(false);
      return this;
    });
  }
  replaceTrack(track, userProvidedOrOptions) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.trackChangeLock.lock();
      try {
        if (!this.sender) {
          throw new TrackInvalidError('unable to replace an unpublished track');
        }
        let userProvidedTrack;
        let stopProcessor;
        if (typeof userProvidedOrOptions === 'boolean') {
          userProvidedTrack = userProvidedOrOptions;
        } else if (userProvidedOrOptions !== undefined) {
          userProvidedTrack = userProvidedOrOptions.userProvidedTrack;
          stopProcessor = userProvidedOrOptions.stopProcessor;
        }
        this.providedByUser = userProvidedTrack !== null && userProvidedTrack !== void 0 ? userProvidedTrack : true;
        this.log.debug('replace MediaStreamTrack', this.logContext);
        yield this.setMediaStreamTrack(track);
        // this must be synced *after* setting mediaStreamTrack above, since it relies
        // on the previous state in order to cleanup
        if (stopProcessor && this.processor) {
          yield this.internalStopProcessor();
        }
      } finally {
        unlock();
      }
      yield this.onSenderTrackSwapped();
      return this;
    });
  }
  /**
   * Hook invoked after the MediaStreamTrack on the sender has been swapped
   * (via replaceTrack, setProcessor, or stopProcessor). Fires outside the
   * trackChangeLock so subclasses can do asynchronous work such as polling
   * for new dimensions without blocking other track operations.
   */
  onSenderTrackSwapped() {
    return __awaiter(this, void 0, void 0, function* () {
      // base implementation is a no-op; LocalVideoTrack overrides this to
      // recompute sender encoding parameters.
    });
  }
  restart(constraints, isUnmuting) {
    return __awaiter(this, void 0, void 0, function* () {
      this.manuallyStopped = false;
      const unlock = yield this.trackChangeLock.lock();
      try {
        if (!constraints) {
          constraints = this._constraints;
        }
        const _constraints = constraints,
          deviceId = _constraints.deviceId,
          facingMode = _constraints.facingMode,
          otherConstraints = __rest(constraints, ["deviceId", "facingMode"]);
        this.log.debug('restarting track with constraints', Object.assign(Object.assign({}, this.logContext), {
          constraints
        }));
        const streamConstraints = {
          audio: false,
          video: false
        };
        if (this.kind === Track.Kind.Video) {
          streamConstraints.video = deviceId || facingMode ? {
            deviceId,
            facingMode
          } : true;
        } else {
          streamConstraints.audio = deviceId ? Object.assign({
            deviceId
          }, otherConstraints) : true;
        }
        // these steps are duplicated from setMediaStreamTrack because we must stop
        // the previous tracks before new tracks can be acquired
        this.attachedElements.forEach(el => {
          detachTrack(this.mediaStreamTrack, el);
        });
        this._mediaStreamTrack.removeEventListener('ended', this.handleEnded);
        // on Safari, the old audio track must be stopped before attempting to acquire
        // the new track, otherwise the new track will stop with
        // 'A MediaStreamTrack ended due to a capture failure`
        this._mediaStreamTrack.stop();
        // create new track and attach
        let mediaStream;
        try {
          mediaStream = yield navigator.mediaDevices.getUserMedia(streamConstraints);
        } catch (e) {
          markDeviceAcquisitionFailure(e);
          throw e;
        }
        const newTrack = mediaStream.getTracks()[0];
        try {
          if (this.kind === Track.Kind.Video) {
            // we already captured the audio track with the constraints, so we only need to apply the video constraints
            yield newTrack.applyConstraints(otherConstraints);
          }
          newTrack.addEventListener('ended', this.handleEnded);
          this.log.debug('re-acquired MediaStreamTrack', this.logContext);
          yield this.setMediaStreamTrack(newTrack, false, isUnmuting);
        } catch (e) {
          // setMediaStreamTrack can throw after acquisition but before it adopts the new track,
          // which would leave the capture device live with nothing referencing it
          if (this._mediaStreamTrack !== newTrack) {
            newTrack.stop();
          }
          throw e;
        }
        this._constraints = constraints;
        this.pendingDeviceChange = false;
        this.emit(TrackEvent.Restarted, this);
        return this;
      } finally {
        // in the finally because a throw can also leave the new track adopted, and so still live
        if (this.manuallyStopped) {
          this.log.warn('track was stopped during a restart, stopping restarted track', this.logContext);
          this.stop();
        }
        unlock();
      }
    });
  }
  setTrackMuted(muted) {
    this.log.debug("setting ".concat(this.kind, " track ").concat(muted ? 'muted' : 'unmuted'), this.logContext);
    if (this.isMuted === muted && this._mediaStreamTrack.enabled !== muted) {
      return;
    }
    this.isMuted = muted;
    this._mediaStreamTrack.enabled = !muted;
    this.emit(muted ? TrackEvent.Muted : TrackEvent.Unmuted, this);
  }
  get needsReAcquisition() {
    return this._mediaStreamTrack.readyState !== 'live' || this._mediaStreamTrack.muted || !this._mediaStreamTrack.enabled || this.reacquireTrack;
  }
  handleAppVisibilityChanged() {
    const _super = Object.create(null, {
      handleAppVisibilityChanged: {
        get: () => super.handleAppVisibilityChanged
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      yield _super.handleAppVisibilityChanged.call(this);
      if (!isMobile()) return;
      this.log.debug("visibility changed, is in Background: ".concat(this.isInBackground), this.logContext);
      if (!this.isInBackground && this.needsReAcquisition && !this.isUserProvided && !this.isMuted) {
        this.log.debug("track needs to be reacquired, restarting ".concat(this.source), this.logContext);
        yield this.restart();
        this.reacquireTrack = false;
      }
    });
  }
  stop() {
    var _a;
    this.manuallyStopped = true;
    super.stop();
    this._mediaStreamTrack.removeEventListener('ended', this.handleEnded);
    this._mediaStreamTrack.removeEventListener('mute', this.handleTrackMuteEvent);
    this._mediaStreamTrack.removeEventListener('unmute', this.handleTrackUnmuteEvent);
    // `stop` is synchronous, so we can't await the teardown of the processor here.
    // make sure a failing teardown doesn't surface as an unhandled rejection.
    (_a = this.processor) === null || _a === void 0 ? void 0 : _a.destroy().catch(error => {
      this.log.error('failed to destroy processor', Object.assign(Object.assign({}, this.logContext), {
        error
      }));
    });
    this.processor = undefined;
  }
  /**
   * pauses publishing to the server without disabling the local MediaStreamTrack
   * this is used to display a user's own video locally while pausing publishing to
   * the server.
   * this API is unsupported on Safari < 12 due to a bug
   **/
  pauseUpstream() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.pauseUpstreamLock.lock();
      try {
        if (this._isUpstreamPaused === true) {
          return;
        }
        if (!this.sender) {
          this.log.warn('unable to pause upstream for an unpublished track', this.logContext);
          return;
        }
        this._isUpstreamPaused = true;
        this.emit(TrackEvent.UpstreamPaused, this);
        const browser = getBrowser();
        if ((browser === null || browser === void 0 ? void 0 : browser.name) === 'Safari' && compareVersions(browser.version, '12.0') < 0) {
          // https://bugs.webkit.org/show_bug.cgi?id=184911
          throw new DeviceUnsupportedError('pauseUpstream is not supported on Safari < 12.');
        }
        if (((_a = this.sender.transport) === null || _a === void 0 ? void 0 : _a.state) !== 'closed') {
          yield this.sender.replaceTrack(null);
        }
      } finally {
        unlock();
      }
    });
  }
  resumeUpstream() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.pauseUpstreamLock.lock();
      try {
        if (this._isUpstreamPaused === false) {
          return;
        }
        if (!this.sender) {
          this.log.warn('unable to resume upstream for an unpublished track', this.logContext);
          return;
        }
        this._isUpstreamPaused = false;
        this.emit(TrackEvent.UpstreamResumed, this);
        if (((_a = this.sender.transport) === null || _a === void 0 ? void 0 : _a.state) !== 'closed') {
          // this operation is noop if mediastreamtrack is already being sent
          yield this.sender.replaceTrack(this.mediaStreamTrack);
        }
      } finally {
        unlock();
      }
    });
  }
  /**
   * Gets the RTCStatsReport for the LocalTrack's underlying RTCRtpSender
   * See https://developer.mozilla.org/en-US/docs/Web/API/RTCStatsReport
   *
   * @returns Promise<RTCStatsReport> | undefined
   */
  getRTCStatsReport() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!((_a = this.sender) === null || _a === void 0 ? void 0 : _a.getStats)) {
        return;
      }
      const statsReport = yield this.sender.getStats();
      return statsReport;
    });
  }
  /**
   * Sets a processor on this track.
   * See https://github.com/livekit/track-processors-js for example usage
   *
   * The processor lives on the track, so stopping the track destroys it. Unpublishing and
   * disconnecting both stop the track by default, which means a processor has to be set again on
   * the track published after a reconnect. See {@link TrackProcessor} for the full lifecycle,
   * including what `stopLocalTrackOnUnpublish: false` changes.
   *
   * @param processor
   * @param showProcessedStreamLocally
   * @returns
   */
  setProcessor(processor_1) {
    return __awaiter(this, arguments, void 0, function (processor) {
      var _this3 = this;
      let showProcessedStreamLocally = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
      return function* () {
        var _a;
        const unlock = yield _this3.trackChangeLock.lock();
        try {
          _this3.log.debug('setting up processor', _this3.logContext);
          const processorElement = document.createElement(_this3.kind);
          const processorOptions = {
            kind: _this3.kind,
            track: _this3._mediaStreamTrack,
            element: processorElement,
            audioContext: _this3.audioContext,
            localTrack: _this3
          };
          yield processor.init(processorOptions);
          _this3.log.debug('processor initialized', _this3.logContext);
          if (_this3.processor) {
            yield _this3.internalStopProcessor();
          }
          if (_this3.kind === 'unknown') {
            throw TypeError('cannot set processor on track of unknown kind');
          }
          attachToElement(_this3._mediaStreamTrack, processorElement);
          processorElement.muted = true;
          processorElement.play().catch(error => {
            if (error instanceof DOMException && error.name === 'AbortError') {
              // This happens on Safari when the processor is restarted, try again after a delay
              _this3.log.warn('failed to play processor element, retrying', Object.assign(Object.assign({}, _this3.logContext), {
                error
              }));
              setTimeout(() => {
                processorElement.play().catch(err => {
                  _this3.log.error('failed to play processor element', Object.assign(Object.assign({}, _this3.logContext), {
                    err
                  }));
                });
              }, 100);
            } else {
              _this3.log.error('failed to play processor element', Object.assign(Object.assign({}, _this3.logContext), {
                error
              }));
            }
          });
          _this3.processor = processor;
          _this3.processorElement = processorElement;
          if (_this3.processor.processedTrack) {
            for (const el of _this3.attachedElements) {
              if (el !== _this3.processorElement && showProcessedStreamLocally) {
                detachTrack(_this3._mediaStreamTrack, el);
                attachToElement(_this3.processor.processedTrack, el);
              }
            }
            yield (_a = _this3.sender) === null || _a === void 0 ? void 0 : _a.replaceTrack(_this3.processor.processedTrack);
          }
          _this3.emit(TrackEvent.TrackProcessorUpdate, _this3.processor);
        } finally {
          unlock();
        }
        yield _this3.onSenderTrackSwapped();
      }();
    });
  }
  /**
   * Returns the processor currently set on this track, or `undefined` if there is none. A track
   * that was stopped by an unpublish or a disconnect has none. See {@link TrackProcessor}.
   */
  getProcessor() {
    return this.processor;
  }
  /**
   * Stops the track processor
   * See https://github.com/livekit/track-processors-js for example usage
   *
   * Only needed to remove a processor from a track that stays published. Unpublishing or
   * disconnecting already destroys it. See {@link TrackProcessor}.
   */
  stopProcessor() {
    return __awaiter(this, arguments, void 0, function () {
      var _this4 = this;
      let keepElement = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
      return function* () {
        const unlock = yield _this4.trackChangeLock.lock();
        try {
          yield _this4.internalStopProcessor(keepElement);
        } finally {
          unlock();
        }
        yield _this4.onSenderTrackSwapped();
      }();
    });
  }
  /**
   * @internal
   * This method assumes the caller has acquired a trackChangeLock already.
   * The public facing method for stopping the processor is `stopProcessor` and it wraps this method in the trackChangeLock.
   */
  internalStopProcessor() {
    return __awaiter(this, arguments, void 0, function () {
      var _this5 = this;
      let keepElement = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
      return function* () {
        var _a, _b;
        if (!_this5.processor) return;
        _this5.log.debug('stopping processor', _this5.logContext);
        (_a = _this5.processor.processedTrack) === null || _a === void 0 ? void 0 : _a.stop();
        yield _this5.processor.destroy();
        _this5.processor = undefined;
        if (!keepElement) {
          (_b = _this5.processorElement) === null || _b === void 0 ? void 0 : _b.remove();
          _this5.processorElement = undefined;
        }
        // apply original track constraints in case the processor changed them
        yield _this5._mediaStreamTrack.applyConstraints(_this5._constraints);
        // force re-setting of the mediaStreamTrack on the sender
        yield _this5.setMediaStreamTrack(_this5._mediaStreamTrack, true);
        _this5.emit(TrackEvent.TrackProcessorUpdate);
      }();
    });
  }
  /** @internal */
  startPreConnectBuffer() {
    let timeslice = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 100;
    if (!isRecordingSupported()) {
      this.log.warn('MediaRecorder is not available, cannot start preconnect buffer', this.logContext);
      return;
    }
    if (!this.localTrackRecorder) {
      let mimeType = 'audio/webm;codecs=opus';
      if (!MediaRecorder.isTypeSupported(mimeType)) {
        // iOS currently only supports video/mp4 as a mime type - even for audio.
        mimeType = 'video/mp4';
      }
      this.localTrackRecorder = new LocalTrackRecorder(this, {
        mimeType
      });
    } else {
      this.log.warn('preconnect buffer already started');
      return;
    }
    this.localTrackRecorder.start(timeslice);
    this.autoStopPreConnectBuffer = setTimeout(() => {
      this.log.warn('preconnect buffer timed out, stopping recording automatically', this.logContext);
      this.stopPreConnectBuffer();
    }, PRE_CONNECT_BUFFER_TIMEOUT);
  }
  /** @internal */
  stopPreConnectBuffer() {
    clearTimeout(this.autoStopPreConnectBuffer);
    if (this.localTrackRecorder) {
      this.localTrackRecorder.stop();
      this.localTrackRecorder = undefined;
    }
  }
  /** @internal */
  getPreConnectBuffer() {
    var _a;
    return (_a = this.localTrackRecorder) === null || _a === void 0 ? void 0 : _a.byteStream;
  }
  getPreConnectBufferMimeType() {
    var _a;
    return (_a = this.localTrackRecorder) === null || _a === void 0 ? void 0 : _a.mimeType;
  }
}class LocalAudioTrack extends LocalTrack {
  /**
   * boolean indicating whether enhanced noise cancellation is currently being used on this track
   */
  get enhancedNoiseCancellation() {
    return this.isKrispNoiseFilterEnabled;
  }
  /**
   *
   * @param mediaTrack
   * @param constraints MediaTrackConstraints that are being used when restarting or reacquiring tracks
   * @param userProvidedTrack Signals to the SDK whether or not the mediaTrack should be managed (i.e. released and reacquired) internally by the SDK
   */
  constructor(mediaTrack, constraints) {
    let userProvidedTrack = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : true;
    let audioContext = arguments.length > 3 ? arguments[3] : undefined;
    let loggerOptions = arguments.length > 4 ? arguments[4] : undefined;
    super(mediaTrack, Track.Kind.Audio, constraints, userProvidedTrack, loggerOptions);
    /** @internal */
    this.stopOnMute = false;
    this.isKrispNoiseFilterEnabled = false;
    this.monitorSender = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.sender) {
        this._currentBitrate = 0;
        return;
      }
      let stats;
      try {
        stats = yield this.getSenderStats();
      } catch (e) {
        this.log.error('could not get audio sender stats', Object.assign(Object.assign({}, this.logContext), {
          error: e
        }));
        return;
      }
      if (stats && this.prevStats) {
        this._currentBitrate = computeBitrate(stats, this.prevStats);
      }
      this.prevStats = stats;
    });
    this.handleKrispNoiseFilterEnable = () => {
      this.isKrispNoiseFilterEnabled = true;
      this.log.debug("Krisp noise filter enabled", this.logContext);
      this.emit(TrackEvent.AudioTrackFeatureUpdate, this, AudioTrackFeature.TF_ENHANCED_NOISE_CANCELLATION, true);
    };
    this.handleKrispNoiseFilterDisable = () => {
      this.isKrispNoiseFilterEnabled = false;
      this.log.debug("Krisp noise filter disabled", this.logContext);
      this.emit(TrackEvent.AudioTrackFeatureUpdate, this, AudioTrackFeature.TF_ENHANCED_NOISE_CANCELLATION, false);
    };
    this.audioContext = audioContext;
    this.checkForSilence();
  }
  mute() {
    const _super = Object.create(null, {
      mute: {
        get: () => super.mute
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.muteLock.lock();
      try {
        if (this.isMuted) {
          this.log.debug('Track already muted', this.logContext);
          return this;
        }
        // disabled special handling as it will cause BT headsets to switch communication modes
        if (this.source === Track.Source.Microphone && this.stopOnMute && !this.isUserProvided) {
          this.log.debug('stopping mic track', this.logContext);
          // also stop the track, so that microphone indicator is turned off
          this._mediaStreamTrack.stop();
        }
        yield _super.mute.call(this);
        return this;
      } finally {
        unlock();
      }
    });
  }
  unmute() {
    const _super = Object.create(null, {
      unmute: {
        get: () => super.unmute
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.muteLock.lock();
      try {
        if (!this.isMuted) {
          this.log.debug('Track already unmuted', this.logContext);
          return this;
        }
        if (this.source === Track.Source.Microphone && (this.stopOnMute || this._mediaStreamTrack.readyState === 'ended' || this.pendingDeviceChange) && !this.isUserProvided) {
          this.log.debug('reacquiring mic track', this.logContext);
          yield this.restart(undefined, true);
        }
        yield _super.unmute.call(this);
        return this;
      } finally {
        unlock();
      }
    });
  }
  restartTrack(options) {
    return __awaiter(this, void 0, void 0, function* () {
      let constraints;
      if (options) {
        const streamConstraints = constraintsForOptions({
          audio: options
        });
        if (typeof streamConstraints.audio !== 'boolean') {
          constraints = streamConstraints.audio;
        }
      }
      yield this.restart(constraints);
    });
  }
  applyConstraints(constraints) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.trackChangeLock.lock();
      try {
        const res = yield this._mediaStreamTrack.applyConstraints(constraints);
        this._constraints = Object.assign(Object.assign({}, this._constraints), constraints);
        return res;
      } finally {
        unlock();
      }
    });
  }
  restart(constraints, isUnmuting) {
    const _super = Object.create(null, {
      restart: {
        get: () => super.restart
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      const track = yield _super.restart.call(this, constraints, isUnmuting);
      this.checkForSilence();
      return track;
    });
  }
  /* @internal */
  startMonitor() {
    if (!isWeb()) {
      return;
    }
    if (this.monitorInterval) {
      return;
    }
    this.monitorInterval = setInterval(() => {
      this.monitorSender();
    }, monitorFrequency);
  }
  /**
   * registers the krisp feature listeners on the given processed track, moving them over from
   * a previously observed one. Passing `undefined` just removes the existing listeners.
   */
  observeProcessedTrack(processedTrack) {
    if (this.observedProcessedTrack === processedTrack) {
      return;
    }
    if (this.observedProcessedTrack) {
      this.observedProcessedTrack.removeEventListener('enable-lk-krisp-noise-filter', this.handleKrispNoiseFilterEnable);
      this.observedProcessedTrack.removeEventListener('disable-lk-krisp-noise-filter', this.handleKrispNoiseFilterDisable);
      this.observedProcessedTrack = undefined;
      if (this.isKrispNoiseFilterEnabled) {
        // we're no longer observing the track that had the filter enabled, so the feature
        // can't be active anymore. Make sure the state doesn't get stuck on `true`.
        this.handleKrispNoiseFilterDisable();
      }
    }
    if (!processedTrack) {
      return;
    }
    processedTrack.addEventListener('enable-lk-krisp-noise-filter', this.handleKrispNoiseFilterEnable);
    processedTrack.addEventListener('disable-lk-krisp-noise-filter', this.handleKrispNoiseFilterDisable);
    this.observedProcessedTrack = processedTrack;
  }
  /**
   * Sets a processor on this track.
   *
   * The processor is destroyed when the track is stopped, and also when the audio context goes
   * away, so a disconnect ends it on both counts and it has to be set again after reconnecting.
   * See {@link TrackProcessor} for the full lifecycle.
   */
  setProcessor(processor) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.trackChangeLock.lock();
      try {
        if (!isReactNative() && !this.audioContext) {
          throw Error('Audio context needs to be set on LocalAudioTrack in order to enable processors');
        }
        if (this.processor) {
          yield this.internalStopProcessor();
        }
        const processorOptions = {
          kind: this.kind,
          track: this._mediaStreamTrack,
          // RN won't have or use AudioContext
          audioContext: this.audioContext,
          localTrack: this
        };
        this.log.debug("setting up audio processor ".concat(processor.name), this.logContext);
        yield processor.init(processorOptions);
        this.processor = processor;
        if (this.processor.processedTrack) {
          yield (_a = this.sender) === null || _a === void 0 ? void 0 : _a.replaceTrack(this.processor.processedTrack);
          this.observeProcessedTrack(this.processor.processedTrack);
        }
        this.emit(TrackEvent.TrackProcessorUpdate, this.processor);
      } finally {
        unlock();
      }
    });
  }
  onMediaStreamTrackChanged() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      // the processor rebuilds its graph around the new track, so the krisp listeners have to
      // follow it onto whatever it hands back
      this.observeProcessedTrack((_a = this.processor) === null || _a === void 0 ? void 0 : _a.processedTrack);
    });
  }
  internalStopProcessor() {
    const _super = Object.create(null, {
      internalStopProcessor: {
        get: () => super.internalStopProcessor
      }
    });
    return __awaiter(this, arguments, void 0, function () {
      var _this = this;
      let keepElement = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
      return function* () {
        _this.observeProcessedTrack(undefined);
        yield _super.internalStopProcessor.call(_this, keepElement);
      }();
    });
  }
  stop() {
    this.observeProcessedTrack(undefined);
    super.stop();
  }
  /**
   * @internal
   * @experimental
   *
   * A processor's nodes (`AudioWorkletNode`, ...) can't move between contexts, so the processor
   * is rebuilt whenever the context changes and torn down when there is none left to run in.
   */
  setAudioContext(audioContext) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (this.audioContext === audioContext) {
        return;
      }
      const unlock = yield this.trackChangeLock.lock();
      try {
        this.audioContext = audioContext;
        if (!this.processor || isReactNative()) {
          return;
        }
        if (!audioContext) {
          this.log.debug('stopping audio processor, audio context has been removed', this.logContext);
          yield this.internalStopProcessor();
          return;
        }
        this.log.debug('restarting audio processor on new audio context', this.logContext);
        yield this.processor.restart({
          kind: this.kind,
          track: this._mediaStreamTrack,
          audioContext,
          localTrack: this
        });
        if (this.processor.processedTrack) {
          yield (_a = this.sender) === null || _a === void 0 ? void 0 : _a.replaceTrack(this.processor.processedTrack);
          this.observeProcessedTrack(this.processor.processedTrack);
        }
        this.emit(TrackEvent.TrackProcessorUpdate, this.processor);
      } finally {
        unlock();
      }
    });
  }
  getSenderStats() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!((_a = this.sender) === null || _a === void 0 ? void 0 : _a.getStats)) {
        return undefined;
      }
      const stats = yield this.sender.getStats();
      let audioStats;
      stats.forEach(v => {
        if (v.type === 'outbound-rtp') {
          audioStats = {
            type: 'audio',
            streamId: v.id,
            packetsSent: v.packetsSent,
            bytesSent: v.bytesSent,
            timestamp: v.timestamp
          };
          // loss, jitter and RTT are only known from what the remote reports back,
          // the same way the video sender picks them up
          const remote = stats.get(v.remoteId);
          if (remote) {
            audioStats.packetsLost = remote.packetsLost;
            audioStats.jitter = remote.jitter;
            audioStats.roundTripTime = remote.roundTripTime;
          }
        }
      });
      return audioStats;
    });
  }
  checkForSilence() {
    return __awaiter(this, void 0, void 0, function* () {
      const trackIsSilent = yield detectSilence(this);
      if (trackIsSilent) {
        if (!this.isMuted) {
          this.log.debug('silence detected on local audio track', this.logContext);
        }
        this.emit(TrackEvent.AudioSilenceDetected);
      }
      return trackIsSilent;
    });
  }
}/* @internal */
const presets169 = Object.values(VideoPresets);
/* @internal */
const presets43 = Object.values(VideoPresets43);
/* @internal */
const presetsScreenShare = Object.values(ScreenSharePresets);
/* @internal */
const defaultSimulcastPresets169 = [VideoPresets.h180, VideoPresets.h360];
/* @internal */
const defaultSimulcastPresets43 = [VideoPresets43.h180, VideoPresets43.h360];
/* @internal */
const computeDefaultScreenShareSimulcastPresets = fromPreset => {
  const layers = [{
    scaleResolutionDownBy: 2,
    fps: fromPreset.encoding.maxFramerate
  }];
  return layers.map(t => {
    var _a, _b;
    return new VideoPreset(Math.floor(fromPreset.width / t.scaleResolutionDownBy), Math.floor(fromPreset.height / t.scaleResolutionDownBy), Math.max(150000, Math.floor(fromPreset.encoding.maxBitrate / (Math.pow(t.scaleResolutionDownBy, 2) * (((_a = fromPreset.encoding.maxFramerate) !== null && _a !== void 0 ? _a : 30) / ((_b = t.fps) !== null && _b !== void 0 ? _b : 30))))), t.fps, fromPreset.encoding.priority);
  });
};
const videoRids = ['q', 'h', 'f'];
/* @internal */
function computeVideoEncodings(isScreenShare, width, height, options) {
  var _a, _b;
  let videoEncoding = options === null || options === void 0 ? void 0 : options.videoEncoding;
  if (isScreenShare) {
    videoEncoding = options === null || options === void 0 ? void 0 : options.screenShareEncoding;
  }
  const useSimulcast = options === null || options === void 0 ? void 0 : options.simulcast;
  const scalabilityMode = options === null || options === void 0 ? void 0 : options.scalabilityMode;
  const videoCodec = options === null || options === void 0 ? void 0 : options.videoCodec;
  // VP9/AV1 published as rid based simulcast rather than SVC, see isSVCSimulcast
  const useSVCSimulcast = isSVCSimulcast(videoCodec, options);
  if (!videoEncoding && !useSimulcast && !scalabilityMode || !width || !height) {
    // when we aren't simulcasting or svc, will need to return a single encoding without
    // capping bandwidth. we always require a encoding for dynacast
    return [{}];
  }
  if (!videoEncoding) {
    // find the right encoding based on width/height
    videoEncoding = determineAppropriateEncoding(isScreenShare, width, height, videoCodec);
    livekitLogger.debug('using video encoding', videoEncoding);
  }
  const sourceFramerate = videoEncoding.maxFramerate;
  const original = new VideoPreset(width, height, videoEncoding.maxBitrate, videoEncoding.maxFramerate, videoEncoding.priority);
  if (scalabilityMode && isSVCCodec(videoCodec) && !useSVCSimulcast) {
    const sm = new ScalabilityMode(scalabilityMode);
    const encodings = [];
    if (sm.spatial > 3) {
      throw new Error("unsupported scalabilityMode: ".concat(scalabilityMode));
    }
    // Browsers that read multiple encodings on an SVC codec as SVC rather than as
    // simulcast, see usesLegacySVCEncodings
    const browser = getBrowser();
    if (usesLegacySVCEncodings()) {
      const bitratesRatio = sm.suffix == 'h' ? 2 : 3;
      // safari 18.4 uses a different svc API that requires scaleResolutionDownBy to be set.
      const requireScale = isSafariSvcApi(browser);
      for (let i = 0; i < sm.spatial; i += 1) {
        // in legacy SVC, scaleResolutionDownBy cannot be set
        encodings.push({
          rid: videoRids[2 - i],
          maxBitrate: videoEncoding.maxBitrate / Math.pow(bitratesRatio, i),
          maxFramerate: original.encoding.maxFramerate,
          scaleResolutionDownBy: requireScale ? Math.pow(2, i) : undefined
        });
      }
      // legacy SVC, scalabilityMode is set only on the first encoding
      /* @ts-ignore */
      encodings[0].scalabilityMode = scalabilityMode;
    } else {
      encodings.push({
        maxBitrate: videoEncoding.maxBitrate,
        maxFramerate: original.encoding.maxFramerate,
        /* @ts-ignore */
        scalabilityMode: scalabilityMode
      });
    }
    if (original.encoding.priority) {
      encodings[0].priority = original.encoding.priority;
      encodings[0].networkPriority = original.encoding.priority;
    }
    livekitLogger.debug("using svc encoding", {
      encodings
    });
    return encodings;
  }
  if (!useSimulcast) {
    return [videoEncoding];
  }
  // Chrome M113+ only treats multiple encodings on an SVC capable codec as real
  // simulcast when every encoding carries an explicit scalabilityMode next to its
  // scaleResolutionDownBy. Without it the encodings are interpreted as legacy SVC.
  const applySVCSimulcastMode = encodings => {
    if (useSVCSimulcast) {
      encodings.forEach(encoding => {
        /* @ts-ignore */
        encoding.scalabilityMode = scalabilityMode;
      });
    }
    return encodings;
  };
  let presets;
  if (isScreenShare) {
    presets = (_a = sortPresets(options === null || options === void 0 ? void 0 : options.screenShareSimulcastLayers)) !== null && _a !== void 0 ? _a : defaultSimulcastLayers(isScreenShare, original);
  } else {
    presets = (_b = sortPresets(options === null || options === void 0 ? void 0 : options.videoSimulcastLayers)) !== null && _b !== void 0 ? _b : defaultSimulcastLayers(isScreenShare, original);
  }
  let midPreset;
  if (presets.length > 0) {
    const lowPreset = presets[0];
    if (presets.length > 1) {
      var _presets = presets;
      var _presets2 = _slicedToArray(_presets, 2);
      midPreset = _presets2[1];
    }
    // NOTE:
    //   1. Ordering of these encodings is important. Chrome seems
    //      to use the index into encodings to decide which layer
    //      to disable when CPU constrained.
    //      So encodings should be ordered in increasing spatial
    //      resolution order.
    //   2. livekit-server translates rids into layers. So, all encodings
    //      should have the base layer `q` and then more added
    //      based on other conditions.
    const size = Math.max(width, height);
    if (size >= 960 && midPreset) {
      return applySVCSimulcastMode(encodingsFromPresets(width, height, [lowPreset, midPreset, original], sourceFramerate));
    }
    if (size >= 480) {
      return applySVCSimulcastMode(encodingsFromPresets(width, height, [lowPreset, original], sourceFramerate));
    }
  }
  return applySVCSimulcastMode(encodingsFromPresets(width, height, [original]));
}
/**
 * Bitrate to hint to the bandwidth estimator through `x-google-start-bitrate`, so that
 * a publish does not spend its first seconds ramping up from a very low rate.
 *
 * It has to be the total the encoder will put on the wire, which means picking the
 * encoding that carries the inclusive bitrate:
 *  - SVC publishes a single stream with the layers built in. `encodings[0]` holds the
 *    full bitrate — the legacy SVC shape orders its encodings `f`..`q`, so that holds
 *    for both SVC shapes.
 *  - Simulcast publishes independent streams ordered `q`..`f`, so the total is the sum.
 *    This includes VP9/AV1 published as rid based simulcast, where `encodings[0]` is
 *    the *smallest* layer even though the codec is SVC capable.
 *
 * @internal
 */
function computeStartTargetBitrate(codec, options, encodings) {
  var _a, _b;
  if (isSVCCodec(codec) && !isSVCSimulcast(codec, options)) {
    return (_b = (_a = encodings[0]) === null || _a === void 0 ? void 0 : _a.maxBitrate) !== null && _b !== void 0 ? _b : 0;
  }
  return encodings.reduce((sum, enc) => {
    var _a;
    return sum + ((_a = enc.maxBitrate) !== null && _a !== void 0 ? _a : 0);
  }, 0);
}
function computeTrackBackupEncodings(track, videoCodec, opts) {
  var _a, _b, _c, _d;
  // backupCodec should not be true anymore, default codec is set in LocalParticipant.publish
  if (!opts.backupCodec || opts.backupCodec === true || opts.backupCodec.codec === opts.videoCodec) {
    // backup codec publishing is disabled
    return;
  }
  if (videoCodec !== opts.backupCodec.codec) {
    livekitLogger.warn('requested a different codec than specified as backup', {
      serverRequested: videoCodec,
      backup: opts.backupCodec.codec
    });
  }
  opts.videoCodec = videoCodec;
  // use backup encoding setting as videoEncoding for backup codec publishing
  opts.videoEncoding = opts.backupCodec.encoding;
  const settings = track.mediaStreamTrack.getSettings();
  const width = (_a = settings.width) !== null && _a !== void 0 ? _a : (_b = track.dimensions) === null || _b === void 0 ? void 0 : _b.width;
  const height = (_c = settings.height) !== null && _c !== void 0 ? _c : (_d = track.dimensions) === null || _d === void 0 ? void 0 : _d.height;
  // disable simulcast for screenshare backup codec since L1Tx is used by primary codec
  if (track.source === Track.Source.ScreenShare && opts.simulcast) {
    opts.simulcast = false;
  }
  const encodings = computeVideoEncodings(track.source === Track.Source.ScreenShare, width, height, opts);
  return encodings;
}
/* @internal */
function determineAppropriateEncoding(isScreenShare, width, height, codec) {
  const presets = presetsForResolution(isScreenShare, width, height);
  let encoding = presets[0].encoding;
  // handle portrait by swapping dimensions
  const size = Math.max(width, height);
  for (let i = 0; i < presets.length; i += 1) {
    const preset = presets[i];
    encoding = preset.encoding;
    if (preset.width >= size) {
      break;
    }
  }
  // presets are based on the assumption of vp8 as a codec
  // for other codecs we adjust the maxBitrate if no specific videoEncoding has been provided
  // users should override these with ones that are optimized for their use case
  // NOTE: SVC codec bitrates are inclusive of all scalability layers. while
  // bitrate for non-SVC codecs does not include other simulcast layers.
  if (codec) {
    switch (codec) {
      case 'av1':
      case 'h265':
        encoding = Object.assign({}, encoding);
        encoding.maxBitrate = encoding.maxBitrate * 0.7;
        break;
      case 'vp9':
        encoding = Object.assign({}, encoding);
        encoding.maxBitrate = encoding.maxBitrate * 0.85;
        break;
    }
  }
  return encoding;
}
/* @internal */
function presetsForResolution(isScreenShare, width, height) {
  if (isScreenShare) {
    return presetsScreenShare;
  }
  const aspect = width > height ? width / height : height / width;
  if (Math.abs(aspect - 16.0 / 9) < Math.abs(aspect - 4.0 / 3)) {
    return presets169;
  }
  return presets43;
}
/* @internal */
function defaultSimulcastLayers(isScreenShare, original) {
  if (isScreenShare) {
    return computeDefaultScreenShareSimulcastPresets(original);
  }
  const width = original.width,
    height = original.height;
  const aspect = width > height ? width / height : height / width;
  if (Math.abs(aspect - 16.0 / 9) < Math.abs(aspect - 4.0 / 3)) {
    return defaultSimulcastPresets169;
  }
  return defaultSimulcastPresets43;
}
// presets should be ordered by low, medium, high
function encodingsFromPresets(width, height, presets, sourceFramerate) {
  const encodings = [];
  presets.forEach((preset, idx) => {
    if (idx >= videoRids.length) {
      return;
    }
    const size = Math.min(width, height);
    const rid = videoRids[idx];
    const encoding = {
      rid,
      scaleResolutionDownBy: Math.max(1, size / Math.min(preset.width, preset.height)),
      maxBitrate: preset.encoding.maxBitrate
    };
    // ensure that the sourceFramerate is the highest framerate applied across all layers so that the
    // original encoding doesn't get bumped unintentionally by any of the other layers
    const maxFramerate = sourceFramerate && preset.encoding.maxFramerate ? Math.min(sourceFramerate, preset.encoding.maxFramerate) : preset.encoding.maxFramerate;
    if (maxFramerate) {
      encoding.maxFramerate = maxFramerate;
    }
    const browser = getBrowser();
    const canSetPriority = (browser === null || browser === void 0 ? void 0 : browser.name) === 'Firefox' && browser.os !== 'iOS' || idx === 0;
    if (preset.encoding.priority && canSetPriority) {
      encoding.priority = preset.encoding.priority;
      encoding.networkPriority = preset.encoding.priority;
    }
    encodings.push(encoding);
  });
  // RN ios simulcast requires all same framerates.
  if (isReactNative() && getReactNativeOs() === 'ios') {
    let topFramerate = undefined;
    encodings.forEach(encoding => {
      if (!topFramerate) {
        topFramerate = encoding.maxFramerate;
      } else if (encoding.maxFramerate && encoding.maxFramerate > topFramerate) {
        topFramerate = encoding.maxFramerate;
      }
    });
    let notifyOnce = true;
    encodings.forEach(encoding => {
      var _a;
      if (encoding.maxFramerate != topFramerate) {
        if (notifyOnce) {
          notifyOnce = false;
          livekitLogger.info("Simulcast on iOS React-Native requires all encodings to share the same framerate.");
        }
        livekitLogger.info("Setting framerate of encoding \"".concat((_a = encoding.rid) !== null && _a !== void 0 ? _a : '', "\" to ").concat(topFramerate));
        encoding.maxFramerate = topFramerate;
      }
    });
  }
  return encodings;
}
/** @internal */
function sortPresets(presets) {
  if (!presets) return;
  // Sort a copy so we don't mutate the caller's array in place. Mutating the
  // passed-in simulcast layers can cause consumers that compare options by value
  // (e.g. components-react's useLiveKitRoom) to detect a spurious change.
  return presets.slice().sort((a, b) => {
    const aEnc = a.encoding;
    const bEnc = b.encoding;
    if (aEnc.maxBitrate > bEnc.maxBitrate) {
      return 1;
    }
    if (aEnc.maxBitrate < bEnc.maxBitrate) return -1;
    if (aEnc.maxBitrate === bEnc.maxBitrate && aEnc.maxFramerate && bEnc.maxFramerate) {
      return aEnc.maxFramerate > bEnc.maxFramerate ? 1 : -1;
    }
    return 0;
  });
}
/** @internal */
class ScalabilityMode {
  constructor(scalabilityMode) {
    const results = scalabilityMode.match(/^L(\d)T(\d)(h|_KEY|_KEY_SHIFT){0,1}$/);
    if (!results) {
      throw new Error('invalid scalability mode');
    }
    this.spatial = parseInt(results[1]);
    this.temporal = parseInt(results[2]);
    if (results.length > 3) {
      switch (results[3]) {
        case 'h':
        case '_KEY':
        case '_KEY_SHIFT':
          this.suffix = results[3];
      }
    }
  }
  toString() {
    var _a;
    return "L".concat(this.spatial, "T").concat(this.temporal).concat((_a = this.suffix) !== null && _a !== void 0 ? _a : '');
  }
}
/**
 * Returns the appropriate degradation preference for a video track based on its source.
 *
 * - Camera: 'maintain-framerate' (smoother video for real-time communication)
 * - Screen share: 'maintain-resolution' (clarity is critical for reading text/UI)
 * - Other/unknown: 'balanced'
 */
function getDefaultDegradationPreference(track) {
  switch (track.source) {
    case Track.Source.Camera:
      return 'maintain-framerate';
    case Track.Source.ScreenShare:
      return 'maintain-resolution';
    default:
      return 'balanced';
  }
}const refreshSubscribedCodecAfterNewCodec = 5000;
export { LocalAudioTrack, LocalTrack, ScalabilityMode, computeDefaultScreenShareSimulcastPresets, computeStartTargetBitrate, computeTrackBackupEncodings, computeVideoEncodings, defaultSimulcastLayers, defaultSimulcastPresets169, defaultSimulcastPresets43, determineAppropriateEncoding, encodingsFromPresets, getDefaultDegradationPreference, presets169, presets43, presetsForResolution, presetsScreenShare, refreshSubscribedCodecAfterNewCodec, sortPresets, videoRids };
