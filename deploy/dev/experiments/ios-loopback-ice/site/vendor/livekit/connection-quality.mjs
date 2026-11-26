// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 31904-31929 行按职责切出，行为不变。
import { ConnectionQuality$1 } from './protobuf-messages-enums.mjs';

var ConnectionQuality;
(function (ConnectionQuality) {
  ConnectionQuality["Excellent"] = "excellent";
  ConnectionQuality["Good"] = "good";
  ConnectionQuality["Poor"] = "poor";
  /**
   * Indicates that a participant has temporarily (or permanently) lost connection to LiveKit.
   * For permanent disconnection a `ParticipantDisconnected` event will be emitted after a timeout
   */
  ConnectionQuality["Lost"] = "lost";
  ConnectionQuality["Unknown"] = "unknown";
})(ConnectionQuality || (ConnectionQuality = {}));
function qualityFromProto(q) {
  switch (q) {
    case ConnectionQuality$1.EXCELLENT:
      return ConnectionQuality.Excellent;
    case ConnectionQuality$1.GOOD:
      return ConnectionQuality.Good;
    case ConnectionQuality$1.POOR:
      return ConnectionQuality.Poor;
    case ConnectionQuality$1.LOST:
      return ConnectionQuality.Lost;
    default:
      return ConnectionQuality.Unknown;
  }
}
export { ConnectionQuality, qualityFromProto };
