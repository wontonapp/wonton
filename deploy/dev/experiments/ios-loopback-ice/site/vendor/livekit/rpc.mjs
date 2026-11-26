// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 30733-31524 行按职责切出，行为不变。
/**
 * Specialized error handling for RPC methods.
 *
 * Instances of this type, when thrown in a method handler, will have their `message`
 * serialized and sent across the wire. The sender will receive an equivalent error on the other side.
 *
 * Built-in types are included but developers may use any string, with a max length of 256 bytes.
 */
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { DataPacket, DataPacket_Kind } from './protobuf-messages-room.mjs';
import { RpcAck, RpcError$1, RpcRequest, RpcResponse } from './protobuf-messages-chat.mjs';
import { eventsExports } from './events.mjs';
import { CLIENT_PROTOCOL_DATA_STREAM_RPC } from './browser.mjs';
import { Track, isReactNative, supportsSetSinkId } from './track.mjs';
import { Future, compareVersions } from './utils-misc.mjs';
import { computeBitrate } from './e2ee-frame.mjs';
import { RemoteTrack } from './remote-track.mjs';

class RpcError extends Error {
  /**
   * Creates an error object with the given code and message, plus an optional data payload.
   *
   * If thrown in an RPC method handler, the error will be sent back to the caller.
   *
   * Error codes 1001-1999 are reserved for built-in errors (see RpcError.ErrorCode for their meanings).
   */
  constructor(code, message, data, options) {
    super(message);
    this.code = code;
    this.message = truncateBytes(message, RpcError.MAX_MESSAGE_BYTES);
    this.data = data ? truncateBytes(data, RpcError.MAX_DATA_BYTES) : undefined;
    if (typeof (options === null || options === void 0 ? void 0 : options.cause) !== 'undefined') {
      this.cause = options === null || options === void 0 ? void 0 : options.cause;
    }
  }
  /**
   * @internal
   */
  static fromProto(proto) {
    return new RpcError(proto.code, proto.message, proto.data);
  }
  /**
   * @internal
   */
  toProto() {
    return new RpcError$1({
      code: this.code,
      message: this.message,
      data: this.data
    });
  }
  /**
   * Creates an error object from the code, with an auto-populated message.
   *
   * @internal
   */
  static builtIn(key, data, options) {
    return new RpcError(RpcError.ErrorCode[key], RpcError.ErrorMessage[key], data, options);
  }
}
RpcError.MAX_MESSAGE_BYTES = 256;
RpcError.MAX_DATA_BYTES = 15360; // 15 KB
RpcError.ErrorCode = {
  APPLICATION_ERROR: 1500,
  CONNECTION_TIMEOUT: 1501,
  RESPONSE_TIMEOUT: 1502,
  RECIPIENT_DISCONNECTED: 1503,
  RESPONSE_PAYLOAD_TOO_LARGE: 1504,
  SEND_FAILED: 1505,
  UNSUPPORTED_METHOD: 1400,
  RECIPIENT_NOT_FOUND: 1401,
  REQUEST_PAYLOAD_TOO_LARGE: 1402,
  UNSUPPORTED_SERVER: 1403,
  UNSUPPORTED_VERSION: 1404
};
/**
 * @internal
 */
RpcError.ErrorMessage = {
  APPLICATION_ERROR: 'Application error in method handler',
  CONNECTION_TIMEOUT: 'Connection timeout',
  RESPONSE_TIMEOUT: 'Response timeout',
  RECIPIENT_DISCONNECTED: 'Recipient disconnected',
  RESPONSE_PAYLOAD_TOO_LARGE: 'Response payload too large',
  SEND_FAILED: 'Failed to send',
  UNSUPPORTED_METHOD: 'Method not supported at destination',
  RECIPIENT_NOT_FOUND: 'Recipient not found',
  REQUEST_PAYLOAD_TOO_LARGE: 'Request payload too large',
  UNSUPPORTED_SERVER: 'RPC not supported by server',
  UNSUPPORTED_VERSION: 'Unsupported RPC version'
};
/*
 * Maximum payload size for RPC requests and responses for clients with a clientProtocol of less
 * than CLIENT_PROTOCOL_DATA_STREAM_RPC.
 *
 * If a payload exceeds this size and the remote client does not support compression,
 * the RPC call will fail with a REQUEST_PAYLOAD_TOO_LARGE(1402) or RESPONSE_PAYLOAD_TOO_LARGE(1504) error.
 */
