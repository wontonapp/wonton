// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 20917-22202 行按职责切出，行为不变。

/** @internal */
import { _ } from './protobuf-helpers.mjs';
import { __awaiter } from './tslib.mjs';
import { SignalTarget } from './protobuf-messages-signal.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { DefaultReconnectPolicy, getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { TypedPromise } from './browser.mjs';
import { ConnectionError, CriticalTimers, NegotiationError, UnexpectedConnectionState } from './errors.mjs';
import { AudioPresets, ScreenSharePresets, VideoPresets } from './util.mjs';
import { ddExtensionURI, isSVCCodec, isSafari, sleep } from './track.mjs';
import { debounce } from './e2ee-frame.mjs';
import { PCEvents, applyVideoStartBitrate, computeConnectionStartBitrate, debounceInterval, findTrackCodecPayload, libExports } from './sdp-parser.mjs';

class PCTransport extends eventsExports.EventEmitter {
  get pc() {
    if (!this._pc) {
      this._pc = this.createPC();
    }
    return this._pc;
  }
  constructor(config) {
    let loggerOptions = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    var _a;
    super();
    this.log = livekitLogger;
    this.iceLog = livekitLogger;
    this.ddExtID = 0;
    this.latestOfferId = 0;
    this.answeredOfferId = 0;
    this.latestAcknowledgedOfferId = 0;
    this.pendingCandidates = [];
    this.restartingIce = false;
    this.renegotiate = false;
    this.trackBitrates = [];
    /**
     * Whether an offer carrying the connection-level `x-google-start-bitrate` has been
     * accepted locally. The hint is written once per peer connection: libwebrtc retains
     * `start_bitrate_bps` in `RtpBitrateConfigurator` and re-applies it on network route
     * changes, so a later rewrite is at best a no-op and at worst restarts a converged
     * bandwidth estimator. A new peer connection (full reconnect) seeds a new estimator.
     */
    this.hasAppliedVideoStartBitrate = false;
    this.remoteStereoMids = [];
    this.remoteNackMids = [];
    // debounced negotiate interface
    this.negotiate = debounce(onError => __awaiter(this, void 0, void 0, function* () {
      this.emit(PCEvents.NegotiationStarted);
      try {
        yield this.createAndSendOffer();
      } catch (e) {
        if (onError) {
          onError(e);
        } else {
          throw e;
        }
      }
    }), debounceInterval);
    this.close = () => {
      if (!this._pc) {
        return;
      }
      this.log.debug('closing peer connection');
      this.pendingInitialOffer = undefined;
      this._pc.close();
      this._pc.onconnectionstatechange = null;
      this._pc.oniceconnectionstatechange = null;
      this._pc.onicegatheringstatechange = null;
      this._pc.ondatachannel = null;
      this._pc.onnegotiationneeded = null;
      this._pc.onsignalingstatechange = null;
      this._pc.onicecandidate = null;
      this._pc.ondatachannel = null;
      this._pc.ontrack = null;
      this._pc.onconnectionstatechange = null;
      this._pc.oniceconnectionstatechange = null;
      this._pc = null;
    };
    this.loggerOptions = loggerOptions;
    this.log = getLogger((_a = loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.PCTransport, () => this.logContext);
    this.iceLog = getLogger(LoggerNames.ICE, () => this.logContext);
    this.config = config;
    this._pc = this.createPC();
    this.offerLock = new _();
  }
  createPC() {
    const pc = new RTCPeerConnection(this.config);
    pc.onicecandidate = ev => {
      var _a;
      if (!ev.candidate) return;
      this.iceLog.debug('local ICE candidate gathered', {
        candidate: ev.candidate.candidate
      });
      (_a = this.onIceCandidate) === null || _a === void 0 ? void 0 : _a.call(this, ev.candidate);
    };
    pc.onicecandidateerror = ev => {
      var _a;
      this.iceLog.debug('ICE candidate error', {
        event: ev
      });
      (_a = this.onIceCandidateError) === null || _a === void 0 ? void 0 : _a.call(this, ev);
    };
    pc.oniceconnectionstatechange = () => {
      var _a;
      this.iceLog.debug("ICE connection state: ".concat(pc.iceConnectionState));
      (_a = this.onIceConnectionStateChange) === null || _a === void 0 ? void 0 : _a.call(this, pc.iceConnectionState);
    };
    pc.onsignalingstatechange = () => {
      var _a;
      this.log.debug("signaling state: ".concat(pc.signalingState));
      (_a = this.onSignalingStatechange) === null || _a === void 0 ? void 0 : _a.call(this, pc.signalingState);
    };
    pc.onconnectionstatechange = () => {
      var _a;
      this.log.debug("connection state: ".concat(pc.connectionState));
      (_a = this.onConnectionStateChange) === null || _a === void 0 ? void 0 : _a.call(this, pc.connectionState);
    };
    pc.ondatachannel = ev => {
      var _a;
      this.log.debug('data channel opened by peer', {
        label: ev.channel.label,
        id: ev.channel.id
      });
      (_a = this.onDataChannel) === null || _a === void 0 ? void 0 : _a.call(this, ev);
    };
    pc.ontrack = ev => {
      var _a;
      (_a = this.onTrack) === null || _a === void 0 ? void 0 : _a.call(this, ev);
    };
    return pc;
  }
  get logContext() {
    var _a, _b;
    return Object.assign({}, (_b = (_a = this.loggerOptions).loggerContextCb) === null || _b === void 0 ? void 0 : _b.call(_a));
  }
  get isICEConnected() {
    return this._pc !== null && (this.pc.iceConnectionState === 'connected' || this.pc.iceConnectionState === 'completed');
  }
  addIceCandidate(candidate) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.pc.remoteDescription && !this.restartingIce) {
        return this.pc.addIceCandidate(candidate);
      }
      this.iceLog.debug('queuing remote ICE candidate until remote description applied', {
        pendingCount: this.pendingCandidates.length + 1
      });
      this.pendingCandidates.push(candidate);
    });
  }
  setRemoteDescription(sd, offerId) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (sd.type === 'answer' && this.latestOfferId > 0 && offerId > 0 && offerId !== this.latestOfferId) {
        this.log.warn('ignoring answer for old offer', {
          offerId,
          latestOfferId: this.latestOfferId
        });
        return false;
      }
      if (sd.type === 'answer') {
        if (isDuplicateAnswer(offerId, this.answerInFlight, this.answeredOfferId, (_a = this._pc) === null || _a === void 0 ? void 0 : _a.signalingState, this.pendingInitialOffer !== undefined)) {
          // a repeat of an answer already applied or being applied (e.g.
          // duplicated by a proxy): there is no local offer left to answer
          this.log.warn('ignoring duplicate answer', {
            offerId
          });
          return false;
        }
        // claimed before any await, so a copy arriving mid-apply is caught too;
        // released however the apply ends, so a genuine retry is still possible
        this.answerInFlight = offerId;
      }
      try {
        return yield this.applyRemoteDescription(sd, offerId);
      } finally {
        if (sd.type === 'answer') this.answerInFlight = undefined;
      }
    });
  }
  applyRemoteDescription(sd, offerId) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      let mungedSDP = undefined;
      if (sd.type === 'offer') {
        let _extractStereoAndNack = extractStereoAndNackAudioFromOffer(sd),
          stereoMids = _extractStereoAndNack.stereoMids,
          nackMids = _extractStereoAndNack.nackMids;
        this.remoteStereoMids = stereoMids;
        this.remoteNackMids = nackMids;
      } else if (sd.type === 'answer') {
        if (this.pendingInitialOffer && this._pc) {
          const initialOffer = this.pendingInitialOffer;
          this.pendingInitialOffer = undefined;
          const sdpParsed = libExports.parse((_a = initialOffer.sdp) !== null && _a !== void 0 ? _a : '');
          sdpParsed.media.forEach(media => {
            ensureIPAddrMatchVersion(media);
          });
          this.log.debug('setting pending initial offer before processing answer');
          yield this.setMungedSDP(initialOffer, libExports.write(sdpParsed));
        }
        const sdpParsed = libExports.parse((_b = sd.sdp) !== null && _b !== void 0 ? _b : '');
        sdpParsed.media.forEach(media => {
          const mid = getMidString(media.mid);
          if (media.type === 'audio') {
            // munge sdp for opus bitrate settings
            this.trackBitrates.some(trackbr => {
              if (!trackbr.transceiver || mid != trackbr.transceiver.mid) {
                return false;
              }
              let codecPayload = 0;
              media.rtp.some(rtp => {
                if (rtp.codec.toUpperCase() === trackbr.codec.toUpperCase()) {
                  codecPayload = rtp.payload;
                  return true;
                }
                return false;
              });
              if (codecPayload === 0) {
                return true;
              }
              let fmtpFound = false;
              for (const fmtp of media.fmtp) {
                if (fmtp.payload === codecPayload) {
                  fmtp.config = fmtp.config.split(';').filter(attr => !attr.includes('maxaveragebitrate')).join(';');
                  if (trackbr.maxbr > 0) {
                    fmtp.config += ";maxaveragebitrate=".concat(trackbr.maxbr * 1000);
                  }
                  fmtpFound = true;
                  break;
                }
              }
              if (!fmtpFound) {
                if (trackbr.maxbr > 0) {
                  media.fmtp.push({
                    payload: codecPayload,
                    config: "maxaveragebitrate=".concat(trackbr.maxbr * 1000)
                  });
                }
              }
              return true;
            });
          }
        });
        // The server's answer sets per-track codec params (e.g. opus `usedtx`) on
        // the sections mapped to a published track, but leaves the pre-populated
        // recvonly placeholder sections with defaults. Conform the placeholders so
        // each shared payload type is consistent across the bundle, otherwise
        // libwebrtc flags a "bundled payload type collision".
        const placeholderMids = this.getPlaceholderMids();
        if (placeholderMids.size > 0) {
          conformBundledCodecFmtp(sdpParsed.media, media => placeholderMids.has(getMidString(media.mid)));
        }
        mungedSDP = libExports.write(sdpParsed);
      }
      yield this.setMungedSDP(sd, mungedSDP, true);
      // only a successfully applied answer makes later copies duplicates
      if (sd.type === 'answer' && offerId > 0) this.answeredOfferId = offerId;
      if (this.pendingCandidates.length > 0) {
        this.iceLog.debug('flushing queued ICE candidates', {
          count: this.pendingCandidates.length
        });
      }
      this.pendingCandidates.forEach(candidate => {
        this.pc.addIceCandidate(candidate);
      });
      this.pendingCandidates = [];
      this.restartingIce = false;
      // Fire OfferAnswered for every successfully applied answer, including the
      // ones that recurse into another offer via `renegotiate`. Callers waiting
      // on a specific offerId can resolve as soon as their offer's answer is in.
      if (sd.type === 'answer') {
        this.latestAcknowledgedOfferId = offerId;
        this.emit(PCEvents.OfferAnswered, offerId);
      }
      if (this.renegotiate) {
        this.renegotiate = false;
        yield this.createAndSendOffer();
      } else if (sd.type === 'answer') {
        this.emit(PCEvents.NegotiationComplete);
        if (sd.sdp) {
          const sdpParsed = libExports.parse(sd.sdp);
          sdpParsed.media.forEach(media => {
            if (media.type === 'video') {
              this.emit(PCEvents.RTPVideoPayloadTypes, media.rtp);
            }
          });
        }
      }
      return true;
    });
  }
  createInitialOffer() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.offerLock.lock();
      try {
        if (this.pc.signalingState !== 'stable') {
          this.log.warn('signaling state is not stable, cannot create initial offer');
          return;
        }
        const offerId = this.latestOfferId + 1;
        this.latestOfferId = offerId;
        const offer = yield this.pc.createOffer();
        this.pendingInitialOffer = {
          sdp: offer.sdp,
          type: offer.type
        };
        const sdpParsed = libExports.parse((_a = offer.sdp) !== null && _a !== void 0 ? _a : '');
        sdpParsed.media.forEach(media => {
          ensureIPAddrMatchVersion(media);
        });
        offer.sdp = libExports.write(sdpParsed);
        return {
          offer,
          offerId
        };
      } finally {
        unlock();
      }
    });
  }
  createAndSendOffer(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.offerLock.lock();
      try {
        if (this.onOffer === undefined) {
          return;
        }
        if (options === null || options === void 0 ? void 0 : options.iceRestart) {
          this.iceLog.debug('restarting ICE');
          this.restartingIce = true;
        }
        if (this._pc && (this._pc.signalingState === 'have-local-offer' || this.pendingInitialOffer)) {
          // we're waiting for the peer to accept our offer, so we'll just wait
          // the only exception to this is when ICE restart is needed
          const currentSD = this._pc.remoteDescription;
          if ((options === null || options === void 0 ? void 0 : options.iceRestart) && currentSD) {
            // roll the remote description back in so createOffer produces a valid
            // ICE-restart offer on top of the already-negotiated state
            yield this._pc.setRemoteDescription(currentSD);
          } else if (options === null || options === void 0 ? void 0 : options.iceRestart) {
            // ICE restart with no remote description to restart on: `renegotiate` would stall
            // (the pending offer is never answered), so throw for the caller to recreate the PC.
            throw new NegotiationError('ICE restart requested without a remote description, peer connection must be recreated');
          } else {
            this.renegotiate = true;
            this.log.debug('requesting renegotiation');
            return;
          }
        } else if (!this._pc || this._pc.signalingState === 'closed') {
          this.log.warn('could not createOffer with closed peer connection');
          return;
        }
        // actually negotiate
        this.log.debug('starting to negotiate');
        // increase the offer id at the start to ensure the offer is always > 0 so that we can use 0 as a default value for legacy behavior
        const offerId = this.latestOfferId + 1;
        this.latestOfferId = offerId;
        const offer = yield this.pc.createOffer(options);
        this.log.debug('original offer', {
          sdp: offer.sdp
        });
        const sdpParsed = libExports.parse((_a = offer.sdp) !== null && _a !== void 0 ? _a : '');
        // One value for every video m-section, written only on the first offer that carries
        // local video: the hint is connection-level in libwebrtc, so differing per-section
        // values would be last-writer-wins on m-section order. Offers before any video is
        // published (data channel or audio only) find no target and leave the latch unset.
        const connectionStartBitrate = this.hasAppliedVideoStartBitrate ? undefined : computeConnectionStartBitrate(sdpParsed.media, this.trackBitrates, this.connectionSetupTimeEstimateMs());
        let appliedVideoStartBitrate = false;
        sdpParsed.media.forEach(media => {
          ensureIPAddrMatchVersion(media);
          if (media.type === 'audio') {
            ensureAudioNackAndStereo(media, ['all'], []);
          } else if (media.type === 'video') {
            this.trackBitrates.some(trackbr => {
              if (!trackbr.cid) {
                return false;
              }
              const codecPayload = findTrackCodecPayload(media, trackbr.cid, trackbr.codec);
              if (codecPayload === undefined) {
                return false;
              }
              if (codecPayload > 0) {
                if (connectionStartBitrate !== undefined) {
                  appliedVideoStartBitrate = applyVideoStartBitrate(media, codecPayload, connectionStartBitrate) || appliedVideoStartBitrate;
                }
                if (isSVCCodec(trackbr.codec) && !isSafari()) {
                  this.ddExtID = ensureVideoDDExtension(media, sdpParsed, this.ddExtID);
                }
              }
              return true;
            });
          }
        });
        // Conform the placeholder sections (pre-populated, or reverted after an
        // unpublish) so every shared payload type carries identical fmtp across the
        // bundle, otherwise libwebrtc flags a "bundled payload type collision".
        // Detection is by transceiver (mids are stable across renegotiations) since
        // an unpublished section keeps its `a=msid`.
        const placeholderMids = this.getPlaceholderMids();
        if (placeholderMids.size > 0) {
          conformBundledCodecFmtp(sdpParsed.media, media => placeholderMids.has(getMidString(media.mid)));
        }
        if (this.latestOfferId > offerId) {
          this.log.warn('latestOfferId mismatch', {
            latestOfferId: this.latestOfferId,
            offerId
          });
          return;
        }
        const mungedSdp = libExports.write(sdpParsed);
        yield this.setMungedSDP(offer, mungedSdp);
        // setMungedSDP falls back to the unmunged SDP on rejection. Only consume the
        // one-shot hint once the SDP carrying it has been accepted locally.
        if (appliedVideoStartBitrate && offer.sdp === mungedSdp) {
          this.hasAppliedVideoStartBitrate = true;
          this.log.info('applied x-google-start-bitrate', {
            startBitrateKbps: connectionStartBitrate,
            connectionSetupTimeMs: this.connectionSetupTimeEstimateMs()
          });
        }
        this.onOffer(offer, this.latestOfferId);
      } finally {
        unlock();
      }
    });
  }
  createAndSetAnswer() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const answer = yield this.pc.createAnswer();
      const sdpParsed = libExports.parse((_a = answer.sdp) !== null && _a !== void 0 ? _a : '');
      sdpParsed.media.forEach(media => {
        ensureIPAddrMatchVersion(media);
        if (media.type === 'audio') {
          ensureAudioNackAndStereo(media, this.remoteStereoMids, this.remoteNackMids);
        }
      });
      yield this.setMungedSDP(answer, libExports.write(sdpParsed));
      return answer;
    });
  }
  /**
   * Returns the mids of transceivers that carry no outgoing track on this
   * (publisher) connection: the pre-populated placeholders added by
   * `RTCEngine.applyInitialPublisherLayout`, plus any transceiver that was used
   * for a track and reverted on unpublish. Their codec fmtp is conformed to the
   * published tracks so a shared payload type stays consistent across the bundle.
   */
  getPlaceholderMids() {
    var _a, _b;
    return placeholderMidsFromTransceivers((_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.getTransceivers()) !== null && _b !== void 0 ? _b : []);
  }
  createDataChannel(label, dataChannelDict) {
    return this.pc.createDataChannel(label, dataChannelDict);
  }
  addTransceiver(mediaStreamTrack, transceiverInit) {
    return this.pc.addTransceiver(mediaStreamTrack, transceiverInit);
  }
  addTransceiverOfKind(kind, transceiverInit) {
    return this.pc.addTransceiver(kind, transceiverInit);
  }
  addTrack(track) {
    if (!this._pc) {
      throw new UnexpectedConnectionState('PC closed, cannot add track');
    }
    return this._pc.addTrack(track);
  }
  setTrackCodecBitrate(info) {
    this.trackBitrates.push(info);
  }
  /**
   * Record how long this peer connection took to set up. Called once, when the initial connect
   * succeeds; resumes and ICE restarts keep the estimator they have and never call this.
   */
  setConnectionSetupTime(ms) {
    this.connectionSetupTimeMs = ms;
  }
  /** Record when the connection attempt that built this peer connection began. */
  setConnectionStartedAt(startedAt) {
    this.connectionStartedAt = startedAt;
  }
  /** The measured setup time, or while still connecting, the time elapsed so far. */
  connectionSetupTimeEstimateMs() {
    if (this.connectionSetupTimeMs !== undefined) {
      return this.connectionSetupTimeMs;
    }
    if (this.connectionStartedAt === undefined) {
      return undefined;
    }
    return Math.round(performance.now() - this.connectionStartedAt);
  }
  setConfiguration(rtcConfig) {
    var _a;
    if (!this._pc) {
      throw new UnexpectedConnectionState('PC closed, cannot configure');
    }
    return (_a = this._pc) === null || _a === void 0 ? void 0 : _a.setConfiguration(rtcConfig);
  }
  canRemoveTrack() {
    var _a;
    return !!((_a = this._pc) === null || _a === void 0 ? void 0 : _a.removeTrack);
  }
  removeTrack(sender) {
    var _a;
    return (_a = this._pc) === null || _a === void 0 ? void 0 : _a.removeTrack(sender);
  }
  getConnectionState() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.connectionState) !== null && _b !== void 0 ? _b : 'closed';
  }
  getICEConnectionState() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.iceConnectionState) !== null && _b !== void 0 ? _b : 'closed';
  }
  getSignallingState() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.signalingState) !== null && _b !== void 0 ? _b : 'closed';
  }
  getTransceivers() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.getTransceivers()) !== null && _b !== void 0 ? _b : [];
  }
  getSenders() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.getSenders()) !== null && _b !== void 0 ? _b : [];
  }
  getLocalDescription() {
    var _a;
    return (_a = this._pc) === null || _a === void 0 ? void 0 : _a.localDescription;
  }
  getRemoteDescription() {
    var _a;
    return (_a = this.pc) === null || _a === void 0 ? void 0 : _a.remoteDescription;
  }
  /** stats of the underlying connection, `undefined` when there is none */
  getStats() {
    var _a;
    return (_a = this._pc) === null || _a === void 0 ? void 0 : _a.getStats();
  }
  getMaxMessageSize() {
    var _a, _b;
    return (_b = (_a = this._pc) === null || _a === void 0 ? void 0 : _a.sctp) === null || _b === void 0 ? void 0 : _b.maxMessageSize;
  }
  getConnectedAddress() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!this._pc) {
        return;
      }
      let selectedCandidatePairId = '';
      const candidatePairs = new Map();
      // id -> candidate ip
      const candidates = new Map();
      const stats = yield this._pc.getStats();
      stats.forEach(v => {
        switch (v.type) {
          case 'transport':
            selectedCandidatePairId = v.selectedCandidatePairId;
            break;
          case 'candidate-pair':
            if (selectedCandidatePairId === '' && v.selected) {
              selectedCandidatePairId = v.id;
            }
            candidatePairs.set(v.id, v);
            break;
          case 'remote-candidate':
            candidates.set(v.id, "".concat(v.address, ":").concat(v.port));
            break;
        }
      });
      if (selectedCandidatePairId === '') {
        return undefined;
      }
      const selectedID = (_a = candidatePairs.get(selectedCandidatePairId)) === null || _a === void 0 ? void 0 : _a.remoteCandidateId;
      if (selectedID === undefined) {
        return undefined;
      }
      return candidates.get(selectedID);
    });
  }
  setMungedSDP(sd, munged, remote) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      const originalSdp = sd.sdp;
      if (munged) {
        sd.sdp = munged;
        try {
          this.log.debug("setting munged ".concat(remote ? 'remote' : 'local', " description"));
          if (remote) {
            yield this.pc.setRemoteDescription(sd);
          } else {
            yield this.pc.setLocalDescription(sd);
          }
          return;
        } catch (e) {
          this.log.warn("not able to set ".concat(sd.type, ", falling back to unmodified sdp"), {
            error: e,
            mungedSdp: munged,
            originalSdp
          });
          sd.sdp = originalSdp;
        }
      }
      try {
        if (remote) {
          yield (_a = this._pc) === null || _a === void 0 ? void 0 : _a.setRemoteDescription(sd);
        } else {
          yield (_b = this._pc) === null || _b === void 0 ? void 0 : _b.setLocalDescription(sd);
        }
      } catch (e) {
        let msg = 'unknown error';
        if (e instanceof Error) {
          msg = e.message;
        } else if (typeof e === 'string') {
          msg = e;
        }
        const fields = {
          error: msg,
          sdp: sd.sdp
        };
        if (munged && munged !== originalSdp) {
          fields.mungedSdp = munged;
        }
        if (!remote && this.pc.remoteDescription) {
          fields.remoteSdp = this.pc.remoteDescription;
        }
        this.log.error("unable to set ".concat(sd.type), {
          fields
        });
        throw new NegotiationError(msg);
      }
    });
  }
}
/**
 * Adds the AV1 dependency descriptor extension to `media` unless it is already there, and
 * returns the id it is mapped to so callers can pass it back in as `ddExtID` (0 when no id has
 * been chosen yet).
 *
 * A bundle has to map one URI to one id, so an id already in use for the extension anywhere in
 * `sdp` wins over both the cached one and a fresh one: Chrome advertises the extension itself on
 * sections it can send on, and an earlier offer may have munged it into others.
 * @internal
 */
