// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 31524-31904 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { AudioTrackFeature } from './protobuf-messages-enums.mjs';
import { Encryption_Type } from './protobuf-messages-room.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { DeviceUnsupportedError, TrackInvalidError } from './errors.mjs';
import { TrackEvent } from './event-enums.mjs';
import { ScreenSharePresets, constraintsForOptions, extractProcessorsFromOptions, getLogContextFromTrack, mergeDefaultOptions, screenCaptureToDisplayMediaStreamOptions } from './util.mjs';
import { Track, isSafari17Based } from './track.mjs';
import { isAudioTrack, isVideoTrack, unwrapConstraint } from './utils-misc.mjs';
import { audioDefaults, videoDefaults } from './pc-transport.mjs';
import { LocalAudioTrack } from './local-track.mjs';
import { mediaTrackToLocalTrack } from './local-track-factory.mjs';
import { LocalVideoTrack } from './local-video-track.mjs';

class TrackPublication extends eventsExports.EventEmitter {
  constructor(kind, id, name, loggerOptions) {
    var _a;
    super();
    this.metadataMuted = false;
    this.encryption = Encryption_Type.NONE;
    this.log = livekitLogger;
    this.handleMuted = () => {
      this.emit(TrackEvent.Muted);
    };
    this.handleUnmuted = () => {
      this.emit(TrackEvent.Unmuted);
    };
    this.log = getLogger((_a = loggerOptions === null || loggerOptions === void 0 ? void 0 : loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Publication);
    this.loggerContextCb = this.loggerContextCb;
    this.setMaxListeners(100);
    this.kind = kind;
    this.trackSid = id;
    this.trackName = name;
    this.source = Track.Source.Unknown;
  }
  /** @internal */
  setTrack(track) {
    if (this.track) {
      this.track.off(TrackEvent.Muted, this.handleMuted);
      this.track.off(TrackEvent.Unmuted, this.handleUnmuted);
    }
    this.track = track;
    if (track) {
      // forward events
      track.on(TrackEvent.Muted, this.handleMuted);
      track.on(TrackEvent.Unmuted, this.handleUnmuted);
    }
  }
  get logContext() {
    var _a;
    return Object.assign(Object.assign({}, (_a = this.loggerContextCb) === null || _a === void 0 ? void 0 : _a.call(this)), getLogContextFromTrack(this));
  }
  get isMuted() {
    return this.metadataMuted;
  }
  get isEnabled() {
    return true;
  }
  get isSubscribed() {
    return this.track !== undefined;
  }
  get isEncrypted() {
    return this.encryption !== Encryption_Type.NONE;
  }
  /**
   * an [AudioTrack] if this publication holds an audio track
   */
  get audioTrack() {
    if (isAudioTrack(this.track)) {
      return this.track;
    }
  }
  /**
   * an [VideoTrack] if this publication holds a video track
   */
  get videoTrack() {
    if (isVideoTrack(this.track)) {
      return this.track;
    }
  }
  /** @internal */
  updateInfo(info) {
    this.trackSid = info.sid;
    this.trackName = info.name;
    this.source = Track.sourceFromProto(info.source);
    this.mimeType = info.mimeType;
    if (this.kind === Track.Kind.Video && info.width > 0) {
      this.dimensions = {
        width: info.width,
        height: info.height
      };
      this.simulcasted = info.simulcast;
    }
    this.encryption = info.encryption;
    this.trackInfo = info;
    this.log.debug('update publication info', Object.assign(Object.assign({}, this.logContext), {
      info
    }));
  }
}
(function (TrackPublication) {
  (function (SubscriptionStatus) {
    SubscriptionStatus["Desired"] = "desired";
    SubscriptionStatus["Subscribed"] = "subscribed";
    SubscriptionStatus["Unsubscribed"] = "unsubscribed";
  })(TrackPublication.SubscriptionStatus || (TrackPublication.SubscriptionStatus = {}));
  (function (PermissionStatus) {
    PermissionStatus["Allowed"] = "allowed";
    PermissionStatus["NotAllowed"] = "not_allowed";
  })(TrackPublication.PermissionStatus || (TrackPublication.PermissionStatus = {}));
})(TrackPublication || (TrackPublication = {}));class LocalTrackPublication extends TrackPublication {
  get isUpstreamPaused() {
    var _a;
    return (_a = this.track) === null || _a === void 0 ? void 0 : _a.isUpstreamPaused;
  }
  constructor(kind, ti, track, loggerOptions) {
    super(kind, ti.sid, ti.name, loggerOptions);
    this.track = undefined;
    this.handleTrackEnded = () => {
      this.emit(TrackEvent.Ended);
    };
    this.handleCpuConstrained = () => {
      if (this.track && isVideoTrack(this.track)) {
        this.emit(TrackEvent.CpuConstrained, this.track);
      }
    };
    this.updateInfo(ti);
    this.setTrack(track);
  }
  setTrack(track) {
    if (this.track) {
      this.track.off(TrackEvent.Ended, this.handleTrackEnded);
      this.track.off(TrackEvent.CpuConstrained, this.handleCpuConstrained);
    }
    super.setTrack(track);
    if (track) {
      track.on(TrackEvent.Ended, this.handleTrackEnded);
      track.on(TrackEvent.CpuConstrained, this.handleCpuConstrained);
    }
  }
  get isMuted() {
    if (this.track) {
      return this.track.isMuted;
    }
    return super.isMuted;
  }
  get audioTrack() {
    return super.audioTrack;
  }
  get videoTrack() {
    return super.videoTrack;
  }
  get isLocal() {
    return true;
  }
  /**
   * Mute the track associated with this publication
   */
  mute() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      return (_a = this.track) === null || _a === void 0 ? void 0 : _a.mute();
    });
  }
  /**
   * Unmute track associated with this publication
   */
  unmute() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      return (_a = this.track) === null || _a === void 0 ? void 0 : _a.unmute();
    });
  }
  /**
   * Pauses the media stream track associated with this publication from being sent to the server
   * and signals "muted" event to other participants
   * Useful if you want to pause the stream without pausing the local media stream track
   */
  pauseUpstream() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      yield (_a = this.track) === null || _a === void 0 ? void 0 : _a.pauseUpstream();
    });
  }
  /**
   * Resumes sending the media stream track associated with this publication to the server after a call to [[pauseUpstream()]]
   * and signals "unmuted" event to other participants (unless the track is explicitly muted)
   */
  resumeUpstream() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      yield (_a = this.track) === null || _a === void 0 ? void 0 : _a.resumeUpstream();
    });
  }
  getTrackFeatures() {
    var _a;
    if (isAudioTrack(this.track)) {
      const settings = this.track.getSourceTrackSettings();
      const features = new Set();
      if (settings.autoGainControl) {
        features.add(AudioTrackFeature.TF_AUTO_GAIN_CONTROL);
      }
      if (settings.echoCancellation) {
        features.add(AudioTrackFeature.TF_ECHO_CANCELLATION);
      }
      if (settings.noiseSuppression) {
        features.add(AudioTrackFeature.TF_NOISE_SUPPRESSION);
      }
      if (settings.channelCount && settings.channelCount > 1) {
        features.add(AudioTrackFeature.TF_STEREO);
      }
      if (!((_a = this.options) === null || _a === void 0 ? void 0 : _a.dtx)) {
        features.add(AudioTrackFeature.TF_NO_DTX);
      }
      if (this.track.enhancedNoiseCancellation) {
        features.add(AudioTrackFeature.TF_ENHANCED_NOISE_CANCELLATION);
      }
      return Array.from(features.values());
    } else return [];
  }
}/**
 * Creates a local video and audio track at the same time. When acquiring both
 * audio and video tracks together, it'll display a single permission prompt to
 * the user instead of two separate ones.
 * @param options
 */
