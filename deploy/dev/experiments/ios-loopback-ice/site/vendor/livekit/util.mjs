// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 13195-13908 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { TrackSource } from './protobuf-messages-enums.mjs';
import { TrackPublishedResponse } from './protobuf-messages-signal.mjs';
import { compact } from './event-enums.mjs';
import { Track, isSafari, sleep } from './track.mjs';


function resolution(width, height) {
  return width && height ? "".concat(width, "x").concat(height) : undefined;
}
/** keeps the derived durations readable; reported values are logged as they are */
function round(seconds) {
  return Math.round(seconds * 10000) / 10000;
}
/**
 * Average time media spent in the jitter buffer, in s. Both counters are
 * cumulative, so this is the average over the lifetime of the stream.
 */
function jitterBuffer(stat) {
  const delay = stat.jitterBufferDelay;
  const emitted = stat.jitterBufferEmittedCount;
  return delay !== undefined && emitted ? round(delay / emitted) : undefined;
}
/**
 * Playout delay in s. Video receive reports it directly, the audio path keeps it
 * in the linked `media-playout` stats summed over the samples played out. Pairs
 * with `RemoteTrack.setPlayoutDelay`.
 */
function playoutDelay(stat, playout) {
  if (stat.playoutDelay !== undefined) {
    return round(stat.playoutDelay);
  }
  const total = playout === null || playout === void 0 ? void 0 : playout.totalPlayoutDelay;
  const samples = playout === null || playout === void 0 ? void 0 : playout.totalSamplesCount;
  return total !== undefined && samples ? round(total / samples) : undefined;
}
/**
 * Picks the interesting fields out of a `getStats()` report and groups them by
 * RTP stream, so a stats dump can be read without unfolding the raw report.
 */
