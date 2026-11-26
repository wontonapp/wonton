// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 12326-12542 行按职责切出，行为不变。

/**
 * LiveKit specific error type representing an error with an associated set of reasons.
 * Use this to represent an error with multiple different but contextually related variants.
 * */
import { RequestResponse_Reason } from './protobuf-messages-signal2.mjs';
import { LivekitError } from './browser.mjs';

class LivekitReasonedError extends LivekitError {}
class SimulatedError extends LivekitError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 'Simulated failure';
    super(-1, message);
    this.name = 'simulated';
  }
}
var ConnectionErrorReason;
(function (ConnectionErrorReason) {
  ConnectionErrorReason[ConnectionErrorReason["NotAllowed"] = 0] = "NotAllowed";
  ConnectionErrorReason[ConnectionErrorReason["ServerUnreachable"] = 1] = "ServerUnreachable";
  ConnectionErrorReason[ConnectionErrorReason["InternalError"] = 2] = "InternalError";
  ConnectionErrorReason[ConnectionErrorReason["Cancelled"] = 3] = "Cancelled";
  ConnectionErrorReason[ConnectionErrorReason["LeaveRequest"] = 4] = "LeaveRequest";
  ConnectionErrorReason[ConnectionErrorReason["Timeout"] = 5] = "Timeout";
  ConnectionErrorReason[ConnectionErrorReason["WebSocket"] = 6] = "WebSocket";
  ConnectionErrorReason[ConnectionErrorReason["ServiceNotFound"] = 7] = "ServiceNotFound";
})(ConnectionErrorReason || (ConnectionErrorReason = {}));
class ConnectionError extends LivekitReasonedError {
  constructor(message, reason, status, context) {
    super(1, message);
    this.name = 'ConnectionError';
    this.status = status;
    this.reason = reason;
    this.context = context;
    this.reasonName = ConnectionErrorReason[reason];
  }
  static notAllowed(message, status, context) {
    return new ConnectionError(message, ConnectionErrorReason.NotAllowed, status, context);
  }
  static timeout(message) {
    return new ConnectionError(message, ConnectionErrorReason.Timeout);
  }
  static leaveRequest(message, context) {
    return new ConnectionError(message, ConnectionErrorReason.LeaveRequest, undefined, context);
  }
  static internal(message, context) {
    return new ConnectionError(message, ConnectionErrorReason.InternalError, undefined, context);
  }
  static cancelled(message) {
    return new ConnectionError(message, ConnectionErrorReason.Cancelled);
  }
  static serverUnreachable(message, status) {
    return new ConnectionError(message, ConnectionErrorReason.ServerUnreachable, status);
  }
  static websocket(message, status, reason) {
    return new ConnectionError(message, ConnectionErrorReason.WebSocket, status, reason);
  }
  static serviceNotFound(message, serviceName) {
    return new ConnectionError(message, ConnectionErrorReason.ServiceNotFound, undefined, serviceName);
  }
}
/**
 * Whether a failed connection attempt may be retried against a different LiveKit Cloud region.
 *
 * LiveKit Cloud signals project-level region pinning by returning 403 on the RTC paths when the
 * project is not allowed in the region the client geo-routed to. `/settings/regions` is
 * deliberately left reachable so the client can discover its allowed regions and connect there, so
 * a 403 must not be treated as terminal.
 *
 * A 401 stays terminal: no other region will accept a token this one rejected. So does the 404
 * "requested room does not exist" case, which is also reported as `NotAllowed`.
 *
 * We key on the status rather than the server's error message because that message is an
 * unversioned human-readable string; matching it would let a copy edit break already-shipped
 * clients. If a 403 really was a permissions failure rather than region pinning, every region
 * attempt fails the same way and the original error still surfaces — at the cost of one extra
 * region lookup.
 */
