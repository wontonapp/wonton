// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 5016-5560 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { ClientConfigSetting } from './protobuf-messages-enums.mjs';
import { Codec } from './protobuf-messages-room.mjs';


const SpeakerInfo = /* @__PURE__ */proto3.makeMessageType("livekit.SpeakerInfo", () => [{
  no: 1,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "level",
  kind: "scalar",
  T: 2
  /* ScalarType.FLOAT */
}, {
  no: 3,
  name: "active",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const UserPacket = /* @__PURE__ */proto3.makeMessageType("livekit.UserPacket", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "participant_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "payload",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}, {
  no: 3,
  name: "destination_sids",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 6,
  name: "destination_identities",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 4,
  name: "topic",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 8,
  name: "id",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 9,
  name: "start_time",
  kind: "scalar",
  T: 4,
  opt: true
}, {
  no: 10,
  name: "end_time",
  kind: "scalar",
  T: 4,
  opt: true
}, {
  no: 11,
  name: "nonce",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}]);
const SipDTMF = /* @__PURE__ */proto3.makeMessageType("livekit.SipDTMF", () => [{
  no: 3,
  name: "code",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "digit",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const Transcription = /* @__PURE__ */proto3.makeMessageType("livekit.Transcription", () => [{
  no: 2,
  name: "transcribed_participant_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "track_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "segments",
  kind: "message",
  T: TranscriptionSegment,
  repeated: true
}]);
const TranscriptionSegment = /* @__PURE__ */proto3.makeMessageType("livekit.TranscriptionSegment", () => [{
  no: 1,
  name: "id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "text",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "start_time",
  kind: "scalar",
  T: 4
  /* ScalarType.UINT64 */
}, {
  no: 4,
  name: "end_time",
  kind: "scalar",
  T: 4
  /* ScalarType.UINT64 */
}, {
  no: 5,
  name: "final",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 6,
  name: "language",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const ChatMessage = /* @__PURE__ */proto3.makeMessageType("livekit.ChatMessage", () => [{
  no: 1,
  name: "id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "timestamp",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 3,
  name: "edit_timestamp",
  kind: "scalar",
  T: 3,
  opt: true
}, {
  no: 4,
  name: "message",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "deleted",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 6,
  name: "generated",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const RpcRequest = /* @__PURE__ */proto3.makeMessageType("livekit.RpcRequest", () => [{
  no: 1,
  name: "id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "method",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "payload",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "response_timeout_ms",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 5,
  name: "version",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 6,
  name: "compressed_payload",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}]);
const RpcAck = /* @__PURE__ */proto3.makeMessageType("livekit.RpcAck", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const RpcResponse = /* @__PURE__ */proto3.makeMessageType("livekit.RpcResponse", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "payload",
  kind: "scalar",
  T: 9,
  oneof: "value"
}, {
  no: 3,
  name: "error",
  kind: "message",
  T: RpcError$1,
  oneof: "value"
}, {
  no: 4,
  name: "compressed_payload",
  kind: "scalar",
  T: 12,
  oneof: "value"
}]);
const RpcError$1 = /* @__PURE__ */proto3.makeMessageType("livekit.RpcError", () => [{
  no: 1,
  name: "code",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "message",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "data",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const ParticipantTracks = /* @__PURE__ */proto3.makeMessageType("livekit.ParticipantTracks", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "track_sids",
  kind: "scalar",
  T: 9,
  repeated: true
}]);
const ServerInfo = /* @__PURE__ */proto3.makeMessageType("livekit.ServerInfo", () => [{
  no: 1,
  name: "edition",
  kind: "enum",
  T: proto3.getEnumType(ServerInfo_Edition)
}, {
  no: 2,
  name: "version",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "protocol",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 4,
  name: "region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "node_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "debug_info",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 7,
  name: "agent_protocol",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}]);
const ServerInfo_Edition = /* @__PURE__ */proto3.makeEnum("livekit.ServerInfo.Edition", [{
  no: 0,
  name: "Standard"
}, {
  no: 1,
  name: "Cloud"
}]);
const ClientInfo = /* @__PURE__ */proto3.makeMessageType("livekit.ClientInfo", () => [{
  no: 1,
  name: "sdk",
  kind: "enum",
  T: proto3.getEnumType(ClientInfo_SDK)
}, {
  no: 2,
  name: "version",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "protocol",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 4,
  name: "os",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "os_version",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "device_model",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 7,
  name: "browser",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 8,
  name: "browser_version",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 9,
  name: "address",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 10,
  name: "network",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 11,
  name: "other_sdks",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 12,
  name: "client_protocol",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 13,
  name: "capabilities",
  kind: "enum",
  T: proto3.getEnumType(ClientInfo_Capability),
  repeated: true
}]);
const ClientInfo_SDK = /* @__PURE__ */proto3.makeEnum("livekit.ClientInfo.SDK", [{
  no: 0,
  name: "UNKNOWN"
}, {
  no: 1,
  name: "JS"
}, {
  no: 2,
  name: "SWIFT"
}, {
  no: 3,
  name: "ANDROID"
}, {
  no: 4,
  name: "FLUTTER"
}, {
  no: 5,
  name: "GO"
}, {
  no: 6,
  name: "UNITY"
}, {
  no: 7,
  name: "REACT_NATIVE"
}, {
  no: 8,
  name: "RUST"
}, {
  no: 9,
  name: "PYTHON"
}, {
  no: 10,
  name: "CPP"
}, {
  no: 11,
  name: "UNITY_WEB"
}, {
  no: 12,
  name: "NODE"
}, {
  no: 13,
  name: "UNREAL"
}, {
  no: 14,
  name: "ESP32"
}, {
  no: 15,
  name: "DOTNET"
}]);
const ClientInfo_Capability = /* @__PURE__ */proto3.makeEnum("livekit.ClientInfo.Capability", [{
  no: 0,
  name: "CAP_UNUSED"
}, {
  no: 1,
  name: "CAP_PACKET_TRAILER"
}, {
  no: 2,
  name: "CAP_COMPRESSION_DEFLATE_RAW"
}]);
const ClientConfiguration = /* @__PURE__ */proto3.makeMessageType("livekit.ClientConfiguration", () => [{
  no: 1,
  name: "video",
  kind: "message",
  T: VideoConfiguration
}, {
  no: 2,
  name: "screen",
  kind: "message",
  T: VideoConfiguration
}, {
  no: 3,
  name: "resume_connection",
  kind: "enum",
  T: proto3.getEnumType(ClientConfigSetting)
}, {
  no: 4,
  name: "disabled_codecs",
  kind: "message",
  T: DisabledCodecs
}, {
  no: 5,
  name: "force_relay",
  kind: "enum",
  T: proto3.getEnumType(ClientConfigSetting)
}]);
const VideoConfiguration = /* @__PURE__ */proto3.makeMessageType("livekit.VideoConfiguration", () => [{
  no: 1,
  name: "hardware_encoder",
  kind: "enum",
  T: proto3.getEnumType(ClientConfigSetting)
}]);
const DisabledCodecs = /* @__PURE__ */proto3.makeMessageType("livekit.DisabledCodecs", () => [{
  no: 1,
  name: "codecs",
  kind: "message",
  T: Codec,
  repeated: true
}, {
  no: 2,
  name: "publish",
  kind: "message",
  T: Codec,
  repeated: true
}]);
const TimedVersion = /* @__PURE__ */proto3.makeMessageType("livekit.TimedVersion", () => [{
  no: 1,
  name: "unix_micro",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 2,
  name: "ticks",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}]);
const DataStream_OperationType = /* @__PURE__ */proto3.makeEnum("livekit.DataStream.OperationType", [{
  no: 0,
  name: "CREATE"
}, {
  no: 1,
  name: "UPDATE"
}, {
  no: 2,
  name: "DELETE"
}, {
  no: 3,
  name: "REACTION"
}]);
export { ChatMessage, ClientConfiguration, ClientInfo, ClientInfo_Capability, ClientInfo_SDK, DataStream_OperationType, DisabledCodecs, ParticipantTracks, RpcAck, RpcError$1, RpcRequest, RpcResponse, ServerInfo, ServerInfo_Edition, SipDTMF, SpeakerInfo, TimedVersion, Transcription, TranscriptionSegment, UserPacket, VideoConfiguration };
