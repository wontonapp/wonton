// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 14613-15232 行按职责切出，行为不变。
import { __asyncGenerator, __await, __awaiter, _slicedToArray } from './tslib.mjs';
import { DisconnectReason } from './protobuf-messages-enums.mjs';
import { ClientInfo, ClientInfo_SDK } from './protobuf-messages-chat.mjs';
import { SignalResponse } from './protobuf-messages-signal.mjs';
import { clientProtocol, protocolVersion, version } from './browser.mjs';
import { ConnectionErrorReason } from './errors.mjs';
import { audioCodecs, getNewAudioContext, videoCodecs } from './util.mjs';
import { Track, isFireFox, isReactNative, isWeb, unpackStreamId } from './track.mjs';


function getLKReactNativeInfo() {
  // global defined only for ReactNative.
  // @ts-ignore
  if (global && global.LiveKitReactNativeGlobal) {
    // @ts-ignore
    return global.LiveKitReactNativeGlobal;
  }
  return undefined;
}
function getReactNativeOs() {
  if (!isReactNative()) {
    return undefined;
  }
  let info = getLKReactNativeInfo();
  if (info) {
    return info.platform;
  }
  return undefined;
}
function getDevicePixelRatio() {
  if (isWeb()) {
    return window.devicePixelRatio;
  }
  if (isReactNative()) {
    let info = getLKReactNativeInfo();
    if (info) {
      return info.devicePixelRatio;
    }
  }
  return 1;
}
/**
 * @param v1 - The first version string to compare.
 * @param v2 - The second version string to compare.
 * @returns A number indicating the order of the versions:
 *   - 1 if v1 is greater than v2
 *   - -1 if v1 is less than v2
 *   - 0 if v1 and v2 are equal
 */
function compareVersions(v1, v2) {
  const parts1 = v1.split('.');
  const parts2 = v2.split('.');
  const k = Math.min(parts1.length, parts2.length);
  for (let i = 0; i < k; ++i) {
    const p1 = parseInt(parts1[i], 10);
    const p2 = parseInt(parts2[i], 10);
    if (p1 > p2) return 1;
    if (p1 < p2) return -1;
    if (i === k - 1 && p1 === p2) return 0;
  }
  if (v1 === '' && v2 !== '') {
    return -1;
  } else if (v2 === '') {
    return 1;
  }
  return parts1.length == parts2.length ? 0 : parts1.length < parts2.length ? -1 : 1;
}
function roDispatchCallback(entries) {
  for (const entry of entries) {
    entry.target.handleResize(entry);
  }
}
function ioDispatchCallback(entries) {
  for (const entry of entries) {
    entry.target.handleVisibilityChanged(entry);
  }
}
let resizeObserver = null;
const getResizeObserver = () => {
  if (!resizeObserver) resizeObserver = new ResizeObserver(roDispatchCallback);
  return resizeObserver;
};
let intersectionObserver = null;
const getIntersectionObserver = () => {
  if (!intersectionObserver) {
    intersectionObserver = new IntersectionObserver(ioDispatchCallback, {
      root: null,
      rootMargin: '0px'
    });
  }
  return intersectionObserver;
};
function getClientInfo(capabilities) {
  var _a;
  const info = new ClientInfo({
    capabilities,
    sdk: ClientInfo_SDK.JS,
    protocol: protocolVersion,
    clientProtocol,
    version
  });
  if (isReactNative()) {
    info.os = (_a = getReactNativeOs()) !== null && _a !== void 0 ? _a : '';
  }
  return info;
}
let emptyVideoStreamTrack;
function getEmptyVideoStreamTrack() {
  if (!emptyVideoStreamTrack) {
    emptyVideoStreamTrack = createDummyVideoStreamTrack();
  }
  return emptyVideoStreamTrack.clone();
}
function createDummyVideoStreamTrack() {
  let width = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 16;
  let height = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 16;
  let enabled = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : false;
  let paintContent = arguments.length > 3 && arguments[3] !== undefined ? arguments[3] : false;
  const canvas = document.createElement('canvas');
  // the canvas size is set to 16 by default, because electron apps seem to fail with smaller values
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d');
  ctx === null || ctx === void 0 ? void 0 : ctx.fillRect(0, 0, canvas.width, canvas.height);
  if (paintContent && ctx) {
    ctx.beginPath();
    ctx.arc(width / 2, height / 2, 50, 0, Math.PI * 2, true);
    ctx.closePath();
    ctx.fillStyle = 'grey';
    ctx.fill();
  }
  // @ts-ignore
  const dummyStream = canvas.captureStream();
  const _dummyStream$getTrack = dummyStream.getTracks(),
    _dummyStream$getTrack2 = _slicedToArray(_dummyStream$getTrack, 1),
    dummyTrack = _dummyStream$getTrack2[0];
  if (!dummyTrack) {
    throw Error('Could not get empty media stream video track');
  }
  dummyTrack.enabled = enabled;
  return dummyTrack;
}
let emptyAudioStreamTrack;
let emptyAudioStreamContext;
/** tracks handed out by `getEmptyAudioStreamTrack` that haven't been released yet */
const outstandingEmptyAudioStreamTracks = new Set();
/**
 * Returns a silent audio track, cloned off a shared `AudioContext` created on first use. Hand the
 * clone back to `releaseEmptyAudioStreamTrack` rather than stopping it, so the context can close.
 * A clone that is only stopped stays counted as outstanding, which holds the shared context open
 * for the lifetime of the page.
 */
