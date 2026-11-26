// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 26901-28502 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { __asyncValues, __awaiter, _slicedToArray } from './tslib.mjs';
import { DataPacket, Encryption_Type } from './protobuf-messages-room.mjs';
import { ClientInfo_Capability, DataStream_OperationType } from './protobuf-messages-chat.mjs';
import { DataStream_ByteHeader, DataStream_Chunk, DataStream_CompressionType, DataStream_Header, DataStream_TextHeader } from './protobuf-messages-stream.mjs';
import { livekitLogger } from './loglevel.mjs';
import { CLIENT_PROTOCOL_DATA_STREAM_V2 } from './browser.mjs';
import { ConnectionError, ConnectionErrorReason, DataStreamError, DataStreamErrorReason } from './errors.mjs';
import { EngineEvent } from './event-enums.mjs';
import { isCloud } from './track.mjs';
import { bigIntToNumber, extractMaxAgeFromRequestHeaders, isDeflateRawCompressionSupported, numberToBigInt, readBytesInChunks, readableFromBytes } from './utils-misc.mjs';
import { DataChannelKind } from './data-channel.mjs';
import { DEFAULT_MAX_AGE_MS, STOP_REFETCH_DELAY_MS, log$4 } from './rtc-engine.mjs';
import { sendStreamTrailer } from './data-streams.mjs';


class RegionUrlProvider {
  static fetchRegionSettings(serverUrl, token, signal) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield RegionUrlProvider.fetchLock.lock();
      try {
        const regionSettingsResponse = yield fetch("".concat(getCloudConfigUrl(serverUrl), "/regions"), {
          headers: {
            authorization: "Bearer ".concat(token)
          },
          signal
        });
        if (regionSettingsResponse.ok) {
          const maxAge = extractMaxAgeFromRequestHeaders(regionSettingsResponse.headers);
          const maxAgeInMs = maxAge ? maxAge * 1000 : DEFAULT_MAX_AGE_MS;
          const regionSettings = yield regionSettingsResponse.json();
          return {
            regionSettings,
            updatedAtInMs: Date.now(),
            maxAgeInMs
          };
        } else {
          if (regionSettingsResponse.status === 401) {
            throw ConnectionError.notAllowed("Could not fetch region settings: ".concat(regionSettingsResponse.statusText), regionSettingsResponse.status);
          } else {
            throw ConnectionError.internal("Could not fetch region settings: ".concat(regionSettingsResponse.statusText));
          }
        }
      } catch (e) {
        if (e instanceof ConnectionError) {
          // rethrow connection errors
          throw e;
        } else if (signal === null || signal === void 0 ? void 0 : signal.aborted) {
          throw ConnectionError.cancelled("Region fetching was aborted");
        } else {
          // wrap other errors as connection errors
          throw ConnectionError.serverUnreachable("Could not fetch region settings, ".concat(e instanceof Error ? "".concat(e.name, ": ").concat(e.message) : e));
        }
      } finally {
        unlock();
      }
    });
  }
  static scheduleRefetch(url, token, maxAgeInMs) {
    return __awaiter(this, void 0, void 0, function* () {
      const timeout = RegionUrlProvider.settingsTimeouts.get(url.hostname);
      clearTimeout(timeout);
      RegionUrlProvider.settingsTimeouts.set(url.hostname, setTimeout(() => __awaiter(this, void 0, void 0, function* () {
        try {
          const newSettings = yield RegionUrlProvider.fetchRegionSettings(url, token);
          RegionUrlProvider.updateCachedRegionSettings(url, token, newSettings);
        } catch (error) {
          if (error instanceof ConnectionError && error.reason === ConnectionErrorReason.NotAllowed) {
            log$4.debug('token is not valid, cancelling auto region refresh');
            return;
          }
          log$4.debug('auto refetching of region settings failed', {
            error
          });
          // continue retrying with the same max age
          RegionUrlProvider.scheduleRefetch(url, token, maxAgeInMs);
        }
      }), maxAgeInMs));
    });
  }
  static updateCachedRegionSettings(url, token, settings) {
    RegionUrlProvider.cache.set(url.hostname, settings);
    RegionUrlProvider.scheduleRefetch(url, token, settings.maxAgeInMs);
  }
  static stopRefetch(hostname) {
    const timeout = RegionUrlProvider.settingsTimeouts.get(hostname);
    if (timeout) {
      clearTimeout(timeout);
      RegionUrlProvider.settingsTimeouts.delete(hostname);
    }
  }
  static scheduleCleanup(hostname) {
    let tracker = RegionUrlProvider.connectionTrackers.get(hostname);
    if (!tracker) {
      return;
    }
    // Cancel any existing cleanup timeout
    if (tracker.cleanupTimeout) {
      clearTimeout(tracker.cleanupTimeout);
    }
    // Schedule cleanup to stop refetch after delay
    tracker.cleanupTimeout = setTimeout(() => {
      const currentTracker = RegionUrlProvider.connectionTrackers.get(hostname);
      if (currentTracker && currentTracker.connectionCount === 0) {
        log$4.debug('stopping region refetch after disconnect delay', {
          hostname
        });
        RegionUrlProvider.stopRefetch(hostname);
      }
      if (currentTracker) {
        currentTracker.cleanupTimeout = undefined;
      }
    }, STOP_REFETCH_DELAY_MS);
  }
  static cancelCleanup(hostname) {
    const tracker = RegionUrlProvider.connectionTrackers.get(hostname);
    if (tracker === null || tracker === void 0 ? void 0 : tracker.cleanupTimeout) {
      clearTimeout(tracker.cleanupTimeout);
      tracker.cleanupTimeout = undefined;
    }
  }
  notifyConnected() {
    const hostname = this.serverUrl.hostname;
    let tracker = RegionUrlProvider.connectionTrackers.get(hostname);
    if (!tracker) {
      tracker = {
        connectionCount: 0
      };
      RegionUrlProvider.connectionTrackers.set(hostname, tracker);
    }
    tracker.connectionCount++;
    // Cancel any scheduled cleanup since we have an active connection
    RegionUrlProvider.cancelCleanup(hostname);
  }
  notifyDisconnected() {
    const hostname = this.serverUrl.hostname;
    const tracker = RegionUrlProvider.connectionTrackers.get(hostname);
    if (!tracker) {
      return;
    }
    tracker.connectionCount = Math.max(0, tracker.connectionCount - 1);
    // If no more connections, schedule cleanup
    if (tracker.connectionCount === 0) {
      RegionUrlProvider.scheduleCleanup(hostname);
    }
  }
  constructor(url, token) {
    this.attemptedRegions = [];
    this.serverUrl = new URL(url);
    this.token = token;
  }
  updateToken(token) {
    var _a;
    this.token = token;
    const url = this.getServerUrl();
    const settings = RegionUrlProvider.cache.get(url.hostname);
    RegionUrlProvider.scheduleRefetch(this.serverUrl, this.token, (_a = settings === null || settings === void 0 ? void 0 : settings.maxAgeInMs) !== null && _a !== void 0 ? _a : DEFAULT_MAX_AGE_MS);
  }
  isCloud() {
    return isCloud(this.serverUrl);
  }
  getServerUrl() {
    return this.serverUrl;
  }
  /** @internal */
  fetchRegionSettings(abortSignal) {
    return __awaiter(this, void 0, void 0, function* () {
      return RegionUrlProvider.fetchRegionSettings(this.serverUrl, this.token, abortSignal);
    });
  }
  getNextBestRegionUrl(abortSignal) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.isCloud()) {
        throw Error('region availability is only supported for LiveKit Cloud domains');
      }
      let cachedSettings = RegionUrlProvider.cache.get(this.serverUrl.hostname);
      if (!cachedSettings || Date.now() - cachedSettings.updatedAtInMs > cachedSettings.maxAgeInMs) {
        cachedSettings = yield this.fetchRegionSettings(abortSignal);
        RegionUrlProvider.updateCachedRegionSettings(this.serverUrl, this.token, cachedSettings);
      }
      const regionsLeft = cachedSettings.regionSettings.regions.filter(region => !this.attemptedRegions.find(attempted => attempted.url === region.url));
      if (regionsLeft.length > 0) {
        const nextRegion = regionsLeft[0];
        this.attemptedRegions.push(nextRegion);
        log$4.info("switching to region: ".concat(nextRegion.region), {
          region: nextRegion.region
        });
        return nextRegion.url;
      } else {
        return null;
      }
    });
  }
  resetAttempts() {
    this.attemptedRegions = [];
  }
  setServerReportedRegions(settings) {
    RegionUrlProvider.updateCachedRegionSettings(this.serverUrl, this.token, settings);
  }
}
RegionUrlProvider.cache = new Map();
RegionUrlProvider.settingsTimeouts = new Map();
RegionUrlProvider.connectionTrackers = new Map();
RegionUrlProvider.fetchLock = new _();
function getCloudConfigUrl(serverUrl) {
  return "".concat(serverUrl.protocol.replace('ws', 'http'), "//").concat(serverUrl.host, "/settings");
}/**
 * A `deflate-raw` compression transform (inverse of {@link inflateRawTransform}): pipe a byte stream
 * through it to get the compressed bytes without buffering the whole payload. Used for the chunked
 * `sendText`/`sendBytes`/`sendFile` paths, where the full payload is known up front but is streamed
 * (e.g. from `file.stream()`) rather than held in memory.
 */