function createLocalTracks(options, loggerOptions) {
  return __awaiter(this, void 0, void 0, function* () {
    options !== null && options !== void 0 ? options : options = {};
    let attemptExactMatch = false;
    const _extractProcessorsFro = extractProcessorsFromOptions(options),
      audioProcessor = _extractProcessorsFro.audioProcessor,
      videoProcessor = _extractProcessorsFro.videoProcessor,
      internalOptions = _extractProcessorsFro.optionsWithoutProcessor;
    let retryAudioOptions = internalOptions.audio;
    let retryVideoOptions = internalOptions.video;
    if (audioProcessor && typeof internalOptions.audio === 'object') {
      internalOptions.audio.processor = audioProcessor;
    }
    if (videoProcessor && typeof internalOptions.video === 'object') {
      internalOptions.video.processor = videoProcessor;
    }
    // if the user passes a device id as a string, we default to exact match
    if (options.audio && typeof internalOptions.audio === 'object' && typeof internalOptions.audio.deviceId === 'string') {
      const deviceId = internalOptions.audio.deviceId;
      internalOptions.audio.deviceId = {
        exact: deviceId
      };
      attemptExactMatch = true;
      retryAudioOptions = Object.assign(Object.assign({}, internalOptions.audio), {
        deviceId: {
          ideal: deviceId
        }
      });
    }
    if (internalOptions.video && typeof internalOptions.video === 'object' && typeof internalOptions.video.deviceId === 'string') {
      const deviceId = internalOptions.video.deviceId;
      internalOptions.video.deviceId = {
        exact: deviceId
      };
      attemptExactMatch = true;
      retryVideoOptions = Object.assign(Object.assign({}, internalOptions.video), {
        deviceId: {
          ideal: deviceId
        }
      });
    }
    if (internalOptions.audio === true) {
      internalOptions.audio = {
        deviceId: 'default'
      };
    } else if (typeof internalOptions.audio === 'object' && internalOptions.audio !== null) {
      internalOptions.audio = Object.assign(Object.assign({}, internalOptions.audio), {
        deviceId: internalOptions.audio.deviceId || 'default'
      });
    }
    if (internalOptions.video === true) {
      internalOptions.video = {
        deviceId: 'default'
      };
    } else if (typeof internalOptions.video === 'object' && !internalOptions.video.deviceId) {
      internalOptions.video.deviceId = 'default';
    }
    const opts = mergeDefaultOptions(internalOptions, audioDefaults, videoDefaults);
    const constraints = constraintsForOptions(opts);
    // Keep a reference to the promise on DeviceManager and await it in getLocalDevices()
    // works around iOS Safari Bug https://bugs.webkit.org/show_bug.cgi?id=179363
    const mediaPromise = navigator.mediaDevices.getUserMedia(constraints);
    if (internalOptions.audio) {
      DeviceManager.userMediaPromiseMap.set('audioinput', mediaPromise);
      mediaPromise.catch(() => DeviceManager.userMediaPromiseMap.delete('audioinput'));
    }
    if (internalOptions.video) {
      DeviceManager.userMediaPromiseMap.set('videoinput', mediaPromise);
      mediaPromise.catch(() => DeviceManager.userMediaPromiseMap.delete('videoinput'));
    }
    try {
      const stream = yield mediaPromise;
      return yield Promise.all(stream.getTracks().map(mediaStreamTrack => __awaiter(this, void 0, void 0, function* () {
        const isAudio = mediaStreamTrack.kind === 'audio';
        let trackConstraints;
        const conOrBool = isAudio ? constraints.audio : constraints.video;
        if (typeof conOrBool !== 'boolean') {
          trackConstraints = conOrBool;
        }
        // update the constraints with the device id the user gave permissions to in the permission prompt
        // otherwise each track restart (e.g. mute - unmute) will try to initialize the device again -> causing additional permission prompts
        const newDeviceId = mediaStreamTrack.getSettings().deviceId;
        if ((trackConstraints === null || trackConstraints === void 0 ? void 0 : trackConstraints.deviceId) && unwrapConstraint(trackConstraints.deviceId) !== newDeviceId) {
          trackConstraints.deviceId = newDeviceId;
        } else if (!trackConstraints) {
          trackConstraints = {
            deviceId: newDeviceId
          };
        }
        const track = mediaTrackToLocalTrack(mediaStreamTrack, trackConstraints, loggerOptions);
        if (track.kind === Track.Kind.Video) {
          track.source = Track.Source.Camera;
        } else if (track.kind === Track.Kind.Audio) {
          track.source = Track.Source.Microphone;
        }
        track.mediaStream = stream;
        if (isAudioTrack(track) && audioProcessor) {
          yield track.setProcessor(audioProcessor);
        } else if (isVideoTrack(track) && videoProcessor) {
          yield track.setProcessor(videoProcessor);
        }
        return track;
      })));
    } catch (e) {
      if (!attemptExactMatch) {
        throw e;
      }
      return createLocalTracks(Object.assign(Object.assign({}, options), {
        audio: retryAudioOptions,
        video: retryVideoOptions
      }), loggerOptions);
    }
  });
}
/**
 * Creates a [[LocalVideoTrack]] with getUserMedia()
 * @param options
 */
