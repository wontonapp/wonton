// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 3888-4093 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';


const VideoCodec = /* @__PURE__ */proto3.makeEnum("livekit.VideoCodec", [{
  no: 0,
  name: "DEFAULT_VC"
}, {
  no: 1,
  name: "H264_BASELINE"
}, {
  no: 2,
  name: "H264_MAIN"
}, {
  no: 3,
  name: "H264_HIGH"
}, {
  no: 4,
  name: "VP8"
}]);
const ImageCodec = /* @__PURE__ */proto3.makeEnum("livekit.ImageCodec", [{
  no: 0,
  name: "IC_DEFAULT"
}, {
  no: 1,
  name: "IC_JPEG"
}]);
const BackupCodecPolicy$1 = /* @__PURE__ */proto3.makeEnum("livekit.BackupCodecPolicy", [{
  no: 0,
  name: "PREFER_REGRESSION"
}, {
  no: 1,
  name: "SIMULCAST"
}, {
  no: 2,
  name: "REGRESSION"
}]);
const TrackType = /* @__PURE__ */proto3.makeEnum("livekit.TrackType", [{
  no: 0,
  name: "AUDIO"
}, {
  no: 1,
  name: "VIDEO"
}, {
  no: 2,
  name: "DATA"
}]);
const TrackSource = /* @__PURE__ */proto3.makeEnum("livekit.TrackSource", [{
  no: 0,
  name: "UNKNOWN"
}, {
  no: 1,
  name: "CAMERA"
}, {
  no: 2,
  name: "MICROPHONE"
}, {
  no: 3,
  name: "SCREEN_SHARE"
}, {
  no: 4,
  name: "SCREEN_SHARE_AUDIO"
}]);
const VideoQuality$1 = /* @__PURE__ */proto3.makeEnum("livekit.VideoQuality", [{
  no: 0,
  name: "LOW"
}, {
  no: 1,
  name: "MEDIUM"
}, {
  no: 2,
  name: "HIGH"
}, {
  no: 3,
  name: "OFF"
}]);
const ConnectionQuality$1 = /* @__PURE__ */proto3.makeEnum("livekit.ConnectionQuality", [{
  no: 0,
  name: "POOR"
}, {
  no: 1,
  name: "GOOD"
}, {
  no: 2,
  name: "EXCELLENT"
}, {
  no: 3,
  name: "LOST"
}]);
const ClientConfigSetting = /* @__PURE__ */proto3.makeEnum("livekit.ClientConfigSetting", [{
  no: 0,
  name: "UNSET"
}, {
  no: 1,
  name: "DISABLED"
}, {
  no: 2,
  name: "ENABLED"
}]);
const DisconnectReason = /* @__PURE__ */proto3.makeEnum("livekit.DisconnectReason", [{
  no: 0,
  name: "UNKNOWN_REASON"
}, {
  no: 1,
  name: "CLIENT_INITIATED"
}, {
  no: 2,
  name: "DUPLICATE_IDENTITY"
}, {
  no: 3,
  name: "SERVER_SHUTDOWN"
}, {
  no: 4,
  name: "PARTICIPANT_REMOVED"
}, {
  no: 5,
  name: "ROOM_DELETED"
}, {
  no: 6,
  name: "STATE_MISMATCH"
}, {
  no: 7,
  name: "JOIN_FAILURE"
}, {
  no: 8,
  name: "MIGRATION"
}, {
  no: 9,
  name: "SIGNAL_CLOSE"
}, {
  no: 10,
  name: "ROOM_CLOSED"
}, {
  no: 11,
  name: "USER_UNAVAILABLE"
}, {
  no: 12,
  name: "USER_REJECTED"
}, {
  no: 13,
  name: "SIP_TRUNK_FAILURE"
}, {
  no: 14,
  name: "CONNECTION_TIMEOUT"
}, {
  no: 15,
  name: "MEDIA_FAILURE"
}, {
  no: 16,
  name: "AGENT_ERROR"
}]);
const ReconnectReason = /* @__PURE__ */proto3.makeEnum("livekit.ReconnectReason", [{
  no: 0,
  name: "RR_UNKNOWN"
}, {
  no: 1,
  name: "RR_SIGNAL_DISCONNECTED"
}, {
  no: 2,
  name: "RR_PUBLISHER_FAILED"
}, {
  no: 3,
  name: "RR_SUBSCRIBER_FAILED"
}, {
  no: 4,
  name: "RR_SWITCH_CANDIDATE"
}]);
const SubscriptionError = /* @__PURE__ */proto3.makeEnum("livekit.SubscriptionError", [{
  no: 0,
  name: "SE_UNKNOWN"
}, {
  no: 1,
  name: "SE_CODEC_UNSUPPORTED"
}, {
  no: 2,
  name: "SE_TRACK_NOTFOUND"
}]);
const AudioTrackFeature = /* @__PURE__ */proto3.makeEnum("livekit.AudioTrackFeature", [{
  no: 0,
  name: "TF_STEREO"
}, {
  no: 1,
  name: "TF_NO_DTX"
}, {
  no: 2,
  name: "TF_AUTO_GAIN_CONTROL"
}, {
  no: 3,
  name: "TF_ECHO_CANCELLATION"
}, {
  no: 4,
  name: "TF_NOISE_SUPPRESSION"
}, {
  no: 5,
  name: "TF_ENHANCED_NOISE_CANCELLATION"
}, {
  no: 6,
  name: "TF_PRECONNECT_BUFFER"
}]);
const PacketTrailerFeature = /* @__PURE__ */proto3.makeEnum("livekit.PacketTrailerFeature", [{
  no: 0,
  name: "PTF_USER_TIMESTAMP"
}, {
  no: 1,
  name: "PTF_FRAME_ID"
}, {
  no: 2,
  name: "PTF_USER_DATA"
}]);
export { AudioTrackFeature, BackupCodecPolicy$1, ClientConfigSetting, ConnectionQuality$1, DisconnectReason, ImageCodec, PacketTrailerFeature, ReconnectReason, SubscriptionError, TrackSource, TrackType, VideoCodec, VideoQuality$1 };