function ensureVideoDDExtension(media, sdp, ddExtID) {
  var _a, _b;
  const id = ddExtensionIDFor(sdp, ddExtID);
  if (id === undefined) {
    return ddExtID;
  }
  if (!((_a = media.ext) === null || _a === void 0 ? void 0 : _a.some(ext => ext.uri === ddExtensionURI))) {
    (_b = media.ext) !== null && _b !== void 0 ? _b : media.ext = [];
    media.ext.push({
      value: id,
      uri: ddExtensionURI
    });
  }
  return id;
}
/**
 * The id to map the dependency descriptor to throughout `sdp`, or undefined when no id would be
 * consistent for the whole bundle and the extension therefore has to be left out.
 */
function ddExtensionIDFor(sdp, cachedID) {
  const mapped = mappedExtensionID(sdp, ddExtensionURI);
  if (mapped !== undefined) {
    // Adopting an id that also stands for another URI is what the browser rejects the bundle
    // over, and its own half of the map is not ours to renumber, so give up on this offer.
    return usedForOtherURI(sdp, mapped, ddExtensionURI) ? undefined : mapped;
  }
  // Reusing the id from the last offer keeps the mapping stable across renegotiations, but only
  // while nothing else has taken it: the browser assigns ids to its own extensions without
  // knowing about ours, so an id that was free when we picked it can since have been claimed —
  // typically by the fuller extension set that arrives with the first section we send on.
  if (cachedID !== 0 && !usedForOtherURI(sdp, cachedID, ddExtensionURI)) {
    return cachedID;
  }
  return unusedExtensionID(sdp);
}
/** The id `uri` is mapped to in `sdp`, if any section maps it. */
function mappedExtensionID(sdp, uri) {
  var _a;
  for (const media of sdp.media) {
    const ext = (_a = media.ext) === null || _a === void 0 ? void 0 : _a.find(candidate => candidate.uri === uri);
    if (ext) {
      return ext.value;
    }
  }
  return undefined;
}
/** Whether `id` stands for anything in `sdp` other than `uri`. */
function usedForOtherURI(sdp, id, uri) {
  return sdp.media.some(media => {
    var _a;
    return (_a = media.ext) === null || _a === void 0 ? void 0 : _a.some(ext => ext.value === id && ext.uri !== uri);
  });
}
/**
 * An id no extension in `sdp` uses. Stays above every id in use rather than filling gaps, so it
 * is less likely to be an id the browser goes on to allocate to another extension, and steps
 * over 15, which RFC 8285 reserves.
 */
