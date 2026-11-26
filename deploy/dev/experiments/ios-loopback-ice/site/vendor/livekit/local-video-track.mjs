// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 24345-25154 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { __asyncValues, __awaiter, _slicedToArray } from './tslib.mjs';
import { VideoLayer } from './protobuf-messages-room.mjs';
import { SubscribedQuality } from './protobuf-messages-signal2.mjs';
import { TrackEvent } from './event-enums.mjs';
import { VideoQuality, constraintsForOptions, isBackupVideoCodec } from './util.mjs';
import { Track, isFireFox, isMobile, isSVCCodec, isSVCSimulcast, isWeb } from './track.mjs';
import { computeBitrate, monitorFrequency } from './e2ee-frame.mjs';
import { LocalTrack, ScalabilityMode, computeTrackBackupEncodings, computeVideoEncodings, refreshSubscribedCodecAfterNewCodec } from './local-track.mjs';


class LocalVideoTrack extends LocalTrack {
  get sender() {
    return this._sender;
  }
  set sender(sender) {
    this._sender = sender;
    if (this.degradationPreference) {
      this.setDegradationPreference(this.degradationPreference);
    }
  }
  /**
   *
   * @param mediaTrack
   * @param constraints MediaTrackConstraints that are being used when restarting or reacquiring tracks
   * @param userProvidedTrack Signals to the SDK whether or not the mediaTrack should be managed (i.e. released and reacquired) internally by the SDK
   */
  constructor(mediaTrack, constraints) {
    let userProvidedTrack = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : true;
    let loggerOptions = arguments.length > 3 ? arguments[3] : undefined;
    super(mediaTrack, Track.Kind.Video, constraints, userProvidedTrack, loggerOptions);
    /* @internal */
    this.simulcastCodecs = new Map();
    this.degradationPreference = 'balanced';
    this.isCpuConstrained = false;
    this.optimizeForPerformance = false;
    this.monitorSender = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.sender) {
        this._currentBitrate = 0;
        return;
      }
      let stats;
      try {
        stats = yield this.getSenderStats();
      } catch (e) {
        this.log.error('could not get video sender stats', Object.assign(Object.assign({}, this.logContext), {
          error: e
        }));
        return;
      }
      const statsMap = new Map(stats.map(s => [s.rid, s]));
      const isCpuConstrained = stats.some(s => s.qualityLimitationReason === 'cpu');
      if (isCpuConstrained !== this.isCpuConstrained) {
        this.isCpuConstrained = isCpuConstrained;
        if (this.isCpuConstrained) {
          this.emit(TrackEvent.CpuConstrained);
        }
      }
      if (this.prevStats) {
        let totalBitrate = 0;
        statsMap.forEach((s, key) => {
          var _a;
          const prev = (_a = this.prevStats) === null || _a === void 0 ? void 0 : _a.get(key);
          totalBitrate += computeBitrate(s, prev);
        });
        this._currentBitrate = totalBitrate;
      }
      this.prevStats = statsMap;
    });
    this.senderLock = new _();
  }
  get isSimulcast() {
    if (this.sender && this.sender.getParameters().encodings.length > 1) {
      return true;
    }
    return false;
  }
  /* @internal */
  startMonitor(signalClient) {
    var _a;
    this.signalClient = signalClient;
    if (!isWeb()) {
      return;
    }
    // save original encodings
    // TODO : merge simulcast tracks stats
    const params = (_a = this.sender) === null || _a === void 0 ? void 0 : _a.getParameters();
    if (params) {
      this.encodings = params.encodings;
    }
    if (this.monitorInterval) {
      return;
    }
    this.monitorInterval = setInterval(() => {
      this.monitorSender();
    }, monitorFrequency);
  }
  stop() {
    this._mediaStreamTrack.getConstraints();
    this.simulcastCodecs.forEach(trackInfo => {
      trackInfo.mediaStreamTrack.stop();
    });
    super.stop();
  }
  pauseUpstream() {
    const _super = Object.create(null, {
      pauseUpstream: {
        get: () => super.pauseUpstream
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      var _a, e_1, _b, _c;
      var _d;
      yield _super.pauseUpstream.call(this);
      try {
        for (var _e = true, _f = __asyncValues(this.simulcastCodecs.values()), _g; _g = yield _f.next(), _a = _g.done, !_a; _e = true) {
          _c = _g.value;
          _e = false;
          const sc = _c;
          yield (_d = sc.sender) === null || _d === void 0 ? void 0 : _d.replaceTrack(null);
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
    });
  }
  resumeUpstream() {
    const _super = Object.create(null, {
      resumeUpstream: {
        get: () => super.resumeUpstream
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      var _a, e_2, _b, _c;
      var _d;
      yield _super.resumeUpstream.call(this);
      try {
        for (var _e = true, _f = __asyncValues(this.simulcastCodecs.values()), _g; _g = yield _f.next(), _a = _g.done, !_a; _e = true) {
          _c = _g.value;
          _e = false;
          const sc = _c;
          yield (_d = sc.sender) === null || _d === void 0 ? void 0 : _d.replaceTrack(sc.mediaStreamTrack);
        }
      } catch (e_2_1) {
        e_2 = {
          error: e_2_1
        };
      } finally {
        try {
          if (!_e && !_a && (_b = _f.return)) yield _b.call(_f);
        } finally {
          if (e_2) throw e_2.error;
        }
      }
    });
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
        if (this.source === Track.Source.Camera && !this.isUserProvided) {
          this.log.debug('stopping camera track', this.logContext);
          // also stop the track, so that camera indicator is turned off
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
        if (this.source === Track.Source.Camera && !this.isUserProvided) {
          this.log.debug('reacquiring camera track', this.logContext);
          yield this.restart(undefined, true);
        }
        yield _super.unmute.call(this);
        return this;
      } finally {
        unlock();
      }
    });
  }
  setTrackMuted(muted) {
    super.setTrackMuted(muted);
    for (const sc of this.simulcastCodecs.values()) {
      sc.mediaStreamTrack.enabled = !muted;
    }
  }
  getSenderStats() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!((_a = this.sender) === null || _a === void 0 ? void 0 : _a.getStats)) {
        return [];
      }
      const items = [];
      const stats = yield this.sender.getStats();
      stats.forEach(v => {
        var _a;
        if (v.type === 'outbound-rtp') {
          const vs = {
            type: 'video',
            streamId: v.id,
            frameHeight: v.frameHeight,
            frameWidth: v.frameWidth,
            framesPerSecond: v.framesPerSecond,
            framesSent: v.framesSent,
            firCount: v.firCount,
            pliCount: v.pliCount,
            nackCount: v.nackCount,
            packetsSent: v.packetsSent,
            bytesSent: v.bytesSent,
            qualityLimitationReason: v.qualityLimitationReason,
            qualityLimitationDurations: v.qualityLimitationDurations,
            qualityLimitationResolutionChanges: v.qualityLimitationResolutionChanges,
            rid: (_a = v.rid) !== null && _a !== void 0 ? _a : v.id,
            retransmittedPacketsSent: v.retransmittedPacketsSent,
            targetBitrate: v.targetBitrate,
            timestamp: v.timestamp
          };
          // locate the appropriate remote-inbound-rtp item
          const r = stats.get(v.remoteId);
          if (r) {
            vs.jitter = r.jitter;
            vs.packetsLost = r.packetsLost;
            vs.roundTripTime = r.roundTripTime;
          }
          items.push(vs);
        }
      });
      // make sure highest res layer is always first
      items.sort((a, b) => {
        var _a, _b;
        return ((_a = b.frameWidth) !== null && _a !== void 0 ? _a : 0) - ((_b = a.frameWidth) !== null && _b !== void 0 ? _b : 0);
      });
      return items;
    });
  }
  /**
   * Whether `codec` is being published as SVC (a single stream carrying all spatial
   * layers) as opposed to rid based simulcast. VP9/AV1 are SVC unless the publisher
   * opted into simulcast, in which case each rid is an independent stream and the
   * layers can be enabled/disabled individually.
   */
  isSvcPublish(codec) {
    return isSVCCodec(codec) && !isSVCSimulcast(codec, this.publishOptions);
  }
  setPublishingQuality(maxQuality) {
    const qualities = [];
    for (let q = VideoQuality.LOW; q <= VideoQuality.HIGH; q += 1) {
      qualities.push(new SubscribedQuality({
        quality: q,
        enabled: q <= maxQuality
      }));
    }
    this.log.debug("setting publishing quality. max quality ".concat(maxQuality), this.logContext);
    this.setPublishingLayers(this.isSvcPublish(this.codec), qualities);
  }
  restartTrack(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, e_3, _b, _c;
      var _d;
      let constraints;
      if (options) {
        const streamConstraints = constraintsForOptions({
          video: options
        });
        if (typeof streamConstraints.video !== 'boolean') {
          constraints = streamConstraints.video;
        }
      }
      yield this.restart(constraints);
      // reset cpu constrained state after track is restarted
      this.isCpuConstrained = false;
      try {
        for (var _e = true, _f = __asyncValues(this.simulcastCodecs.values()), _g; _g = yield _f.next(), _a = _g.done, !_a; _e = true) {
          _c = _g.value;
          _e = false;
          const sc = _c;
          if (sc.sender && ((_d = sc.sender.transport) === null || _d === void 0 ? void 0 : _d.state) !== 'closed') {
            sc.mediaStreamTrack = this.mediaStreamTrack.clone();
            yield sc.sender.replaceTrack(sc.mediaStreamTrack);
          }
        }
      } catch (e_3_1) {
        e_3 = {
          error: e_3_1
        };
      } finally {
        try {
          if (!_e && !_a && (_b = _f.return)) yield _b.call(_f);
        } finally {
          if (e_3) throw e_3.error;
        }
      }
      // The new MediaStreamTrack may have different dimensions than the previous one
      // (e.g. switching between cameras with different native resolutions), which would
      // leave the sender's encoding parameters (scaleResolutionDownBy, maxBitrate, etc.)
      // based on the old dimensions. Recompute them so the encoded output matches the
      // new source.
      yield this.onSenderTrackSwapped();
    });
  }
  onSenderTrackSwapped() {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.refreshSenderEncodings();
    });
  }
  /**
   * Recomputes encoding parameters for this track's senders based on the current
   * MediaStreamTrack dimensions and reapplies them via setParameters. This is a no-op
   * if the track hasn't been published yet or if the track is in performance-optimized
   * mode (which manages its own encodings).
   */
  refreshSenderEncodings() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!this.sender || !this.publishOptions || this.optimizeForPerformance) {
        return;
      }
      const unlock = yield this.senderLock.lock();
      try {
        let dims;
        try {
          dims = yield this.waitForDimensions();
        } catch (e) {
          this.log.warn('could not determine new track dimensions, skipping encoding recompute', Object.assign(Object.assign({}, this.logContext), {
            error: e
          }));
          return;
        }
        if (this.lastEncodedDimensions && this.lastEncodedDimensions.width === dims.width && this.lastEncodedDimensions.height === dims.height) {
          return;
        }
        const isScreenShare = this.source === Track.Source.ScreenShare;
        const newEncodings = computeVideoEncodings(isScreenShare, dims.width, dims.height, Object.assign({}, this.publishOptions));
        yield this.applyEncodingsToSender(this.sender, newEncodings);
        this.encodings = newEncodings;
        this.lastEncodedDimensions = dims;
        for (const _ref of this.simulcastCodecs) {
          var _ref2 = _slicedToArray(_ref, 2);
          const codec = _ref2[0];
          const sc = _ref2[1];
          if (!sc.sender || ((_a = sc.sender.transport) === null || _a === void 0 ? void 0 : _a.state) === 'closed') {
            continue;
          }
          if (!isBackupVideoCodec(codec)) {
            continue;
          }
          const backupOpts = Object.assign({}, this.publishOptions);
          const backupEncodings = computeTrackBackupEncodings(this, codec, backupOpts);
          if (!backupEncodings) {
            continue;
          }
          yield this.applyEncodingsToSender(sc.sender, backupEncodings);
          sc.encodings = backupEncodings;
        }
      } catch (e) {
        this.log.warn('failed to apply recomputed encodings', Object.assign(Object.assign({}, this.logContext), {
          error: e
        }));
      } finally {
        unlock();
      }
    });
  }
  applyEncodingsToSender(sender, encodings) {
    return __awaiter(this, void 0, void 0, function* () {
      const params = sender.getParameters();
      if (!params.encodings || params.encodings.length !== encodings.length) {
        return;
      }
      params.encodings.forEach((existing, idx) => {
        // preserve disabled layers (dynacast / Firefox workaround in
        // setPublishingLayersForSender set scaleResolutionDownBy/maxBitrate to sentinel
        // values for disabled layers — don't clobber those).
        if (existing.active === false) {
          return;
        }
        const next = encodings[idx];
        if (next.scaleResolutionDownBy !== undefined) {
          existing.scaleResolutionDownBy = next.scaleResolutionDownBy;
        }
        if (next.maxBitrate !== undefined) {
          existing.maxBitrate = next.maxBitrate;
        }
        if (next.maxFramerate !== undefined) {
          existing.maxFramerate = next.maxFramerate;
        }
        if (next.priority !== undefined) {
          existing.priority = next.priority;
          existing.networkPriority = next.priority;
        }
      });
      this.log.debug('updating sender encodings after track restart', Object.assign(Object.assign({}, this.logContext), {
        encodings: params.encodings
      }));
      yield sender.setParameters(params);
    });
  }
  /**
   * Sets a processor on this track, and wires its processed track into the simulcast codecs.
   *
   * The processor is destroyed when the track is stopped, which unpublishing and disconnecting
   * both do by default, so it has to be set again after reconnecting. See {@link TrackProcessor}
   * for the full lifecycle.
   */
  setProcessor(processor_1) {
    const _super = Object.create(null, {
      setProcessor: {
        get: () => super.setProcessor
      }
    });
    return __awaiter(this, arguments, void 0, function (processor) {
      var _this = this;
      let showProcessedStreamLocally = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
      return function* () {
        var _a, e_4, _b, _c;
        var _d, _e;
        yield _super.setProcessor.call(_this, processor, showProcessedStreamLocally);
        if ((_d = _this.processor) === null || _d === void 0 ? void 0 : _d.processedTrack) {
          try {
            for (var _f = true, _g = __asyncValues(_this.simulcastCodecs.values()), _h; _h = yield _g.next(), _a = _h.done, !_a; _f = true) {
              _c = _h.value;
              _f = false;
              const sc = _c;
              yield (_e = sc.sender) === null || _e === void 0 ? void 0 : _e.replaceTrack(_this.processor.processedTrack);
            }
          } catch (e_4_1) {
            e_4 = {
              error: e_4_1
            };
          } finally {
            try {
              if (!_f && !_a && (_b = _g.return)) yield _b.call(_g);
            } finally {
              if (e_4) throw e_4.error;
            }
          }
        }
      }();
    });
  }
  setDegradationPreference(preference) {
    return __awaiter(this, void 0, void 0, function* () {
      this.degradationPreference = preference;
      // applied one sender at a time on purpose, see applyDegradationPreference
      yield this.applyDegradationPreference(this.sender);
      for (const sc of this.simulcastCodecs.values()) {
        yield this.applyDegradationPreference(sc.sender);
      }
    });
  }
  /**
   * Degradation preference is a property of the sender, not of the track, so every sender
   * publishing this track needs it applied separately. A backup codec publishes over its
   * own sender, which would otherwise let the browser resolve a preference implicitly and
   * diverge from the primary encoder.
   *
   * Callers apply this sequentially rather than concurrently: `setParameters` is only valid
   * against the parameters most recently returned by `getParameters`, which is why this file
   * serializes other sender parameter updates through `senderLock`.
   */
  applyDegradationPreference(sender) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!sender) {
        return;
      }
      try {
        this.log.debug("setting degradationPreference to ".concat(this.degradationPreference), this.logContext);
        const params = sender.getParameters();
        params.degradationPreference = this.degradationPreference;
        yield sender.setParameters(params);
      } catch (e) {
        this.log.warn("failed to set degradationPreference", Object.assign({
          error: e
        }, this.logContext));
      }
    });
  }
  addSimulcastTrack(codec, encodings) {
    if (this.simulcastCodecs.has(codec)) {
      this.log.error("".concat(codec, " already added, skipping adding simulcast codec"), this.logContext);
      return;
    }
    const simulcastCodecInfo = {
      codec,
      mediaStreamTrack: this.mediaStreamTrack.clone(),
      sender: undefined,
      encodings
    };
    this.simulcastCodecs.set(codec, simulcastCodecInfo);
    return simulcastCodecInfo;
  }
  setSimulcastTrackSender(codec, sender) {
    return __awaiter(this, void 0, void 0, function* () {
      const simulcastCodecInfo = this.simulcastCodecs.get(codec);
      if (!simulcastCodecInfo) {
        return;
      }
      simulcastCodecInfo.sender = sender;
      // the backup codec publishes over its own sender, so it needs the same degradation
      // preference the primary sender resolved to.
      yield this.applyDegradationPreference(sender);
      // browser will reenable disabled codec/layers after new codec has been published,
      // so refresh subscribedCodecs after publish a new codec
      setTimeout(() => {
        this.refreshSubscribedCodecs();
      }, refreshSubscribedCodecAfterNewCodec);
    });
  }
  /** @interal */
  refreshSubscribedCodecs() {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.subscribedCodecs) {
        return this.setPublishingCodecs(this.subscribedCodecs).catch(e => this.log.warn("failed to refresh codec for track", Object.assign(Object.assign({}, this.logContext), {
          error: e
        })));
      }
    });
  }
  /**
   * @internal
   * Sets codecs that should be publishing, returns new codecs that have not yet
   * been published
   */
  setPublishingCodecs(codecs) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, codecs_1, codecs_1_1;
      var _b, e_5, _c, _d;
      this.log.debug('setting publishing codecs', Object.assign(Object.assign({}, this.logContext), {
        codecs,
        currentCodec: this.codec
      }));
      // only enable simulcast codec for preference codec setted
      if (!this.codec && codecs.length > 0) {
        yield this.setPublishingLayers(this.isSvcPublish(codecs[0].codec), codecs[0].qualities);
        return [];
      }
      this.subscribedCodecs = codecs;
      const newCodecs = [];
      try {
        for (_a = true, codecs_1 = __asyncValues(codecs); codecs_1_1 = yield codecs_1.next(), _b = codecs_1_1.done, !_b; _a = true) {
          _d = codecs_1_1.value;
          _a = false;
          const codec = _d;
          if (!this.codec || this.codec === codec.codec) {
            yield this.setPublishingLayers(this.isSvcPublish(codec.codec), codec.qualities);
          } else {
            const simulcastCodecInfo = this.simulcastCodecs.get(codec.codec);
            this.log.debug("try setPublishingCodec for ".concat(codec.codec), Object.assign(Object.assign({}, this.logContext), {
              simulcastCodecInfo
            }));
            if (!simulcastCodecInfo || !simulcastCodecInfo.sender) {
              for (const q of codec.qualities) {
                if (q.enabled) {
                  newCodecs.push(codec.codec);
                  break;
                }
              }
            } else if (simulcastCodecInfo.encodings) {
              this.log.debug("try setPublishingLayersForSender ".concat(codec.codec), this.logContext);
              yield setPublishingLayersForSender(simulcastCodecInfo.sender, simulcastCodecInfo.encodings, codec.qualities, this.senderLock, this.isSvcPublish(codec.codec), this.log, this.logContext);
            }
          }
        }
      } catch (e_5_1) {
        e_5 = {
          error: e_5_1
        };
      } finally {
        try {
          if (!_a && !_b && (_c = codecs_1.return)) yield _c.call(codecs_1);
        } finally {
          if (e_5) throw e_5.error;
        }
      }
      return newCodecs;
    });
  }
  /**
   * @internal
   * Sets layers that should be publishing
   */
  setPublishingLayers(isSvc, qualities) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.optimizeForPerformance) {
        this.log.info('skipping setPublishingLayers due to optimized publishing performance', Object.assign(Object.assign({}, this.logContext), {
          qualities
        }));
        return;
      }
      this.log.debug('setting publishing layers', Object.assign(Object.assign({}, this.logContext), {
        qualities
      }));
      if (!this.sender || !this.encodings) {
        return;
      }
      yield setPublishingLayersForSender(this.sender, this.encodings, qualities, this.senderLock, isSvc, this.log, this.logContext);
    });
  }
  /**
   * Designed for lower powered devices, reduces video publishing quality and disables simulcast.
   * @experimental
   */
  prioritizePerformance() {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.sender) {
        throw new Error('sender not found');
      }
      const unlock = yield this.senderLock.lock();
      try {
        this.optimizeForPerformance = true;
        const params = this.sender.getParameters();
        params.encodings = params.encodings.map((e, idx) => {
          var _a;
          return Object.assign(Object.assign({}, e), {
            active: idx === 0,
            scaleResolutionDownBy: Math.max(1, Math.ceil(((_a = this.mediaStreamTrack.getSettings().height) !== null && _a !== void 0 ? _a : 360) / 360)),
            scalabilityMode: idx === 0 && isSVCCodec(this.codec) ? 'L1T3' : undefined,
            maxFramerate: idx === 0 ? 15 : 0,
            maxBitrate: idx === 0 ? e.maxBitrate : 0
          });
        });
        this.log.debug('setting performance optimised encodings', Object.assign(Object.assign({}, this.logContext), {
          encodings: params.encodings
        }));
        this.encodings = params.encodings;
        yield this.sender.setParameters(params);
      } catch (e) {
        this.log.error('failed to set performance optimised encodings', Object.assign(Object.assign({}, this.logContext), {
          error: e
        }));
        this.optimizeForPerformance = false;
      } finally {
        unlock();
      }
    });
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
      if (this.isInBackground && this.source === Track.Source.Camera) {
        this._mediaStreamTrack.enabled = false;
      }
    });
  }
}
function setPublishingLayersForSender(sender, senderEncodings, qualities, senderLock, isSVC, log, logContext) {
  return __awaiter(this, void 0, void 0, function* () {
    const unlock = yield senderLock.lock();
    log.debug('setPublishingLayersForSender', Object.assign(Object.assign({}, logContext), {
      sender,
      qualities,
      senderEncodings
    }));
    try {
      const params = sender.getParameters();
      const encodings = params.encodings;
      if (!encodings) {
        return;
      }
      if (encodings.length !== senderEncodings.length) {
        log.warn('cannot set publishing layers, encodings mismatch', Object.assign(Object.assign({}, logContext), {
          encodings,
          senderEncodings
        }));
        return;
      }
      let hasChanged = false;
      /* disable closable spatial layer as it has video blur / frozen issue with current server / client
      1. chrome 113: when switching to up layer with scalability Mode change, it will generate a
            low resolution frame and recover very quickly, but noticable
      2. livekit sfu: additional pli request cause video frozen for a few frames, also noticable */
      const closableSpatial = false;
      /* @ts-ignore */
      if (closableSpatial && encodings[0].scalabilityMode) ; else {
        if (isSVC) {
          const hasEnabledEncoding = qualities.some(q => q.enabled);
          if (hasEnabledEncoding) {
            qualities.forEach(q => q.enabled = true);
          }
        }
        // simulcast dynacast encodings
        encodings.forEach((encoding, idx) => {
          var _a;
          let rid = (_a = encoding.rid) !== null && _a !== void 0 ? _a : '';
          if (rid === '') {
            rid = 'q';
          }
          const quality = videoQualityForRid(rid);
          const subscribedQuality = qualities.find(q => q.quality === quality);
          if (!subscribedQuality) {
            return;
          }
          if (encoding.active !== subscribedQuality.enabled) {
            hasChanged = true;
            encoding.active = subscribedQuality.enabled;
            log.debug("setting layer ".concat(subscribedQuality.quality, " to ").concat(encoding.active ? 'enabled' : 'disabled'), logContext);
            // FireFox does not support setting encoding.active to false, so we
            // have a workaround of lowering its bitrate and resolution to the min.
            if (isFireFox()) {
              if (subscribedQuality.enabled) {
                encoding.scaleResolutionDownBy = senderEncodings[idx].scaleResolutionDownBy;
                encoding.maxBitrate = senderEncodings[idx].maxBitrate;
                /* @ts-ignore */
                encoding.maxFrameRate = senderEncodings[idx].maxFrameRate;
              } else {
                encoding.scaleResolutionDownBy = 4;
                encoding.maxBitrate = 10;
                /* @ts-ignore */
                encoding.maxFrameRate = 2;
              }
            }
          }
        });
      }
      if (hasChanged) {
        params.encodings = encodings;
        log.debug("setting encodings", Object.assign(Object.assign({}, logContext), {
          encodings: params.encodings
        }));
        yield sender.setParameters(params);
      }
    } finally {
      unlock();
    }
  });
}
function videoQualityForRid(rid) {
  switch (rid) {
    case 'f':
      return VideoQuality.HIGH;
    case 'h':
      return VideoQuality.MEDIUM;
    case 'q':
      return VideoQuality.LOW;
    default:
      return VideoQuality.HIGH;
  }
}
function videoLayersFromEncodings(width, height, encodings, svc) {
  // default to a single layer, HQ
  if (!encodings) {
    return [new VideoLayer({
      quality: VideoQuality.HIGH,
      width,
      height,
      bitrate: 0,
      ssrc: 0
    })];
  }
  if (svc) {
    // svc layers
    /* @ts-ignore */
    const encodingSM = encodings[0].scalabilityMode;
    const sm = new ScalabilityMode(encodingSM);
    const layers = [];
    const resRatio = sm.suffix == 'h' ? 1.5 : 2;
    const bitratesRatio = sm.suffix == 'h' ? 2 : 3;
    for (let i = 0; i < sm.spatial; i += 1) {
      layers.push(new VideoLayer({
        quality: Math.min(VideoQuality.HIGH, sm.spatial - 1) - i,
        width: Math.ceil(width / Math.pow(resRatio, i)),
        height: Math.ceil(height / Math.pow(resRatio, i)),
        bitrate: encodings[0].maxBitrate ? Math.ceil(encodings[0].maxBitrate / Math.pow(bitratesRatio, i)) : 0,
        ssrc: 0
      }));
    }
    return layers;
  }
  return encodings.map(encoding => {
    var _a, _b, _c;
    const scale = (_a = encoding.scaleResolutionDownBy) !== null && _a !== void 0 ? _a : 1;
    let quality = videoQualityForRid((_b = encoding.rid) !== null && _b !== void 0 ? _b : '');
    return new VideoLayer({
      quality,
      width: Math.ceil(width / scale),
      height: Math.ceil(height / scale),
      bitrate: (_c = encoding.maxBitrate) !== null && _c !== void 0 ? _c : 0,
      ssrc: 0
    });
  });
}const minReconnectWait = 2 * 1000;
export { LocalVideoTrack, minReconnectWait, setPublishingLayersForSender, videoLayersFromEncodings, videoQualityForRid };
