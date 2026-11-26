// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 4093-5016 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { MetricsBatch } from './protobuf-messages-metrics.mjs';
import { AudioTrackFeature, BackupCodecPolicy$1, DisconnectReason, PacketTrailerFeature, TrackSource, TrackType, VideoQuality$1 } from './protobuf-messages-enums.mjs';
import { ChatMessage, ClientInfo_Capability, RpcAck, RpcRequest, RpcResponse, SipDTMF, SpeakerInfo, TimedVersion, Transcription, UserPacket } from './protobuf-messages-chat.mjs';
import { DataStream_Chunk, DataStream_Header, DataStream_Trailer } from './protobuf-messages-stream.mjs';


const Room$1 = /* @__PURE__ */proto3.makeMessageType("livekit.Room", () => [{
  no: 1,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "empty_timeout",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 14,
  name: "departure_timeout",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "max_participants",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 5,
  name: "creation_time",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 15,
  name: "creation_time_ms",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 6,
  name: "turn_password",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 7,
  name: "enabled_codecs",
  kind: "message",
  T: Codec,
  repeated: true
}, {
  no: 8,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 9,
  name: "num_participants",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 11,
  name: "num_publishers",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 10,
  name: "active_recording",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 13,
  name: "version",
  kind: "message",
  T: TimedVersion
}]);
const Codec = /* @__PURE__ */proto3.makeMessageType("livekit.Codec", () => [{
  no: 1,
  name: "mime",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "fmtp_line",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const ParticipantPermission = /* @__PURE__ */proto3.makeMessageType("livekit.ParticipantPermission", () => [{
  no: 1,
  name: "can_subscribe",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 2,
  name: "can_publish",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "can_publish_data",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 9,
  name: "can_publish_sources",
  kind: "enum",
  T: proto3.getEnumType(TrackSource),
  repeated: true
}, {
  no: 7,
  name: "hidden",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 8,
  name: "recorder",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 10,
  name: "can_update_metadata",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 11,
  name: "agent",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 12,
  name: "can_subscribe_metrics",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 13,
  name: "can_manage_agent_session",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const ParticipantInfo = /* @__PURE__ */proto3.makeMessageType("livekit.ParticipantInfo", () => [{
  no: 1,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "state",
  kind: "enum",
  T: proto3.getEnumType(ParticipantInfo_State)
}, {
  no: 4,
  name: "tracks",
  kind: "message",
  T: TrackInfo,
  repeated: true
}, {
  no: 5,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "joined_at",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 17,
  name: "joined_at_ms",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 9,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 10,
  name: "version",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 11,
  name: "permission",
  kind: "message",
  T: ParticipantPermission
}, {
  no: 12,
  name: "region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 13,
  name: "is_publisher",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 14,
  name: "kind",
  kind: "enum",
  T: proto3.getEnumType(ParticipantInfo_Kind)
}, {
  no: 15,
  name: "attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 16,
  name: "disconnect_reason",
  kind: "enum",
  T: proto3.getEnumType(DisconnectReason)
}, {
  no: 18,
  name: "kind_details",
  kind: "enum",
  T: proto3.getEnumType(ParticipantInfo_KindDetail),
  repeated: true
}, {
  no: 19,
  name: "data_tracks",
  kind: "message",
  T: DataTrackInfo$1,
  repeated: true
}, {
  no: 20,
  name: "client_protocol",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 21,
  name: "capabilities",
  kind: "enum",
  T: proto3.getEnumType(ClientInfo_Capability),
  repeated: true
}]);
const ParticipantInfo_State = /* @__PURE__ */proto3.makeEnum("livekit.ParticipantInfo.State", [{
  no: 0,
  name: "JOINING"
}, {
  no: 1,
  name: "JOINED"
}, {
  no: 2,
  name: "ACTIVE"
}, {
  no: 3,
  name: "DISCONNECTED"
}]);
const ParticipantInfo_Kind = /* @__PURE__ */proto3.makeEnum("livekit.ParticipantInfo.Kind", [{
  no: 0,
  name: "STANDARD"
}, {
  no: 1,
  name: "INGRESS"
}, {
  no: 2,
  name: "EGRESS"
}, {
  no: 3,
  name: "SIP"
}, {
  no: 4,
  name: "AGENT"
}, {
  no: 7,
  name: "CONNECTOR"
}, {
  no: 8,
  name: "BRIDGE"
}]);
const ParticipantInfo_KindDetail = /* @__PURE__ */proto3.makeEnum("livekit.ParticipantInfo.KindDetail", [{
  no: 0,
  name: "CLOUD_AGENT"
}, {
  no: 1,
  name: "FORWARDED"
}, {
  no: 2,
  name: "CONNECTOR_WHATSAPP"
}, {
  no: 3,
  name: "CONNECTOR_TWILIO"
}, {
  no: 4,
  name: "BRIDGE_RTSP"
}, {
  no: 5,
  name: "SIMULATION"
}]);
const Encryption_Type = /* @__PURE__ */proto3.makeEnum("livekit.Encryption.Type", [{
  no: 0,
  name: "NONE"
}, {
  no: 1,
  name: "GCM"
}, {
  no: 2,
  name: "CUSTOM"
}]);
const SimulcastCodecInfo = /* @__PURE__ */proto3.makeMessageType("livekit.SimulcastCodecInfo", () => [{
  no: 1,
  name: "mime_type",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "mid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "cid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "layers",
  kind: "message",
  T: VideoLayer,
  repeated: true
}, {
  no: 5,
  name: "video_layer_mode",
  kind: "enum",
  T: proto3.getEnumType(VideoLayer_Mode)
}, {
  no: 6,
  name: "sdp_cid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const TrackInfo = /* @__PURE__ */proto3.makeMessageType("livekit.TrackInfo", () => [{
  no: 1,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "type",
  kind: "enum",
  T: proto3.getEnumType(TrackType)
}, {
  no: 3,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "muted",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 5,
  name: "width",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 6,
  name: "height",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 7,
  name: "simulcast",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 8,
  name: "disable_dtx",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 9,
  name: "source",
  kind: "enum",
  T: proto3.getEnumType(TrackSource)
}, {
  no: 10,
  name: "layers",
  kind: "message",
  T: VideoLayer,
  repeated: true
}, {
  no: 11,
  name: "mime_type",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 12,
  name: "mid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 13,
  name: "codecs",
  kind: "message",
  T: SimulcastCodecInfo,
  repeated: true
}, {
  no: 14,
  name: "stereo",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 15,
  name: "disable_red",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 16,
  name: "encryption",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 17,
  name: "stream",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 18,
  name: "version",
  kind: "message",
  T: TimedVersion
}, {
  no: 19,
  name: "audio_features",
  kind: "enum",
  T: proto3.getEnumType(AudioTrackFeature),
  repeated: true
}, {
  no: 20,
  name: "backup_codec_policy",
  kind: "enum",
  T: proto3.getEnumType(BackupCodecPolicy$1)
}, {
  no: 21,
  name: "packet_trailer_features",
  kind: "enum",
  T: proto3.getEnumType(PacketTrailerFeature),
  repeated: true
}]);
const DataTrackInfo$1 = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackInfo", () => [{
  no: 1,
  name: "pub_handle",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "encryption",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 5,
  name: "frame_encoding",
  kind: "message",
  T: DataTrackFrameEncoding,
  opt: true
}, {
  no: 6,
  name: "schema",
  kind: "message",
  T: DataTrackSchemaId,
  opt: true
}]);
const DataTrackFrameEncoding = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackFrameEncoding", () => [{
  no: 1,
  name: "well_known",
  kind: "enum",
  T: proto3.getEnumType(DataTrackFrameEncoding_WellKnownFrameEncoding),
  oneof: "value"
}, {
  no: 2,
  name: "custom",
  kind: "scalar",
  T: 9,
  oneof: "value"
}]);
const DataTrackFrameEncoding_WellKnownFrameEncoding = /* @__PURE__ */proto3.makeEnum("livekit.DataTrackFrameEncoding.WellKnownFrameEncoding", [{
  no: 0,
  name: "WELL_KNOWN_FRAME_ENCODING_UNSPECIFIED",
  localName: "UNSPECIFIED"
}, {
  no: 1,
  name: "WELL_KNOWN_FRAME_ENCODING_ROS1",
  localName: "ROS1"
}, {
  no: 2,
  name: "WELL_KNOWN_FRAME_ENCODING_CDR",
  localName: "CDR"
}, {
  no: 3,
  name: "WELL_KNOWN_FRAME_ENCODING_PROTOBUF",
  localName: "PROTOBUF"
}, {
  no: 4,
  name: "WELL_KNOWN_FRAME_ENCODING_FLATBUFFER",
  localName: "FLATBUFFER"
}, {
  no: 5,
  name: "WELL_KNOWN_FRAME_ENCODING_CBOR",
  localName: "CBOR"
}, {
  no: 6,
  name: "WELL_KNOWN_FRAME_ENCODING_MSGPACK",
  localName: "MSGPACK"
}, {
  no: 7,
  name: "WELL_KNOWN_FRAME_ENCODING_JSON",
  localName: "JSON"
}]);
const DataTrackSchemaEncoding = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackSchemaEncoding", () => [{
  no: 1,
  name: "well_known",
  kind: "enum",
  T: proto3.getEnumType(DataTrackSchemaEncoding_WellKnownSchemaEncoding),
  oneof: "value"
}, {
  no: 2,
  name: "custom",
  kind: "scalar",
  T: 9,
  oneof: "value"
}]);
const DataTrackSchemaEncoding_WellKnownSchemaEncoding = /* @__PURE__ */proto3.makeEnum("livekit.DataTrackSchemaEncoding.WellKnownSchemaEncoding", [{
  no: 0,
  name: "WELL_KNOWN_SCHEMA_ENCODING_UNSPECIFIED",
  localName: "UNSPECIFIED"
}, {
  no: 1,
  name: "WELL_KNOWN_SCHEMA_ENCODING_PROTOBUF",
  localName: "PROTOBUF"
}, {
  no: 2,
  name: "WELL_KNOWN_SCHEMA_ENCODING_FLATBUFFER",
  localName: "FLATBUFFER"
}, {
  no: 3,
  name: "WELL_KNOWN_SCHEMA_ENCODING_ROS1_MSG",
  localName: "ROS1_MSG"
}, {
  no: 4,
  name: "WELL_KNOWN_SCHEMA_ENCODING_ROS2_MSG",
  localName: "ROS2_MSG"
}, {
  no: 5,
  name: "WELL_KNOWN_SCHEMA_ENCODING_ROS2_IDL",
  localName: "ROS2_IDL"
}, {
  no: 6,
  name: "WELL_KNOWN_SCHEMA_ENCODING_OMG_IDL",
  localName: "OMG_IDL"
}, {
  no: 7,
  name: "WELL_KNOWN_SCHEMA_ENCODING_JSON_SCHEMA",
  localName: "JSON_SCHEMA"
}]);
const DataTrackSchemaId = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackSchemaId", () => [{
  no: 1,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "encoding",
  kind: "message",
  T: DataTrackSchemaEncoding
}]);
const DataTrackSubscriptionOptions = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackSubscriptionOptions", () => [{
  no: 1,
  name: "target_fps",
  kind: "scalar",
  T: 13,
  opt: true
}]);
const DataBlobKey = /* @__PURE__ */proto3.makeMessageType("livekit.DataBlobKey", () => [{
  no: 1,
  name: "generic",
  kind: "scalar",
  T: 9,
  oneof: "key"
}, {
  no: 2,
  name: "schema_id",
  kind: "message",
  T: DataTrackSchemaId,
  oneof: "key"
}]);
const DataBlob = /* @__PURE__ */proto3.makeMessageType("livekit.DataBlob", () => [{
  no: 1,
  name: "key",
  kind: "message",
  T: DataBlobKey
}, {
  no: 2,
  name: "contents",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}]);
