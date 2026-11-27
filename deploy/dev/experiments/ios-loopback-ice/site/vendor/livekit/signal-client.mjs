// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 18616-19813 行按职责切出，行为不变。
/**
 * [WebSocket](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket) with [Streams API](https://developer.mozilla.org/en-US/docs/Web/API/Streams_API)
 *
 * @see https://web.dev/websocketstream/
 */
import { _, e } from './protobuf-helpers.mjs';
import { protoInt64 } from './protobuf-int64.mjs';
import { __awaiter } from './tslib.mjs';
import { DisconnectReason } from './protobuf-messages-enums.mjs';
import { Encryption_Type } from './protobuf-messages-room.mjs';
import { LeaveRequest, LeaveRequest_Action, MuteTrackRequest, PublishDataTrackRequest, SignalRequest, TrickleRequest, UnpublishDataTrackRequest, UpdateDataSubscription, UpdateDataSubscription_Update, UpdateLocalAudioTrack, UpdateParticipantMetadata, UpdateVideoLayers } from './protobuf-messages-signal.mjs';
import { Ping, SubscriptionPermission } from './protobuf-messages-signal2.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { TypedPromise } from './browser.mjs';
import { ConnectionError, ConnectionErrorReason, CriticalTimers } from './errors.mjs';
import { sleep } from './track.mjs';
import { createRtcUrl, createValidateUrl, getAbortReasonAsString, getClientInfo, getErrorDescription, parseSignalResponse } from './utils-misc.mjs';
import { AsyncQueue, abortSignalTimeout, announceMachine, createSignalMachine } from './data-track-core.mjs';
import { createConnectionParams, createJoinRequestConnectionParams, toProtoSessionDescription } from './sdp-parser.mjs';

class WebSocketStream {
  get readyState() {
    return this.ws.readyState;
  }
  constructor(url) {
    let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    var _a, _b;
    if ((_a = options.signal) === null || _a === void 0 ? void 0 : _a.aborted) {
      throw new DOMException('This operation was aborted', 'AbortError');
    }
    this.url = url;
    const ws = new WebSocket(url, (_b = options.protocols) !== null && _b !== void 0 ? _b : []);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    const closeWithInfo = function () {
      let _ref = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {},
        code = _ref.closeCode,
        reason = _ref.reason;
      return ws.close(code, reason === undefined ? undefined : truncateCloseReason(reason));
    };
    this.opened = new TypedPromise((resolve, reject) => {
      const rejectHandler = () => {
        reject(ConnectionError.websocket('Encountered websocket error during connection establishment'));
      };
      ws.onopen = () => {
        resolve({
          readable: new ReadableStream({
            start(controller) {
              ws.onmessage = _ref2 => {
                let data = _ref2.data;
                return controller.enqueue(data);
              };
              ws.onerror = e => controller.error(ConnectionError.websocket(getErrorDescription(e, 'websocket')));
              ws.onclose = ev => {
                if (ev.wasClean || ev.code === 1000) {
                  controller.close();
                } else {
                  controller.error(ConnectionError.websocket("WS closed unexpectedly with code ".concat(ev.code)));
                }
              };
            },
            cancel: closeWithInfo
          }),
          writable: new WritableStream({
            write(chunk) {
              ws.send(chunk);
            },
            abort() {
              ws.close();
            },
            close: closeWithInfo
          }),
          protocol: ws.protocol,
          extensions: ws.extensions
        });
        ws.removeEventListener('error', rejectHandler);
      };
      ws.addEventListener('error', rejectHandler);
    });
    this.closed = new TypedPromise((resolve, reject) => {
      const rejectHandler = () => __awaiter(this, void 0, void 0, function* () {
        const closePromise = new TypedPromise(res => {
          if (ws.readyState === WebSocket.CLOSED) return;else {
            ws.addEventListener('close', closeEv => {
              res(closeEv);
            }, {
              once: true
            });
          }
        });
        const reason = yield TypedPromise.race([sleep(250), closePromise]);
        if (!reason) {
          reject(ConnectionError.websocket('Encountered unspecified websocket error without a timely close event'));
        } else {
          // if we can infer the close reason from the close event then resolve the promise, we don't need to throw
          resolve(reason);
        }
      });
      ws.addEventListener('close', _ref3 => {
        let code = _ref3.code,
          reason = _ref3.reason;
        resolve({
          closeCode: code,
          reason
        });
        ws.removeEventListener('error', rejectHandler);
      });
      ws.addEventListener('error', rejectHandler);
    });
    if (options.signal) {
      options.signal.onabort = () => ws.close();
    }
    this.close = closeWithInfo;
  }
}
/** `WebSocket.close` throws a `SyntaxError` for a reason longer than this many UTF-8 bytes. */
const MAX_CLOSE_REASON_BYTES = 123;
function truncateCloseReason(reason) {
  const encoder = new TextEncoder();
  if (encoder.encode(reason).length <= MAX_CLOSE_REASON_BYTES) {
    return reason;
  }
  // cut on code point boundaries, so a multi-byte character is never split
  let truncated = '';
  let bytes = 0;
  for (const char of reason) {
    bytes += encoder.encode(char).length;
    if (bytes > MAX_CLOSE_REASON_BYTES) {
      break;
    }
    truncated += char;
  }
  return truncated;
}const passThroughQueueSignals = ['syncState', 'trickle', 'offer', 'answer', 'simulate', 'leave'];
/**
 * Whether a failed resume may be followed by another one. Mirrors how `RTCEngine` classifies these
 * errors: a server leave ends the session, and an expired token cannot be recovered by retrying.
 */
