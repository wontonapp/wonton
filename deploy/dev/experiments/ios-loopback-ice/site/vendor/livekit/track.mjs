// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 13908-14613 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { TrackSource, TrackType } from './protobuf-messages-enums.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { TypedPromise, getBrowser } from './browser.mjs';
import { CriticalTimers } from './errors.mjs';
import { BACKGROUND_REACTION_DELAY, getLogContextFromTrack, recycledElements, summarizeStatsReport } from './util.mjs';
import { compareVersions } from './utils-misc.mjs';


class Track extends eventsExports.EventEmitter {
  /**
   * indicates current state of stream, it'll indicate `paused` if the track
   * has been paused by congestion controller
   */
  get streamState() {
    return this._streamState;
  }
  /** @internal */
  setStreamState(value) {
    if (this._streamState !== value) {
      this.log.debug("stream state changed: ".concat(this._streamState, " -> ").concat(value));
    }
    this._streamState = value;
  }
  constructor(mediaTrack, kind) {
    let loggerOptions = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : {};
    var _a;
    super();
    this.attachedElements = [];
    this.isMuted = false;
    this._streamState = Track.StreamState.Active;
    this.isInBackground = false;
    this._currentBitrate = 0;
    this.finalStatsLogged = false;
    this.log = livekitLogger;
    this.appVisibilityChangedListener = () => {
      if (this.backgroundTimeout) {
        clearTimeout(this.backgroundTimeout);
      }
      // delay app visibility update if it goes to hidden
      // update immediately if it comes back to focus
      if (document.visibilityState === 'hidden') {
        this.backgroundTimeout = setTimeout(() => this.handleAppVisibilityChanged(), BACKGROUND_REACTION_DELAY);
      } else {
        this.handleAppVisibilityChanged();
      }
    };
    this.loggerContextCb = loggerOptions.loggerContextCb;
    this.log = getLogger((_a = loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Track, () => this.logContext);
    this.setMaxListeners(100);
    this.kind = kind;
    this._mediaStreamTrack = mediaTrack;
    this._mediaStreamID = mediaTrack.id;
    this.source = Track.Source.Unknown;
  }
  get logContext() {
    var _a;
    return Object.assign(Object.assign({}, (_a = this.loggerContextCb) === null || _a === void 0 ? void 0 : _a.call(this)), getLogContextFromTrack(this));
  }
  /** current receive bits per second */
  get currentBitrate() {
    return this._currentBitrate;
  }
  get mediaStreamTrack() {
    return this._mediaStreamTrack;
  }
  /**
   * @internal
   * used for keep mediaStream's first id, since it's id might change
   * if we disable/enable a track
   */
  get mediaStreamID() {
    return this._mediaStreamID;
  }
  attach(element) {
    let elementType = 'audio';
    if (this.kind === Track.Kind.Video) {
      elementType = 'video';
    }
    if (this.attachedElements.length === 0 && this.kind === Track.Kind.Video) {
      this.addAppVisibilityListener();
    }
    if (!element) {
      if (elementType === 'audio') {
        recycledElements.forEach(e => {
          if (e.parentElement === null && !element) {
            element = e;
          }
        });
        if (element) {
          // remove it from pool
          recycledElements.splice(recycledElements.indexOf(element), 1);
        }
      }
      if (!element) {
        element = document.createElement(elementType);
      }
    }
    if (!this.attachedElements.includes(element)) {
      this.attachedElements.push(element);
    }
    // even if we believe it's already attached to the element, it's possible
    // the element's srcObject was set to something else out of band.
    // we'll want to re-attach it in that case
    attachToElement(this.mediaStreamTrack, element);
    // handle auto playback failures
    const allMediaStreamTracks = element.srcObject.getTracks();
    const hasAudio = allMediaStreamTracks.some(tr => tr.kind === 'audio');
    // manually play media to detect auto playback status
    element.play().then(() => {
      this.emit(hasAudio ? TrackEvent.AudioPlaybackStarted : TrackEvent.VideoPlaybackStarted);
    }).catch(e => {
      if (e.name === 'NotAllowedError') {
        this.emit(hasAudio ? TrackEvent.AudioPlaybackFailed : TrackEvent.VideoPlaybackFailed, e);
      } else if (e.name === 'AbortError') {
        // commonly triggered by another `play` request, only log for debugging purposes
        this.log.debug("".concat(hasAudio ? 'audio' : 'video', " playback aborted, likely due to new play request"));
      } else {
        this.log.warn("could not playback ".concat(hasAudio ? 'audio' : 'video'), {
          error: e
        });
      }
      // If audio playback isn't allowed make sure we still play back the video
      if (hasAudio && element && allMediaStreamTracks.some(tr => tr.kind === 'video') && e.name === 'NotAllowedError') {
        element.muted = true;
        element.play().catch(() => {
          // catch for Safari, exceeded options at this point to automatically play the media element
        });
      }
    });
    this.emit(TrackEvent.ElementAttached, element);
    return element;
  }
  detach(element) {
    try {
      // detach from a single element
      if (element) {
        detachTrack(this.mediaStreamTrack, element);
        const idx = this.attachedElements.indexOf(element);
        if (idx >= 0) {
          this.attachedElements.splice(idx, 1);
          this.recycleElement(element);
          this.emit(TrackEvent.ElementDetached, element);
        }
        return element;
      }
      const detached = [];
      this.attachedElements.forEach(elm => {
        detachTrack(this.mediaStreamTrack, elm);
        detached.push(elm);
        this.recycleElement(elm);
        this.emit(TrackEvent.ElementDetached, elm);
      });
      // remove all tracks
      this.attachedElements = [];
      return detached;
    } finally {
      if (this.attachedElements.length === 0) {
        this.removeAppVisibilityListener();
      }
    }
  }
  stop() {
    this.log.debug('stopping track');
    this.stopMonitor();
    this._mediaStreamTrack.stop();
  }
  enable() {
    this._mediaStreamTrack.enabled = true;
  }
  disable() {
    this._mediaStreamTrack.enabled = false;
  }
  /* @internal */
  stopMonitor() {
    if (this.monitorInterval) {
      clearInterval(this.monitorInterval);
    }
    if (this.timeSyncHandle !== undefined) {
      cancelAnimationFrame(this.timeSyncHandle);
      this.timeSyncHandle = undefined;
    }
    this.logFinalStats();
  }
  /**
   * Dumps the raw stats of the track as it ends, once: a track that stops
   * between two of the room's stats dumps is gone by the time the next one runs.
   */
  logFinalStats() {
    if (this.finalStatsLogged) {
      return;
    }
    this.finalStatsLogged = true;
    this.getRTCStatsReport().then(report => {
      if (report) {
        this.log.info('final track stats', summarizeStatsReport(report));
      }
    }).catch(error => this.log.debug('could not collect final track stats', {
      error
    }));
  }
  /** @internal */
  updateLoggerOptions(loggerOptions) {
    if (loggerOptions.loggerContextCb) {
      this.loggerContextCb = loggerOptions.loggerContextCb;
    }
    if (loggerOptions.loggerName) {
      this.log = getLogger(loggerOptions.loggerName, () => this.logContext);
    }
  }
  recycleElement(element) {
    if (element instanceof HTMLAudioElement) {
      // we only need to re-use a single element
      let shouldCache = true;
      element.pause();
      recycledElements.forEach(e => {
        if (!e.parentElement) {
          shouldCache = false;
        }
      });
      if (shouldCache) {
        recycledElements.push(element);
      }
    }
  }
  handleAppVisibilityChanged() {
    return __awaiter(this, void 0, void 0, function* () {
      this.isInBackground = document.visibilityState === 'hidden';
      if (!this.isInBackground && this.kind === Track.Kind.Video) {
        setTimeout(() => this.attachedElements.forEach(el => el.play().catch(() => {
          /** catch clause necessary for Safari */
        })), 0);
      }
    });
  }
  addAppVisibilityListener() {
    if (isWeb()) {
      this.isInBackground = document.visibilityState === 'hidden';
      document.addEventListener('visibilitychange', this.appVisibilityChangedListener);
    } else {
      this.isInBackground = false;
    }
  }
  removeAppVisibilityListener() {
    if (isWeb()) {
      document.removeEventListener('visibilitychange', this.appVisibilityChangedListener);
    }
  }
}
function attachToElement(track, element) {
  let mediaStream;
  if (element.srcObject instanceof MediaStream) {
    mediaStream = element.srcObject;
  } else {
    mediaStream = new MediaStream();
  }
  // check if track matches existing track
  let existingTracks;
  if (track.kind === 'audio') {
    existingTracks = mediaStream.getAudioTracks();
  } else {
    existingTracks = mediaStream.getVideoTracks();
  }
  if (!existingTracks.includes(track)) {
    existingTracks.forEach(et => {
      mediaStream.removeTrack(et);
    });
    mediaStream.addTrack(track);
  }
  if (!isSafari() || !(element instanceof HTMLVideoElement)) {
    // when in low power mode (applies to both macOS and iOS), Safari will show a play/pause overlay
    // when a video starts that has the `autoplay` attribute is set.
    // we work around this by _not_ setting the autoplay attribute on safari and instead call `setTimeout(() => el.play(),0)` further down
    element.autoplay = true;
  }
  // In case there are no audio tracks present on the mediastream, we set the element as muted to ensure autoplay works
  element.muted = mediaStream.getAudioTracks().length === 0;
  if (element instanceof HTMLVideoElement) {
    element.playsInline = true;
  }
  // avoid flicker
  if (element.srcObject !== mediaStream) {
    element.srcObject = mediaStream;
    if ((isSafari() || isFireFox()) && element instanceof HTMLVideoElement) {
      // Firefox also has a timing issue where video doesn't actually get attached unless
      // performed out-of-band
      // Safari 15 has a bug where in certain layouts, video element renders
      // black until the page is resized or other changes take place.
      // Resetting the src triggers it to render.
      // https://developer.apple.com/forums/thread/690523
      setTimeout(() => {
        element.srcObject = mediaStream;
        // Safari 15 sometimes fails to start a video
        // when the window is backgrounded before the first frame is drawn
        // manually calling play here seems to fix that
        element.play().catch(() => {
          /** do nothing */
        });
      }, 0);
    }
  }
}
/** @internal */
function detachTrack(track, element) {
  if (element.srcObject instanceof MediaStream) {
    const mediaStream = element.srcObject;
    mediaStream.removeTrack(track);
    if (mediaStream.getTracks().length > 0) {
      element.srcObject = mediaStream;
    } else {
      element.srcObject = null;
    }
  }
}
(function (Track) {
  let Kind;
  (function (Kind) {
    Kind["Audio"] = "audio";
    Kind["Video"] = "video";
    Kind["Unknown"] = "unknown";
  })(Kind = Track.Kind || (Track.Kind = {}));
  let Source;
  (function (Source) {
    Source["Camera"] = "camera";
    Source["Microphone"] = "microphone";
    Source["ScreenShare"] = "screen_share";
    Source["ScreenShareAudio"] = "screen_share_audio";
    Source["Unknown"] = "unknown";
  })(Source = Track.Source || (Track.Source = {}));
  let StreamState$1;
  (function (StreamState) {
    StreamState["Active"] = "active";
    StreamState["Paused"] = "paused";
    StreamState["Unknown"] = "unknown";
  })(StreamState$1 = Track.StreamState || (Track.StreamState = {}));
  /** @internal */
  function kindToProto(k) {
    switch (k) {
      case Kind.Audio:
        return TrackType.AUDIO;
      case Kind.Video:
        return TrackType.VIDEO;
      default:
        // FIXME this was UNRECOGNIZED before
        return TrackType.DATA;
    }
  }
  Track.kindToProto = kindToProto;
  /** @internal */
  function kindFromProto(t) {
    switch (t) {
      case TrackType.AUDIO:
        return Kind.Audio;
      case TrackType.VIDEO:
        return Kind.Video;
      default:
        return Kind.Unknown;
    }
  }
  Track.kindFromProto = kindFromProto;
  /** @internal */
  function sourceToProto(s) {
    switch (s) {
      case Source.Camera:
        return TrackSource.CAMERA;
      case Source.Microphone:
        return TrackSource.MICROPHONE;
      case Source.ScreenShare:
        return TrackSource.SCREEN_SHARE;
      case Source.ScreenShareAudio:
        return TrackSource.SCREEN_SHARE_AUDIO;
      default:
        return TrackSource.UNKNOWN;
    }
  }
  Track.sourceToProto = sourceToProto;
  /** @internal */
  function sourceFromProto(s) {
    switch (s) {
      case TrackSource.CAMERA:
        return Source.Camera;
      case TrackSource.MICROPHONE:
        return Source.Microphone;
      case TrackSource.SCREEN_SHARE:
        return Source.ScreenShare;
      case TrackSource.SCREEN_SHARE_AUDIO:
        return Source.ScreenShareAudio;
      default:
        return Source.Unknown;
    }
  }
  Track.sourceFromProto = sourceFromProto;
  /** @internal */
  function streamStateFromProto(s) {
    switch (s) {
      case StreamState.ACTIVE:
        return StreamState$1.Active;
      case StreamState.PAUSED:
        return StreamState$1.Paused;
      default:
        return StreamState$1.Unknown;
    }
  }
  Track.streamStateFromProto = streamStateFromProto;
})(Track || (Track = {}));const separator = '|';
const ddExtensionURI = 'https://aomediacodec.github.io/av1-rtp-spec/#dependency-descriptor-rtp-header-extension';
function unpackStreamId(packed) {
  const parts = packed.split(separator);
  if (parts.length > 1) {
    return [parts[0], packed.substr(parts[0].length + 1)];
  }
  return [packed, ''];
}
function sleep(duration) {
  return new TypedPromise(resolve => CriticalTimers.setTimeout(resolve, duration));
}
/** @internal */
function supportsTransceiver() {
  return 'addTransceiver' in RTCPeerConnection.prototype;
}
/** @internal */
function supportsAddTrack() {
  return 'addTrack' in RTCPeerConnection.prototype;
}
function supportsAdaptiveStream() {
  return typeof ResizeObserver !== 'undefined' && typeof IntersectionObserver !== 'undefined';
}
function supportsDynacast() {
  return supportsTransceiver();
}
function supportsAV1() {
  if (!('getCapabilities' in RTCRtpSender)) {
    return false;
  }
  if (isSafari() || isFireFox()) {
    // Safari 17 on iPhone14 reports AV1 capability, but does not actually support it
    // Firefox does support AV1, but SVC publishing is not supported
    return false;
  }
  const capabilities = RTCRtpSender.getCapabilities('video');
  let hasAV1 = false;
  if (capabilities) {
    for (const codec of capabilities.codecs) {
      if (codec.mimeType.toLowerCase() === 'video/av1') {
        hasAV1 = true;
        break;
      }
    }
  }
  return hasAV1;
}
function supportsVP9() {
  if (!('getCapabilities' in RTCRtpSender)) {
    return false;
  }
  if (isFireFox()) {
    // technically speaking FireFox supports VP9, but SVC publishing is broken
    // https://bugzilla.mozilla.org/show_bug.cgi?id=1633876
    return false;
  }
  if (isSafari()) {
    const browser = getBrowser();
    if ((browser === null || browser === void 0 ? void 0 : browser.version) && compareVersions(browser.version, '16') < 0) {
      // Safari 16 and below does not support VP9
      return false;
    }
    if ((browser === null || browser === void 0 ? void 0 : browser.os) === 'iOS' && (browser === null || browser === void 0 ? void 0 : browser.osVersion) && compareVersions(browser.osVersion, '16') < 0) {
      // Safari 16 and below on iOS does not support VP9 we need the iOS check to account for other browsers running webkit under the hood
      return false;
    }
  }
  const capabilities = RTCRtpSender.getCapabilities('video');
  let hasVP9 = false;
  if (capabilities) {
    for (const codec of capabilities.codecs) {
      if (codec.mimeType.toLowerCase() === 'video/vp9') {
        hasVP9 = true;
        break;
      }
    }
  }
  return hasVP9;
}
function supportsH265() {
  if (!('getCapabilities' in RTCRtpSender)) {
    return false;
  }
  const capabilities = RTCRtpSender.getCapabilities('video');
  let hasH265 = false;
  if (capabilities) {
    for (const codec of capabilities.codecs) {
      if (codec.mimeType.toLowerCase() === 'video/h265') {
        hasH265 = true;
        break;
      }
    }
  }
  return hasH265;
}
function isSVCCodec(codec) {
  return codec === 'av1' || codec === 'vp9';
}
/**
 * Opts `transceiver` into negotiating the AV1 dependency descriptor, reporting whether it will be.
 *
 * Chrome only offers the extension on transceivers that can send, so one we create to receive on
 * never negotiates it — and Chrome 152 stopped decoding AV1 that arrives without it: frames get
 * assembled, none ever decode, and the receiver asks for a keyframe forever. Asking through the
 * transceiver rather than munging the extension into the SDP leaves the browser owning the
 * extension id, which is what keeps that id consistent across the bundle and across
 * renegotiations.
 *
 * A no-op where the browser offers no such control, or does not know the extension at all.
 * @internal
 */
function negotiateDependencyDescriptor(transceiver) {
  var _a;
  const extensions = (_a = transceiver.getHeaderExtensionsToNegotiate) === null || _a === void 0 ? void 0 : _a.call(transceiver);
  if (!extensions || !transceiver.setHeaderExtensionsToNegotiate) {
    return false;
  }
  const dd = extensions.find(ext => ext.uri === ddExtensionURI);
  if (!dd) {
    return false;
  }
  if (dd.direction !== 'stopped') {
    return true;
  }
  // sendrecv rather than recvonly, so that the extension is written as a plain `a=extmap` line —
  // the form the server already emits where it is the one offering — rather than one carrying a
  // `/recvonly` suffix that its parser may not expect
  dd.direction = 'sendrecv';
  try {
    transceiver.setHeaderExtensionsToNegotiate(extensions);
    return true;
  } catch (e) {
    // a rejected direction throws. Negotiating without the extension is what happened before this
    // existed, so it is not worth failing the connection over
    return false;
  }
}
/**
 * VP9 and AV1 are published as SVC (a single RTP stream carrying every spatial layer)
 * by default. They can instead be published as real, rid based simulcast — one
 * independent stream per rid, each carrying a single spatial layer — when the caller
 * opts in with `simulcast: true` and a single spatial layer scalability mode (`L1Tx`).
 *
 * The SFU has to be told about this: without an explicit
 * `SimulcastCodec.videoLayerMode` it assumes `MULTIPLE_SPATIAL_LAYERS_PER_STREAM` for
 * any SVC capable codec.
 */
function isSVCSimulcast(codec, options) {
  var _a;
  return isSVCCodec(codec) && !!(options === null || options === void 0 ? void 0 : options.simulcast) && !!((_a = options.scalabilityMode) === null || _a === void 0 ? void 0 : _a.startsWith('L1T'));
}
/**
 * Whether the browser reads multiple encodings on an SVC capable codec as *legacy SVC*
 * rather than as real simulcast.
 *
 * Before Chrome M113, supplying more than one encoding for VP9/AV1 selected SVC mode;
 * only from M113 does libwebrtc treat such encodings as simulcast, and only when each
 * one carries its own scalabilityMode. Safari (and anything WebKit based, i. e. every
 * browser on iOS) still uses the old interpretation, as does React Native's libwebrtc.
 * Announced at https://groups.google.com/g/discuss-webrtc/c/-QQ3pxrl-fw
 *
 * Where this is true the rids would not exist on the wire, so VP9/AV1 must be published
 * as SVC no matter what the caller asked for.
 */
function usesLegacySVCEncodings() {
  const browser = getBrowser();
  return isSafariBased() ||
  // Even tho RN runs M114, it does not produce SVC layers when a single encoding
  // is provided. So we'll use the legacy SVC specification for now.
  // TODO: when we upstream libwebrtc, this will need additional verification
  isReactNative() || (browser === null || browser === void 0 ? void 0 : browser.name) === 'Chrome' && compareVersions(browser.version, '113') < 0;
}
/**
 * Last server version that doesn't support vp9/av1 simulcast.
 */
const svcSimulcastMinServerVersion = '1.13.6';
/**
 * Whether the connected server honours `SimulcastCodec.videoLayerMode`, i. e. whether
 * VP9/AV1 can be published as rid based simulcast. An unknown version is treated as
 * unsupported so the publish falls back to SVC.
 */
function isSVCSimulcastSupportedByServer(serverVersion) {
  if (!serverVersion) {
    return false;
  }
  return compareVersions(serverVersion, svcSimulcastMinServerVersion) > 0;
}
function supportsSetSinkId(elm) {
  if (!document || isSafariBased()) {
    return false;
  }
  if (!elm) {
    elm = document.createElement('audio');
  }
  return 'setSinkId' in elm;
}
/**
 * Checks whether or not setting an audio output via {@link Room#setActiveDevice}
 * is supported for the current browser.
 */
function supportsAudioOutputSelection() {
  // Note: this is method publicly exported under a user friendly name and currently only proxying `supportsSetSinkId`
  return supportsSetSinkId();
}
function isBrowserSupported() {
  if (typeof RTCPeerConnection === 'undefined') {
    return false;
  }
  return supportsTransceiver() || supportsAddTrack();
}
function isFireFox() {
  var _a;
  return ((_a = getBrowser()) === null || _a === void 0 ? void 0 : _a.name) === 'Firefox';
}
function isChromiumBased() {
  const browser = getBrowser();
  return !!browser && browser.name === 'Chrome' && browser.os !== 'iOS';
}
function isScriptTransformSupportedForWorker() {
  // Chrome occasionally throws an `InvalidState` error when using script transforms directly after introducing this API in 141.
  // Disabling it for Chrome based browsers until the API has stabilized.
  // @ts-ignore
  return typeof window !== 'undefined' &&
  // @ts-ignore
  typeof window.RTCRtpScriptTransform !== 'undefined' && !isChromiumBased();
}
function isSafari() {
  var _a;
  return ((_a = getBrowser()) === null || _a === void 0 ? void 0 : _a.name) === 'Safari';
}
function isSafariBased() {
  const b = getBrowser();
  return (b === null || b === void 0 ? void 0 : b.name) === 'Safari' || (b === null || b === void 0 ? void 0 : b.os) === 'iOS';
}
/**
 * iPadOS has sent the desktop Macintosh user agent by default since iPadOS 13, so `getBrowser()`
 * reports `os: 'macOS'` there and every `os === 'iOS'` check misses it. macOS Safari implements no
 * touch events at all (`TouchEvent` undefined, `maxTouchPoints` 0) regardless of any touchscreen
 * attached, so touch capability separates the two. Measured on an iPad 6 / iPadOS 17
 * (`maxTouchPoints` 5) against macOS Safari 26.4 (`maxTouchPoints` 0).
 *
 * Deliberately a separate helper rather than a fix in `browserParser`: iPadOS spoofs the Mac
 * version too, so reporting `os: 'iOS'` there would leave `osVersion` at `10.15.7` and silently
 * break {@link isSafari17Based} and {@link isSafariSvcApi}, which compare it.
 */
function isIPadOS() {
  if (!isWeb()) {
    return false;
  }
  const b = getBrowser();
  return (b === null || b === void 0 ? void 0 : b.name) === 'Safari' && (b === null || b === void 0 ? void 0 : b.os) === 'macOS' && navigator.maxTouchPoints > 1;
}
/**
 * iPhone, iPad and iOS-hosted browsers — everything running the Apple camera capture pipeline,
 * including iPads that present themselves as a Mac.
 */
function isAppleMobile() {
  var _a;
  return ((_a = getBrowser()) === null || _a === void 0 ? void 0 : _a.os) === 'iOS' || isIPadOS();
}
function isSafari17Based() {
  const b = getBrowser();
  return (b === null || b === void 0 ? void 0 : b.name) === 'Safari' && b.version.startsWith('17.') || (b === null || b === void 0 ? void 0 : b.os) === 'iOS' && !!(b === null || b === void 0 ? void 0 : b.osVersion) && compareVersions(b.osVersion, '17') >= 0;
}
function isSafariSvcApi(browser) {
  if (!browser) {
    browser = getBrowser();
  }
  // Safari 18.4 requires legacy svc api and scaleResolutionDown to be set
  return (browser === null || browser === void 0 ? void 0 : browser.name) === 'Safari' && compareVersions(browser.version, '18.3') > 0 || (browser === null || browser === void 0 ? void 0 : browser.os) === 'iOS' && !!(browser === null || browser === void 0 ? void 0 : browser.osVersion) && compareVersions(browser.osVersion, '18.3') > 0;
}
function isMobile() {
  var _a, _b;
  if (!isWeb()) return false;
  return (
    // @ts-expect-error `userAgentData` is not yet part of typescript
    (_b = (_a = navigator.userAgentData) === null || _a === void 0 ? void 0 : _a.mobile) !== null && _b !== void 0 ? _b : /Tablet|iPad|Mobile|Android|BlackBerry/.test(navigator.userAgent)
  );
}
function isE2EESimulcastSupported() {
  const browser = getBrowser();
  const supportedSafariVersion = '17.2'; // see https://bugs.webkit.org/show_bug.cgi?id=257803
  if (browser) {
    if (browser.name !== 'Safari' && browser.os !== 'iOS') {
      return true;
    } else if (browser.os === 'iOS' && browser.osVersion && compareVersions(browser.osVersion, supportedSafariVersion) >= 0) {
      return true;
    } else if (browser.name === 'Safari' && compareVersions(browser.version, supportedSafariVersion) >= 0) {
      return true;
    } else {
      return false;
    }
  }
}
function isWeb() {
  return typeof document !== 'undefined';
}
function isReactNative() {
  // navigator.product is deprecated on browsers, but will be set appropriately for react-native.
  return navigator.product == 'ReactNative';
}
function isCloud(serverUrl) {
  return serverUrl.hostname.endsWith('.livekit.cloud') || serverUrl.hostname.endsWith('.livekit.run');
}
function extractProjectFromUrl(serverUrl) {
  if (!isCloud(serverUrl)) {
    return null;
  }
  return serverUrl.hostname.split('.')[0];
}
export { Track, attachToElement, ddExtensionURI, detachTrack, extractProjectFromUrl, isAppleMobile, isBrowserSupported, isChromiumBased, isCloud, isE2EESimulcastSupported, isFireFox, isIPadOS, isMobile, isReactNative, isSVCCodec, isSVCSimulcast, isSVCSimulcastSupportedByServer, isSafari, isSafari17Based, isSafariBased, isSafariSvcApi, isScriptTransformSupportedForWorker, isWeb, negotiateDependencyDescriptor, separator, sleep, supportsAV1, supportsAdaptiveStream, supportsAddTrack, supportsAudioOutputSelection, supportsDynacast, supportsH265, supportsSetSinkId, supportsTransceiver, supportsVP9, svcSimulcastMinServerVersion, unpackStreamId, usesLegacySVCEncodings };