function unusedExtensionID(sdp) {
  let maxID = 0;
  sdp.media.forEach(media => {
    var _a;
    (_a = media.ext) === null || _a === void 0 ? void 0 : _a.forEach(ext => {
      if (ext.value > maxID) {
        maxID = ext.value;
      }
    });
  });
  return maxID + 1 === 15 ? 16 : maxID + 1;
}
/**
 * Checks whether an fmtp config declares `param` as an exact, `;`-delimited
 * token. A plain substring check conflates distinct opus parameters — e.g.
 * `stereo=1` is a substring of `sprop-stereo=1` — so `param` must match a whole
 * parameter, not appear anywhere within the config string.
 * @internal
 */
function fmtpConfigHasParam(config, param) {
  return config.split(';').some(entry => entry.trim() === param);
}
/** @internal */
function ensureAudioNackAndStereo(media, stereoMids, nackMids) {
  // sdp-transform types don't include number however the parser outputs mids as numbers in some cases
  const mid = getMidString(media.mid);
  // found opus codec to add nack fb
  let opusPayload = 0;
  media.rtp.some(rtp => {
    // rtpmap encoding names are case-insensitive (RFC 4855)
    if (rtp.codec.toLowerCase() === 'opus') {
      opusPayload = rtp.payload;
      return true;
    }
    return false;
  });
  // add nack rtcpfb if not exist
  if (opusPayload > 0) {
    if (!media.rtcpFb) {
      media.rtcpFb = [];
    }
    if (nackMids.includes(mid) && !media.rtcpFb.some(fb => fb.payload === opusPayload && fb.type === 'nack')) {
      media.rtcpFb.push({
        payload: opusPayload,
        type: 'nack'
      });
    }
    if (stereoMids.includes(mid) || stereoMids.length === 1 && stereoMids[0] === 'all') {
      media.fmtp.some(fmtp => {
        if (fmtp.payload === opusPayload) {
          if (!fmtpConfigHasParam(fmtp.config, 'stereo=1')) {
            fmtp.config += ';stereo=1';
          }
          return true;
        }
        return false;
      });
    }
  }
}
/**
 * Returns the mids of transceivers that carry no outgoing track: the
 * pre-populated placeholders added by `RTCEngine.applyInitialPublisherLayout`,
 * plus any transceiver that was used for a track and reverted on unpublish. The
 * `sender.track` check is the reliable signal — an unpublished section keeps its
 * `a=msid` (and its stale send-derived fmtp), so it can't be told apart from a
 * real send by SDP alone. Transceiver mids are stable across renegotiations, so
 * this works for every offer/answer after the first.
 * @internal
 */