function isRecoverableReconnectError(error) {
  if (error instanceof ConnectionError) {
    return error.reason !== ConnectionErrorReason.LeaveRequest && error.reason !== ConnectionErrorReason.NotAllowed;
  }
  return true;
}
function canPassThroughQueue(req) {
  const canPass = passThroughQueueSignals.indexOf(req.case) >= 0;
  livekitLogger.trace('request allowed to bypass queue:', {
    canPass,
    req
  });
  return canPass;
}
var SignalConnectionState;
(function (SignalConnectionState) {
  SignalConnectionState[SignalConnectionState["CONNECTING"] = 0] = "CONNECTING";
  SignalConnectionState[SignalConnectionState["CONNECTED"] = 1] = "CONNECTED";
  SignalConnectionState[SignalConnectionState["RECONNECTING"] = 2] = "RECONNECTING";
  SignalConnectionState[SignalConnectionState["DISCONNECTING"] = 3] = "DISCONNECTING";
  SignalConnectionState[SignalConnectionState["DISCONNECTED"] = 4] = "DISCONNECTED";
})(SignalConnectionState || (SignalConnectionState = {}));
/**
 * Public projection of the lifecycle machine's states. `new`, `offline` and `closed` are all
 * reported as `DISCONNECTED`: they differ in what may happen next
 */
function lifecycleToConnectionState(lifecycle) {
  switch (lifecycle) {
    case 'connected':
      return SignalConnectionState.CONNECTED;
    case 'connecting':
      return SignalConnectionState.CONNECTING;
    case 'reconnecting':
      return SignalConnectionState.RECONNECTING;
    case 'disconnecting':
      return SignalConnectionState.DISCONNECTING;
    default:
      return SignalConnectionState.DISCONNECTED;
  }
}
/** specifies how much time (in ms) we allow for the ws to close its connection gracefully before continuing */
const MAX_WS_CLOSE_TIME = 250;
/**
 * How long (in ms) to wait for the first message after the WebSocket upgrade.
 */