function deflateRawTransform() {
  return new CompressionStream('deflate-raw');
}
/**
 * A `deflate-raw` decompression transform (inverse of {@link deflateRawTransform}): pipe a
 * stream of compressed bytes through it to get the decompressed bytes. Inflate emits output greedily,
 * so as long as the sender flushed at write boundaries each write's content is produced as soon as
 * its compressed bytes arrive.
 */
function inflateRawTransform() {
  return new DecompressionStream('deflate-raw');
}
/**
 * Decompresses a raw-deflate byte array in full (inverse of {@link deflateRawCompress}).
 * `maxByteLength` bounds the decompressed output (decompression-bomb guard); exceeding it rejects
 * with a `PayloadTooLarge` error.
 */
function deflateRawDecompress(data, maxByteLength) {
  return __awaiter(this, void 0, void 0, function* () {
    const ds = new DecompressionStream('deflate-raw');
    const writer = ds.writable.getWriter();
    // The writer promises are intentionally not awaited (output is consumed via `collect`), but
    // they reject when the byte cap cancels the readable mid-stream — swallow that so an enforced
    // cap doesn't surface as an unhandled rejection.
    writer.write(data).catch(() => {});
    writer.close().catch(() => {});
    return collect(ds.readable, maxByteLength);
  });
}
/**
 * Drains a byte stream, concatenating all of its chunks into a single array. When
 * `maxByteLength` is given, drops the stream and throws `PayloadTooLarge` as soon as the
 * accumulated output exceeds it.
 */
function collect(stream, maxByteLength) {
  return __awaiter(this, void 0, void 0, function* () {
    const reader = stream.getReader();
    const chunks = [];
    let total = 0;
    while (true) {
      const _yield$reader$read = yield reader.read(),
        done = _yield$reader$read.done,
        value = _yield$reader$read.value;
      if (done) {
        break;
      }
      chunks.push(value);
      total += value.byteLength;
      if (typeof maxByteLength === 'number' && total > maxByteLength) {
        yield reader.cancel();
        throw new DataStreamError("Decompressed payload exceeds the maximum payload size of ".concat(maxByteLength, " bytes"), DataStreamErrorReason.PayloadTooLarge);
      }
    }
    const result = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      result.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return result;
  });
}/**
 * Maximum size of a single data-stream chunk in bytes, and the budget used to decide whether a
 * payload can be sent inline as a single header packet. Kept below the ~16k data-channel MTU to
 * leave headroom for protocol framing and E2EE overhead.
 *
 * @internal
 */
const STREAM_CHUNK_SIZE_BYTES = 15000;
/**
 * Default cap on the number of decompressed bytes a single incoming compressed data stream may
 * produce (5 GB). A tiny compressed payload can inflate to an arbitrarily large output
 * (decompression bomb), so the decompressor's output is bounded rather than trusting the wire
 * size; streams exceeding the cap error with `PayloadTooLarge`.
 *
 * @internal
 */
const DEFAULT_MAX_PAYLOAD_BYTE_LENGTH = 5000000000;class BaseStreamReader {
  get info() {
    return this._info;
  }
  /** @internal */
  validateBytesReceived() {
    let doneReceiving = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : false;
    if (typeof this.totalByteSize !== 'number' || this.totalByteSize === 0) {
      return;
    }
    if (doneReceiving && this.bytesReceived < this.totalByteSize) {
      throw new DataStreamError("Not enough chunk(s) received - expected ".concat(this.totalByteSize, " bytes of data total, only received ").concat(this.bytesReceived, " bytes"), DataStreamErrorReason.Incomplete);
    } else if (this.bytesReceived > this.totalByteSize) {
      throw new DataStreamError("Extra chunk(s) received - expected ".concat(this.totalByteSize, " bytes of data total, received ").concat(this.bytesReceived, " bytes"), DataStreamErrorReason.LengthExceeded);
    }
  }
  constructor(info, stream, totalByteSize) {
    this.reader = stream;
    this.totalByteSize = totalByteSize;
    this._info = info;
    this.bytesReceived = 0;
  }
  /**
   * Counts a chunk's bytes against `totalByteSize` and reports progress. Chunk ordering and
   * de-duplication happen upstream in the manager's `ensureOrderedChunks`, so every chunk reaching
   * here is new and in order.
   */
  handleChunkReceived(chunk) {
    var _a;
    this.bytesReceived += chunk.content.byteLength;
    this.validateBytesReceived();
    const currentProgress = this.totalByteSize ? this.bytesReceived / this.totalByteSize : undefined;
    (_a = this.onProgress) === null || _a === void 0 ? void 0 : _a.call(this, currentProgress);
  }
}
class ByteStreamReader extends BaseStreamReader {
  [Symbol.asyncIterator]() {
    const reader = this.reader.getReader();
    // Suppress unhandled rejection on reader.closed — errors are
    // already propagated through reader.read() to the consumer.
    reader.closed.catch(() => {});
    const cleanup = () => {
      reader.releaseLock();
      this.signal = undefined;
    };
    return {
      next: () => __awaiter(this, void 0, void 0, function* () {
        var _a;
        try {
          const signal = this.signal;
          if (signal === null || signal === void 0 ? void 0 : signal.aborted) {
            throw signal.reason;
          }
          const result = yield new Promise((resolve, reject) => {
            if (signal) {
              const onAbort = () => reject(signal.reason);
              signal.addEventListener('abort', onAbort, {
                once: true
              });
              reader.read().then(resolve, reject).finally(() => {
                signal.removeEventListener('abort', onAbort);
              });
            } else {
              reader.read().then(resolve, reject);
            }
          });
          if (result.done) {
            this.validateBytesReceived(true);
            if (typeof this.totalByteSize === 'number') {
              (_a = this.onProgress) === null || _a === void 0 ? void 0 : _a.call(this, 1);
            }
            return {
              done: true,
              value: undefined
            };
          } else {
            this.handleChunkReceived(result.value);
            return {
              done: false,
              value: result.value.content
            };
          }
        } catch (err) {
          cleanup();
          throw err;
        }
      }),
      // note: `return` runs only for premature exits, see:
      // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Iteration_protocols#errors_during_iteration
      return() {
        return __awaiter(this, void 0, void 0, function* () {
          cleanup();
          return {
            done: true,
            value: undefined
          };
        });
      }
    };
  }
  /**
   * Injects an AbortSignal, which if aborted, will terminate the currently active
   * stream iteration operation.
   *
   * Note that when using AbortSignal.timeout(...), the timeout applies across
   * the whole iteration operation, not just one individual chunk read.
   */
  withAbortSignal(signal) {
    this.signal = signal;
    return this;
  }
  readAll() {
    return __awaiter(this, arguments, void 0, function () {
      var _this = this;
      let opts = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {};
      return function* () {
        var _a, e_1, _b, _c;
        let chunks = new Set();
        const iterator = opts.signal ? _this.withAbortSignal(opts.signal) : _this;
        try {
          for (var _d = true, iterator_1 = __asyncValues(iterator), iterator_1_1; iterator_1_1 = yield iterator_1.next(), _a = iterator_1_1.done, !_a; _d = true) {
            _c = iterator_1_1.value;
            _d = false;
            const chunk = _c;
            chunks.add(chunk);
          }
        } catch (e_1_1) {
          e_1 = {
            error: e_1_1
          };
        } finally {
          try {
            if (!_d && !_a && (_b = iterator_1.return)) yield _b.call(iterator_1);
          } finally {
            if (e_1) throw e_1.error;
          }
        }
        return Array.from(chunks);
      }();
    });
  }
}
/**
 * A class to read chunks from a ReadableStream and decode them as UTF-8 text.
 *
 * NOTE: chunk-level `version` (resending a chunk at an already-received `chunkIndex` to supersede
 * it) is not supported. The reader used to rebuild the whole string from a per-index chunk map and
 * yield it as `TextStreamChunk.collected`, which made superseding work; 5d4a6346 (#1410, text auto
 * chunking) changed the iterator to yield each chunk's text as it arrives, and a streaming reader
 * cannot retract text it has already handed to the consumer. No sender emits a versioned chunk.
 */