const MAX_V1_PAYLOAD_BYTES = 15360; // 15 KB
/**
 * Topic used for v2 RPC request data streams.
 * @internal
 */
const RPC_REQUEST_DATA_STREAM_TOPIC = 'lk.rpc_request';
/**
 * Topic used for v2 RPC response data streams.
 * @internal
 */
const RPC_RESPONSE_DATA_STREAM_TOPIC = 'lk.rpc_response';
/** @internal */
var RpcRequestAttrs;
(function (RpcRequestAttrs) {
  RpcRequestAttrs["RPC_REQUEST_ID"] = "lk.rpc_request_id";
  RpcRequestAttrs["RPC_REQUEST_METHOD"] = "lk.rpc_request_method";
  RpcRequestAttrs["RPC_REQUEST_RESPONSE_TIMEOUT_MS"] = "lk.rpc_request_response_timeout_ms";
  RpcRequestAttrs["RPC_REQUEST_VERSION"] = "lk.rpc_request_version";
})(RpcRequestAttrs || (RpcRequestAttrs = {}));
/** Initial version of rpc which uses RpcRequest / RpcResponse messages.
 * @internal
 **/
const RPC_VERSION_V1 = 1;
/** Rpc version backed by data streams instead of RpcRequest / RpcResponse.
 * @internal
 **/
const RPC_VERSION_V2 = 2;
/**
 * @internal
 */
function byteLength(str) {
  const encoder = new TextEncoder();
  return encoder.encode(str).length;
}
/**
 * @internal
 */
function truncateBytes(str, maxBytes) {
  if (byteLength(str) <= maxBytes) {
    return str;
  }
  let low = 0;
  let high = str.length;
  const encoder = new TextEncoder();
  while (low < high) {
    const mid = Math.floor((low + high + 1) / 2);
    if (encoder.encode(str.slice(0, mid)).length <= maxBytes) {
      low = mid;
    } else {
      high = mid - 1;
    }
  }
  // Never end on the first half of a surrogate pair: it would be encoded as a replacement
  // character, and the truncated string would no longer round trip.
  const lastCode = low > 0 ? str.charCodeAt(low - 1) : 0;
  if (lastCode >= 0xd800 && lastCode <= 0xdbff) {
    low -= 1;
  }
  return str.slice(0, low);
}/**
 * Manages the client (caller) side of RPC: sending requests, tracking pending
 * ack/response state, and handling incoming ack/response packets.
 * @internal
 */