function getEmptyAudioStreamTrack() {
  if (!emptyAudioStreamTrack) {
    // implementation adapted from https://blog.mozilla.org/webrtc/warm-up-with-replacetrack/
    const ctx = new AudioContext();
    const oscillator = ctx.createOscillator();
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0, 0);
    const dst = ctx.createMediaStreamDestination();
    oscillator.connect(gain);
    gain.connect(dst);
    oscillator.start();
    var _dst$stream$getAudioT = dst.stream.getAudioTracks();
    var _dst$stream$getAudioT2 = _slicedToArray(_dst$stream$getAudioT, 1);
    emptyAudioStreamTrack = _dst$stream$getAudioT2[0];
    if (!emptyAudioStreamTrack) {
      // don't hold on to a context we can't get a track out of
      ctx.close().catch(() => {});
      throw Error('Could not get empty media stream audio track');
    }
    emptyAudioStreamTrack.enabled = false;
    emptyAudioStreamContext = ctx;
  }
  const track = emptyAudioStreamTrack.clone();
  outstandingEmptyAudioStreamTracks.add(track);
  return track;
}
/**
 * Stops a track obtained from `getEmptyAudioStreamTrack` and closes the shared `AudioContext`
 * backing it once every track it handed out has been released. Does nothing for a track this
 * module never handed out, or one that has been released already.
 */
function releaseEmptyAudioStreamTrack(track) {
  return __awaiter(this, void 0, void 0, function* () {
    if (!outstandingEmptyAudioStreamTracks.delete(track)) {
      // the track wasn't obtained from `getEmptyAudioStreamTrack`, or has been released before
      return;
    }
    track.stop();
    if (outstandingEmptyAudioStreamTracks.size > 0) {
      return;
    }
    const context = emptyAudioStreamContext;
    emptyAudioStreamTrack === null || emptyAudioStreamTrack === void 0 ? void 0 : emptyAudioStreamTrack.stop();
    emptyAudioStreamTrack = undefined;
    emptyAudioStreamContext = undefined;
    yield context === null || context === void 0 ? void 0 : context.close();
  });
}
/** An object that represents a serialized version of a `new Promise((resolve, reject) => {})`
 * constructor. Wait for a promise resolution with `await future.promise` and explicitly resolve or
 * reject the inner promise with `future.resolve(...)` or `future.reject(...)`.
 */