class TextStreamReader extends BaseStreamReader {
  /**
   * Async iterator implementation to allow usage of `for await...of` syntax.
   * Yields each chunk's decoded text as it arrives - a delta, not the string accumulated so far.
   */
  [Symbol.asyncIterator]() {
    const reader = this.reader.getReader();
    // Suppress unhandled rejection on reader.closed — errors are
    // already propagated through reader.read() to the consumer.
    reader.closed.catch(() => {});
    const decoder = new TextDecoder('utf-8');
    const signal = this.signal;
    const cleanup = () => {
      reader.releaseLock();
      this.signal = undefined;
    };
    return {
      next: () => __awaiter(this, void 0, void 0, function* () {
        var _a;
        try {
          if (signal === null || signal === void 0 ? void 0 : signal.aborted) {
            throw signal.reason;
          }
          const result = yield new Promise((resolve, reject) => {
            if (signal) {
              const onAbort = () => reject(signal.reason);
              signal.addEventListener('abort', onAbort, {
                once: true
              });
              reader.read().then(resolve, reject).finally(() => {
                signal.removeEventListener('abort', onAbort);
              });
            } else {
              reader.read().then(resolve, reject);
            }
          });
          if (result.done) {
            this.validateBytesReceived(true);
            if (typeof this.totalByteSize === 'number') {
              (_a = this.onProgress) === null || _a === void 0 ? void 0 : _a.call(this, 1);
            }
            return {
              done: true,
              value: undefined
            };
          } else {
            this.handleChunkReceived(result.value);
            let decodedResult;
            try {
              decodedResult = decoder.decode(result.value.content);
            } catch (err) {
              throw new DataStreamError("Cannot decode datastream chunk ".concat(result.value.chunkIndex, " as text: ").concat(err), DataStreamErrorReason.DecodeFailed);
            }
            return {
              done: false,
              value: decodedResult
            };
          }
        } catch (err) {
          cleanup();
          throw err;
        }
      }),
      // note: `return` runs only for premature exits, see:
      // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Iteration_protocols#errors_during_iteration
      return() {
        return __awaiter(this, void 0, void 0, function* () {
          cleanup();
          return {
            done: true,
            value: undefined
          };
        });
      }
    };
  }
  /**
   * Injects an AbortSignal, which if aborted, will terminate the currently active
   * stream iteration operation.
   *
   * Note that when using AbortSignal.timeout(...), the timeout applies across
   * the whole iteration operation, not just one individual chunk read.
   */
  withAbortSignal(signal) {
    this.signal = signal;
    return this;
  }
  readAll() {
    return __awaiter(this, arguments, void 0, function () {
      var _this2 = this;
      let opts = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {};
      return function* () {
        var _a, e_2, _b, _c;
        let finalString = '';
        const iterator = opts.signal ? _this2.withAbortSignal(opts.signal) : _this2;
        try {
          for (var _d = true, iterator_2 = __asyncValues(iterator), iterator_2_1; iterator_2_1 = yield iterator_2.next(), _a = iterator_2_1.done, !_a; _d = true) {
            _c = iterator_2_1.value;
            _d = false;
            const chunk = _c;
            finalString += chunk;
          }
        } catch (e_2_1) {
          e_2 = {
            error: e_2_1
          };
        } finally {
          try {
            if (!_d && !_a && (_b = iterator_2.return)) yield _b.call(iterator_2);
          } finally {
            if (e_2) throw e_2.error;
          }
        }
        return finalString;
      }();
    });
  }
}class IncomingDataStreamManager {
  constructor() {
    let maxPayloadByteLength = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : DEFAULT_MAX_PAYLOAD_BYTE_LENGTH;
    this.log = livekitLogger;
    this.byteStreamControllers = new Map();
    this.textStreamControllers = new Map();
    this.byteStreamHandlers = new Map();
    this.textStreamHandlers = new Map();
    this.isConnected = false;
    this.bufferedPackets = [];
    this.maxPayloadByteLength = maxPayloadByteLength;
  }
  setConnected(connected) {
    this.isConnected = connected;
    if (connected) {
      this.flushBufferedPackets();
    }
  }
  flushBufferedPackets() {
    const packets = this.bufferedPackets;
    this.bufferedPackets = [];
    for (const _ref of packets) {
      const packet = _ref.packet;
      const encryptionType = _ref.encryptionType;
      this.handleDataStreamPacket(packet, encryptionType);
    }
  }
  registerTextStreamHandler(topic, callback) {
    if (this.textStreamHandlers.has(topic)) {
      throw new DataStreamError("A text stream handler for topic \"".concat(topic, "\" has already been set."), DataStreamErrorReason.HandlerAlreadyRegistered);
    }
    this.textStreamHandlers.set(topic, callback);
  }
  unregisterTextStreamHandler(topic) {
    this.textStreamHandlers.delete(topic);
  }
  registerByteStreamHandler(topic, callback) {
    if (this.byteStreamHandlers.has(topic)) {
      throw new DataStreamError("A byte stream handler for topic \"".concat(topic, "\" has already been set."), DataStreamErrorReason.HandlerAlreadyRegistered);
    }
    this.byteStreamHandlers.set(topic, callback);
  }
  unregisterByteStreamHandler(topic) {
    this.byteStreamHandlers.delete(topic);
  }
  clearControllers() {
    // Error any in flight streams so readers do not wait forever for chunks that can no longer arrive.
    const abnormalEndError = new DataStreamError('Room disconnected in the middle of receiving data', DataStreamErrorReason.AbnormalEnd);
    for (const _ref2 of this.byteStreamControllers.values()) {
      const controller = _ref2.controller;
      controller.error(abnormalEndError);
    }
    for (const _ref3 of this.textStreamControllers.values()) {
      const controller = _ref3.controller;
      controller.error(abnormalEndError);
    }
    this.byteStreamControllers.clear();
    this.textStreamControllers.clear();
    this.bufferedPackets = [];
  }
  validateParticipantHasNoActiveDataStreams(participantIdentity) {
    // Terminate any in flight data stream receives from the given participant
    const textStreamsBeingSentByDisconnectingParticipant = Array.from(this.textStreamControllers.entries()).filter(entry => entry[1].sendingParticipantIdentity === participantIdentity);
    const byteStreamsBeingSentByDisconnectingParticipant = Array.from(this.byteStreamControllers.entries()).filter(entry => entry[1].sendingParticipantIdentity === participantIdentity);
    if (textStreamsBeingSentByDisconnectingParticipant.length > 0 || byteStreamsBeingSentByDisconnectingParticipant.length > 0) {
      const abnormalEndError = new DataStreamError("Participant ".concat(participantIdentity, " unexpectedly disconnected in the middle of sending data"), DataStreamErrorReason.AbnormalEnd);
      for (const _ref4 of byteStreamsBeingSentByDisconnectingParticipant) {
        var _ref5 = _slicedToArray(_ref4, 2);
        const id = _ref5[0];
        const controller = _ref5[1];
        controller.controller.error(abnormalEndError);
        this.byteStreamControllers.delete(id);
      }
      for (const _ref6 of textStreamsBeingSentByDisconnectingParticipant) {
        var _ref7 = _slicedToArray(_ref6, 2);
        const id = _ref7[0];
        const controller = _ref7[1];
        controller.controller.error(abnormalEndError);
        this.textStreamControllers.delete(id);
      }
    }
  }
  handleDataStreamPacket(packet, encryptionType) {
    if (!this.isConnected) {
      this.bufferedPackets.push({
        packet,
        encryptionType
      });
      return;
    }
    switch (packet.value.case) {
      case 'streamHeader':
        return this.handleStreamHeader(packet.value.value, packet.participantIdentity, encryptionType);
      case 'streamChunk':
        return this.handleStreamChunk(packet.value.value, encryptionType);
      case 'streamTrailer':
        return this.handleStreamTrailer(packet.value.value, encryptionType);
      default:
        throw new Error("DataPacket of value \"".concat(packet.value.case, "\" is not data stream related!"));
    }
  }
  handleStreamHeader(streamHeader, participantIdentity, encryptionType) {
    var _a;
    switch (streamHeader.contentHeader.case) {
      case 'byteHeader':
        {
          const streamHandlerCallback = this.byteStreamHandlers.get(streamHeader.topic);
          if (!streamHandlerCallback) {
            this.log.debug('ignoring incoming byte stream due to no handler for topic', streamHeader.topic);
            return;
          }
          let streamController;
          const info = {
            id: streamHeader.streamId,
            name: (_a = streamHeader.contentHeader.value.name) !== null && _a !== void 0 ? _a : 'unknown',
            mimeType: streamHeader.mimeType,
            size: streamHeader.totalLength ? Number(streamHeader.totalLength) : undefined,
            topic: streamHeader.topic,
            timestamp: bigIntToNumber(streamHeader.timestamp),
            attributes: streamHeader.attributes,
            encryptionType
          };
          // Determine if the byte payload needs to be decompressed.
          let compressed;
          switch (streamHeader.compression) {
            case DataStream_CompressionType.DEFLATE_RAW:
              if (!isDeflateRawCompressionSupported()) {
                // NOTE: this shouldn't really ever happen, if this warning is logged then the sender
                // isn't properly abiding by the data streams v2 protocol.
                livekitLogger.warn("Data stream ".concat(streamHeader.streamId, " received with deflate-raw compression, but this browser does not support deflate-raw decompression. Dropping..."));
                return;
              }
              compressed = true;
              break;
            case DataStream_CompressionType.NONE:
              compressed = false;
              break;
            default:
              // NOTE: this shouldn't really ever happen, if this warning is logged then the sender
              // isn't properly abiding by the data streams v2 protocol.
              livekitLogger.warn("Data stream ".concat(streamHeader.streamId, " received with unknown compression type ").concat(streamHeader.compression, ", dropping..."));
              return;
          }
          // Single-packet stream: the entire payload was packaged into the header's `inlineContent`.
          // Synthesize an already-complete stream and skip waiting for chunk/trailer packets.
          const inlineContent = streamHeader.inlineContent;
          if (typeof inlineContent !== 'undefined') {
            // Inline bytes are the raw payload, optionally deflate-raw compressed.
            streamHandlerCallback(new ByteStreamReader(info, createInlineStream(streamHeader.streamId, compressed ? deflateRawDecompress(inlineContent, this.maxPayloadByteLength) : inlineContent), bigIntToNumber(streamHeader.totalLength)), {
              identity: participantIdentity
            });
            return;
          }
          const stream = new ReadableStream({
            start: controller => {
              streamController = controller;
              if (this.byteStreamControllers.has(streamHeader.streamId)) {
                throw new DataStreamError("A data stream read is already in progress for a stream with id ".concat(streamHeader.streamId, "."), DataStreamErrorReason.AlreadyOpened);
              }
              this.byteStreamControllers.set(streamHeader.streamId, {
                info,
                controller: streamController,
                startTime: Date.now(),
                sendingParticipantIdentity: participantIdentity
              });
            }
          });
          streamHandlerCallback(new ByteStreamReader(info, compressed ? inflateRawByteChunkStream(stream, streamHeader.streamId, this.maxPayloadByteLength) : stream.pipeThrough(ensureOrderedChunks(streamHeader.streamId)),
          // `totalLength` is the pre-compression size, and the reader counts decompressed bytes,
          // so it applies to both paths (mirrors text).
          bigIntToNumber(streamHeader.totalLength)), {
            identity: participantIdentity
          });
          return;
        }
      case 'textHeader':
        {
          const streamHandlerCallback = this.textStreamHandlers.get(streamHeader.topic);
          if (!streamHandlerCallback) {
            this.log.debug('ignoring incoming text stream due to no handler for topic', streamHeader.topic);
            return;
          }
          let streamController;
          const info = {
            id: streamHeader.streamId,
            mimeType: streamHeader.mimeType,
            size: streamHeader.totalLength ? Number(streamHeader.totalLength) : undefined,
            topic: streamHeader.topic,
            timestamp: Number(streamHeader.timestamp),
            attributes: streamHeader.attributes,
            encryptionType,
            attachedStreamIds: streamHeader.contentHeader.value.attachedStreamIds
          };
          // Determine if the byte payload needs to be decompressed.
          let compressed;
          switch (streamHeader.compression) {
            case DataStream_CompressionType.DEFLATE_RAW:
              if (!isDeflateRawCompressionSupported()) {
                // NOTE: this shouldn't really ever happen, if this warning is logged then the sender
                // isn't properly abiding by the data streams v2 protocol.
                livekitLogger.warn("Data stream ".concat(streamHeader.streamId, " received with deflate-raw compression, but this browser does not support deflate-raw decompression. Dropping..."));
                return;
              }
              compressed = true;
              break;
            case DataStream_CompressionType.NONE:
              compressed = false;
              break;
            default:
              // NOTE: this shouldn't really ever happen, if this warning is logged then the sender
              // isn't properly abiding by the data streams v2 protocol.
              livekitLogger.warn("Data stream ".concat(streamHeader.streamId, " received with unknown compression type ").concat(streamHeader.compression, ", dropping..."));
              return;
          }
          // Single-packet stream: the entire payload was smuggled into the header's `inlineContent`.
          // Synthesize an already-complete stream and skip waiting for chunk/trailer packets.
          const inlineContent = streamHeader.inlineContent;
          if (typeof inlineContent !== 'undefined') {
            // Inline text is the raw UTF-8 payload, optionally deflate-raw compressed.
            const content = compressed ? deflateRawDecompress(inlineContent, this.maxPayloadByteLength) : inlineContent;
            streamHandlerCallback(new TextStreamReader(info, createInlineStream(streamHeader.streamId, content), bigIntToNumber(streamHeader.totalLength)), {
              identity: participantIdentity
            });
            return;
          }
          const stream = new ReadableStream({
            start: controller => {
              streamController = controller;
              if (this.textStreamControllers.has(streamHeader.streamId)) {
                throw new DataStreamError("A data stream read is already in progress for a stream with id ".concat(streamHeader.streamId, "."), DataStreamErrorReason.AlreadyOpened);
              }
              this.textStreamControllers.set(streamHeader.streamId, {
                info,
                controller: streamController,
                startTime: Date.now(),
                sendingParticipantIdentity: participantIdentity
              });
            }
          });
          streamHandlerCallback(new TextStreamReader(info, compressed ? inflateRawChunkStream(stream, streamHeader.streamId, this.maxPayloadByteLength) : stream.pipeThrough(ensureOrderedChunks(streamHeader.streamId)),
          // `totalLength` is the pre-compression size, and the reader sees decompressed bytes, so
          // it applies to both paths.
          bigIntToNumber(streamHeader.totalLength)), {
            identity: participantIdentity
          });
          return;
        }
    }
  }
  handleStreamChunk(chunk, encryptionType) {
    const fileBuffer = this.byteStreamControllers.get(chunk.streamId);
    if (fileBuffer) {
      if (fileBuffer.info.encryptionType !== encryptionType) {
        fileBuffer.controller.error(new DataStreamError("Encryption type mismatch for stream ".concat(chunk.streamId, ". Expected ").concat(encryptionType, ", got ").concat(fileBuffer.info.encryptionType), DataStreamErrorReason.EncryptionTypeMismatch));
        this.byteStreamControllers.delete(chunk.streamId);
      } else {
        fileBuffer.controller.enqueue(chunk);
      }
    }
    const textBuffer = this.textStreamControllers.get(chunk.streamId);
    if (textBuffer) {
      if (textBuffer.info.encryptionType !== encryptionType) {
        textBuffer.controller.error(new DataStreamError("Encryption type mismatch for stream ".concat(chunk.streamId, ". Expected ").concat(encryptionType, ", got ").concat(textBuffer.info.encryptionType), DataStreamErrorReason.EncryptionTypeMismatch));
        this.textStreamControllers.delete(chunk.streamId);
      } else {
        textBuffer.controller.enqueue(chunk);
      }
    }
  }
  handleStreamTrailer(trailer, encryptionType) {
    const textBuffer = this.textStreamControllers.get(trailer.streamId);
    if (textBuffer) {
      if (textBuffer.info.encryptionType !== encryptionType) {
        textBuffer.controller.error(new DataStreamError("Encryption type mismatch for stream ".concat(trailer.streamId, ". Expected ").concat(encryptionType, ", got ").concat(textBuffer.info.encryptionType), DataStreamErrorReason.EncryptionTypeMismatch));
      } else {
        textBuffer.info.attributes = Object.assign(Object.assign({}, textBuffer.info.attributes), trailer.attributes);
        if (trailer.reason) {
          // A non-empty reason marks an abnormal close by the sender (e.g. an aborted send);
          // surface it as an error rather than pretending the stream completed.
          textBuffer.controller.error(new DataStreamError("Data stream ".concat(trailer.streamId, " closed abnormally: ").concat(trailer.reason), DataStreamErrorReason.AbnormalEnd));
        } else {
          textBuffer.controller.close();
        }
      }
      this.textStreamControllers.delete(trailer.streamId);
    }
    const fileBuffer = this.byteStreamControllers.get(trailer.streamId);
    if (fileBuffer) {
      if (fileBuffer.info.encryptionType !== encryptionType) {
        fileBuffer.controller.error(new DataStreamError("Encryption type mismatch for stream ".concat(trailer.streamId, ". Expected ").concat(encryptionType, ", got ").concat(fileBuffer.info.encryptionType), DataStreamErrorReason.EncryptionTypeMismatch));
      } else {
        fileBuffer.info.attributes = Object.assign(Object.assign({}, fileBuffer.info.attributes), trailer.attributes);
        if (trailer.reason) {
          // A non-empty reason marks an abnormal close by the sender (e.g. an aborted send);
          // surface it as an error rather than pretending the stream completed.
          fileBuffer.controller.error(new DataStreamError("Data stream ".concat(trailer.streamId, " closed abnormally: ").concat(trailer.reason), DataStreamErrorReason.AbnormalEnd));
        } else {
          fileBuffer.controller.close();
        }
      }
      this.byteStreamControllers.delete(trailer.streamId);
    }
  }
}
/**
 * Builds a `ReadableStream` that yields the given content as a single chunk and then immediately
 * closes - used to surface an inline (single-packet) data stream as a fully-formed stream. `content`
 * may be a promise (e.g. async gzip decompression); a rejection errors the stream.
 */