function createLocalVideoTrack(options) {
  return __awaiter(this, void 0, void 0, function* () {
    const tracks = yield createLocalTracks({
      audio: false,
      video: options !== null && options !== void 0 ? options : true
    });
    return tracks[0];
  });
}
function createLocalAudioTrack(options) {
  return __awaiter(this, void 0, void 0, function* () {
    const tracks = yield createLocalTracks({
      audio: options !== null && options !== void 0 ? options : true,
      video: false
    });
    return tracks[0];
  });
}
/**
 * Creates a screen capture tracks with getDisplayMedia().
 * A LocalVideoTrack is always created and returned.
 * If { audio: true }, and the browser supports audio capture, a LocalAudioTrack is also created.
 */
function createLocalScreenTracks(options) {
  return __awaiter(this, void 0, void 0, function* () {
    if (options === undefined) {
      options = {};
    }
    if (options.resolution === undefined && !isSafari17Based()) {
      options.resolution = ScreenSharePresets.h1080fps30.resolution;
    }
    if (navigator.mediaDevices.getDisplayMedia === undefined) {
      throw new DeviceUnsupportedError('getDisplayMedia not supported');
    }
    const constraints = screenCaptureToDisplayMediaStreamOptions(options);
    const stream = yield navigator.mediaDevices.getDisplayMedia(constraints);
    const tracks = stream.getVideoTracks();
    if (tracks.length === 0) {
      throw new TrackInvalidError('no video track found');
    }
    const screenVideo = new LocalVideoTrack(tracks[0], undefined, false);
    screenVideo.source = Track.Source.ScreenShare;
    const localTracks = [screenVideo];
    if (stream.getAudioTracks().length > 0) {
      const screenAudio = new LocalAudioTrack(stream.getAudioTracks()[0], undefined, false);
      screenAudio.source = Track.Source.ScreenShareAudio;
      localTracks.push(screenAudio);
    }
    return localTracks;
  });
}
export { LocalTrackPublication, TrackPublication, createLocalAudioTrack, createLocalScreenTracks, createLocalTracks, createLocalVideoTrack };