function summarizeStatsReport(report) {
  var _a, _b;
  const byId = new Map();
  const candidatePairs = [];
  const inbound = [];
  const outbound = [];
  let transport;
  report.forEach(stat => byId.set(stat.id, stat));
  const codecOf = stat => {
    var _a;
    return stat.codecId ? (_a = byId.get(stat.codecId)) === null || _a === void 0 ? void 0 : _a.mimeType : undefined;
  };
  const relatedOf = (stat, key) => stat[key] ? byId.get(stat[key]) : undefined;
  report.forEach(stat => {
    switch (stat.type) {
      case 'inbound-rtp':
        {
          const playout = relatedOf(stat, 'playoutId');
          inbound.push(compact({
            kind: stat.kind,
            ssrc: stat.ssrc,
            mid: stat.mid,
            // matches `streamTrackID` in the track's own log context
            trackId: stat.trackIdentifier,
            codec: codecOf(stat),
            decoder: stat.decoderImplementation,
            resolution: resolution(stat.frameWidth, stat.frameHeight),
            fps: stat.framesPerSecond,
            bytesReceived: stat.bytesReceived,
            packetsReceived: stat.packetsReceived,
            packetsLost: stat.packetsLost,
            packetsDiscarded: stat.packetsDiscarded,
            // frames received without frames decoded is a decode failure
            framesReceived: stat.framesReceived,
            framesDecoded: stat.framesDecoded,
            framesDropped: stat.framesDropped,
            keyFramesDecoded: stat.keyFramesDecoded,
            freezeCount: stat.freezeCount,
            totalFreezesDuration: stat.totalFreezesDuration,
            pauseCount: stat.pauseCount,
            nackCount: stat.nackCount,
            pliCount: stat.pliCount,
            firCount: stat.firCount,
            jitter: stat.jitter,
            jitterBuffer: jitterBuffer(stat),
            playoutDelay: playoutDelay(stat, playout),
            audioLevel: stat.audioLevel,
            totalSamplesReceived: stat.totalSamplesReceived,
            concealedSamples: stat.concealedSamples
          }));
          break;
        }
      case 'outbound-rtp':
        {
          const remote = relatedOf(stat, 'remoteId');
          const source = relatedOf(stat, 'mediaSourceId');
          outbound.push(compact({
            kind: stat.kind,
            ssrc: stat.ssrc,
            mid: stat.mid,
            rid: stat.rid,
            trackId: source === null || source === void 0 ? void 0 : source.trackIdentifier,
            active: stat.active,
            codec: codecOf(stat),
            encoder: stat.encoderImplementation,
            resolution: resolution(stat.frameWidth, stat.frameHeight),
            fps: stat.framesPerSecond,
            // what the source produces, to tell a stalled capture from a stalled encoder
            captureResolution: resolution(source === null || source === void 0 ? void 0 : source.width, source === null || source === void 0 ? void 0 : source.height),
            captureFps: source === null || source === void 0 ? void 0 : source.framesPerSecond,
            audioLevel: source === null || source === void 0 ? void 0 : source.audioLevel,
            targetBitrate: stat.targetBitrate,
            bytesSent: stat.bytesSent,
            packetsSent: stat.packetsSent,
            retransmittedPacketsSent: stat.retransmittedPacketsSent,
            framesEncoded: stat.framesEncoded,
            keyFramesEncoded: stat.keyFramesEncoded,
            limitedBy: stat.qualityLimitationReason === 'none' ? undefined : stat.qualityLimitationReason,
            nackCount: stat.nackCount,
            pliCount: stat.pliCount,
            firCount: stat.firCount,
            // loss, jitter and RTT are only known from what the remote reports
            remotePacketsLost: remote === null || remote === void 0 ? void 0 : remote.packetsLost,
            remoteFractionLost: remote === null || remote === void 0 ? void 0 : remote.fractionLost,
            remoteJitter: remote === null || remote === void 0 ? void 0 : remote.jitter,
            remoteRoundTripTime: remote === null || remote === void 0 ? void 0 : remote.roundTripTime
          }));
          break;
        }
      case 'transport':
        transport = stat;
        break;
      case 'candidate-pair':
        candidatePairs.push(stat);
        break;
    }
  });
  const selectedPairId = transport === null || transport === void 0 ? void 0 : transport.selectedCandidatePairId;
  const pair = (_b = (_a = selectedPairId ? byId.get(selectedPairId) : undefined) !== null && _a !== void 0 ? _a : candidatePairs.find(candidate => candidate.selected)) !== null && _b !== void 0 ? _b : candidatePairs.find(candidate => candidate.nominated);
  const local = (pair === null || pair === void 0 ? void 0 : pair.localCandidateId) ? byId.get(pair.localCandidateId) : undefined;
  const remote = (pair === null || pair === void 0 ? void 0 : pair.remoteCandidateId) ? byId.get(pair.remoteCandidateId) : undefined;
  const connection = compact({
    ice: transport === null || transport === void 0 ? void 0 : transport.iceState,
    dtls: transport === null || transport === void 0 ? void 0 : transport.dtlsState,
    route: local && remote ? "".concat(local.candidateType, "/").concat(local.protocol, " -> ").concat(remote.candidateType) : undefined,
    network: local === null || local === void 0 ? void 0 : local.networkType,
    currentRoundTripTime: pair === null || pair === void 0 ? void 0 : pair.currentRoundTripTime,
    // the send bandwidth estimate; no RTP stream reports it
    availableOutgoingBitrate: pair === null || pair === void 0 ? void 0 : pair.availableOutgoingBitrate,
    availableIncomingBitrate: pair === null || pair === void 0 ? void 0 : pair.availableIncomingBitrate,
    bytesSent: pair === null || pair === void 0 ? void 0 : pair.bytesSent,
    bytesReceived: pair === null || pair === void 0 ? void 0 : pair.bytesReceived,
    candidatePairChanges: transport === null || transport === void 0 ? void 0 : transport.selectedCandidatePairChanges
  });
  return {
    connection: Object.keys(connection).length > 0 ? connection : undefined,
    outbound: outbound.length > 0 ? outbound : undefined,
    inbound: inbound.length > 0 ? inbound : undefined
  };
}function cloneDeep$1(value) {
  if (typeof value === 'undefined') {
    return value;
  }
  if (typeof structuredClone === 'function') {
    if (typeof value === 'object' && value !== null) {
      // ensure that the value is not a proxy by spreading it
      return structuredClone(Object.assign({}, value));
    }
    return structuredClone(value);
  } else {
    return JSON.parse(JSON.stringify(value));
  }
}class VideoPreset {
  constructor(widthOrOptions, height, maxBitrate, maxFramerate, priority) {
    if (typeof widthOrOptions === 'object') {
      this.width = widthOrOptions.width;
      this.height = widthOrOptions.height;
      this.aspectRatio = widthOrOptions.aspectRatio;
      this.encoding = {
        maxBitrate: widthOrOptions.maxBitrate,
        maxFramerate: widthOrOptions.maxFramerate,
        priority: widthOrOptions.priority
      };
    } else if (height !== undefined && maxBitrate !== undefined) {
      this.width = widthOrOptions;
      this.height = height;
      this.aspectRatio = widthOrOptions / height;
      this.encoding = {
        maxBitrate,
        maxFramerate,
        priority
      };
    } else {
      throw new TypeError('Unsupported options: provide at least width, height and maxBitrate');
    }
  }
  get resolution() {
    return {
      width: this.width,
      height: this.height,
      frameRate: this.encoding.maxFramerate,
      aspectRatio: this.aspectRatio
    };
  }
}
// `red` is not technically a codec, but treated as one in signalling protocol
const audioCodecs = ['opus', 'red'];
const backupVideoCodecs = ['vp8', 'h264'];
const videoCodecs = ['vp8', 'h264', 'vp9', 'av1', 'h265'];
function isBackupVideoCodec(codec) {
  return !!backupVideoCodecs.find(backup => backup === codec);
}
/** @deprecated Use {@link isBackupVideoCodec} instead */
const isBackupCodec = isBackupVideoCodec;
var BackupCodecPolicy;
(function (BackupCodecPolicy) {
  // codec regression is preferred, the sfu will try to regress codec if possible but not guaranteed
  BackupCodecPolicy[BackupCodecPolicy["PREFER_REGRESSION"] = 0] = "PREFER_REGRESSION";
  // multi-codec simulcast, publish both primary and backup codec at the same time
  BackupCodecPolicy[BackupCodecPolicy["SIMULCAST"] = 1] = "SIMULCAST";
  // always use backup codec only
  BackupCodecPolicy[BackupCodecPolicy["REGRESSION"] = 2] = "REGRESSION";
})(BackupCodecPolicy || (BackupCodecPolicy = {}));
var AudioPresets;
(function (AudioPresets) {
  AudioPresets.telephone = {
    maxBitrate: 12000
  };
  AudioPresets.speech = {
    maxBitrate: 24000
  };
  AudioPresets.music = {
    maxBitrate: 48000
  };
  AudioPresets.musicStereo = {
    maxBitrate: 64000
  };
  AudioPresets.musicHighQuality = {
    maxBitrate: 96000
  };
  AudioPresets.musicHighQualityStereo = {
    maxBitrate: 128000
  };
})(AudioPresets || (AudioPresets = {}));
/**
 * Sane presets for video resolution/encoding
 */