class Future {
  get isResolved() {
    return this._isResolved;
  }
  constructor(futureBase, onFinally) {
    this._isResolved = false;
    this.onFinally = onFinally;
    this.promise = new Promise((resolve, reject) => __awaiter(this, void 0, void 0, function* () {
      this.resolve = resolve;
      this.reject = reject;
      if (futureBase) {
        yield futureBase(resolve, reject);
      }
    })).finally(() => {
      var _a;
      this._isResolved = true;
      (_a = this.onFinally) === null || _a === void 0 ? void 0 : _a.call(this);
    });
  }
}
/**
 * Creates and returns an analyser web audio node that is attached to the provided track.
 * Additionally returns a convenience method `calculateVolume` to perform instant volume readings on that track.
 * Call the returned `cleanup` function to close the audioContext that has been created for the instance of this helper
 */
function createAudioAnalyser(track, options) {
  const opts = Object.assign({
    cloneTrack: false,
    fftSize: 2048,
    smoothingTimeConstant: 0.8,
    minDecibels: -100,
    maxDecibels: -80
  }, options);
  const audioContext = getNewAudioContext();
  if (!audioContext) {
    throw new Error('Audio Context not supported on this browser');
  }
  const streamTrack = opts.cloneTrack ? track.mediaStreamTrack.clone() : track.mediaStreamTrack;
  const mediaStreamSource = audioContext.createMediaStreamSource(new MediaStream([streamTrack]));
  const analyser = audioContext.createAnalyser();
  analyser.minDecibels = opts.minDecibels;
  analyser.maxDecibels = opts.maxDecibels;
  analyser.fftSize = opts.fftSize;
  analyser.smoothingTimeConstant = opts.smoothingTimeConstant;
  mediaStreamSource.connect(analyser);
  const dataArray = new Uint8Array(analyser.frequencyBinCount);
  /**
   * Calculates the current volume of the track in the range from 0 to 1
   */
  const calculateVolume = () => {
    analyser.getByteFrequencyData(dataArray);
    let sum = 0;
    for (const amplitude of dataArray) {
      sum += Math.pow(amplitude / 255, 2);
    }
    const volume = Math.sqrt(sum / dataArray.length);
    return volume;
  };
  let isCleanedUp = false;
  const cleanup = () => __awaiter(this, void 0, void 0, function* () {
    if (isCleanedUp) {
      // closing an already closed context would reject, make repeated cleanups a no-op instead
      return;
    }
    isCleanedUp = true;
    mediaStreamSource.disconnect();
    analyser.disconnect();
    if (opts.cloneTrack) {
      streamTrack.stop();
    }
    yield audioContext.close();
  });
  return {
    calculateVolume,
    analyser,
    cleanup
  };
}
function isAudioCodec(maybeCodec) {
  return audioCodecs.includes(maybeCodec);
}
function isVideoCodec(maybeCodec) {
  return videoCodecs.includes(maybeCodec);
}
function unwrapConstraint(constraint) {
  if (typeof constraint === 'string' || typeof constraint === 'number') {
    return constraint;
  }
  if (Array.isArray(constraint)) {
    return constraint[0];
  }
  if (constraint.exact !== undefined) {
    if (Array.isArray(constraint.exact)) {
      return constraint.exact[0];
    }
    return constraint.exact;
  }
  if (constraint.ideal !== undefined) {
    if (Array.isArray(constraint.ideal)) {
      return constraint.ideal[0];
    }
    return constraint.ideal;
  }
  throw Error('could not unwrap constraint');
}
/**
 * The getUserMedia rejections that no softer constraint can recover from, because the user or the
 * document refused capture outright. Retrying one of these would put a second prompt in front of
 * someone who already said no. The last two names are legacy spellings still reported by older
 * engines. Matched by `name` rather than with `instanceof`, because these are not reliably distinct
 * constructors across environments. happy-dom, which the tests run under, aliases them to `Event`.
 */
function isPermissionDeniedError(e) {
  return typeof e === 'object' && e !== null && 'name' in e && (e.name === 'NotAllowedError' || e.name === 'SecurityError' || e.name === 'PermissionDeniedError' || e.name === 'PermissionDismissedError');
}
const deviceAcquisitionFailures = new WeakSet();
/** Records that `e` is the rejection getUserMedia itself produced. */
function markDeviceAcquisitionFailure(e) {
  if (typeof e === 'object' && e !== null) {
    deviceAcquisitionFailures.add(e);
  }
}
/**
 * Whether a track restart failed at getUserMedia rather than in the work that adopts the new track.
 * Recorded on the side rather than by wrapping, because callers of `restartTrack` match on the
 * rejection's own name.
 */