function placeholderMidsFromTransceivers(transceivers) {
  const mids = new Set();
  for (const transceiver of transceivers) {
    if (transceiver.mid && !transceiver.sender.track) {
      mids.add(transceiver.mid);
    }
  }
  return mids;
}
/**
 * Within a BUNDLE group a payload type must map to identical codec parameters
 * across every m-line. When the same payload type carries different fmtp between
 * sections — e.g. opus `usedtx=1` on the published microphone but not on the
 * pre-populated recvonly placeholders, or H.265 with different `level-id` between
 * a published video track and a placeholder — libwebrtc flags a "bundled payload
 * type collision".
 *
 * Rewrite the placeholder sections so every shared payload type carries the
 * same fmtp. Real (non-placeholder) sections always win the canonical value, so
 * a published track's encoder parameters are never altered. When no real
 * section declares a payload type — e.g. a placeholder that was reused for a
 * track and then reverted to recvonly keeps its send-derived `level-id` while
 * fresh placeholders use the default — the placeholders still converge on the
 * first value seen, so two placeholders can't disagree either. Only placeholder
 * sections are ever rewritten, and it is codec-agnostic (opus, H.265, ...).
 * `isPlaceholder` identifies the sections to conform.
 * @internal
 */
function conformBundledCodecFmtp(media, isPlaceholder) {
  var _a, _b;
  // Canonical fmtp per payload type. Payload types are unique within a BUNDLE
  // group, so keying by payload alone (across audio and video) is safe. A real
  // section's value always takes precedence; otherwise the first placeholder
  // value seen is used so divergent placeholders still converge.
  const canonicalByPayload = new Map();
  const fromRealSection = new Set();
  for (const m of media) {
    const placeholder = isPlaceholder(m);
    for (const fmtp of (_a = m.fmtp) !== null && _a !== void 0 ? _a : []) {
      if (!placeholder) {
        canonicalByPayload.set(fmtp.payload, fmtp.config);
        fromRealSection.add(fmtp.payload);
      } else if (!canonicalByPayload.has(fmtp.payload)) {
        canonicalByPayload.set(fmtp.payload, fmtp.config);
      }
    }
  }
  if (canonicalByPayload.size === 0) {
    return;
  }
  // Conform placeholder sections to the canonical fmtp for each shared payload.
  for (const m of media) {
    if (!isPlaceholder(m)) {
      continue;
    }
    for (const fmtp of (_b = m.fmtp) !== null && _b !== void 0 ? _b : []) {
      const config = canonicalByPayload.get(fmtp.payload);
      if (config !== undefined && fmtp.config !== config) {
        fmtp.config = config;
      }
    }
  }
}
/** @internal */
/**
 * Whether an incoming answer repeats one already applied or being applied.
 * Any answer for the offer whose answer is in flight is a duplicate, numbered
 * or not. With an offerId (current servers) an answer is also a duplicate if
 * that offer's answer was already applied. Without one, fall back to the
 * signaling state: in `stable` no local offer is pending, unless the initial
 * offer is still deferred (it is set locally only once its answer arrives).
 */