const VideoPresets = {
  h90: new VideoPreset(160, 90, 90000, 20),
  h180: new VideoPreset(320, 180, 160000, 20),
  h216: new VideoPreset(384, 216, 180000, 20),
  h360: new VideoPreset(640, 360, 450000, 20),
  h540: new VideoPreset(960, 540, 800000, 25),
  h720: new VideoPreset(1280, 720, 1700000, 30),
  h1080: new VideoPreset(1920, 1080, 3000000, 30),
  h1440: new VideoPreset(2560, 1440, 5000000, 30),
  h2160: new VideoPreset(3840, 2160, 8000000, 30)
};
/**
 * Four by three presets
 */
const VideoPresets43 = {
  h120: new VideoPreset(160, 120, 70000, 20),
  h180: new VideoPreset(240, 180, 125000, 20),
  h240: new VideoPreset(320, 240, 140000, 20),
  h360: new VideoPreset(480, 360, 330000, 20),
  h480: new VideoPreset(640, 480, 500000, 20),
  h540: new VideoPreset(720, 540, 600000, 25),
  h720: new VideoPreset(960, 720, 1300000, 30),
  h1080: new VideoPreset(1440, 1080, 2300000, 30),
  h1440: new VideoPreset(1920, 1440, 3800000, 30)
};
const ScreenSharePresets = {
  h360fps3: new VideoPreset(640, 360, 200000, 3, 'medium'),
  h360fps15: new VideoPreset(640, 360, 400000, 15, 'medium'),
  h720fps5: new VideoPreset(1280, 720, 800000, 5, 'medium'),
  h720fps15: new VideoPreset(1280, 720, 1500000, 15, 'medium'),
  h720fps30: new VideoPreset(1280, 720, 2000000, 30, 'medium'),
  h1080fps15: new VideoPreset(1920, 1080, 2500000, 15, 'medium'),
  h1080fps30: new VideoPreset(1920, 1080, 5000000, 30, 'medium'),
  // original resolution, without resizing
  original: new VideoPreset(0, 0, 7000000, 30, 'medium')
};function mergeDefaultOptions(options, audioDefaults, videoDefaults) {
  var _a, _b;
  var _c, _d;
  const _extractProcessorsFro = extractProcessorsFromOptions(options !== null && options !== void 0 ? options : {}),
    optionsWithoutProcessor = _extractProcessorsFro.optionsWithoutProcessor,
    audioProcessor = _extractProcessorsFro.audioProcessor,
    videoProcessor = _extractProcessorsFro.videoProcessor;
  const defaultAudioProcessor = audioDefaults === null || audioDefaults === void 0 ? void 0 : audioDefaults.processor;
  const defaultVideoProcessor = videoDefaults === null || videoDefaults === void 0 ? void 0 : videoDefaults.processor;
  const clonedOptions = optionsWithoutProcessor !== null && optionsWithoutProcessor !== void 0 ? optionsWithoutProcessor : {};
  if (clonedOptions.audio === true) clonedOptions.audio = {};
  if (clonedOptions.video === true) clonedOptions.video = {};
  // use defaults
  if (clonedOptions.audio) {
    mergeObjectWithoutOverwriting(clonedOptions.audio, audioDefaults);
    (_a = (_c = clonedOptions.audio).deviceId) !== null && _a !== void 0 ? _a : _c.deviceId = {
      ideal: 'default'
    };
    if (audioProcessor || defaultAudioProcessor) {
      clonedOptions.audio.processor = audioProcessor !== null && audioProcessor !== void 0 ? audioProcessor : defaultAudioProcessor;
    }
  }
  if (clonedOptions.video) {
    mergeObjectWithoutOverwriting(clonedOptions.video, videoDefaults);
    (_b = (_d = clonedOptions.video).deviceId) !== null && _b !== void 0 ? _b : _d.deviceId = {
      ideal: 'default'
    };
    if (videoProcessor || defaultVideoProcessor) {
      clonedOptions.video.processor = videoProcessor !== null && videoProcessor !== void 0 ? videoProcessor : defaultVideoProcessor;
    }
  }
  return clonedOptions;
}
function mergeObjectWithoutOverwriting(mainObject, objectToMerge) {
  Object.keys(objectToMerge).forEach(key => {
    if (mainObject[key] === undefined) mainObject[key] = objectToMerge[key];
  });
  return mainObject;
}
function constraintsForOptions(options) {
  var _a, _b;
  var _c, _d;
  const constraints = {};
  if (options.video) {
    // default video options
    if (typeof options.video === 'object') {
      const videoOptions = {};
      const target = videoOptions;
      const source = options.video;
      Object.keys(source).forEach(key => {
        switch (key) {
          case 'resolution':
            // flatten VideoResolution fields
            mergeObjectWithoutOverwriting(target, source.resolution);
            break;
          default:
            target[key] = source[key];
        }
      });
      constraints.video = videoOptions;
      (_a = (_c = constraints.video).deviceId) !== null && _a !== void 0 ? _a : _c.deviceId = {
        ideal: 'default'
      };
    } else {
      constraints.video = options.video ? {
        deviceId: {
          ideal: 'default'
        }
      } : false;
    }
  } else {
    constraints.video = false;
  }
  if (options.audio) {
    if (typeof options.audio === 'object') {
      constraints.audio = options.audio;
      (_b = (_d = constraints.audio).deviceId) !== null && _b !== void 0 ? _b : _d.deviceId = {
        ideal: 'default'
      };
    } else {
      constraints.audio = {
        deviceId: {
          ideal: 'default'
        }
      };
    }
  } else {
    constraints.audio = false;
  }
  return constraints;
}
/**
 * This function detects silence on a given [[Track]] instance.
 * Returns true if the track seems to be entirely silent.
 */