function isDeviceAcquisitionFailure(e) {
  return typeof e === 'object' && e !== null && deviceAcquisitionFailures.has(e);
}
function toWebsocketUrl(url) {
  if (url.startsWith('http')) {
    return url.replace(/^(http)/, 'ws');
  }
  return url;
}
function toHttpUrl(url) {
  if (url.startsWith('ws')) {
    return url.replace(/^(ws)/, 'http');
  }
  return url;
}
function extractTranscriptionSegments(transcription, firstReceivedTimesMap) {
  return transcription.segments.map(_ref => {
    let id = _ref.id,
      text = _ref.text,
      language = _ref.language,
      startTime = _ref.startTime,
      endTime = _ref.endTime,
      final = _ref.final;
    var _a;
    const firstReceivedTime = (_a = firstReceivedTimesMap.get(id)) !== null && _a !== void 0 ? _a : Date.now();
    const lastReceivedTime = Date.now();
    if (final) {
      firstReceivedTimesMap.delete(id);
    } else {
      firstReceivedTimesMap.set(id, firstReceivedTime);
    }
    return {
      id,
      text,
      startTime: Number.parseInt(startTime.toString()),
      endTime: Number.parseInt(endTime.toString()),
      final,
      language,
      firstReceivedTime,
      lastReceivedTime
    };
  });
}
function extractChatMessage(msg) {
  const id = msg.id,
    timestamp = msg.timestamp,
    message = msg.message,
    editTimestamp = msg.editTimestamp;
  return {
    id,
    timestamp: Number.parseInt(timestamp.toString()),
    editTimestamp: editTimestamp ? Number.parseInt(editTimestamp.toString()) : undefined,
    message
  };
}
function getDisconnectReasonFromConnectionError(e) {
  switch (e.reason) {
    case ConnectionErrorReason.LeaveRequest:
      return e.context;
    case ConnectionErrorReason.Cancelled:
      return DisconnectReason.CLIENT_INITIATED;
    case ConnectionErrorReason.NotAllowed:
      return DisconnectReason.USER_REJECTED;
    case ConnectionErrorReason.ServerUnreachable:
      return DisconnectReason.JOIN_FAILURE;
    default:
      return DisconnectReason.UNKNOWN_REASON;
  }
}
/** convert bigints to numbers preserving undefined values */
function bigIntToNumber(value) {
  return value !== undefined ? Number(value) : undefined;
}
/** convert numbers to bigints preserving undefined values */
function numberToBigInt(value) {
  return value !== undefined ? BigInt(value) : undefined;
}
function isLocalTrack(track) {
  return !!track && !(track instanceof MediaStreamTrack) && track.isLocal;
}
function isAudioTrack(track) {
  return !!track && track.kind == Track.Kind.Audio;
}
function isVideoTrack(track) {
  return !!track && track.kind == Track.Kind.Video;
}
function isLocalVideoTrack(track) {
  return isLocalTrack(track) && isVideoTrack(track);
}
function isLocalAudioTrack(track) {
  return isLocalTrack(track) && isAudioTrack(track);
}
function isRemoteTrack(track) {
  return !!track && !track.isLocal;
}
function isRemotePub(pub) {
  return !!pub && !pub.isLocal;
}
function isRemoteVideoTrack(track) {
  return isRemoteTrack(track) && isVideoTrack(track);
}
function isLocalParticipant(p) {
  return p.isLocal;
}
function isRemoteParticipant(p) {
  return !p.isLocal;
}
function splitUtf8(s, n) {
  // adapted from https://stackoverflow.com/a/6043797
  const result = [];
  let encoded = new TextEncoder().encode(s);
  while (encoded.length > n) {
    let k = n;
    while (k > 0) {
      const byte = encoded[k];
      if (byte !== undefined && (byte & 0xc0) !== 0x80) {
        break;
      }
      k--;
    }
    result.push(encoded.slice(0, k));
    encoded = encoded.slice(k);
  }
  if (encoded.length > 0) {
    result.push(encoded);
  }
  return result;
}
/** Wraps a byte array in a `ReadableStream` that yields it as a single chunk and then closes. */
function readableFromBytes(bytes) {
  return new ReadableStream({
    start(controller) {
      controller.enqueue(bytes);
      controller.close();
    }
  });
}
/**
 * Re-chunks a byte stream into pieces of exactly `chunkSize` bytes (the final piece may be
 * smaller), coalescing or splitting the source's pieces as needed. Memory use is bounded to roughly
 * `chunkSize` plus one source read, so it never buffers the whole stream — used to pack
 * `CompressionStream`/`file.stream()` output into MTU-sized data-stream chunks.
 */