function createInlineStream(streamId, content) {
  return new ReadableStream({
    start: controller => __awaiter(this, void 0, void 0, function* () {
      const bytes = yield content;
      controller.enqueue(new DataStream_Chunk({
        streamId,
        chunkIndex: BigInt(0),
        content: bytes
      }));
      controller.close();
    })
  });
}
/**
 * Validates that chunks are received in order, dropping duplicates and throwing if gaps are found.
 *
 * Reassembly (and, for compressed streams, a stateful decompressor) silently corrupts on duplicated
 * or out-of-order input, so duplicates are dropped (with a warning - in-order delivery is expected
 * on the reliable channel, but reconnect handling may replay) and a gap is a hard error. Empty
 * chunks consume their index and are then dropped, so they never reach the reader. Applied to every
 * chunked stream, compressed or not.
 */
function ensureOrderedChunks(streamId) {
  let lastChunkIndex = -1;
  return new TransformStream({
    transform: (value, controller) => {
      const index = bigIntToNumber(value.chunkIndex);
      if (index <= lastChunkIndex) {
        livekitLogger.warn("ignoring duplicate chunk ".concat(index, " ").concat(value.version > 0 ? "(version ".concat(value.version, ")") : '', " for data stream ").concat(streamId, " (last processed: ").concat(lastChunkIndex, ")"));
        return;
      }
      if (index > lastChunkIndex + 1) {
        throw new DataStreamError("Missing chunk(s) ".concat(lastChunkIndex + 1, "..").concat(index - 1, " for data stream ").concat(streamId, " - cannot reassemble payload"), DataStreamErrorReason.Incomplete);
      }
      lastChunkIndex = index;
      if (value.content.length === 0) {
        return;
      }
      controller.enqueue(value);
    }
  });
}
/** Unwraps compressed `DataStream_Chunk`s to their compressed bytes (in `chunkIndex` order), */
function chunksToBytes() {
  return new TransformStream({
    transform: (value, controller) => {
      controller.enqueue(value.content);
    }
  });
}
/** Re-wraps decompressed bytes into contiguous `DataStream_Chunk`s, skipping inflate's empty reads. */
function bytesToChunks(streamId) {
  let outIndex = 0;
  return new TransformStream({
    transform: (value, controller) => {
      // Inflate can emit empty reads; only synthesize a chunk when there is content.
      if (value.byteLength > 0) {
        controller.enqueue(new DataStream_Chunk({
          streamId,
          chunkIndex: numberToBigInt(outIndex),
          content: value
        }));
        outIndex += 1;
      }
    }
  });
}
/**
 * Reframes decompressed byte chunks onto UTF-8 character boundaries via a streaming `TextDecoder`
 * (a write larger than the MTU spans several packets, which may split a codepoint), so each
 * synthesized text chunk decodes independently. The `flush` emits the decoder's trailing bytes.
 */