function detectSilence(track_1) {
  return __awaiter(this, arguments, void 0, function (track) {
    let timeOffset = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 200;
    return function* () {
      const ctx = getNewAudioContext();
      if (!ctx) {
        return false;
      }
      let source;
      let analyser;
      try {
        analyser = ctx.createAnalyser();
        analyser.fftSize = 2048;
        const bufferLength = analyser.frequencyBinCount;
        const dataArray = new Uint8Array(bufferLength);
        source = ctx.createMediaStreamSource(new MediaStream([track.mediaStreamTrack]));
        source.connect(analyser);
        // read through a function so that `state` isn't narrowed to its value from before `resume`
        const isRunning = () => ctx.state === 'running';
        if (!isRunning()) {
          try {
            // `resume` can stay pending indefinitely on a blocked page rather than rejecting, which
            // would keep the context and its nodes alive past the `finally` below
            yield Promise.race([ctx.resume(), sleep(200)]);
          } catch (_a) {
            // autoplay policies can keep us from resuming the context
          }
          if (!isRunning()) {
            // a context that isn't running doesn't process any audio, so the analyser would only ever
            // read silence. Report the track as not silent rather than raising a false positive.
            return false;
          }
        }
        yield sleep(timeOffset);
        analyser.getByteTimeDomainData(dataArray);
        const someNoise = dataArray.some(sample => sample !== 128 && sample !== 0);
        return !someNoise;
      } finally {
        source === null || source === void 0 ? void 0 : source.disconnect();
        analyser === null || analyser === void 0 ? void 0 : analyser.disconnect();
        // don't let a failing teardown mask the detection result
        yield ctx.close().catch(() => {});
      }
    }();
  });
}
/**
 * @internal
 */