function isDuplicateAnswer(offerId, answerInFlight, answeredOfferId, signalingState, hasPendingInitialOffer) {
  if (answerInFlight === offerId) return true;
  if (offerId > 0) return offerId === answeredOfferId;
  return signalingState === 'stable' && !hasPendingInitialOffer;
}
function extractStereoAndNackAudioFromOffer(offer) {
  var _a;
  const stereoMids = [];
  const nackMids = [];
  const sdpParsed = libExports.parse((_a = offer.sdp) !== null && _a !== void 0 ? _a : '');
  let opusPayload = 0;
  sdpParsed.media.forEach(media => {
    var _a;
    const mid = getMidString(media.mid);
    if (media.type === 'audio') {
      media.rtp.some(rtp => {
        // rtpmap encoding names are case-insensitive (RFC 4855)
        if (rtp.codec.toLowerCase() === 'opus') {
          opusPayload = rtp.payload;
          return true;
        }
        return false;
      });
      if ((_a = media.rtcpFb) === null || _a === void 0 ? void 0 : _a.some(fb => fb.payload === opusPayload && fb.type === 'nack')) {
        nackMids.push(mid);
      }
      media.fmtp.some(fmtp => {
        if (fmtp.payload === opusPayload) {
          if (fmtpConfigHasParam(fmtp.config, 'sprop-stereo=1')) {
            stereoMids.push(mid);
          }
          return true;
        }
        return false;
      });
    }
  });
  return {
    stereoMids,
    nackMids
  };
}
function ensureIPAddrMatchVersion(media) {
  // Chrome could generate sdp with c = IN IP4 <ipv6 addr>
  // in edge case and return error when set sdp.This is not a
  // sdk error but correct it if the issue detected.
  if (media.connection) {
    const isV6 = media.connection.ip.indexOf(':') >= 0;
    if (media.connection.version === 4 && isV6 || media.connection.version === 6 && !isV6) {
      // fallback to dummy address
      media.connection.ip = '0.0.0.0';
      media.connection.version = 4;
    }
  }
}
function getMidString(mid) {
  return typeof mid === 'number' ? mid.toFixed(0) : mid;
}const defaultVideoCodec = 'vp8';
const publishDefaults = {
  audioPreset: AudioPresets.music,
  dtx: true,
  red: true,
  forceStereo: false,
  simulcast: true,
  screenShareEncoding: ScreenSharePresets.h1080fps15.encoding,
  stopMicTrackOnMute: false,
  videoCodec: defaultVideoCodec,
  backupCodec: true,
  preConnectBuffer: false
};
const audioDefaults = {
  deviceId: {
    ideal: 'default'
  },
  autoGainControl: true,
  echoCancellation: true,
  noiseSuppression: true,
  voiceIsolation: true
};
const videoDefaults = {
  deviceId: {
    ideal: 'default'
  },
  resolution: VideoPresets.h720.resolution
};
const roomOptionDefaults = {
  adaptiveStream: false,
  dynacast: false,
  stopLocalTrackOnUnpublish: true,
  reconnectPolicy: new DefaultReconnectPolicy(),
  disconnectOnPageLeave: true,
  webAudioMix: false,
  singlePeerConnection: true
};
const roomConnectOptionDefaults = {
  autoSubscribe: true,
  disableIceLite: false,
  maxRetries: 1,
  peerConnectionTimeout: 15000,
  websocketTimeout: 15000
};var PCTransportState;
(function (PCTransportState) {
  PCTransportState[PCTransportState["NEW"] = 0] = "NEW";
  PCTransportState[PCTransportState["CONNECTING"] = 1] = "CONNECTING";
  PCTransportState[PCTransportState["CONNECTED"] = 2] = "CONNECTED";
  PCTransportState[PCTransportState["FAILED"] = 3] = "FAILED";
  PCTransportState[PCTransportState["CLOSING"] = 4] = "CLOSING";
  PCTransportState[PCTransportState["CLOSED"] = 5] = "CLOSED";
})(PCTransportState || (PCTransportState = {}));
class PCTransportManager {
  get needsPublisher() {
    return this.isPublisherConnectionRequired;
  }
  get needsSubscriber() {
    return this.isSubscriberConnectionRequired;
  }
  get currentState() {
    return this.state;
  }
  get mode() {
    return this._mode;
  }
  constructor(mode, loggerOptions, rtcConfig) {
    var _a;
    this.peerConnectionTimeout = roomConnectOptionDefaults.peerConnectionTimeout;
    this.log = livekitLogger;
    this.iceLog = livekitLogger;
    this.updateState = () => {
      var _a, _b;
      const previousState = this.state;
      const connectionStates = this.requiredTransports.map(tr => tr.getConnectionState());
      if (connectionStates.every(st => st === 'connected')) {
        this.state = PCTransportState.CONNECTED;
      } else if (connectionStates.some(st => st === 'failed')) {
        this.state = PCTransportState.FAILED;
      } else if (connectionStates.some(st => st === 'connecting')) {
        this.state = PCTransportState.CONNECTING;
      } else if (connectionStates.every(st => st === 'closed')) {
        this.state = PCTransportState.CLOSED;
      } else if (connectionStates.some(st => st === 'closed')) {
        this.state = PCTransportState.CLOSING;
      } else if (connectionStates.every(st => st === 'new')) {
        this.state = PCTransportState.NEW;
      }
      if (previousState !== this.state) {
        this.log.debug("pc state change: from ".concat(PCTransportState[previousState], " to ").concat(PCTransportState[this.state]));
        (_a = this.onStateChange) === null || _a === void 0 ? void 0 : _a.call(this, this.state, this.publisher.getConnectionState(), (_b = this.subscriber) === null || _b === void 0 ? void 0 : _b.getConnectionState());
      }
    };
    this.loggerOptions = loggerOptions;
    this.log = getLogger((_a = loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.PCManager, () => this.logContext);
    this.iceLog = getLogger(LoggerNames.ICE, () => this.logContext);
    this.isPublisherConnectionRequired = mode !== 'subscriber-primary';
    this.isSubscriberConnectionRequired = mode === 'subscriber-primary';
    this.publisher = new PCTransport(rtcConfig, loggerOptions);
    this._mode = mode;
    if (mode !== 'publisher-only') {
      this.subscriber = new PCTransport(rtcConfig, loggerOptions);
      this.subscriber.onConnectionStateChange = this.updateState;
      this.subscriber.onIceConnectionStateChange = this.updateState;
      this.subscriber.onSignalingStatechange = this.updateState;
      this.subscriber.onIceCandidate = candidate => {
        var _a;
        (_a = this.onIceCandidate) === null || _a === void 0 ? void 0 : _a.call(this, candidate, SignalTarget.SUBSCRIBER);
      };
      // in subscriber primary mode, server side opens sub data channels.
      this.subscriber.onDataChannel = ev => {
        var _a;
        (_a = this.onDataChannel) === null || _a === void 0 ? void 0 : _a.call(this, ev);
      };
      this.subscriber.onTrack = ev => {
        var _a;
        (_a = this.onTrack) === null || _a === void 0 ? void 0 : _a.call(this, ev);
      };
    }
    this.publisher.onConnectionStateChange = this.updateState;
    this.publisher.onIceConnectionStateChange = this.updateState;
    this.publisher.onSignalingStatechange = this.updateState;
    this.publisher.onIceCandidate = candidate => {
      var _a;
      (_a = this.onIceCandidate) === null || _a === void 0 ? void 0 : _a.call(this, candidate, SignalTarget.PUBLISHER);
    };
    this.publisher.onTrack = ev => {
      var _a;
      (_a = this.onTrack) === null || _a === void 0 ? void 0 : _a.call(this, ev);
    };
    this.publisher.onOffer = (offer, offerId) => {
      var _a;
      (_a = this.onPublisherOffer) === null || _a === void 0 ? void 0 : _a.call(this, offer, offerId);
    };
    this.state = PCTransportState.NEW;
    this.connectionLock = new _();
    this.remoteOfferLock = new _();
  }
  get logContext() {
    var _a, _b;
    return Object.assign({}, (_b = (_a = this.loggerOptions).loggerContextCb) === null || _b === void 0 ? void 0 : _b.call(_a));
  }
  requirePublisher() {
    let require = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
    this.isPublisherConnectionRequired = require;
    this.updateState();
  }
  createAndSendPublisherOffer(options) {
    return this.publisher.createAndSendOffer(options);
  }
  setPublisherAnswer(sd, offerId) {
    return this.publisher.setRemoteDescription(sd, offerId);
  }
  removeTrack(sender) {
    return this.publisher.removeTrack(sender);
  }
  close() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (this.publisher && this.publisher.getSignallingState() !== 'closed') {
        const publisher = this.publisher;
        for (const sender of publisher.getSenders()) {
          try {
            // TODO: react-native-webrtc doesn't have removeTrack yet.
            if (publisher.canRemoveTrack()) {
              publisher.removeTrack(sender);
            }
          } catch (e) {
            this.log.warn('could not removeTrack', {
              error: e
            });
          }
        }
      }
      yield Promise.all([this.publisher.close(), (_a = this.subscriber) === null || _a === void 0 ? void 0 : _a.close()]);
      this.updateState();
    });
  }
  /**
   * Restarts ICE on the transports that need it. Only the publisher: the server restarts the
   * subscriber's ICE itself and follows with a fresh offer.
   *
   * The subscriber deliberately does NOT enter `restartingIce` here. Queueing its remote
   * candidates would guard against candidates for a new generation arriving before the offer
   * that introduces it, but the server does not send them in that order -- on a same-node
   * resume it buffers them until the offer has gone out, and on a reconnect that lands on
   * another node it withholds subscriber candidates until immediately before creating the
   * offer. Setting the flag only risks withholding candidates during the window that decides
   * whether the reconnect succeeded.
   */
  triggerIceRestart() {
    return __awaiter(this, void 0, void 0, function* () {
      this.iceLog.warn('triggering ICE restart');
      if (this.needsPublisher) {
        yield this.createAndSendPublisherOffer({
          iceRestart: true
        });
      }
    });
  }
  addIceCandidate(candidate, target) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      this.iceLog.debug('adding remote ICE candidate', {
        target,
        candidate
      });
      if (target === SignalTarget.PUBLISHER) {
        yield this.publisher.addIceCandidate(candidate);
      } else {
        yield (_a = this.subscriber) === null || _a === void 0 ? void 0 : _a.addIceCandidate(candidate);
      }
    });
  }
  createSubscriberAnswerFromOffer(sd, offerId) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      this.log.debug('received server offer', {
        RTCSdpType: sd.type,
        sdp: sd.sdp,
        signalingState: (_a = this.subscriber) === null || _a === void 0 ? void 0 : _a.getSignallingState().toString()
      });
      const unlock = yield this.remoteOfferLock.lock();
      try {
        const success = yield (_b = this.subscriber) === null || _b === void 0 ? void 0 : _b.setRemoteDescription(sd, offerId);
        if (!success) {
          return undefined;
        }
        // answer the offer
        const answer = yield (_c = this.subscriber) === null || _c === void 0 ? void 0 : _c.createAndSetAnswer();
        return answer;
      } finally {
        unlock();
      }
    });
  }
  updateConfiguration(config, iceRestart) {
    var _a;
    this.log.debug('updating rtc configuration', {
      iceRestart
    });
    this.publisher.setConfiguration(config);
    (_a = this.subscriber) === null || _a === void 0 ? void 0 : _a.setConfiguration(config);
    if (iceRestart) {
      this.triggerIceRestart();
    }
  }
  ensurePCTransportConnection(abortController, timeout) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const unlock = yield this.connectionLock.lock();
      try {
        if (this.isPublisherConnectionRequired && this.publisher.getConnectionState() !== 'connected' && this.publisher.getConnectionState() !== 'connecting') {
          this.log.debug('negotiation required, start negotiating');
          this.publisher.negotiate().catch(err => {
            this.log.error(err);
          });
        }
        yield Promise.all((_a = this.requiredTransports) === null || _a === void 0 ? void 0 : _a.map(transport => this.ensureTransportConnected(transport, abortController, timeout)));
      } finally {
        unlock();
      }
    });
  }
  negotiate(abortController) {
    return __awaiter(this, void 0, void 0, function* () {
      return new TypedPromise((resolve, reject) => {
        // Capture the publisher's latest offer id at request time. We are done
        // when an offer with a higher id has had its answer successfully
        // applied — that offer is the one that includes any transceiver/SDP
        // changes that motivated this negotiate call. Concurrent callers each
        // get their own checkpoint and resolve independently.
        const checkpoint = this.publisher.latestOfferId;
        // Race: an answer past our checkpoint already arrived before we had a
        // chance to subscribe.
        if (this.publisher.latestAcknowledgedOfferId > checkpoint) {
          this.log.debug("negotiation already handled in more recent acknowledged offer", this.logContext);
          resolve();
          return;
        }
        let cleanedUp = false;
        const cleanup = () => {
          if (cleanedUp) return;
          cleanedUp = true;
          clearTimeout(deadlineTimer);
          this.publisher.off(PCEvents.OfferAnswered, onAnswered);
          abortController.signal.removeEventListener('abort', onAbort);
        };
        const onAnswered = offerId => {
          if (offerId > checkpoint) {
            cleanup();
            resolve();
          }
        };
        const onAbort = () => {
          cleanup();
          reject(new NegotiationError('negotiation aborted'));
        };
        // Single hard deadline as a backstop. Not reset on cycle progress —
        // progress is tracked via OfferAnswered, not NegotiationStarted.
        const deadlineTimer = setTimeout(() => {
          cleanup();
          reject(new NegotiationError('negotiation timed out'));
        }, this.peerConnectionTimeout);
        abortController.signal.addEventListener('abort', onAbort);
        this.publisher.on(PCEvents.OfferAnswered, onAnswered);
        this.publisher.negotiate(e => {
          cleanup();
          if (e instanceof Error) {
            reject(e);
          } else {
            reject(new Error(String(e)));
          }
        });
      });
    });
  }
  addPublisherTransceiver(track, transceiverInit) {
    return this.publisher.addTransceiver(track, transceiverInit);
  }
  addPublisherTransceiverOfKind(kind, transceiverInit) {
    return this.publisher.addTransceiverOfKind(kind, transceiverInit);
  }
  getMidForReceiver(receiver) {
    const transceivers = this.subscriber ? this.subscriber.getTransceivers() : this.publisher.getTransceivers();
    const matchingTransceiver = transceivers.find(transceiver => transceiver.receiver === receiver);
    return matchingTransceiver === null || matchingTransceiver === void 0 ? void 0 : matchingTransceiver.mid;
  }
  getMaxPublisherMessageSize() {
    return this.publisher.getMaxMessageSize();
  }
  addPublisherTrack(track) {
    return this.publisher.addTrack(track);
  }
  createPublisherDataChannel(label, dataChannelDict) {
    return this.publisher.createDataChannel(label, dataChannelDict);
  }
  /**
   * Returns the first required transport's address if no explicit target is specified
   */
  getConnectedAddress(target) {
    if (target === SignalTarget.PUBLISHER) {
      return this.publisher.getConnectedAddress();
    } else if (target === SignalTarget.SUBSCRIBER) {
      return this.publisher.getConnectedAddress();
    }
    return this.requiredTransports[0].getConnectedAddress();
  }
  get requiredTransports() {
    const transports = [];
    if (this.isPublisherConnectionRequired) {
      transports.push(this.publisher);
    }
    if (this.isSubscriberConnectionRequired && this.subscriber) {
      transports.push(this.subscriber);
    }
    return transports;
  }
  ensureTransportConnected(pcTransport_1, abortController_1) {
    return __awaiter(this, arguments, void 0, function (pcTransport, abortController) {
      var _this = this;
      let timeout = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : this.peerConnectionTimeout;
      return function* () {
        const connectionState = pcTransport.getConnectionState();
        if (connectionState === 'connected') {
          return;
        }
        return new Promise((resolve, reject) => __awaiter(_this, void 0, void 0, function* () {
          const abortHandler = () => {
            this.log.warn('abort transport connection');
            CriticalTimers.clearTimeout(connectTimeout);
            reject(ConnectionError.cancelled('room connection has been cancelled'));
          };
          if (abortController === null || abortController === void 0 ? void 0 : abortController.signal.aborted) {
            abortHandler();
          }
          abortController === null || abortController === void 0 ? void 0 : abortController.signal.addEventListener('abort', abortHandler);
          const connectTimeout = CriticalTimers.setTimeout(() => {
            abortController === null || abortController === void 0 ? void 0 : abortController.signal.removeEventListener('abort', abortHandler);
            reject(ConnectionError.internal('could not establish pc connection'));
          }, timeout);
          while (this.state !== PCTransportState.CONNECTED) {
            yield sleep(50); // FIXME we shouldn't rely on `sleep` in the connection paths, as it invokes `setTimeout` which can be drastically throttled by browser implementations
            if (abortController === null || abortController === void 0 ? void 0 : abortController.signal.aborted) {
              reject(ConnectionError.cancelled('room connection has been cancelled'));
              return;
            }
          }
          CriticalTimers.clearTimeout(connectTimeout);
          abortController === null || abortController === void 0 ? void 0 : abortController.signal.removeEventListener('abort', abortHandler);
          resolve();
        }));
      }();
    });
  }
}
export { PCTransport, PCTransportManager, PCTransportState, audioDefaults, conformBundledCodecFmtp, ddExtensionIDFor, defaultVideoCodec, ensureAudioNackAndStereo, ensureIPAddrMatchVersion, ensureVideoDDExtension, extractStereoAndNackAudioFromOffer, fmtpConfigHasParam, getMidString, isDuplicateAnswer, mappedExtensionID, placeholderMidsFromTransceivers, publishDefaults, roomConnectOptionDefaults, roomOptionDefaults, unusedExtensionID, usedForOtherURI, videoDefaults };