class RpcClientManager extends eventsExports.EventEmitter {
  constructor(log, outgoingDataStreamManager, getRemoteParticipantClientProtocol, getServerVersion) {
    super();
    this.pendingAcks = new Map();
    this.pendingResponses = new Map();
    this.log = log;
    this.outgoingDataStreamManager = outgoingDataStreamManager;
    this.getRemoteParticipantClientProtocol = getRemoteParticipantClientProtocol;
    this.getServerVersion = getServerVersion;
  }
  performRpc(_a) {
    return __awaiter(this, arguments, void 0, function (_ref) {
      var _this = this;
      let destinationIdentity = _ref.destinationIdentity,
        method = _ref.method,
        payload = _ref.payload,
        _ref$responseTimeout = _ref.responseTimeout,
        responseTimeoutMs = _ref$responseTimeout === void 0 ? 15000 : _ref$responseTimeout;
      return function* () {
        const maxRoundTripLatencyMs = 7000;
        const minEffectiveTimeoutMs = maxRoundTripLatencyMs + 1000;
        const remoteClientProtocol = _this.getRemoteParticipantClientProtocol(destinationIdentity);
        const payloadBytes = byteLength(payload);
        // Only enforce the legacy size limit when on rpc v1
        if (payloadBytes > MAX_V1_PAYLOAD_BYTES && remoteClientProtocol < CLIENT_PROTOCOL_DATA_STREAM_RPC) {
          throw RpcError.builtIn('REQUEST_PAYLOAD_TOO_LARGE');
        }
        const serverVersion = _this.getServerVersion();
        if (serverVersion && compareVersions(serverVersion, '1.8.0') < 0) {
          throw RpcError.builtIn('UNSUPPORTED_SERVER');
        }
        const effectiveTimeoutMs = Math.max(responseTimeoutMs, minEffectiveTimeoutMs);
        const id = crypto.randomUUID();
        const completionFuture = new Future();
        let responseTimeoutId = null;
        const ackTimeoutId = setTimeout(() => {
          var _a;
          _this.pendingAcks.delete(id);
          (_a = completionFuture.reject) === null || _a === void 0 ? void 0 : _a.call(completionFuture, RpcError.builtIn('CONNECTION_TIMEOUT'));
          _this.pendingResponses.delete(id);
          if (responseTimeoutId !== null) {
            clearTimeout(responseTimeoutId);
          }
        }, maxRoundTripLatencyMs);
        _this.pendingAcks.set(id, {
          resolve: () => {
            clearTimeout(ackTimeoutId);
          },
          participantIdentity: destinationIdentity
        });
        _this.pendingResponses.set(id, {
          completionFuture,
          participantIdentity: destinationIdentity
        });
        // Observe the future before publishing: a slow send can outlast the ack timer.
        // Keep the returned promise rejected for callers, while marking its rejection
        // handled until performRpc has finished publishing and can return it.
        const completionPromise = completionFuture.promise.finally(() => {
          if (responseTimeoutId !== null) {
            clearTimeout(responseTimeoutId);
          }
          if (_this.pendingAcks.has(id)) {
            _this.log.warn('RPC response received before ack', id);
            _this.pendingAcks.delete(id);
            clearTimeout(ackTimeoutId);
          }
        });
        completionPromise.catch(() => {});
        try {
          yield _this.publishRpcRequest(destinationIdentity, id, method, payload, effectiveTimeoutMs, remoteClientProtocol);
        } catch (e) {
          // The caller receives the publish error. Do not leave an ack timer that
          // later rejects a future the caller never received.
          clearTimeout(ackTimeoutId);
          _this.pendingAcks.delete(id);
          _this.pendingResponses.delete(id);
          throw e;
        }
        if (_this.pendingResponses.has(id)) {
          responseTimeoutId = setTimeout(() => {
            var _a;
            _this.pendingResponses.delete(id);
            (_a = completionFuture.reject) === null || _a === void 0 ? void 0 : _a.call(completionFuture, RpcError.builtIn('RESPONSE_TIMEOUT'));
          }, responseTimeoutMs);
        }
        return [id, completionPromise];
      }();
    });
  }
  publishRpcRequest(destinationIdentity, requestId, method, payload, responseTimeout, remoteClientProtocol) {
    return __awaiter(this, void 0, void 0, function* () {
      if (remoteClientProtocol >= CLIENT_PROTOCOL_DATA_STREAM_RPC) {
        // Send payload as a data stream - a "version 2" rpc request.
        yield this.outgoingDataStreamManager.sendText(payload, {
          topic: RPC_REQUEST_DATA_STREAM_TOPIC,
          destinationIdentities: [destinationIdentity],
          attributes: {
            [RpcRequestAttrs.RPC_REQUEST_ID]: requestId,
            [RpcRequestAttrs.RPC_REQUEST_METHOD]: method,
            [RpcRequestAttrs.RPC_REQUEST_RESPONSE_TIMEOUT_MS]: "".concat(responseTimeout),
            [RpcRequestAttrs.RPC_REQUEST_VERSION]: "".concat(RPC_VERSION_V2)
          }
        });
        return;
      }
      // Fallback to sending a literal RpcRequest - a "version 1" rpc request.
      this.emit('sendDataPacket', {
        packet: new DataPacket({
          destinationIdentities: [destinationIdentity],
          kind: DataPacket_Kind.RELIABLE,
          value: {
            case: 'rpcRequest',
            value: new RpcRequest({
              id: requestId,
              method,
              payload,
              responseTimeoutMs: responseTimeout,
              version: RPC_VERSION_V1
            })
          }
        })
      });
    });
  }
  /**
   * Handle an incoming data stream containing an RPC response payload.
   * @internal
   */
  handleIncomingDataStream(reader, senderIdentity, attributes) {
    return __awaiter(this, void 0, void 0, function* () {
      const associatedRequestId = attributes[RpcRequestAttrs.RPC_REQUEST_ID];
      if (!associatedRequestId) {
        this.log.warn("RPC data stream malformed: ".concat(RpcRequestAttrs.RPC_REQUEST_ID, " not set."));
        // NOTE: no response can be sent here, because there's no request id so associate
        // so logging is the best we can do here.
        return;
      }
      const pending = this.pendingResponses.get(associatedRequestId);
      if (pending && pending.participantIdentity !== senderIdentity) {
        this.log.warn("RPC response stream for ".concat(associatedRequestId, " arrived from unexpected sender ").concat(senderIdentity, ", expected ").concat(pending.participantIdentity, ". Ignoring."));
        return;
      }
      let payload;
      try {
        payload = yield reader.readAll();
      } catch (e) {
        this.log.warn("Error reading RPC response payload: ".concat(e));
        this.handleIncomingRpcResponseFailure(associatedRequestId, RpcError.builtIn('APPLICATION_ERROR', 'Error reading RPC response payload', {
          cause: e
        }));
        return;
      }
      this.handleIncomingRpcResponseSuccess(associatedRequestId, payload);
    });
  }
  /** @internal */
  handleIncomingRpcResponseSuccess(requestId, payload) {
    var _a, _b;
    const handler = this.pendingResponses.get(requestId);
    if (handler) {
      (_b = (_a = handler.completionFuture).resolve) === null || _b === void 0 ? void 0 : _b.call(_a, payload);
      this.pendingResponses.delete(requestId);
    } else {
      this.log.error('Response received for unexpected RPC request', requestId);
    }
  }
  /** @internal */
  handleIncomingRpcResponseFailure(requestId, error) {
    var _a, _b;
    const handler = this.pendingResponses.get(requestId);
    if (handler) {
      (_b = (_a = handler.completionFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, error);
      this.pendingResponses.delete(requestId);
    } else {
      this.log.error('Response received for unexpected RPC request', requestId);
    }
  }
  /** @internal */
  handleIncomingRpcAck(requestId) {
    const handler = this.pendingAcks.get(requestId);
    if (handler) {
      handler.resolve();
      this.pendingAcks.delete(requestId);
    } else {
      this.log.error("Ack received for unexpected RPC request: ".concat(requestId));
    }
  }
  /** @internal */
  handleParticipantDisconnected(participantIdentity) {
    var _a;
    for (const _ref2 of this.pendingAcks) {
      var _ref3 = _slicedToArray(_ref2, 2);
      const id = _ref3[0];
      const pendingIdentity = _ref3[1].participantIdentity;
      if (pendingIdentity === participantIdentity) {
        this.pendingAcks.delete(id);
      }
    }
    for (const _ref4 of this.pendingResponses) {
      var _ref5 = _slicedToArray(_ref4, 2);
      const id = _ref5[0];
      var _ref5$ = _ref5[1];
      const pendingIdentity = _ref5$.participantIdentity;
      const completionFuture = _ref5$.completionFuture;
      if (pendingIdentity === participantIdentity) {
        (_a = completionFuture.reject) === null || _a === void 0 ? void 0 : _a.call(completionFuture, RpcError.builtIn('RECIPIENT_DISCONNECTED'));
        this.pendingResponses.delete(id);
      }
    }
  }
}/**
 * Manages the server (handler) side of RPC: processing incoming requests,
 * managing registered method handlers, and sending responses.
 * @internal
 */
class RpcServerManager extends eventsExports.EventEmitter {
  constructor(log, outgoingDataStreamManager, getRemoteParticipantClientProtocol) {
    super();
    this.rpcHandlers = new Map();
    this.log = log;
    this.outgoingDataStreamManager = outgoingDataStreamManager;
    this.getRemoteParticipantClientProtocol = getRemoteParticipantClientProtocol;
  }
  registerRpcMethod(method, handler) {
    if (this.rpcHandlers.has(method)) {
      throw Error("RPC handler already registered for method ".concat(method, ", unregisterRpcMethod before trying to register again"));
    }
    this.rpcHandlers.set(method, handler);
  }
  unregisterRpcMethod(method) {
    this.rpcHandlers.delete(method);
  }
  /**
   * Handle an incoming RPCRequest message containing a payload.
   * This handles "version 1" of rpc requests.
   * @internal
   */
  handleIncomingRpcRequest(callerIdentity, rpcRequest) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      this.publishRpcAck(callerIdentity, rpcRequest.id);
      if (rpcRequest.version !== 1) {
        this.publishRpcResponsePacket(callerIdentity, rpcRequest.id, null, RpcError.builtIn('UNSUPPORTED_VERSION'));
        return;
      }
      const handler = this.rpcHandlers.get(rpcRequest.method);
      if (!handler) {
        this.publishRpcResponsePacket(callerIdentity, rpcRequest.id, null, RpcError.builtIn('UNSUPPORTED_METHOD'));
        return;
      }
      let response;
      try {
        response = yield handler({
          requestId: rpcRequest.id,
          callerIdentity,
          payload: rpcRequest.payload,
          responseTimeout: rpcRequest.responseTimeoutMs
        });
      } catch (error) {
        let responseError;
        if (error instanceof RpcError) {
          responseError = error;
        } else {
          this.log.warn("Uncaught error returned by RPC handler for ".concat(rpcRequest.method, ". Returning APPLICATION_ERROR instead."), error);
          responseError = RpcError.builtIn('APPLICATION_ERROR', "Uncaught error: ".concat((_a = error === null || error === void 0 ? void 0 : error.message) !== null && _a !== void 0 ? _a : error), {
            cause: error
          });
        }
        this.publishRpcResponsePacket(callerIdentity, rpcRequest.id, null, responseError);
        return;
      }
      yield this.publishRpcResponse(callerIdentity, rpcRequest.id, response !== null && response !== void 0 ? response : '');
    });
  }
  /**
   * Handle an incoming data stream containing a RPC request payload.
   * This handles "version 2" of rpc requests.
   * @internal
   */
  handleIncomingDataStream(reader, callerIdentity, dataStreamAttrs) {
    return __awaiter(this, void 0, void 0, function* () {
      const requestId = dataStreamAttrs[RpcRequestAttrs.RPC_REQUEST_ID];
      const method = dataStreamAttrs[RpcRequestAttrs.RPC_REQUEST_METHOD];
      const responseTimeout = parseInt(dataStreamAttrs[RpcRequestAttrs.RPC_REQUEST_RESPONSE_TIMEOUT_MS], 10);
      const version = parseInt(dataStreamAttrs[RpcRequestAttrs.RPC_REQUEST_VERSION], 10);
      if (!requestId || !method || Number.isNaN(responseTimeout) || Number.isNaN(version)) {
        this.log.warn("RPC data stream malformed: ".concat(RpcRequestAttrs.RPC_REQUEST_ID, " / ").concat(RpcRequestAttrs.RPC_REQUEST_METHOD, " / ").concat(RpcRequestAttrs.RPC_REQUEST_RESPONSE_TIMEOUT_MS, " / ").concat(RpcRequestAttrs.RPC_REQUEST_VERSION, " not set."));
        this.publishRpcResponsePacket(callerIdentity, requestId, null, RpcError.builtIn('APPLICATION_ERROR', 'RPC data stream malformed'));
        return;
      }
      this.publishRpcAck(callerIdentity, requestId);
      if (version !== RPC_VERSION_V2) {
        this.publishRpcResponsePacket(callerIdentity, requestId, null, RpcError.builtIn('UNSUPPORTED_VERSION'));
        return;
      }
      let payload;
      try {
        payload = yield reader.readAll();
      } catch (e) {
        this.log.warn("Error reading RPC request payload: ".concat(e));
        this.publishRpcResponsePacket(callerIdentity, requestId, null, RpcError.builtIn('APPLICATION_ERROR', 'Error reading RPC request payload', {
          cause: e
        }));
        return;
      }
      const handler = this.rpcHandlers.get(method);
      if (!handler) {
        this.publishRpcResponsePacket(callerIdentity, requestId, null, RpcError.builtIn('UNSUPPORTED_METHOD'));
        return;
      }
      let response;
      try {
        response = yield handler({
          requestId,
          callerIdentity,
          payload,
          responseTimeout
        });
      } catch (error) {
        let responseError;
        if (error instanceof RpcError) {
          responseError = error;
        } else {
          this.log.warn("Uncaught error returned by RPC handler for ".concat(method, ". Returning APPLICATION_ERROR instead."), error);
          responseError = RpcError.builtIn('APPLICATION_ERROR');
        }
        this.publishRpcResponsePacket(callerIdentity, requestId, null, responseError);
        return;
      }
      yield this.publishRpcResponse(callerIdentity, requestId, response !== null && response !== void 0 ? response : '');
    });
  }
  publishRpcAck(destinationIdentity, requestId) {
    this.emit('sendDataPacket', {
      packet: new DataPacket({
        destinationIdentities: [destinationIdentity],
        kind: DataPacket_Kind.RELIABLE,
        value: {
          case: 'rpcAck',
          value: new RpcAck({
            requestId
          })
        }
      })
    });
  }
  publishRpcResponsePacket(destinationIdentity, requestId, payload, error) {
    this.emit('sendDataPacket', {
      packet: new DataPacket({
        destinationIdentities: [destinationIdentity],
        kind: DataPacket_Kind.RELIABLE,
        value: {
          case: 'rpcResponse',
          value: new RpcResponse({
            requestId,
            value: error ? {
              case: 'error',
              value: error.toProto()
            } : {
              case: 'payload',
              value: payload !== null && payload !== void 0 ? payload : ''
            }
          })
        }
      })
    });
  }
  /**
   * Send a successful RPC response payload, choosing the transport based on
   * the caller's client protocol version.
   */
  publishRpcResponse(destinationIdentity, requestId, payload) {
    return __awaiter(this, void 0, void 0, function* () {
      const callerClientProtocol = this.getRemoteParticipantClientProtocol(destinationIdentity);
      if (callerClientProtocol >= CLIENT_PROTOCOL_DATA_STREAM_RPC) {
        // Send response as a data stream
        yield this.outgoingDataStreamManager.sendText(payload, {
          topic: RPC_RESPONSE_DATA_STREAM_TOPIC,
          destinationIdentities: [destinationIdentity],
          attributes: {
            [RpcRequestAttrs.RPC_REQUEST_ID]: requestId
          }
        });
        return;
      }
      // Legacy client: enforce size limit and send uncompressed payload inline
      const responseBytes = byteLength(payload);
      if (responseBytes > MAX_V1_PAYLOAD_BYTES) {
        this.log.warn("RPC Response payload too large for request ".concat(requestId, ". To send larger responses, consider updating the sending client."));
        this.publishRpcResponsePacket(destinationIdentity, requestId, null, RpcError.builtIn('RESPONSE_PAYLOAD_TOO_LARGE'));
        return;
      }
      this.publishRpcResponsePacket(destinationIdentity, requestId, payload, null);
    });
  }
}class RemoteAudioTrack extends RemoteTrack {
  constructor(mediaTrack, sid, receiver, audioContext, audioOutput, loggerOptions) {
    super(mediaTrack, sid, Track.Kind.Audio, receiver, loggerOptions);
    /**
     * the nodes of the graph currently wired up, in order, so teardown can undo exactly the edges
     * `connectWebAudio` created. Reading `webAudioPluginNodes` instead would miss the old plugins
     * once `setWebAudioPlugins` has replaced them.
     */
    this.connectedNodes = [];
    this.monitorReceiver = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.receiver) {
        this._currentBitrate = 0;
        return;
      }
      const stats = yield this.getReceiverStats();
      if (stats && this.prevStats && this.receiver) {
        this._currentBitrate = computeBitrate(stats, this.prevStats);
      }
      this.prevStats = stats;
    });
    this.audioContext = audioContext;
    this.webAudioPluginNodes = [];
    if (audioOutput) {
      this.sinkId = audioOutput.deviceId;
    }
  }
  /**
   * sets the volume for all attached audio elements
   */
  setVolume(volume) {
    var _a;
    if (this.audioContext) {
      // playback goes through the web audio graph, the attached elements themselves stay silent
      (_a = this.gainNode) === null || _a === void 0 ? void 0 : _a.gain.setTargetAtTime(volume, 0, 0.1);
    } else {
      for (const el of this.attachedElements) {
        el.volume = volume;
      }
    }
    if (isReactNative()) {
      // @ts-ignore
      this._mediaStreamTrack._setVolume(volume);
    }
    this.elementVolume = volume;
  }
  /**
   * gets the volume of attached audio elements (loudest)
   */
  getVolume() {
    if (this.elementVolume !== undefined) {
      return this.elementVolume;
    }
    if (isReactNative()) {
      // RN volume value defaults to 1.0 if hasn't been changed.
      return 1.0;
    }
    let highestVolume = 0;
    this.attachedElements.forEach(element => {
      if (element.volume > highestVolume) {
        highestVolume = element.volume;
      }
    });
    return highestVolume;
  }
  /**
   * calls setSinkId on all attached elements, if supported
   * @param deviceId audio output device
   */
  setSinkId(deviceId) {
    return __awaiter(this, void 0, void 0, function* () {
      this.sinkId = deviceId;
      yield Promise.all(this.attachedElements.map(elm => {
        if (!supportsSetSinkId(elm)) {
          return;
        }
        /* @ts-ignore */
        return elm.setSinkId(deviceId);
      }));
    });
  }
  attach(element) {
    // the graph is fed by the track rather than by a specific element, so one connection covers
    // all attached elements
    const needsNewWebAudioConnection = !this.sourceNode;
    if (!element) {
      element = super.attach();
    } else {
      super.attach(element);
    }
    if (this.sinkId && supportsSetSinkId(element)) {
      element.setSinkId(this.sinkId).catch(e => {
        this.log.error('Failed to set sink id on remote audio track', e, this.logContext);
      });
    }
    if (this.audioContext) {
      // audio is routed through the web audio graph, so every attached element has to stay
      // silent, no matter whether it is the one the graph was built from
      element.volume = 0;
      element.muted = true;
      if (needsNewWebAudioConnection) {
        this.log.debug('using audio context mapping', this.logContext);
        this.connectWebAudio(this.audioContext, element);
      }
    }
    if (this.elementVolume !== undefined) {
      // make sure volume setting is being applied to the newly attached element
      this.setVolume(this.elementVolume);
    }
    return element;
  }
  detach(element) {
    let detached;
    if (!element) {
      detached = super.detach();
      this.disconnectWebAudio();
    } else {
      detached = super.detach(element);
      // if there are still any attached elements after detaching, connect webaudio to the first element that's left
      // disconnect webaudio otherwise
      if (this.audioContext) {
        if (this.attachedElements.length > 0) {
          this.connectWebAudio(this.audioContext, this.attachedElements[0]);
        } else {
          this.disconnectWebAudio();
        }
      }
    }
    return detached;
  }
  /**
   * @internal
   * @experimental
   */
  setAudioContext(audioContext) {
    var _a;
    if (this.audioContext === audioContext) {
      return;
    }
    this.audioContext = audioContext;
    if (audioContext) {
      // playback moves into the web audio graph, mute the elements to avoid double playback
      for (const element of this.attachedElements) {
        element.volume = 0;
        element.muted = true;
      }
      if (this.attachedElements.length > 0) {
        this.connectWebAudio(audioContext, this.attachedElements[0]);
      }
    } else {
      this.disconnectWebAudio();
      // playback moves back into the elements, which `attach` had muted in favour of the graph
      for (const element of this.attachedElements) {
        element.muted = false;
        element.volume = (_a = this.elementVolume) !== null && _a !== void 0 ? _a : 1;
      }
    }
  }
  /**
   * @internal
   * @experimental
   * @param {AudioNode[]} nodes - An array of WebAudio nodes. These nodes should not be connected to each other when passed, as the sdk will take care of connecting them in the order of the array.
   */
  setWebAudioPlugins(nodes) {
    this.webAudioPluginNodes = nodes;
    if (this.attachedElements.length > 0 && this.audioContext) {
      this.connectWebAudio(this.audioContext, this.attachedElements[0]);
    }
  }
  connectWebAudio(context, element) {
    this.disconnectWebAudio();
    // @ts-ignore attached elements always have a srcObject set
    this.sourceNode = context.createMediaStreamSource(element.srcObject);
    let lastNode = this.sourceNode;
    this.connectedNodes = [this.sourceNode];
    this.webAudioPluginNodes.forEach(node => {
      lastNode.connect(node);
      lastNode = node;
      this.connectedNodes.push(node);
    });
    this.gainNode = context.createGain();
    lastNode.connect(this.gainNode);
    this.connectedNodes.push(this.gainNode);
    this.gainNode.connect(context.destination);
    if (this.elementVolume !== undefined) {
      this.gainNode.gain.setTargetAtTime(this.elementVolume, 0, 0.1);
    }
    // try to resume the context if it isn't running already
    if (context.state !== 'running') {
      context.resume().then(() => {
        if (context.state !== 'running') {
          this.emit(TrackEvent.AudioPlaybackFailed, new Error("Audio Context couldn't be started automatically"));
        }
      }).catch(e => {
        this.emit(TrackEvent.AudioPlaybackFailed, e);
      });
    }
  }
  disconnectWebAudio() {
    var _a, _b;
    // the plugin nodes belong to whoever passed them in and may feed graphs of their own, so only
    // the edges this track created come down. A bare `node.disconnect()` would drop the
    // application's own routing with them
    for (let i = 0; i < this.connectedNodes.length - 1; i += 1) {
      try {
        this.connectedNodes[i].disconnect(this.connectedNodes[i + 1]);
      } catch (_c) {
        // a plugin node the application already disconnected itself, the rest of the chain still
        // has to come apart
      }
    }
    this.connectedNodes = [];
    // the source and gain nodes are ours alone, so every remaining edge of theirs is ours to drop
    (_a = this.gainNode) === null || _a === void 0 ? void 0 : _a.disconnect();
    (_b = this.sourceNode) === null || _b === void 0 ? void 0 : _b.disconnect();
    this.gainNode = undefined;
    this.sourceNode = undefined;
  }
  getReceiverStats() {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.receiver || !this.receiver.getStats) {
        return;
      }
      const stats = yield this.receiver.getStats();
      let receiverStats;
      stats.forEach(v => {
        if (v.type === 'inbound-rtp') {
          receiverStats = {
            type: 'audio',
            streamId: v.id,
            timestamp: v.timestamp,
            jitter: v.jitter,
            bytesReceived: v.bytesReceived,
            concealedSamples: v.concealedSamples,
            concealmentEvents: v.concealmentEvents,
            silentConcealedSamples: v.silentConcealedSamples,
            silentConcealmentEvents: v.silentConcealmentEvents,
            totalAudioEnergy: v.totalAudioEnergy,
            totalSamplesDuration: v.totalSamplesDuration
          };
        }
      });
      return receiverStats;
    });
  }
}
export { MAX_V1_PAYLOAD_BYTES, RPC_REQUEST_DATA_STREAM_TOPIC, RPC_RESPONSE_DATA_STREAM_TOPIC, RPC_VERSION_V1, RPC_VERSION_V2, RemoteAudioTrack, RpcClientManager, RpcError, RpcRequestAttrs, RpcServerManager, byteLength, truncateBytes };