function getNewAudioContext() {
  var _a;
  const AudioContext =
  // @ts-ignore
  typeof window !== 'undefined' && (window.AudioContext || window.webkitAudioContext);
  if (AudioContext) {
    const audioContext = new AudioContext({
      latencyHint: 'interactive'
    });
    // If the audio context is suspended, we need to resume it when the user clicks on the page
    if (audioContext.state === 'suspended' && typeof window !== 'undefined' && ((_a = window.document) === null || _a === void 0 ? void 0 : _a.body)) {
      const handleResume = () => __awaiter(this, void 0, void 0, function* () {
        var _a;
        try {
          if (audioContext.state === 'suspended') {
            yield audioContext.resume();
          }
        } catch (e) {
          console.warn('Error trying to auto-resume audio context', e);
        } finally {
          (_a = window.document.body) === null || _a === void 0 ? void 0 : _a.removeEventListener('click', handleResume);
        }
      });
      // https://developer.mozilla.org/en-US/docs/Web/API/BaseAudioContext/statechange_event
      audioContext.addEventListener('statechange', () => {
        var _a;
        if (audioContext.state === 'closed') {
          (_a = window.document.body) === null || _a === void 0 ? void 0 : _a.removeEventListener('click', handleResume);
        }
      });
      window.document.body.addEventListener('click', handleResume);
    }
    return audioContext;
  }
}
/**
 * @internal
 */
function kindToSource(kind) {
  if (kind === 'audioinput') {
    return Track.Source.Microphone;
  } else if (kind === 'videoinput') {
    return Track.Source.Camera;
  } else {
    return Track.Source.Unknown;
  }
}
/**
 * @internal
 */
function sourceToKind(source) {
  if (source === Track.Source.Microphone) {
    return 'audioinput';
  } else if (source === Track.Source.Camera) {
    return 'videoinput';
  } else {
    return undefined;
  }
}
/**
 * @internal
 */