const VideoLayer = /* @__PURE__ */proto3.makeMessageType("livekit.VideoLayer", () => [{
  no: 1,
  name: "quality",
  kind: "enum",
  T: proto3.getEnumType(VideoQuality$1)
}, {
  no: 2,
  name: "width",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 3,
  name: "height",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "bitrate",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 5,
  name: "ssrc",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 6,
  name: "spatial_layer",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 7,
  name: "rid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 8,
  name: "repair_ssrc",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const VideoLayer_Mode = /* @__PURE__ */proto3.makeEnum("livekit.VideoLayer.Mode", [{
  no: 0,
  name: "MODE_UNUSED"
}, {
  no: 1,
  name: "ONE_SPATIAL_LAYER_PER_STREAM"
}, {
  no: 2,
  name: "MULTIPLE_SPATIAL_LAYERS_PER_STREAM"
}, {
  no: 3,
  name: "ONE_SPATIAL_LAYER_PER_STREAM_INCOMPLETE_RTCP_SR"
}]);
const DataPacket = /* @__PURE__ */proto3.makeMessageType("livekit.DataPacket", () => [{
  no: 1,
  name: "kind",
  kind: "enum",
  T: proto3.getEnumType(DataPacket_Kind)
}, {
  no: 4,
  name: "participant_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "destination_identities",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 2,
  name: "user",
  kind: "message",
  T: UserPacket,
  oneof: "value"
}, {
  no: 3,
  name: "speaker",
  kind: "message",
  T: ActiveSpeakerUpdate,
  oneof: "value"
}, {
  no: 6,
  name: "sip_dtmf",
  kind: "message",
  T: SipDTMF,
  oneof: "value"
}, {
  no: 7,
  name: "transcription",
  kind: "message",
  T: Transcription,
  oneof: "value"
}, {
  no: 8,
  name: "metrics",
  kind: "message",
  T: MetricsBatch,
  oneof: "value"
}, {
  no: 9,
  name: "chat_message",
  kind: "message",
  T: ChatMessage,
  oneof: "value"
}, {
  no: 10,
  name: "rpc_request",
  kind: "message",
  T: RpcRequest,
  oneof: "value"
}, {
  no: 11,
  name: "rpc_ack",
  kind: "message",
  T: RpcAck,
  oneof: "value"
}, {
  no: 12,
  name: "rpc_response",
  kind: "message",
  T: RpcResponse,
  oneof: "value"
}, {
  no: 13,
  name: "stream_header",
  kind: "message",
  T: DataStream_Header,
  oneof: "value"
}, {
  no: 14,
  name: "stream_chunk",
  kind: "message",
  T: DataStream_Chunk,
  oneof: "value"
}, {
  no: 15,
  name: "stream_trailer",
  kind: "message",
  T: DataStream_Trailer,
  oneof: "value"
}, {
  no: 18,
  name: "encrypted_packet",
  kind: "message",
  T: EncryptedPacket,
  oneof: "value"
}, {
  no: 16,
  name: "sequence",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 17,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const DataPacket_Kind = /* @__PURE__ */proto3.makeEnum("livekit.DataPacket.Kind", [{
  no: 0,
  name: "RELIABLE"
}, {
  no: 1,
  name: "LOSSY"
}]);
const EncryptedPacket = /* @__PURE__ */proto3.makeMessageType("livekit.EncryptedPacket", () => [{
  no: 1,
  name: "encryption_type",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 2,
  name: "iv",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}, {
  no: 3,
  name: "key_index",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "encrypted_value",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}]);
const EncryptedPacketPayload = /* @__PURE__ */proto3.makeMessageType("livekit.EncryptedPacketPayload", () => [{
  no: 1,
  name: "user",
  kind: "message",
  T: UserPacket,
  oneof: "value"
}, {
  no: 3,
  name: "chat_message",
  kind: "message",
  T: ChatMessage,
  oneof: "value"
}, {
  no: 4,
  name: "rpc_request",
  kind: "message",
  T: RpcRequest,
  oneof: "value"
}, {
  no: 5,
  name: "rpc_ack",
  kind: "message",
  T: RpcAck,
  oneof: "value"
}, {
  no: 6,
  name: "rpc_response",
  kind: "message",
  T: RpcResponse,
  oneof: "value"
}, {
  no: 7,
  name: "stream_header",
  kind: "message",
  T: DataStream_Header,
  oneof: "value"
}, {
  no: 8,
  name: "stream_chunk",
  kind: "message",
  T: DataStream_Chunk,
  oneof: "value"
}, {
  no: 9,
  name: "stream_trailer",
  kind: "message",
  T: DataStream_Trailer,
  oneof: "value"
}]);
const ActiveSpeakerUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.ActiveSpeakerUpdate", () => [{
  no: 1,
  name: "speakers",
  kind: "message",
  T: SpeakerInfo,
  repeated: true
}]);
export { ActiveSpeakerUpdate, Codec, DataBlob, DataBlobKey, DataPacket, DataPacket_Kind, DataTrackFrameEncoding, DataTrackFrameEncoding_WellKnownFrameEncoding, DataTrackInfo$1, DataTrackSchemaEncoding, DataTrackSchemaEncoding_WellKnownSchemaEncoding, DataTrackSchemaId, DataTrackSubscriptionOptions, EncryptedPacket, EncryptedPacketPayload, Encryption_Type, ParticipantInfo, ParticipantInfo_Kind, ParticipantInfo_KindDetail, ParticipantInfo_State, ParticipantPermission, Room$1, SimulcastCodecInfo, TrackInfo, VideoLayer, VideoLayer_Mode };