function bytesToDecodedUtf8(streamId) {
  const decoder = new TextDecoder('utf-8');
  const encoder = new TextEncoder();
  let outIndex = 0;
  const decodeOrThrow = bytes => {
    try {
      return bytes ? decoder.decode(bytes, {
        stream: true
      }) : decoder.decode();
    } catch (err) {
      throw new DataStreamError("Cannot decode compressed data stream ".concat(streamId, " as text: ").concat(err), DataStreamErrorReason.DecodeFailed);
    }
  };
  return new TransformStream({
    transform: (value, controller) => {
      const text = decodeOrThrow(value);
      // Everything so far may have been a partial codepoint; only emit once we have characters.
      if (text.length > 0) {
        controller.enqueue(new DataStream_Chunk({
          streamId,
          chunkIndex: numberToBigInt(outIndex),
          content: encoder.encode(text)
        }));
        outIndex += 1;
      }
    },
    flush: controller => {
      const tail = decodeOrThrow();
      if (tail.length > 0) {
        controller.enqueue(new DataStream_Chunk({
          streamId,
          chunkIndex: numberToBigInt(outIndex),
          content: encoder.encode(tail)
        }));
        outIndex += 1;
      }
    }
  });
}
/**
 * Transforms a stream of deflate-raw-compressed byte `DataStream_Chunk`s into a stream of
 * decompressed chunks, so `ByteStreamReader` consumes it unchanged. All chunk contents are fed (in
 * `chunkIndex` order) through ONE raw-deflate decompressor for the stream's lifetime; decompressed
 * output is re-wrapped as chunks as soon as it is produced. The sender (sendFile) compresses the
 * whole payload in one shot, but the format also supports a single context-takeover stream
 * sync-flushed at write boundaries, so a future incremental streamBytes could compress with no
 * protocol change. Errors and cancellation propagate through the pipe chain.
 */
function inflateRawByteChunkStream(raw, streamId, maxPayloadByteLength) {
  return raw.pipeThrough(ensureOrderedChunks(streamId)).pipeThrough(chunksToBytes()).pipeThrough(inflateRawTransform()).pipeThrough(maxDecompressedLengthGuard(streamId, maxPayloadByteLength)).pipeThrough(bytesToChunks(streamId));
}
/**
 * Transforms a stream of deflate-raw-compressed text `DataStream_Chunk`s into a stream of
 * decompressed chunks, so `TextStreamReader` consumes it unchanged. Builds on
 * {@link inflateRawByteChunkStream} (single decompressor + ordering guard) and adds a streaming
 * `TextDecoder` that reframes the decompressed bytes on UTF-8 character boundaries so each
 * synthesized chunk decodes independently. Errors and cancellation propagate through the pipe chain.
 */
function inflateRawChunkStream(raw, streamId, maxPayloadByteLength) {
  return raw.pipeThrough(ensureOrderedChunks(streamId)).pipeThrough(chunksToBytes()).pipeThrough(inflateRawTransform()).pipeThrough(maxDecompressedLengthGuard(streamId, maxPayloadByteLength)).pipeThrough(bytesToDecodedUtf8(streamId));
}
/**
 * Caps the total decompressed byte count of a compressed stream (decompression-bomb guard): a
 * tiny compressed payload can inflate to an arbitrarily large output, so the decompressor's
 * output is bounded rather than trusting the wire size. Exceeding the cap errors the stream with
 * `PayloadTooLarge`.
 */
function maxDecompressedLengthGuard(streamId, maxByteLength) {
  let total = 0;
  return new TransformStream({
    transform: (value, controller) => {
      total += value.byteLength;
      if (total > maxByteLength) {
        throw new DataStreamError("Data stream ".concat(streamId, " exceeds the maximum payload size of ").concat(maxByteLength, " bytes"), DataStreamErrorReason.PayloadTooLarge);
      }
      controller.enqueue(value);
    }
  });
}class BaseStreamWriter {
  constructor(writableStream, info, onClose) {
    this.writableStream = writableStream;
    this.defaultWriter = writableStream.getWriter();
    this.onClose = onClose;
    this.info = info;
  }
  write(chunk) {
    return this.defaultWriter.write(chunk);
  }
  close() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      yield this.defaultWriter.close();
      this.defaultWriter.releaseLock();
      (_a = this.onClose) === null || _a === void 0 ? void 0 : _a.call(this);
    });
  }
}
class TextStreamWriter extends BaseStreamWriter {}
class ByteStreamWriter extends BaseStreamWriter {}/** Builds the `DataStream_Header` for a text stream from its info and stream options. */
function buildTextStreamHeader(info, options, v2) {
  var _a;
  return new DataStream_Header({
    streamId: info.id,
    mimeType: info.mimeType,
    topic: info.topic,
    timestamp: numberToBigInt(info.timestamp),
    totalLength: numberToBigInt(info.size),
    attributes: info.attributes,
    compression: (_a = v2 === null || v2 === void 0 ? void 0 : v2.compression) !== null && _a !== void 0 ? _a : DataStream_CompressionType.NONE,
    inlineContent: v2 === null || v2 === void 0 ? void 0 : v2.inlineContent,
    contentHeader: {
      case: 'textHeader',
      value: new DataStream_TextHeader({
        version: options === null || options === void 0 ? void 0 : options.version,
        attachedStreamIds: info.attachedStreamIds,
        replyToStreamId: options === null || options === void 0 ? void 0 : options.replyToStreamId,
        operationType: (options === null || options === void 0 ? void 0 : options.type) === 'update' ? DataStream_OperationType.UPDATE : DataStream_OperationType.CREATE
      })
    }
  });
}
/** Builds the `DataStream_Header` for a byte stream from its info. */
function buildByteStreamHeader(info, v2) {
  var _a;
  return new DataStream_Header({
    streamId: info.id,
    mimeType: info.mimeType,
    topic: info.topic,
    timestamp: numberToBigInt(info.timestamp),
    totalLength: numberToBigInt(info.size),
    attributes: info.attributes,
    compression: (_a = v2 === null || v2 === void 0 ? void 0 : v2.compression) !== null && _a !== void 0 ? _a : DataStream_CompressionType.NONE,
    inlineContent: v2 === null || v2 === void 0 ? void 0 : v2.inlineContent,
    contentHeader: {
      case: 'byteHeader',
      value: new DataStream_ByteHeader({
        name: info.name
      })
    }
  });
}
/** Wraps a `DataStream_Header` in a `DataPacket` ready to be sent over a data channel. */
function createStreamHeaderPacket(header, destinationIdentities) {
  return new DataPacket({
    destinationIdentities,
    value: {
      case: 'streamHeader',
      value: header
    }
  });
}const textEncoder = new TextEncoder();
/**
 * Manages sending custom user data via data channels.
 * @internal
 */