const JOIN_RESPONSE_TIMEOUT = 5000;
/** @internal */
class SignalClient {
  get currentState() {
    return lifecycleToConnectionState(this.lifecycleState);
  }
  get isDisconnected() {
    const state = this.currentState;
    return state === SignalConnectionState.DISCONNECTING || state === SignalConnectionState.DISCONNECTED;
  }
  /** Runtime lifecycle state, finer grained than the public {@link currentState} projection. */
  get lifecycleState() {
    return this.machine.currentState();
  }
  /** Id of the current connection attempt and of the transport it owns. */
  get attemptId() {
    return this.machine.context.attemptId;
  }
  /**
   * Applies a lifecycle input and reports whether it moved the machine
   */
  sendLifecycleInput(input) {
    const before = this.lifecycleState;
    this.machine.handle(input.type, input);
    return this.lifecycleState !== before;
  }
  /**
   * Waits out a close that is already in flight. Establishing a session while one is tearing down
   * would race it for the transport — the teardown can close the socket the new attempt just
   * opened.
   */
  settleInFlightClose() {
    return __awaiter(this, void 0, void 0, function* () {
      this.log.debug('waiting for an in-flight close to settle before establishing a session');
      (yield this.closingLock.lock())();
    });
  }
  get isEstablishingConnection() {
    return this.lifecycleState === 'connecting' || this.lifecycleState === 'reconnecting';
  }
  getNextRequestId() {
    this._requestId += 1;
    return this._requestId;
  }
  constructor() {
    let useJSON = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : false;
    let loggerOptions = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    var _a;
    /** signal rtt in milliseconds */
    this.rtt = 0;
    this.log = livekitLogger;
    this._requestId = 0;
    this.useV0SignalPath = false;
    /** @internal */
    this.resetCallbacks = () => {
      this.onAnswer = undefined;
      this.onLeave = undefined;
      this.onLocalTrackPublished = undefined;
      this.onLocalTrackUnpublished = undefined;
      this.onNegotiateRequested = undefined;
      this.onOffer = undefined;
      this.onRemoteMuteChanged = undefined;
      this.onSubscribedQualityUpdate = undefined;
      this.onTokenRefresh = undefined;
      this.onTrickle = undefined;
      this.onClose = undefined;
      this.onMediaSectionsRequirement = undefined;
    };
    this.loggerContextCb = loggerOptions.loggerContextCb;
    this.log = getLogger((_a = loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Signal, () => this.logContext);
    this.useJSON = useJSON;
    this.requestQueue = new AsyncQueue();
    this.queuedRequests = [];
    this.closingLock = new _();
    this.connectionLock = new _();
    this.machine = createSignalMachine();
    this.machine.on('transitioned', _ref => {
      let fromState = _ref.fromState,
        toState = _ref.toState;
      this.log.debug("signal lifecycle: ".concat(fromState, " -> ").concat(toState));
    });
    this.machine.on('nohandler', _ref2 => {
      let inputName = _ref2.inputName;
      this.log.debug("ignoring signal lifecycle input ".concat(inputName, " in state ").concat(this.lifecycleState));
    });
    // no-op unless a development tool asked for it; see utils/machineInspector
    announceMachine('signal', this.machine);
  }
  get logContext() {
    var _a, _b;
    return (_b = (_a = this.loggerContextCb) === null || _a === void 0 ? void 0 : _a.call(this)) !== null && _b !== void 0 ? _b : {};
  }
  join(url_1, token_1, opts_1, abortSignal_1) {
    return __awaiter(this, arguments, void 0, function (url, token, opts, abortSignal) {
      var _this = this;
      let useV0Path = arguments.length > 4 && arguments[4] !== undefined ? arguments[4] : false;
      let publisherOffer = arguments.length > 5 ? arguments[5] : undefined;
      return function* () {
        if (_this.lifecycleState === 'disconnecting') {
          yield _this.settleInFlightClose();
        }
        if (!_this.sendLifecycleInput({
          type: 'connect'
        })) {
          // Proceeding would open a transport the lifecycle does not own: its completion would be
          // discarded and the session left claiming a transport that had been torn down. Every caller
          // that restarts a session closes the previous one first (see RTCEngine.restartConnection).
          throw ConnectionError.internal("cannot establish a signal session from '".concat(_this.lifecycleState, "', close the current one first"));
        }
        _this.options = opts;
        try {
          const res = yield _this.connect(url, token, opts, abortSignal, useV0Path, publisherOffer);
          return res;
        } catch (e) {
          // reported here rather than at each rejection site inside connect(), so that no failure
          // path can leave the machine stuck in `connecting`
          _this.sendLifecycleInput({
            type: 'connectFailed',
            error: e
          });
          throw e;
        }
      }();
    });
  }
  reconnect(url, token, sid, reason) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.options) {
        this.log.warn('attempted to reconnect without signal options being set, ignoring');
        return;
      }
      if (this.lifecycleState === 'disconnecting') {
        yield this.settleInFlightClose();
      }
      if (!this.sendLifecycleInput({
        type: 'reconnect'
      })) {
        throw ConnectionError.internal("cannot resume the signal session from '".concat(this.lifecycleState, "'"));
      }
      // clear ping interval and restart it once reconnected
      this.clearPingInterval();
      try {
        const res = yield this.connect(url, token, Object.assign(Object.assign({}, this.options), {
          reconnect: true,
          sid,
          reconnectReason: reason
        }), undefined, this.useV0SignalPath);
        return res;
      } catch (e) {
        this.sendLifecycleInput({
          type: 'reconnectFailed',
          error: e,
          recoverable: isRecoverableReconnectError(e)
        });
        throw e;
      }
    });
  }
  connect(url_1, token_1, opts_1, abortSignal_1) {
    return __awaiter(this, arguments, void 0, function (url, token, opts, abortSignal) {
      var _this2 = this;
      let useV0Path = arguments.length > 4 && arguments[4] !== undefined ? arguments[4] : false;
      let publisherOffer = arguments.length > 5 ? arguments[5] : undefined;
      return function* () {
        const unlock = yield _this2.connectionLock.lock();
        let rtcUrl;
        let validateUrl;
        try {
          _this2.connectOptions = opts;
          _this2.useV0SignalPath = useV0Path;
          const clientInfo = getClientInfo(opts.clientInfoCapabilities);
          const params = useV0Path ? createConnectionParams(token, clientInfo, opts) : yield createJoinRequestConnectionParams(token, clientInfo, opts, publisherOffer);
          rtcUrl = createRtcUrl(url, params, useV0Path).toString();
          validateUrl = createValidateUrl(rtcUrl).toString();
        } catch (e) {
          // the executor below owns the lock from here on
          unlock();
          throw e;
        }
        if (abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.aborted) {
          unlock();
          _this2.log.debug('signal connect abandoned before opening a transport');
          throw ConnectionError.cancelled('Connection aborted');
        }
        return new Promise((resolve, reject) => __awaiter(_this2, void 0, void 0, function* () {
          var _a, _b, _c, _d, _e;
          try {
            let alreadyAborted = false;
            const abortHandler = eventOrError => __awaiter(this, void 0, void 0, function* () {
              if (alreadyAborted) {
                return;
              }
              alreadyAborted = true;
              const target = eventOrError instanceof Event ? eventOrError.currentTarget : eventOrError;
              const reason = getAbortReasonAsString(target, 'Abort handler called');
              // send leave if we have an active stream writer (connection is open)
              if (this.streamWriter && !this.isDisconnected) {
                this.sendLeave().then(() => this.close(true, reason)).catch(e => {
                  this.log.error(e);
                  this.close(true, reason);
                });
              } else {
                this.close(true, reason);
              }
              cleanupAbortHandlers();
              // The caller may already have classified this failure: the connect timeout hands us a
              // Timeout error. Only a genuine abort is a cancellation, and reporting a stalled connect as
              // one makes the engine read it as user intent — skipping region failover and never
              // recording the attempt against the backoff strategy (see Room.connect).
              reject(eventOrError instanceof ConnectionError ? eventOrError : ConnectionError.cancelled(reason));
            });
            abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.addEventListener('abort', abortHandler);
            const cleanupAbortHandlers = () => {
              clearTimeout(wsTimeout);
              abortSignal === null || abortSignal === void 0 ? void 0 : abortSignal.removeEventListener('abort', abortHandler);
            };
            const wsTimeout = setTimeout(() => {
              abortHandler(ConnectionError.timeout('room connection has timed out (signal)'));
            }, opts.websocketTimeout);
            const redactedUrl = new URL(rtcUrl);
            if (redactedUrl.searchParams.has('access_token')) {
              redactedUrl.searchParams.set('access_token', '<redacted>');
            }
            if (this.ws) {
              const startClose = performance.now();
              yield this.teardownTransport('replaced by a new connection attempt');
              this.log.debug("closed previous ws connection in ".concat(performance.now() - startClose, "ms"));
            }
            if (alreadyAborted) {
              this.log.debug('signal connect abandoned after tearing down the previous transport');
              return;
            }
            // the transport created below belongs to this attempt; events arriving from it after a
            // newer attempt has started are dropped by the machine
            const attemptId = this.attemptId;
            this.log.info("signal connecting to ".concat(redactedUrl), {
              reconnect: opts.reconnect,
              reconnectReason: opts.reconnectReason
            });
            this.ws = new WebSocketStream(rtcUrl);
            // A failed upgrade closes the socket, so `opened` and `closed` settle for the same cause — but
            // only the `opened` path can classify it (a 401 is known after asking the validate endpoint).
            // Noting the failure *before* the close handler is registered means it runs first, so a close
            // that merely reflects a failed upgrade stands down and lets the classified error reach the
            // caller. Any other close still fails the attempt straight away: after a successful upgrade
            // nothing else would, short of the first-message timeout.
            let upgradeFailed = false;
            this.ws.opened.catch(() => {
              upgradeFailed = true;
            });
            try {
              this.ws.closed.then(closeInfo => {
                if (this.isEstablishingConnection && !upgradeFailed) {
                  reject(ConnectionError.internal("Websocket got closed during a (re)connection attempt: ".concat(closeInfo.reason)));
                }
                this.log.debug('websocket closed', {
                  reason: closeInfo.reason,
                  code: closeInfo.closeCode,
                  attemptId,
                  state: this.lifecycleState
                });
                // Every close of the live transport is reported, including a clean 1000 one: a server
                // that drops signalling closes cleanly — a migration that never sends its
                // `Leave{action=RESUME}` does exactly that — and treating that as "nothing happened"
                // leaves the client believing it is still connected until something else notices.
                // Closes we caused ourselves, and closes from a transport that has since been
                // replaced, are dropped by the machine's state and attempt guards below rather than by
                // a close-code test.
                this.handleOnClose(closeInfo.reason || (closeInfo.closeCode === 1000 ? 'server closed the signal connection' : 'Unexpected WS error'), attemptId);
                return;
              }).catch(reason => {
                if (this.isEstablishingConnection && !upgradeFailed) {
                  reject(ConnectionError.internal("Websocket error during a (re)connection attempt: ".concat(reason)));
                }
              });
              const connection = yield this.ws.opened.catch(reason => __awaiter(this, void 0, void 0, function* () {
                if (alreadyAborted) {
                  // the abort handler has already rejected this attempt; don't probe the validate
                  // endpoint for a failure nobody is waiting on
                  this.log.debug('signal connect abandoned while the websocket was opening');
                  return;
                }
                if (this.lifecycleState !== 'connected') {
                  // claimed synchronously, before the await below: the socket's close event is already
                  // on its way and would otherwise reject first with a less useful error
                  clearTimeout(wsTimeout);
                  const error = yield this.handleConnectionError(reason, validateUrl);
                  reject(error);
                  return;
                }
                // other errors, handle
                this.handleWSError(reason);
                reject(reason);
                return;
              }));
              clearTimeout(wsTimeout);
              if (!connection) {
                return;
              }
              const signalReader = connection.readable.getReader();
              this.streamWriter = connection.writable.getWriter();
              // wsTimeout only guarded the upgrade; guard the first-message read with
              // its own deadline so a silent server can't hang join() forever.
              // During reconnect, drop any stray messages that arrive before the
              // ReconnectResponse so we do not declare the channel reconnected on the
              // wrong signal.
              let firstSignalResponse;
              try {
                const deadline = Date.now() + JOIN_RESPONSE_TIMEOUT;
                // eslint-disable-next-line no-constant-condition
                while (true) {
                  const remaining = deadline - Date.now();
                  if (remaining <= 0) {
                    throw ConnectionError.timeout('signal connection timed out while waiting for the first message');
                  }
                  let readTimeout;
                  const readResult = yield Promise.race([signalReader.read(), new Promise((_, rejectRead) => {
                    readTimeout = setTimeout(() => {
                      rejectRead(ConnectionError.timeout('signal connection timed out while waiting for the first message'));
                    }, remaining);
                  })]);
                  clearTimeout(readTimeout);
                  if (!readResult.value) {
                    throw ConnectionError.internal('no message received as first message');
                  }
                  const parsed = parseSignalResponse(readResult.value);
                  if (this.lifecycleState === 'reconnecting' && ((_a = parsed.message) === null || _a === void 0 ? void 0 : _a.case) !== 'reconnect' && ((_b = parsed.message) === null || _b === void 0 ? void 0 : _b.case) !== 'leave') {
                    this.log.warn('dropping signal message while awaiting reconnect response', {
                      messageCase: (_c = parsed.message) === null || _c === void 0 ? void 0 : _c.case
                    });
                    continue;
                  }
                  firstSignalResponse = parsed;
                  break;
                }
              } catch (e) {
                // No usable first message in time: release the reader and tear down the ws
                // so we surface the timeout instead of leaking an open connection.
                signalReader.releaseLock();
                reject(e);
                this.close();
                return;
              }
              signalReader.releaseLock();
              if (alreadyAborted) {
                // The abort handler has already rejected this attempt and is closing the transport.
                // Don't act on a join that raced it: `onJoined` would publish an abandoned session's
                // state to the room.
                this.log.debug('signal connect abandoned while waiting for the first message');
                return;
              }
              // Validate the first message
              const validation = this.validateFirstMessage(firstSignalResponse, (_d = opts.reconnect) !== null && _d !== void 0 ? _d : false);
              if (!validation.isValid) {
                reject(validation.error);
                return;
              }
              // Handle join response
              if (((_e = firstSignalResponse.message) === null || _e === void 0 ? void 0 : _e.case) === 'join') {
                // Set up ping configuration
                this.pingTimeoutDuration = firstSignalResponse.message.value.pingTimeout;
                this.pingIntervalDuration = firstSignalResponse.message.value.pingInterval;
                if (this.pingTimeoutDuration && this.pingTimeoutDuration > 0) {
                  this.log.debug('ping config', {
                    timeout: this.pingTimeoutDuration,
                    interval: this.pingIntervalDuration
                  });
                }
                if (this.onJoined) {
                  this.onJoined(firstSignalResponse.message.value);
                }
              }
              this.handleSignalConnected(connection, wsTimeout, attemptId);
              resolve(validation.response);
            } catch (e) {
              reject(e);
            } finally {
              cleanupAbortHandlers();
            }
          } finally {
            unlock();
          }
        }));
      }();
    });
  }
  startReadingLoop(signalReader) {
    return __awaiter(this, void 0, void 0, function* () {
      const attemptId = this.attemptId;
      while (true) {
        if (this.signalLatency) {
          yield sleep(this.signalLatency);
        }
        try {
          const _yield$signalReader$r = yield signalReader.read(),
            done = _yield$signalReader$r.done,
            value = _yield$signalReader$r.value;
          if (done) {
            break;
          }
          const resp = parseSignalResponse(value);
          this.handleSignalResponse(resp);
        } catch (e) {
          this.log.error("error reading from signal stream", {
            error: e
          });
          yield this.handleOnClose('error in reading loop', attemptId);
          break;
        }
      }
    });
  }
  close() {
    return __awaiter(this, arguments, void 0, function () {
      var _this3 = this;
      let updateState = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : true;
      let reason = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 'Close method called on signal client';
      return function* () {
        // when the lifecycle is already shutting down (or another close owns it), only the transport
        // teardown below still applies — it is idempotent, so callers can always await a close
        const drivesLifecycle = updateState && _this3.sendLifecycleInput({
          type: 'close',
          reason
        });
        const unlock = yield _this3.closingLock.lock();
        try {
          yield _this3.teardownTransport(reason);
        } finally {
          if (drivesLifecycle) {
            _this3.sendLifecycleInput({
              type: 'closeComplete'
            });
          }
          unlock();
        }
      }();
    });
  }
  /**
   * Releases the transport and everything tied to it, without touching the lifecycle state. Used
   * both by {@link close} and by paths that replace the transport under a live lifecycle (a new
   * attempt, or an unexpected close that leaves the client in `offline`).
   */
  teardownTransport(reason) {
    return __awaiter(this, void 0, void 0, function* () {
      try {
        this.clearPingInterval();
        const ws = this.ws;
        if (ws) {
          // released before closing, so a close that throws cannot leave the transport attached
          this.ws = undefined;
          this.streamWriter = undefined;
          ws.close({
            closeCode: 1000,
            reason
          });
          // calling `ws.close()` only starts the closing handshake (CLOSING state), prefer to wait until state is actually CLOSED
          yield Promise.race([ws.closed, sleep(MAX_WS_CLOSE_TIME)]);
        }
      } catch (e) {
        this.log.debug('websocket error while closing', {
          error: e
        });
      }
    });
  }
  // initial offer after joining
  sendOffer(offer, offerId) {
    this.log.debug('sending offer', {
      offerSdp: offer.sdp
    });
    this.sendRequest({
      case: 'offer',
      value: toProtoSessionDescription(offer, offerId)
    });
  }
  // answer a server-initiated offer
  sendAnswer(answer, offerId) {
    this.log.debug('sending answer', {
      answerSdp: answer.sdp
    });
    return this.sendRequest({
      case: 'answer',
      value: toProtoSessionDescription(answer, offerId)
    });
  }
  sendIceCandidate(candidate, target) {
    this.log.debug('sending ice candidate', {
      candidate
    });
    return this.sendRequest({
      case: 'trickle',
      value: new TrickleRequest({
        candidateInit: JSON.stringify(candidate),
        target
      })
    });
  }
  sendMuteTrack(trackSid, muted) {
    return this.sendRequest({
      case: 'mute',
      value: new MuteTrackRequest({
        sid: trackSid,
        muted
      })
    });
  }
  sendAddTrack(req) {
    return this.sendRequest({
      case: 'addTrack',
      value: req
    });
  }
  sendUpdateLocalMetadata(metadata_1, name_1) {
    return __awaiter(this, arguments, void 0, function (metadata, name) {
      var _this4 = this;
      let attributes = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : {};
      return function* () {
        const requestId = _this4.getNextRequestId();
        yield _this4.sendRequest({
          case: 'updateMetadata',
          value: new UpdateParticipantMetadata({
            requestId,
            metadata,
            name,
            attributes
          })
        });
        return requestId;
      }();
    });
  }
  sendUpdateTrackSettings(settings) {
    this.sendRequest({
      case: 'trackSetting',
      value: settings
    });
  }
  sendUpdateSubscription(sub) {
    return this.sendRequest({
      case: 'subscription',
      value: sub
    });
  }
  sendSyncState(sync) {
    return this.sendRequest({
      case: 'syncState',
      value: sync
    });
  }
  sendUpdateVideoLayers(trackSid, layers) {
    return this.sendRequest({
      case: 'updateLayers',
      value: new UpdateVideoLayers({
        trackSid,
        layers
      })
    });
  }
  sendUpdateSubscriptionPermissions(allParticipants, trackPermissions) {
    return this.sendRequest({
      case: 'subscriptionPermission',
      value: new SubscriptionPermission({
        allParticipants,
        trackPermissions
      })
    });
  }
  sendSimulateScenario(scenario) {
    return this.sendRequest({
      case: 'simulate',
      value: scenario
    });
  }
  sendPing() {
    /** send both of ping and pingReq for compatibility to old and new server */
    return Promise.all([this.sendRequest({
      case: 'ping',
      value: protoInt64.parse(Date.now())
    }), this.sendRequest({
      case: 'pingReq',
      value: new Ping({
        timestamp: protoInt64.parse(Date.now()),
        rtt: protoInt64.parse(this.rtt)
      })
    })]);
  }
  sendUpdateLocalAudioTrack(trackSid, features) {
    return this.sendRequest({
      case: 'updateAudioTrack',
      value: new UpdateLocalAudioTrack({
        trackSid,
        features
      })
    });
  }
  sendLeave() {
    return this.sendRequest({
      case: 'leave',
      value: new LeaveRequest({
        reason: DisconnectReason.CLIENT_INITIATED,
        // server doesn't process this field, keeping it here to indicate the intent of a full disconnect
        action: LeaveRequest_Action.DISCONNECT
      })
    });
  }
  sendPublishDataTrackRequest(handle, name, usesE2ee) {
    return this.sendRequest({
      case: 'publishDataTrackRequest',
      value: new PublishDataTrackRequest({
        pubHandle: handle,
        name: name,
        encryption: usesE2ee ? Encryption_Type.GCM : Encryption_Type.NONE
      })
    });
  }
  sendUnPublishDataTrackRequest(handle) {
    return this.sendRequest({
      case: 'unpublishDataTrackRequest',
      value: new UnpublishDataTrackRequest({
        pubHandle: handle
      })
    });
  }
  sendUpdateDataSubscription(sid, subscribe) {
    return this.sendRequest({
      case: 'updateDataSubscription',
      value: new UpdateDataSubscription({
        // FIXME: consider refactoring to allow caller to pass an array of events through
        updates: [new UpdateDataSubscription_Update({
          trackSid: sid,
          subscribe
        })]
      })
    });
  }
  sendRequest(message_1) {
    return __awaiter(this, arguments, void 0, function (message) {
      var _this5 = this;
      let fromQueue = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
      return function* () {
        // capture all requests while reconnecting and put them in a queue
        // unless the request originates from the queue, then don't enqueue again
        const canQueue = !fromQueue && !canPassThroughQueue(message);
        const isHoldingRequests = _this5.lifecycleState === 'reconnecting' || _this5.queuedRequests.length > 0;
        if (canQueue && isHoldingRequests) {
          _this5.queuedRequests.push(() => __awaiter(_this5, void 0, void 0, function* () {
            yield this.sendRequest(message, true);
          }));
          return;
        }
        // make sure previously queued requests are being sent first
        if (!fromQueue) {
          yield _this5.requestQueue.flush();
        }
        if (_this5.signalLatency) {
          yield sleep(_this5.signalLatency);
        }
        // `leave` is the one request whose purpose is to be sent on the way out (an aborted connect
        // attempt tells the server before tearing down), so it is allowed through for as long as the
        // transport is still there
        const isLeaveOnShutdown = message.case === 'leave' && !!_this5.streamWriter;
        if (_this5.isDisconnected && !isLeaveOnShutdown) {
          // Skip requests if the signal layer is disconnected
          // This can happen if an event is sent in the mist of room.connect() initializing
          _this5.log.debug("skipping signal request (type: ".concat(message.case, ") - SignalClient disconnected"));
          return;
        }
        if (!_this5.streamWriter) {
          _this5.log.error("cannot send signal request before connected, type: ".concat(message === null || message === void 0 ? void 0 : message.case));
          return;
        }
        const req = new SignalRequest({
          message
        });
        try {
          if (_this5.useJSON) {
            yield _this5.streamWriter.write(req.toJsonString());
          } else {
            yield _this5.streamWriter.write(req.toBinary().buffer);
          }
        } catch (e) {
          _this5.log.error('error sending signal message', {
            error: e
          });
        }
      }();
    });
  }
  handleSignalResponse(res) {
    var _a, _b;
    const msg = res.message;
    if (msg == undefined) {
      this.log.debug('received unsupported message');
      return;
    }
    let pingHandled = false;
    if (msg.case === 'answer') {
      const sd = fromProtoSessionDescription(msg.value);
      if (this.onAnswer) {
        this.onAnswer(sd, msg.value.id, msg.value.midToTrackId);
      }
    } else if (msg.case === 'offer') {
      const sd = fromProtoSessionDescription(msg.value);
      if (this.onOffer) {
        this.onOffer(sd, msg.value.id, msg.value.midToTrackId);
      }
    } else if (msg.case === 'trickle') {
      const candidate = JSON.parse(msg.value.candidateInit);
      if (this.onTrickle) {
        this.onTrickle(candidate, msg.value.target);
      }
    } else if (msg.case === 'update') {
      if (this.onParticipantUpdate) {
        this.onParticipantUpdate((_a = msg.value.participants) !== null && _a !== void 0 ? _a : []);
      }
    } else if (msg.case === 'trackPublished') {
      if (this.onLocalTrackPublished) {
        this.onLocalTrackPublished(msg.value);
      }
    } else if (msg.case === 'speakersChanged') {
      if (this.onSpeakersChanged) {
        this.onSpeakersChanged((_b = msg.value.speakers) !== null && _b !== void 0 ? _b : []);
      }
    } else if (msg.case === 'leave') {
      if (this.onLeave) {
        this.onLeave(msg.value);
      }
    } else if (msg.case === 'mute') {
      if (this.onRemoteMuteChanged) {
        this.onRemoteMuteChanged(msg.value.sid, msg.value.muted);
      }
    } else if (msg.case === 'roomUpdate') {
      if (this.onRoomUpdate && msg.value.room) {
        this.onRoomUpdate(msg.value.room);
      }
    } else if (msg.case === 'connectionQuality') {
      if (this.onConnectionQuality) {
        this.onConnectionQuality(msg.value);
      }
    } else if (msg.case === 'streamStateUpdate') {
      if (this.onStreamStateUpdate) {
        this.onStreamStateUpdate(msg.value);
      }
    } else if (msg.case === 'subscribedQualityUpdate') {
      if (this.onSubscribedQualityUpdate) {
        this.onSubscribedQualityUpdate(msg.value);
      }
    } else if (msg.case === 'subscriptionPermissionUpdate') {
      if (this.onSubscriptionPermissionUpdate) {
        this.onSubscriptionPermissionUpdate(msg.value);
      }
    } else if (msg.case === 'refreshToken') {
      if (this.onTokenRefresh) {
        this.onTokenRefresh(msg.value);
      }
    } else if (msg.case === 'trackUnpublished') {
      if (this.onLocalTrackUnpublished) {
        this.onLocalTrackUnpublished(msg.value);
      }
    } else if (msg.case === 'subscriptionResponse') {
      if (this.onSubscriptionError) {
        this.onSubscriptionError(msg.value);
      }
    } else if (msg.case === 'pong') ; else if (msg.case === 'pongResp') {
      this.rtt = Date.now() - Number.parseInt(msg.value.lastPingTimestamp.toString());
      this.resetPingTimeout();
      pingHandled = true;
    } else if (msg.case === 'requestResponse') {
      if (this.onRequestResponse) {
        this.onRequestResponse(msg.value);
      }
    } else if (msg.case === 'trackSubscribed') {
      if (this.onLocalTrackSubscribed) {
        this.onLocalTrackSubscribed(msg.value.trackSid);
      }
    } else if (msg.case === 'roomMoved') {
      if (this.onTokenRefresh) {
        this.onTokenRefresh(msg.value.token);
      }
      if (this.onRoomMoved) {
        this.onRoomMoved(msg.value);
      }
    } else if (msg.case === 'mediaSectionsRequirement') {
      if (this.onMediaSectionsRequirement) {
        this.onMediaSectionsRequirement(msg.value);
      }
    } else if (msg.case === 'publishDataTrackResponse') {
      if (this.onPublishDataTrackResponse) {
        this.onPublishDataTrackResponse(msg.value);
      }
    } else if (msg.case === 'unpublishDataTrackResponse') {
      if (this.onUnPublishDataTrackResponse) {
        this.onUnPublishDataTrackResponse(msg.value);
      }
    } else if (msg.case === 'dataTrackSubscriberHandles') {
      if (this.onDataTrackSubscriberHandles) {
        this.onDataTrackSubscriberHandles(msg.value);
      }
    } else {
      this.log.debug('unsupported message', {
        msgCase: msg.case
      });
    }
    if (!pingHandled) {
      this.resetPingTimeout();
    }
  }
  setReconnected() {
    while (this.queuedRequests.length > 0) {
      const req = this.queuedRequests.shift();
      if (req) {
        this.requestQueue.run(req);
      }
    }
  }
  /**
   * Handles a transport we lost without asking to. The client goes to `offline` rather than
   * `closed`: whether this session gets resumed, restarted or given up on is the engine's call.
   */
  handleOnClose(reason_1) {
    return __awaiter(this, arguments, void 0, function (reason) {
      var _this6 = this;
      let attemptId = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : this.attemptId;
      let simulateReconnectDelay = arguments.length > 2 ? arguments[2] : undefined;
      return function* () {
        const onCloseCallback = _this6.onClose;
        if (!_this6.sendLifecycleInput({
          type: 'transportFailed',
          attemptId,
          reason
        })) {
          // a close we caused ourselves, or one from a transport that has since been replaced: logged
          // rather than dropped silently, because a close that goes unnoticed leaves the client writing
          // to a dead socket
          _this6.log.debug("ignoring transport close in state ".concat(_this6.lifecycleState), {
            reason,
            attemptId,
            currentAttemptId: _this6.attemptId
          });
          return;
        }
        yield _this6.teardownTransport(reason);
        _this6.log.info("websocket connection closed: ".concat(reason), {
          reason
        });
        if (simulateReconnectDelay) {
          _this6.log.info("waiting for ".concat(simulateReconnectDelay, "ms until reconnecting signal"));
          yield sleep(simulateReconnectDelay);
        }
        if (onCloseCallback) {
          onCloseCallback(reason);
        }
      }();
    });
  }
  handleWSError(error) {
    this.log.error('websocket error', {
      error
    });
  }
  /**
   * Resets the ping timeout and starts a new timeout.
   * Call this after receiving a pong message
   */
  resetPingTimeout() {
    this.clearPingTimeout();
    if (!this.pingTimeoutDuration) {
      this.log.warn('ping timeout duration not set');
      return;
    }
    this.pingTimeout = CriticalTimers.setTimeout(() => {
      this.log.warn("ping timeout triggered. last pong received at: ".concat(new Date(Date.now() - this.pingTimeoutDuration * 1000).toUTCString()));
      this.handleOnClose('ping timeout');
    }, this.pingTimeoutDuration * 1000);
  }
  /**
   * Clears ping timeout (does not start a new timeout)
   */
  clearPingTimeout() {
    if (this.pingTimeout) {
      CriticalTimers.clearTimeout(this.pingTimeout);
    }
  }
  startPingInterval() {
    this.clearPingInterval();
    this.resetPingTimeout();
    if (!this.pingIntervalDuration) {
      this.log.warn('ping interval duration not set');
      return;
    }
    this.log.debug('start ping interval');
    this.pingInterval = CriticalTimers.setInterval(() => {
      this.sendPing();
    }, this.pingIntervalDuration * 1000);
  }
  clearPingInterval() {
    this.log.debug('clearing ping interval');
    this.clearPingTimeout();
    if (this.pingInterval) {
      CriticalTimers.clearInterval(this.pingInterval);
    }
  }
  /**
   * Handles the successful connection to the signal server
   * @param connection The WebSocket connection
   * @param timeoutHandle The timeout handle to clear
   * @param firstMessage Optional first message to process
   * @internal
   */
  handleSignalConnected(connection, timeoutHandle, attemptId) {
    clearTimeout(timeoutHandle);
    const established = this.sendLifecycleInput(this.lifecycleState === 'reconnecting' ? {
      type: 'reconnectComplete',
      attemptId
    } : {
      type: 'connectComplete',
      attemptId
    });
    if (!established) {
      // The attempt was abandoned while we waited for the server's first message: it was closed, or
      // a newer attempt superseded it. Arming the ping interval and the read loop here would outlive
      // the session that owns them, so leave the transport to whoever holds the lifecycle now.
      this.log.debug('discarding a connection whose attempt no longer owns the session', {
        attemptId,
        currentAttemptId: this.attemptId,
        state: this.lifecycleState
      });
      return;
    }
    this.log.info('signal connected');
    this.startPingInterval();
    this.startReadingLoop(connection.readable.getReader());
  }
  /**
   * Validates the first message received from the signal server
   * @param firstSignalResponse The first signal response received
   * @param isReconnect Whether this is a reconnection attempt
   * @returns Validation result with response or error
   * @internal
   */
  validateFirstMessage(firstSignalResponse, isReconnect) {
    var _a, _b, _c, _d;
    if (((_a = firstSignalResponse.message) === null || _a === void 0 ? void 0 : _a.case) === 'join') {
      return {
        isValid: true,
        response: firstSignalResponse.message.value
      };
    } else if (this.lifecycleState === 'reconnecting' && ((_b = firstSignalResponse.message) === null || _b === void 0 ? void 0 : _b.case) === 'reconnect') {
      return {
        isValid: true,
        response: firstSignalResponse.message.value
      };
    } else if (this.isEstablishingConnection && ((_c = firstSignalResponse.message) === null || _c === void 0 ? void 0 : _c.case) === 'leave') {
      return {
        isValid: false,
        error: ConnectionError.leaveRequest('Received leave request while trying to (re)connect', firstSignalResponse.message.value.reason)
      };
    } else if (!isReconnect) {
      // non-reconnect case, should receive join response first
      return {
        isValid: false,
        error: ConnectionError.internal("did not receive join response, got ".concat((_d = firstSignalResponse.message) === null || _d === void 0 ? void 0 : _d.case, " instead"))
      };
    }
    return {
      isValid: false,
      error: ConnectionError.internal('Unexpected first message')
    };
  }
  /**
   * Handles WebSocket connection errors by validating with the server
   * @param reason The error that occurred
   * @param validateUrl The URL to validate the connection with
   * @returns A ConnectionError with appropriate reason and status
   * @internal
   */
  handleConnectionError(reason, validateUrl) {
    return __awaiter(this, void 0, void 0, function* () {
      try {
        const resp = yield fetch(validateUrl, {
          signal: abortSignalTimeout(3000)
        });
        switch (resp.status) {
          case 404:
            const errorMsg = yield resp.text();
            if (errorMsg.includes('requested room does not exist')) {
              return ConnectionError.notAllowed(errorMsg, resp.status);
            }
            return ConnectionError.serviceNotFound('v1 RTC path not found. Consider upgrading your LiveKit server version', 'v0-rtc');
          case 401:
          case 403:
            const msg = yield resp.text();
            return ConnectionError.notAllowed(msg, resp.status);
          default:
            break;
        }
        if (reason instanceof ConnectionError) {
          return reason;
        } else {
          return ConnectionError.internal("Encountered unknown websocket error during connection: ".concat(reason), {
            status: resp.status,
            statusText: resp.statusText
          });
        }
      } catch (e) {
        if (e instanceof ConnectionError) {
          return e;
        } else {
          return ConnectionError.serverUnreachable(e instanceof Error ? e.message : 'server was not reachable');
        }
      }
    });
  }
}
function fromProtoSessionDescription(sd) {
  const rsd = {
    type: 'offer',
    sdp: sd.sdp
  };
  switch (sd.type) {
    case 'answer':
    case 'offer':
    case 'pranswer':
    case 'rollback':
      rsd.type = sd.type;
      break;
  }
  return rsd;
}
export { JOIN_RESPONSE_TIMEOUT, MAX_CLOSE_REASON_BYTES, MAX_WS_CLOSE_TIME, SignalClient, SignalConnectionState, WebSocketStream, canPassThroughQueue, fromProtoSessionDescription, isRecoverableReconnectError, lifecycleToConnectionState, passThroughQueueSignals, truncateCloseReason };