function canFailOverToAnotherRegion(error) {
  if (error.reason === ConnectionErrorReason.Cancelled) {
    return false;
  }
  if (error.reason === ConnectionErrorReason.NotAllowed) {
    return error.status === 403;
  }
  return true;
}
class DeviceUnsupportedError extends LivekitError {
  constructor(message) {
    super(21, message !== null && message !== void 0 ? message : 'device is unsupported');
    this.name = 'DeviceUnsupportedError';
  }
}
class TrackInvalidError extends LivekitError {
  constructor(message) {
    super(20, message !== null && message !== void 0 ? message : 'track is invalid');
    this.name = 'TrackInvalidError';
  }
}
class UnsupportedServer extends LivekitError {
  constructor(message) {
    super(10, message || 'unsupported server');
    this.name = 'UnsupportedServer';
  }
}
class UnexpectedConnectionState extends LivekitError {
  constructor(message) {
    super(12, message || 'unexpected connection state');
    this.name = 'UnexpectedConnectionState';
  }
}
class NegotiationError extends LivekitError {
  constructor(message) {
    super(13, message || 'unable to negotiate');
    this.name = 'NegotiationError';
  }
}
class PublishDataError extends LivekitError {
  constructor(message) {
    super(14, message || 'unable to publish data');
    this.name = 'PublishDataError';
  }
}
class PublishTrackError extends LivekitError {
  constructor(message, status) {
    super(15, message);
    this.name = 'PublishTrackError';
    this.status = status;
  }
}
class SignalRequestError extends LivekitReasonedError {
  constructor(message, reason) {
    super(15, message);
    this.name = 'SignalRequestError';
    this.reason = reason;
    this.reasonName = typeof reason === 'string' ? reason : RequestResponse_Reason[reason];
  }
}
// NOTE: matches with https://github.com/livekit/client-sdk-swift/blob/f37bbd260d61e165084962db822c79f995f1a113/Sources/LiveKit/DataStream/StreamError.swift#L17
var DataStreamErrorReason;
(function (DataStreamErrorReason) {
  // Unable to open a stream with the same ID more than once.
  DataStreamErrorReason[DataStreamErrorReason["AlreadyOpened"] = 0] = "AlreadyOpened";
  // Stream closed abnormally by remote participant.
  DataStreamErrorReason[DataStreamErrorReason["AbnormalEnd"] = 1] = "AbnormalEnd";
  // Incoming chunk data could not be decoded.
  DataStreamErrorReason[DataStreamErrorReason["DecodeFailed"] = 2] = "DecodeFailed";
  // Read length exceeded total length specified in stream header.
  DataStreamErrorReason[DataStreamErrorReason["LengthExceeded"] = 3] = "LengthExceeded";
  // Read length less than total length specified in stream header.
  DataStreamErrorReason[DataStreamErrorReason["Incomplete"] = 4] = "Incomplete";
  // Unable to register a stream handler more than once.
  DataStreamErrorReason[DataStreamErrorReason["HandlerAlreadyRegistered"] = 7] = "HandlerAlreadyRegistered";
  // Encryption type mismatch.
  DataStreamErrorReason[DataStreamErrorReason["EncryptionTypeMismatch"] = 8] = "EncryptionTypeMismatch";
  // The serialized stream header packet (driven mainly by attributes) exceeds the MTU budget.
  DataStreamErrorReason[DataStreamErrorReason["HeaderTooLarge"] = 9] = "HeaderTooLarge";
  // The stream's (decompressed) payload exceeds the maximum allowed size.
  DataStreamErrorReason[DataStreamErrorReason["PayloadTooLarge"] = 10] = "PayloadTooLarge";
})(DataStreamErrorReason || (DataStreamErrorReason = {}));
class DataStreamError extends LivekitReasonedError {
  constructor(message, reason) {
    super(16, message);
    this.name = 'DataStreamError';
    this.reason = reason;
    this.reasonName = DataStreamErrorReason[reason];
  }
}
class SignalReconnectError extends LivekitError {
  constructor(message) {
    super(18, message);
    this.name = 'SignalReconnectError';
  }
}
var MediaDeviceFailure;
(function (MediaDeviceFailure) {
  // user rejected permissions
  MediaDeviceFailure["PermissionDenied"] = "PermissionDenied";
  // device is not available
  MediaDeviceFailure["NotFound"] = "NotFound";
  // device is in use. On Windows, only a single tab may get access to a device at a time.
  MediaDeviceFailure["DeviceInUse"] = "DeviceInUse";
  MediaDeviceFailure["Other"] = "Other";
})(MediaDeviceFailure || (MediaDeviceFailure = {}));
(function (MediaDeviceFailure) {
  function getFailure(error) {
    if (error && 'name' in error) {
      if (error.name === 'NotFoundError' || error.name === 'DevicesNotFoundError') {
        return MediaDeviceFailure.NotFound;
      }
      if (error.name === 'NotAllowedError' || error.name === 'PermissionDeniedError') {
        return MediaDeviceFailure.PermissionDenied;
      }
      if (error.name === 'NotReadableError' || error.name === 'TrackStartError') {
        return MediaDeviceFailure.DeviceInUse;
      }
      return MediaDeviceFailure.Other;
    }
  }
  MediaDeviceFailure.getFailure = getFailure;
})(MediaDeviceFailure || (MediaDeviceFailure = {}));/**
 * Timers that can be overridden with platform specific implementations
 * that ensure that they are fired. These should be used when it is critical
 * that the timer fires on time.
 */
class CriticalTimers {}
CriticalTimers.setTimeout = function () {
  return setTimeout(...arguments);
};
CriticalTimers.setInterval =
// eslint-disable-next-line @typescript-eslint/no-implied-eval
function () {
  return setInterval(...arguments);
};
CriticalTimers.clearTimeout = function () {
  return clearTimeout(...arguments);
};
CriticalTimers.clearInterval = function () {
  return clearInterval(...arguments);
};
export { ConnectionError, ConnectionErrorReason, CriticalTimers, DataStreamError, DataStreamErrorReason, DeviceUnsupportedError, LivekitReasonedError, MediaDeviceFailure, NegotiationError, PublishDataError, PublishTrackError, SignalReconnectError, SignalRequestError, SimulatedError, TrackInvalidError, UnexpectedConnectionState, UnsupportedServer, canFailOverToAnotherRegion };