function screenCaptureToDisplayMediaStreamOptions(options) {
  var _a, _b;
  let videoConstraints = (_a = options.video) !== null && _a !== void 0 ? _a : true;
  // treat 0 as uncapped
  if (options.resolution && options.resolution.width > 0 && options.resolution.height > 0) {
    videoConstraints = typeof videoConstraints === 'boolean' ? {} : videoConstraints;
    if (isSafari()) {
      videoConstraints = Object.assign(Object.assign({}, videoConstraints), {
        width: {
          max: options.resolution.width
        },
        height: {
          max: options.resolution.height
        },
        frameRate: options.resolution.frameRate
      });
    } else {
      videoConstraints = Object.assign(Object.assign({}, videoConstraints), {
        width: {
          ideal: options.resolution.width
        },
        height: {
          ideal: options.resolution.height
        },
        frameRate: options.resolution.frameRate
      });
    }
  }
  return {
    audio: (_b = options.audio) !== null && _b !== void 0 ? _b : false,
    video: videoConstraints,
    // @ts-expect-error support for experimental display media features
    controller: options.controller,
    selfBrowserSurface: options.selfBrowserSurface,
    surfaceSwitching: options.surfaceSwitching,
    systemAudio: options.systemAudio,
    preferCurrentTab: options.preferCurrentTab
  };
}
function mimeTypeToVideoCodecString(mimeType) {
  return mimeType.split('/')[1].toLowerCase();
}
function getTrackPublicationInfo(tracks) {
  const infos = [];
  tracks.forEach(track => {
    if (track.track !== undefined) {
      infos.push(new TrackPublishedResponse({
        cid: track.track.mediaStreamID,
        track: track.trackInfo
      }));
    }
  });
  return infos;
}
function getLogContextFromTrack(track) {
  if ('mediaStreamTrack' in track) {
    return {
      trackID: track.sid,
      source: track.source,
      muted: track.isMuted,
      enabled: track.mediaStreamTrack.enabled,
      kind: track.kind,
      streamID: track.mediaStreamID,
      streamTrackID: track.mediaStreamTrack.id
    };
  } else {
    return {
      trackID: track.trackSid,
      enabled: track.isEnabled,
      muted: track.isMuted,
      trackInfo: Object.assign({
        mimeType: track.mimeType,
        name: track.trackName,
        encrypted: track.isEncrypted,
        kind: track.kind,
        source: track.source
      }, track.track ? getLogContextFromTrack(track.track) : {})
    };
  }
}
function supportsSynchronizationSources() {
  return typeof RTCRtpReceiver !== 'undefined' && typeof RTCRtpReceiver.prototype.getSynchronizationSources === 'function';
}
function diffAttributes(oldValues, newValues) {
  var _a;
  if (oldValues === undefined) {
    oldValues = {};
  }
  if (newValues === undefined) {
    newValues = {};
  }
  const allKeys = [...Object.keys(newValues), ...Object.keys(oldValues)];
  const diff = {};
  for (const key of allKeys) {
    if (oldValues[key] !== newValues[key]) {
      diff[key] = (_a = newValues[key]) !== null && _a !== void 0 ? _a : '';
    }
  }
  return diff;
}
/** @internal */
function extractProcessorsFromOptions(options) {
  const newOptions = Object.assign({}, options);
  let audioProcessor;
  let videoProcessor;
  if (typeof newOptions.audio === 'object' && newOptions.audio.processor) {
    audioProcessor = newOptions.audio.processor;
    newOptions.audio = Object.assign(Object.assign({}, newOptions.audio), {
      processor: undefined
    });
  }
  if (typeof newOptions.video === 'object' && newOptions.video.processor) {
    videoProcessor = newOptions.video.processor;
    newOptions.video = Object.assign(Object.assign({}, newOptions.video), {
      processor: undefined
    });
  }
  return {
    audioProcessor,
    videoProcessor,
    optionsWithoutProcessor: cloneDeep$1(newOptions)
  };
}
function getTrackSourceFromProto(source) {
  switch (source) {
    case TrackSource.CAMERA:
      return Track.Source.Camera;
    case TrackSource.MICROPHONE:
      return Track.Source.Microphone;
    case TrackSource.SCREEN_SHARE:
      return Track.Source.ScreenShare;
    case TrackSource.SCREEN_SHARE_AUDIO:
      return Track.Source.ScreenShareAudio;
    default:
      return Track.Source.Unknown;
  }
}
function areDimensionsSmaller(a, b) {
  return a.width * a.height < b.width * b.height;
}
function layerDimensionsFor(trackInfo, quality) {
  var _a;
  return (_a = trackInfo.layers) === null || _a === void 0 ? void 0 : _a.find(l => l.quality === quality);
}
/**
 * Waits until `track` has actually produced a video frame.
 *
 * iOS keeps reporting the camera's sensor frame from `getSettings()` until the camera has
 * delivered its first picture, so a portrait capture initially reads as landscape. The arrival
 * of that first frame is the only reliable signal that the reported settings describe reality.
 *
 * Best effort, never throws. Resolves `true` when a frame was observed, `false` when no frame
 * can be observed (the track is already ended, muted or disabled) or `timeoutMs` elapsed first.
 *
 * @internal
 */