class OutgoingDataStreamManager {
  constructor(engine, log, getRemoteParticipantClientProtocol, getRemoteParticipantCapabilities, getAllRemoteParticipantIdentities) {
    this.engine = engine;
    this.log = log;
    this.getRemoteParticipantClientProtocol = getRemoteParticipantClientProtocol;
    this.getRemoteParticipantCapabilities = getRemoteParticipantCapabilities;
    this.getAllRemoteParticipantIdentities = getAllRemoteParticipantIdentities;
  }
  setupEngine(engine) {
    this.engine = engine;
  }
  /** {@inheritDoc LocalParticipant.sendText} */
  sendText(text, options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e;
      const streamId = crypto.randomUUID();
      const textInBytes = textEncoder.encode(text);
      const totalTextLength = textInBytes.byteLength;
      const compress = (_a = options === null || options === void 0 ? void 0 : options.compress) !== null && _a !== void 0 ? _a : true;
      let info = {
        id: streamId,
        mimeType: 'text/plain',
        timestamp: Date.now(),
        topic: (_b = options === null || options === void 0 ? void 0 : options.topic) !== null && _b !== void 0 ? _b : '',
        size: totalTextLength,
        // NOTE: size is always the pre-compression byte length
        attributes: options === null || options === void 0 ? void 0 : options.attributes,
        encryptionType: ((_c = this.engine.e2eeManager) === null || _c === void 0 ? void 0 : _c.isDataChannelEncryptionEnabled) ? Encryption_Type.GCM : Encryption_Type.NONE
      };
      const compressEligible = compress && isDeflateRawCompressionSupported() && this.allRecipientsSupportV2(options === null || options === void 0 ? void 0 : options.destinationIdentities) && this.allRecipientsSupportCompression(options === null || options === void 0 ? void 0 : options.destinationIdentities);
      let compressedStream = compressEligible ? MaybeCollectedStream.fromStream(readableFromBytes(textInBytes).pipeThrough(deflateRawTransform())) : null;
      // Phase 1: Try to send as a single packet data stream
      const noAttachments = !(options === null || options === void 0 ? void 0 : options.attachments) || options.attachments.length === 0;
      if (noAttachments && this.allRecipientsSupportV2(options === null || options === void 0 ? void 0 : options.destinationIdentities)) {
        // The payload rides in the header's `inlineContent` (raw bytes). Keep the compressed form only
        // if it actually shrinks the payload (deflate framing makes tiny strings larger). The
        // compression flag is carried in the header's `compression` field; user attributes are left
        // untouched.
        let inlineContent = textInBytes;
        let compression = DataStream_CompressionType.NONE;
        if (compressedStream) {
          const collectedBytes = yield compressedStream.collect();
          if (collectedBytes.byteLength < textInBytes.byteLength) {
            inlineContent = collectedBytes;
            compression = DataStream_CompressionType.DEFLATE_RAW;
          }
        }
        const header = buildTextStreamHeader(info, undefined, {
          compression,
          inlineContent
        });
        const packet = createStreamHeaderPacket(header, options === null || options === void 0 ? void 0 : options.destinationIdentities);
        if (packet.toBinary().byteLength <= STREAM_CHUNK_SIZE_BYTES) {
          yield this.engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
          (_d = options === null || options === void 0 ? void 0 : options.onProgress) === null || _d === void 0 ? void 0 : _d.call(options, 1);
          return info;
        }
      }
      const fileIds = (_e = options === null || options === void 0 ? void 0 : options.attachments) === null || _e === void 0 ? void 0 : _e.map(() => crypto.randomUUID());
      // Progress is split evenly across the text part (slot 0) and one slot per attachment, then
      // normalized to a [0,1] fraction. Each slot climbs monotonically to 1, so the aggregate ends at
      // exactly 1 once every part has completed.
      const parts = fileIds ? fileIds.length + 1 : 1;
      const progresses = new Array(parts).fill(0);
      const handleProgress = (progress, idx) => {
        var _a;
        progresses[idx] = progress;
        (_a = options === null || options === void 0 ? void 0 : options.onProgress) === null || _a === void 0 ? void 0 : _a.call(options, progresses.reduce((acc, val) => acc + val, 0) / parts);
      };
      // Phase 2: Try to send a multi packet data stream with compressed bytes
      if (compressedStream) {
        info.attachedStreamIds = fileIds;
        const header = buildTextStreamHeader(info, undefined, {
          compression: DataStream_CompressionType.DEFLATE_RAW
        });
        const packet = createStreamHeaderPacket(header, options === null || options === void 0 ? void 0 : options.destinationIdentities);
        yield this.sendChunkedByteStream(packet, streamId, options === null || options === void 0 ? void 0 : options.destinationIdentities, compressedStream.stream().pipeThrough(progressReportingStream(textInBytes.length, progress => handleProgress(progress, 0))));
        // Ensure there's always a 100% progress event fired, even if the string is zero bytes long
        if (textInBytes.length === 0) {
          handleProgress(1, 0);
        }
      } else {
        // Phase 3 / fallback: header + plain uncompressed chunk packets + trailer.
        const writer = yield this.streamText({
          streamId,
          totalSize: totalTextLength,
          destinationIdentities: options === null || options === void 0 ? void 0 : options.destinationIdentities,
          topic: options === null || options === void 0 ? void 0 : options.topic,
          attachedStreamIds: fileIds,
          attributes: options === null || options === void 0 ? void 0 : options.attributes
        });
        yield writer.write(text);
        // set text part of progress to 1
        handleProgress(1, 0);
        yield writer.close();
        info = writer.info;
      }
      if ((options === null || options === void 0 ? void 0 : options.attachments) && fileIds) {
        yield Promise.all(options.attachments.map((file, idx) => __awaiter(this, void 0, void 0, function* () {
          return this._sendFile(fileIds[idx], file, {
            topic: options.topic,
            mimeType: file.type,
            destinationIdentities: options.destinationIdentities,
            compress: options.compress,
            onProgress: progress => {
              handleProgress(progress, idx + 1);
            }
          });
        })));
      }
      return info;
    });
  }
  /**
   * Sends a complete in-memory byte payload. Mirrors {@link sendText}'s semantics: when every
   * recipient supports data streams v2 the payload rides inline in a single header packet
   * (optionally deflate-raw compressed), otherwise it is sent as a (optionally compressed)
   * chunked byte stream. Unlike {@link sendFile}, the whole payload is already in memory, so the
   * inline single-packet fast path applies.
   */
  sendBytes(bytes, options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e, _f, _g;
      const streamId = crypto.randomUUID();
      const destinationIdentities = options === null || options === void 0 ? void 0 : options.destinationIdentities;
      const compress = (_a = options === null || options === void 0 ? void 0 : options.compress) !== null && _a !== void 0 ? _a : true;
      const info = {
        id: streamId,
        name: (_b = options === null || options === void 0 ? void 0 : options.name) !== null && _b !== void 0 ? _b : 'unknown',
        mimeType: (_c = options === null || options === void 0 ? void 0 : options.mimeType) !== null && _c !== void 0 ? _c : 'application/octet-stream',
        timestamp: Date.now(),
        topic: (_d = options === null || options === void 0 ? void 0 : options.topic) !== null && _d !== void 0 ? _d : '',
        size: bytes.byteLength,
        // NOTE: size is always the pre-compression byte length
        attributes: options === null || options === void 0 ? void 0 : options.attributes,
        encryptionType: ((_e = this.engine.e2eeManager) === null || _e === void 0 ? void 0 : _e.isDataChannelEncryptionEnabled) ? Encryption_Type.GCM : Encryption_Type.NONE
      };
      const progressMonitorTap = progressReportingStream(bytes.length, options === null || options === void 0 ? void 0 : options.onProgress);
      const compressEligible = compress && isDeflateRawCompressionSupported() && this.allRecipientsSupportV2(destinationIdentities) && this.allRecipientsSupportCompression(destinationIdentities);
      let compressedStream = compressEligible ? MaybeCollectedStream.fromStream(readableFromBytes(bytes).pipeThrough(progressMonitorTap).pipeThrough(deflateRawTransform())) : null;
      // Phase 1: Try to send as a single packet data stream
      if (this.allRecipientsSupportV2(destinationIdentities)) {
        // The payload rides in the header's `inlineContent` (raw bytes). Keep the compressed form only
        // if it actually shrinks the payload (deflate framing makes tiny payloads larger). The
        // compression flag is carried in the header's `compression` field; user attributes are left
        // untouched.
        let inlineContent = bytes;
        let compression = DataStream_CompressionType.NONE;
        if (compressedStream) {
          const collectedBytes = yield compressedStream.collect();
          if (collectedBytes.byteLength < bytes.byteLength) {
            inlineContent = collectedBytes;
            compression = DataStream_CompressionType.DEFLATE_RAW;
          }
        }
        const header = buildByteStreamHeader(info, {
          compression,
          inlineContent
        });
        const packet = createStreamHeaderPacket(header, destinationIdentities);
        if (packet.toBinary().byteLength <= STREAM_CHUNK_SIZE_BYTES) {
          yield this.engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
          (_f = options === null || options === void 0 ? void 0 : options.onProgress) === null || _f === void 0 ? void 0 : _f.call(options, 1);
          return info;
        }
      }
      // Phase 2/3: header + (optionally compressed) chunk packets + trailer.
      const header = buildByteStreamHeader(info, {
        compression: compressedStream ? DataStream_CompressionType.DEFLATE_RAW : DataStream_CompressionType.NONE
      });
      const packet = createStreamHeaderPacket(header, destinationIdentities);
      const source = compressedStream ? compressedStream.stream() : readableFromBytes(bytes).pipeThrough(progressMonitorTap);
      yield this.sendChunkedByteStream(packet, streamId, destinationIdentities, source);
      // Ensure there's always a 100% progress event fired, even if the buffer is zero bytes long
      if (bytes.length === 0) {
        (_g = options === null || options === void 0 ? void 0 : options.onProgress) === null || _g === void 0 ? void 0 : _g.call(options, 1);
      }
      return info;
    });
  }
  /**
   * Returns true only if every recipient is known to support data streams v2 (single-packet inline
   * streams and compression). For a targeted send this checks the named destination identities; for
   * a broadcast (no explicit destinations) it checks every remote participant currently in the room.
   * An empty room (nobody to receive) is considered eligible.
   */
  allRecipientsSupportV2(destinationIdentities) {
    const identities = destinationIdentities && destinationIdentities.length > 0 ? destinationIdentities : this.getAllRemoteParticipantIdentities();
    return identities.every(identity => this.getRemoteParticipantClientProtocol(identity) >= CLIENT_PROTOCOL_DATA_STREAM_V2);
  }
  /**
   * Returns true only if every recipient advertises the deflate-raw compression capability (so it
   * can decompress a compressed stream). Resolved the same way as {@link allRecipientsSupportV2}:
   * named destinations, or every remote participant for a broadcast; an empty room is eligible.
   */
  allRecipientsSupportCompression(destinationIdentities) {
    const identities = destinationIdentities && destinationIdentities.length > 0 ? destinationIdentities : this.getAllRemoteParticipantIdentities();
    return identities.every(identity => this.getRemoteParticipantCapabilities(identity).includes(ClientInfo_Capability.CAP_COMPRESSION_DEFLATE_RAW));
  }
  /**
   * Shared chunked-stream send for `sendText`/`sendFile`: sends the prebuilt header packet, then
   * forwards `source` (optionally deflate-raw compressed) as `streamChunk` packets re-chunked to
   * the MTU budget with contiguous indices, then sends the trailer. The source is consumed
   * incrementally, so a `file.stream()` is never buffered in full. The platform compressor can't
   * flush mid-stream, so compression is only used when the whole payload is available as a stream
   * up front (not for incremental writers like `streamText`/`streamBytes`).
   */
  sendChunkedByteStream(headerPacket, streamId, destinationIdentities, source) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, e_1, _b, _c;
      const engine = this.engine;
      yield sendHeaderPacket(engine, headerPacket);
      let chunkId = 0;
      try {
        for (var _d = true, _e = __asyncValues(readBytesInChunks(source, STREAM_CHUNK_SIZE_BYTES)), _f; _f = yield _e.next(), _a = _f.done, !_a; _d = true) {
          _c = _f.value;
          _d = false;
          const chunk = _c;
          const chunkPacket = new DataPacket({
            destinationIdentities,
            value: {
              case: 'streamChunk',
              value: new DataStream_Chunk({
                content: chunk,
                streamId,
                chunkIndex: numberToBigInt(chunkId)
              })
            }
          });
          yield engine.sendDataPacket(chunkPacket, DataChannelKind.RELIABLE);
          chunkId += 1;
        }
      } catch (e_1_1) {
        e_1 = {
          error: e_1_1
        };
      } finally {
        try {
          if (!_d && !_a && (_b = _e.return)) yield _b.call(_e);
        } finally {
          if (e_1) throw e_1.error;
        }
      }
      yield sendStreamTrailer(streamId, destinationIdentities, engine);
    });
  }
  /**
   * @internal
   */
  streamText(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      const streamId = (_a = options === null || options === void 0 ? void 0 : options.streamId) !== null && _a !== void 0 ? _a : crypto.randomUUID();
      const destinationIdentities = options === null || options === void 0 ? void 0 : options.destinationIdentities;
      const info = {
        id: streamId,
        mimeType: 'text/plain',
        timestamp: Date.now(),
        topic: (_b = options === null || options === void 0 ? void 0 : options.topic) !== null && _b !== void 0 ? _b : '',
        size: options === null || options === void 0 ? void 0 : options.totalSize,
        attributes: options === null || options === void 0 ? void 0 : options.attributes,
        encryptionType: ((_c = this.engine.e2eeManager) === null || _c === void 0 ? void 0 : _c.isDataChannelEncryptionEnabled) ? Encryption_Type.GCM : Encryption_Type.NONE,
        attachedStreamIds: options === null || options === void 0 ? void 0 : options.attachedStreamIds
      };
      const header = buildTextStreamHeader(info, options);
      const packet = createStreamHeaderPacket(header, destinationIdentities);
      yield sendHeaderPacket(this.engine, packet);
      let chunkId = 0;
      const engine = this.engine;
      // Incremental text streams are never compressed (CompressionStream does not support flushing
      // mid-stream); one-shot compression lives in sendText.
      //
      // Note that a future streamText could send a context-takeover style deflate-raw stream with
      // intermedia explicit `Z_SYNC_FLUSH`s - receivers already will handle this properly today.
      const writableStream = new WritableStream({
        write(text) {
          return __awaiter(this, void 0, void 0, function* () {
            for (const textByteChunk of splitUtf8(text, STREAM_CHUNK_SIZE_BYTES)) {
              const chunk = new DataStream_Chunk({
                content: textByteChunk,
                streamId,
                chunkIndex: numberToBigInt(chunkId)
              });
              const chunkPacket = new DataPacket({
                destinationIdentities,
                value: {
                  case: 'streamChunk',
                  value: chunk
                }
              });
              yield engine.sendDataPacket(chunkPacket, DataChannelKind.RELIABLE);
              chunkId += 1;
            }
          });
        },
        close() {
          return __awaiter(this, void 0, void 0, function* () {
            yield sendStreamTrailer(streamId, destinationIdentities, engine);
          });
        },
        abort(err) {
          console.log('Sink error:', err);
          // TODO handle aborts to signal something to receiver side
        }
      });
      let onEngineClose = () => __awaiter(this, void 0, void 0, function* () {
        yield writer.close();
      });
      // FIXME: make this a global event to ensure "max listener" warning won't get logged for lots of
      // in flight data streams.
      engine.once(EngineEvent.Closing, onEngineClose);
      const writer = new TextStreamWriter(writableStream, info, () => this.engine.off(EngineEvent.Closing, onEngineClose));
      return writer;
    });
  }
  sendFile(file, options) {
    return __awaiter(this, void 0, void 0, function* () {
      const streamId = crypto.randomUUID();
      yield this._sendFile(streamId, file, options);
      return {
        id: streamId
      };
    });
  }
  /**
   * Streams a file as a chunked byte stream, compressed (deflate-raw) when the runtime supports it
   * and every recipient is on data streams v2. The file is piped `file.stream()` →
   * (`CompressionStream`) → chunk packets via {@link sendChunkedByteStream}, so it is never fully
   * buffered in memory — unlike {@link sendBytes}, there is no inline single-packet fast path for
   * files (the compressed size can't be known up front without buffering the whole file).
   */
  _sendFile(streamId, file, options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e;
      const destinationIdentities = options === null || options === void 0 ? void 0 : options.destinationIdentities;
      const compress = ((_a = options === null || options === void 0 ? void 0 : options.compress) !== null && _a !== void 0 ? _a : true) && isDeflateRawCompressionSupported() && this.allRecipientsSupportV2(destinationIdentities) && this.allRecipientsSupportCompression(destinationIdentities);
      const info = {
        id: streamId,
        name: file.name,
        mimeType: (_b = options === null || options === void 0 ? void 0 : options.mimeType) !== null && _b !== void 0 ? _b : file.type,
        topic: (_c = options === null || options === void 0 ? void 0 : options.topic) !== null && _c !== void 0 ? _c : '',
        timestamp: Date.now(),
        size: file.size,
        encryptionType: ((_d = this.engine.e2eeManager) === null || _d === void 0 ? void 0 : _d.isDataChannelEncryptionEnabled) ? Encryption_Type.GCM : Encryption_Type.NONE
      };
      // Phase 1: Try to send as a single packet data stream
      //
      // This is not being done explictly for files, because it's challenging to determine ahead of
      // time how well the file contents will compress (and whether the total output will be under the
      // MTU). Revisit this in the future though.
      // Phase 2 (compressed) / Phase 3 (uncompressed fallback): header + chunk packets + trailer. Both
      // funnel through the shared sendChunkedByteStream primitive, differing only by whether the file
      // stream is wrapped in the deflate-raw compressor; the file is never fully buffered in memory.
      const header = buildByteStreamHeader(info, {
        compression: compress ? DataStream_CompressionType.DEFLATE_RAW : DataStream_CompressionType.NONE
      });
      const packet = createStreamHeaderPacket(header, destinationIdentities);
      const tapped = file.stream().pipeThrough(progressReportingStream(file.size, options === null || options === void 0 ? void 0 : options.onProgress));
      const source = compress ? tapped.pipeThrough(deflateRawTransform()) : tapped;
      yield this.sendChunkedByteStream(packet, streamId, destinationIdentities, source);
      // Ensure there's always a 100% progress event fired, even if the file is zero bytes long
      if (file.size === 0) {
        (_e = options === null || options === void 0 ? void 0 : options.onProgress) === null || _e === void 0 ? void 0 : _e.call(options, 1);
      }
      return info;
    });
  }
  streamBytes(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c, _d, _e;
      const streamId = (_a = options === null || options === void 0 ? void 0 : options.streamId) !== null && _a !== void 0 ? _a : crypto.randomUUID();
      const destinationIdentities = options === null || options === void 0 ? void 0 : options.destinationIdentities;
      const info = {
        id: streamId,
        mimeType: (_b = options === null || options === void 0 ? void 0 : options.mimeType) !== null && _b !== void 0 ? _b : 'application/octet-stream',
        topic: (_c = options === null || options === void 0 ? void 0 : options.topic) !== null && _c !== void 0 ? _c : '',
        timestamp: Date.now(),
        attributes: options === null || options === void 0 ? void 0 : options.attributes,
        size: options === null || options === void 0 ? void 0 : options.totalSize,
        name: (_d = options === null || options === void 0 ? void 0 : options.name) !== null && _d !== void 0 ? _d : 'unknown',
        encryptionType: ((_e = this.engine.e2eeManager) === null || _e === void 0 ? void 0 : _e.isDataChannelEncryptionEnabled) ? Encryption_Type.GCM : Encryption_Type.NONE
      };
      const header = buildByteStreamHeader(info);
      const packet = createStreamHeaderPacket(header, destinationIdentities);
      yield sendHeaderPacket(this.engine, packet);
      let chunkId = 0;
      const writeMutex = new _();
      const engine = this.engine;
      const logLocal = this.log;
      // Incremental byte streams are never compressed (CompressionStream does not support flushing
      // mid-stream); one-shot compression lives in sendFile.
      //
      // Note that a future streamBytes could send a context-takeover style deflate-raw stream with
      // intermedia explicit `Z_SYNC_FLUSH`s - receivers already will handle this properly today.
      const writableStream = new WritableStream({
        write(chunk) {
          return __awaiter(this, void 0, void 0, function* () {
            const unlock = yield writeMutex.lock();
            let byteOffset = 0;
            try {
              while (byteOffset < chunk.byteLength) {
                const subChunk = chunk.slice(byteOffset, byteOffset + STREAM_CHUNK_SIZE_BYTES);
                const chunkPacket = new DataPacket({
                  destinationIdentities,
                  value: {
                    case: 'streamChunk',
                    value: new DataStream_Chunk({
                      content: subChunk,
                      streamId,
                      chunkIndex: numberToBigInt(chunkId)
                    })
                  }
                });
                yield engine.sendDataPacket(chunkPacket, DataChannelKind.RELIABLE);
                chunkId += 1;
                byteOffset += subChunk.byteLength;
              }
            } finally {
              unlock();
            }
          });
        },
        close() {
          return __awaiter(this, void 0, void 0, function* () {
            yield sendStreamTrailer(streamId, destinationIdentities, engine);
          });
        },
        abort(err) {
          logLocal.error('Sink error:', err);
        }
      });
      const byteWriter = new ByteStreamWriter(writableStream, info);
      return byteWriter;
    });
  }
}
/**
 * Wraps a stream of compressed bytes and defers buffering it into memory until (and only if) the
 * full length is actually needed.
 *
 * The ideal way to send a compressed stream is incrementally: compress each chunk and send it as
 * soon as it's ready, without ever holding the whole payload in memory. But to decide whether a
 * payload is small enough to fit in a single-packet data stream, we have to know the compressed
 * length — and the only way to learn that is to compress everything and add up the bytes. That
 * forces us to buffer the entire compressed output before we can make the decision.
 *
 * This class lets us have it both ways. It holds the compressed stream unread by default and only
 * collects it into memory ({@link collect}) when a caller needs the length for the single-packet
 * size check. If that check never happens, the stream is instead {@link stream|passed straight
 * through} to the downstream consumer without ever being fully buffered. Once collected, later
 * calls reuse the buffered bytes rather than re-collecting.
 */