function readBytesInChunks(source, chunkSize) {
  return __asyncGenerator(this, arguments, function* readBytesInChunks_1() {
    const reader = source.getReader();
    let buffer = new Uint8Array(0);
    try {
      while (true) {
        const _yield$__await = yield __await(reader.read()),
          done = _yield$__await.done,
          value = _yield$__await.value;
        if (done) {
          break;
        }
        if (value.byteLength === 0) {
          continue;
        }
        const merged = new Uint8Array(buffer.byteLength + value.byteLength);
        merged.set(buffer);
        merged.set(value, buffer.byteLength);
        buffer = merged;
        while (buffer.byteLength >= chunkSize) {
          yield yield __await(buffer.slice(0, chunkSize));
          buffer = buffer.slice(chunkSize);
        }
      }
      if (buffer.byteLength > 0) {
        yield yield __await(buffer);
      }
    } finally {
      reader.releaseLock();
    }
  });
}
function extractMaxAgeFromRequestHeaders(headers) {
  var _a;
  const cacheControl = headers.get('Cache-Control');
  if (cacheControl) {
    const maxAge = (_a = cacheControl.match(/(?:^|[,\s])max-age=(\d+)/)) === null || _a === void 0 ? void 0 : _a[1];
    if (maxAge) {
      return parseInt(maxAge, 10);
    }
  }
  return undefined;
}
function isCompressionStreamSupported() {
  return typeof CompressionStream !== 'undefined';
}
let deflateRawCompressionSupported = null;
/**
 * Whether this runtime can compress AND decompress the `deflate-raw` format, which is a stricter
 * requirement than {@link isCompressionStreamSupported}: Chromium 80-102 included
 * `CompressionStream` supporting only `gzip` / `deflate`, and `deflate-raw` was added in Chromium
 * 103 (Firefox 113, Safari 16.4).
 */