function waitForFirstVideoFrame(track_1, timeoutMs_1) {
  return __awaiter(this, arguments, void 0, function (track, timeoutMs) {
    let candidateElements = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : [];
    return function* () {
      var _a;
      if (typeof document === 'undefined') {
        return false;
      }
      if (track.readyState === 'ended' || track.muted || !track.enabled) {
        // no frame is ever going to arrive, don't hold the caller up for the full timeout
        return false;
      }
      // requestVideoFrameCallback (Safari 15.4 and up) is a platform capability, so the answer is the
      // same for every element
      const supportsFrameCallback = typeof HTMLVideoElement.prototype.requestVideoFrameCallback === 'function';
      // prefer an element that is already rendering this track (a local preview, typically) over
      // spinning up a second decode just to observe a frame
      const existingElement = candidateElements.find(el => el instanceof HTMLVideoElement && el.srcObject instanceof MediaStream && el.srcObject.getVideoTracks().includes(track) && (
      // without requestVideoFrameCallback we depend on `loadeddata`, which is one-shot: an element
      // that already decoded a frame (readyState >= HAVE_CURRENT_DATA) never fires it again.
      // Leave those alone and set up our own element, which starts out empty.
      supportsFrameCallback || el.readyState < 2));
      const element = existingElement !== null && existingElement !== void 0 ? existingElement : document.createElement('video');
      if (!existingElement) {
        element.playsInline = true;
        element.muted = true;
        element.srcObject = new MediaStream([track]);
        // the element is deliberately never added to the DOM, it only exists to pull frames.
        // WebKit delivers frames to a paused element too, so a rejected play() (Safari's low power
        // mode, for instance) is not a reason to stop waiting
        element.play().catch(error => {
          livekitLogger.debug('could not play element while waiting for the first video frame', {
            error
          });
        });
      }
      let timer;
      let frameHandle;
      let onLoadedData;
      try {
        return yield new Promise(resolve => {
          timer = setTimeout(() => resolve(false), timeoutMs);
          if (supportsFrameCallback) {
            frameHandle = element.requestVideoFrameCallback(() => resolve(true));
          } else {
            // Safari below 15.4 has no requestVideoFrameCallback. `loadeddata` fires once the first
            // frame has been decoded, which is the same signal a few milliseconds earlier.
            onLoadedData = () => resolve(true);
            element.addEventListener('loadeddata', onLoadedData);
          }
        });
      } finally {
        clearTimeout(timer);
        if (frameHandle !== undefined) {
          (_a = element.cancelVideoFrameCallback) === null || _a === void 0 ? void 0 : _a.call(element, frameHandle);
        }
        if (onLoadedData) {
          element.removeEventListener('loadeddata', onLoadedData);
        }
        if (!existingElement) {
          element.pause();
          element.srcObject = null;
        }
      }
    }();
  });
}const BACKGROUND_REACTION_DELAY = 5000;
// keep old audio elements when detached, we would re-use them since on iOS
// Safari tracks which audio elements have been "blessed" by the user.
const recycledElements = [];
var VideoQuality;
(function (VideoQuality) {
  VideoQuality[VideoQuality["LOW"] = 0] = "LOW";
  VideoQuality[VideoQuality["MEDIUM"] = 1] = "MEDIUM";
  VideoQuality[VideoQuality["HIGH"] = 2] = "HIGH";
})(VideoQuality || (VideoQuality = {}));
export { AudioPresets, BACKGROUND_REACTION_DELAY, BackupCodecPolicy, ScreenSharePresets, VideoPreset, VideoPresets, VideoPresets43, VideoQuality, areDimensionsSmaller, audioCodecs, backupVideoCodecs, cloneDeep$1, constraintsForOptions, detectSilence, diffAttributes, extractProcessorsFromOptions, getLogContextFromTrack, getNewAudioContext, getTrackPublicationInfo, getTrackSourceFromProto, isBackupCodec, isBackupVideoCodec, jitterBuffer, kindToSource, layerDimensionsFor, mergeDefaultOptions, mergeObjectWithoutOverwriting, mimeTypeToVideoCodecString, playoutDelay, recycledElements, resolution, round, screenCaptureToDisplayMediaStreamOptions, sourceToKind, summarizeStatsReport, supportsSynchronizationSources, videoCodecs, waitForFirstVideoFrame };