class MaybeCollectedStream {
  constructor(state) {
    this.state = state;
  }
  static fromStream(stream) {
    return new MaybeCollectedStream({
      type: 'stream',
      stream
    });
  }
  /** Collect data from the stream into memory and return as a Uint8Array. */
  collect() {
    return __awaiter(this, void 0, void 0, function* () {
      switch (this.state.type) {
        case 'stream':
          const bytes = yield collect(this.state.stream);
          this.state = {
            type: 'collected',
            bytes
          };
          return bytes;
        case 'collected':
          return this.state.bytes;
      }
    });
  }
  /** Pass wrapped stream through to downstream consumer. */
  stream() {
    switch (this.state.type) {
      case 'stream':
        return this.state.stream;
      case 'collected':
        return readableFromBytes(this.state.bytes);
    }
  }
}
/**
 * A pass-through byte transform that reports progress as bytes flow through it, measured against
 * `totalLength`. Used as a "tap" to instrument the source of a chunked send without threading a
 * progress callback through the send primitive.
 *
 * IMPORTANT: This should be upstream of any compression step.
 *
 * Emits nothing when the total is unknown or zero.
 */
function progressReportingStream(totalPreCompressionLength, onProgress) {
  let sent = 0;
  return new TransformStream({
    transform(chunk, controller) {
      sent += chunk.byteLength;
      if (onProgress && typeof totalPreCompressionLength === 'number' && totalPreCompressionLength > 0) {
        onProgress(Math.min(sent / totalPreCompressionLength, 1));
      }
      controller.enqueue(chunk);
    }
  });
}
/**
 * Sends a stream `streamHeader` packet, enforcing that it fits the MTU budget. The header carries
 * the user attributes (plus topic/framing), and a single `DataPacket` larger than the MTU can't be
 * reliably sent — so an oversized header (almost always due to large attributes) is a hard error
 * rather than a malformed packet on the wire. The inline fast path does its own size check and
 * falls back to the chunked path instead of calling this.
 */