function isDeflateRawCompressionSupported() {
  if (typeof deflateRawCompressionSupported !== 'boolean') {
    deflateRawCompressionSupported = probeDeflateRawCompressionSupport();
  }
  return deflateRawCompressionSupported;
}
function probeDeflateRawCompressionSupport() {
  if (typeof CompressionStream === 'undefined' || typeof DecompressionStream === 'undefined') {
    return false;
  }
  try {
    // There is no api to query format support, and constructing with an unsupported format throws
    // synchronously, so constructing one of each is the probe. Both are checked because the
    // deflate-raw capability advertised to peers is as much about decompressing what they send us
    // as it is about compressing what we send them.
    new CompressionStream('deflate-raw');
    new DecompressionStream('deflate-raw');
    return true;
  } catch (_a) {
    return false;
  }
}
function isPublisherOfferWithJoinSupported() {
  // we have connectivity issue about publisher offer with join on firefox #1919
  return isCompressionStreamSupported() && !isFireFox();
}
function extractTrackSid(mediaTrack, stream) {
  const _unpackStreamId = unpackStreamId(stream.id),
    _unpackStreamId2 = _slicedToArray(_unpackStreamId, 2),
    streamId = _unpackStreamId2[1];
  if (streamId === null || streamId === void 0 ? void 0 : streamId.startsWith('TR')) {
    return streamId;
  }
  if (mediaTrack.id.startsWith('TR')) {
    return mediaTrack.id;
  }
}function createRtcUrl(url, searchParams) {
  let useV0Path = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : false;
  const v0Url = createV0RtcUrl(url, searchParams);
  if (useV0Path) {
    return v0Url;
  } else {
    return appendUrlPath(v0Url, 'v1');
  }
}
function createV0RtcUrl(url, searchParams) {
  const urlObj = new URL(toWebsocketUrl(url));
  searchParams.forEach((value, key) => {
    urlObj.searchParams.set(key, value);
  });
  return appendUrlPath(urlObj, 'rtc');
}
function createValidateUrl(rtcWsUrl) {
  const urlObj = new URL(toHttpUrl(rtcWsUrl));
  return appendUrlPath(urlObj, 'validate');
}
function ensureTrailingSlash(path) {
  return path.endsWith('/') ? path : "".concat(path, "/");
}
function appendUrlPath(urlObj, path) {
  urlObj.pathname = "".concat(ensureTrailingSlash(urlObj.pathname)).concat(path);
  return urlObj;
}
function parseSignalResponse(value) {
  if (typeof value === 'string') {
    return SignalResponse.fromJson(JSON.parse(value), {
      ignoreUnknownFields: true
    });
  } else if (value instanceof ArrayBuffer) {
    return SignalResponse.fromBinary(new Uint8Array(value));
  }
  throw new Error("could not decode websocket message: ".concat(typeof value));
}
function getAbortReasonAsString(signal) {
  let defaultMessage = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 'Unknown reason';
  // the connect timeout hands this an Error rather than a signal, and its message is the only
  // description of what went wrong — without this a timeout reports itself as a generic abort
  if (signal instanceof Error) {
    return signal.message;
  }
  if (!(signal instanceof AbortSignal)) {
    return defaultMessage;
  }
  const reason = signal.reason;
  switch (typeof reason) {
    case 'string':
      return reason;
    case 'object':
      return reason instanceof Error ? reason.message : defaultMessage;
    default:
      return 'toString' in reason ? reason.toString() : defaultMessage;
  }
}
function getErrorDescription(error, errorCategory) {
  if (error instanceof Error) {
    if (error.name && error.message) {
      return "".concat(error.name, ": ").concat(error.message);
    }
    return error.name;
  }
  return "Encountered unknown ".concat(errorCategory, " error: ").concat(String(error));
}
export { Future, appendUrlPath, bigIntToNumber, compareVersions, createAudioAnalyser, createDummyVideoStreamTrack, createRtcUrl, createV0RtcUrl, createValidateUrl, deflateRawCompressionSupported, deviceAcquisitionFailures, emptyAudioStreamContext, emptyAudioStreamTrack, emptyVideoStreamTrack, ensureTrailingSlash, extractChatMessage, extractMaxAgeFromRequestHeaders, extractTrackSid, extractTranscriptionSegments, getAbortReasonAsString, getClientInfo, getDevicePixelRatio, getDisconnectReasonFromConnectionError, getEmptyAudioStreamTrack, getEmptyVideoStreamTrack, getErrorDescription, getIntersectionObserver, getLKReactNativeInfo, getReactNativeOs, getResizeObserver, intersectionObserver, ioDispatchCallback, isAudioCodec, isAudioTrack, isCompressionStreamSupported, isDeflateRawCompressionSupported, isDeviceAcquisitionFailure, isLocalAudioTrack, isLocalParticipant, isLocalTrack, isLocalVideoTrack, isPermissionDeniedError, isPublisherOfferWithJoinSupported, isRemoteParticipant, isRemotePub, isRemoteTrack, isRemoteVideoTrack, isVideoCodec, isVideoTrack, markDeviceAcquisitionFailure, numberToBigInt, outstandingEmptyAudioStreamTracks, parseSignalResponse, probeDeflateRawCompressionSupport, readBytesInChunks, readableFromBytes, releaseEmptyAudioStreamTrack, resizeObserver, roDispatchCallback, splitUtf8, toHttpUrl, toWebsocketUrl, unwrapConstraint };