function sendHeaderPacket(engine, packet) {
  return __awaiter(this, void 0, void 0, function* () {
    if (packet.toBinary().byteLength > STREAM_CHUNK_SIZE_BYTES) {
      throw new DataStreamError("data stream header exceeds the ".concat(STREAM_CHUNK_SIZE_BYTES, "-byte limit; reduce attribute size"), DataStreamErrorReason.HeaderTooLarge);
    }
    yield engine.sendDataPacket(packet, DataChannelKind.RELIABLE);
  });
}
export { BaseStreamReader, BaseStreamWriter, ByteStreamReader, ByteStreamWriter, DEFAULT_MAX_PAYLOAD_BYTE_LENGTH, IncomingDataStreamManager, MaybeCollectedStream, OutgoingDataStreamManager, RegionUrlProvider, STREAM_CHUNK_SIZE_BYTES, TextStreamReader, TextStreamWriter, buildByteStreamHeader, buildTextStreamHeader, bytesToChunks, bytesToDecodedUtf8, chunksToBytes, collect, createInlineStream, createStreamHeaderPacket, deflateRawDecompress, deflateRawTransform, ensureOrderedChunks, getCloudConfigUrl, inflateRawByteChunkStream, inflateRawChunkStream, inflateRawTransform, maxDecompressedLengthGuard, progressReportingStream, sendHeaderPacket, textEncoder };
