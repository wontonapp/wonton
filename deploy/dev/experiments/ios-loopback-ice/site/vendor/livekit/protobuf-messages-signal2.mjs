// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 7588-8278 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { ConnectionQuality$1, ReconnectReason, SubscriptionError, VideoQuality$1 } from './protobuf-messages-enums.mjs';
import { ParticipantInfo, Room$1 } from './protobuf-messages-room.mjs';
import { ClientInfo, SpeakerInfo } from './protobuf-messages-chat.mjs';
import { RoomConfiguration, SubscribedAudioCodec } from './protobuf-messages-stream.mjs';
import { AddTrackRequest, CandidateProtocol, MuteTrackRequest, PublishDataTrackRequest, PublishDataTrackResponse, SessionDescription, SignalTarget, StreamState, TrackPublishedResponse, TrickleRequest, UnpublishDataTrackRequest, UpdateDataSubscription, UpdateLocalAudioTrack, UpdateLocalVideoTrack, UpdateParticipantMetadata, UpdateSubscription } from './protobuf-messages-signal.mjs';


const SpeakersChanged = /* @__PURE__ */proto3.makeMessageType("livekit.SpeakersChanged", () => [{
  no: 1,
  name: "speakers",
  kind: "message",
  T: SpeakerInfo,
  repeated: true
}]);
const RoomUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.RoomUpdate", () => [{
  no: 1,
  name: "room",
  kind: "message",
  T: Room$1
}]);
const ConnectionQualityInfo = /* @__PURE__ */proto3.makeMessageType("livekit.ConnectionQualityInfo", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "quality",
  kind: "enum",
  T: proto3.getEnumType(ConnectionQuality$1)
}, {
  no: 3,
  name: "score",
  kind: "scalar",
  T: 2
  /* ScalarType.FLOAT */
}]);
const ConnectionQualityUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.ConnectionQualityUpdate", () => [{
  no: 1,
  name: "updates",
  kind: "message",
  T: ConnectionQualityInfo,
  repeated: true
}]);
const StreamStateInfo = /* @__PURE__ */proto3.makeMessageType("livekit.StreamStateInfo", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "state",
  kind: "enum",
  T: proto3.getEnumType(StreamState)
}]);
const StreamStateUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.StreamStateUpdate", () => [{
  no: 1,
  name: "stream_states",
  kind: "message",
  T: StreamStateInfo,
  repeated: true
}]);
const SubscribedQuality = /* @__PURE__ */proto3.makeMessageType("livekit.SubscribedQuality", () => [{
  no: 1,
  name: "quality",
  kind: "enum",
  T: proto3.getEnumType(VideoQuality$1)
}, {
  no: 2,
  name: "enabled",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const SubscribedCodec = /* @__PURE__ */proto3.makeMessageType("livekit.SubscribedCodec", () => [{
  no: 1,
  name: "codec",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "qualities",
  kind: "message",
  T: SubscribedQuality,
  repeated: true
}]);
const SubscribedQualityUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.SubscribedQualityUpdate", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "subscribed_qualities",
  kind: "message",
  T: SubscribedQuality,
  repeated: true
}, {
  no: 3,
  name: "subscribed_codecs",
  kind: "message",
  T: SubscribedCodec,
  repeated: true
}]);
const SubscribedAudioCodecUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.SubscribedAudioCodecUpdate", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "subscribed_audio_codecs",
  kind: "message",
  T: SubscribedAudioCodec,
  repeated: true
}]);
const TrackPermission = /* @__PURE__ */proto3.makeMessageType("livekit.TrackPermission", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "all_tracks",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "track_sids",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 4,
  name: "participant_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const SubscriptionPermission = /* @__PURE__ */proto3.makeMessageType("livekit.SubscriptionPermission", () => [{
  no: 1,
  name: "all_participants",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 2,
  name: "track_permissions",
  kind: "message",
  T: TrackPermission,
  repeated: true
}]);
const SubscriptionPermissionUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.SubscriptionPermissionUpdate", () => [{
  no: 1,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "allowed",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const RoomMovedResponse = /* @__PURE__ */proto3.makeMessageType("livekit.RoomMovedResponse", () => [{
  no: 1,
  name: "room",
  kind: "message",
  T: Room$1
}, {
  no: 2,
  name: "token",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "participant",
  kind: "message",
  T: ParticipantInfo
}, {
  no: 4,
  name: "other_participants",
  kind: "message",
  T: ParticipantInfo,
  repeated: true
}]);
const SyncState = /* @__PURE__ */proto3.makeMessageType("livekit.SyncState", () => [{
  no: 1,
  name: "answer",
  kind: "message",
  T: SessionDescription
}, {
  no: 2,
  name: "subscription",
  kind: "message",
  T: UpdateSubscription
}, {
  no: 3,
  name: "publish_tracks",
  kind: "message",
  T: TrackPublishedResponse,
  repeated: true
}, {
  no: 4,
  name: "data_channels",
  kind: "message",
  T: DataChannelInfo,
  repeated: true
}, {
  no: 5,
  name: "offer",
  kind: "message",
  T: SessionDescription
}, {
  no: 6,
  name: "track_sids_disabled",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 7,
  name: "datachannel_receive_states",
  kind: "message",
  T: DataChannelReceiveState,
  repeated: true
}, {
  no: 8,
  name: "publish_data_tracks",
  kind: "message",
  T: PublishDataTrackResponse,
  repeated: true
}, {
  no: 9,
  name: "data_subscription",
  kind: "message",
  T: UpdateDataSubscription
}]);
const DataChannelReceiveState = /* @__PURE__ */proto3.makeMessageType("livekit.DataChannelReceiveState", () => [{
  no: 1,
  name: "publisher_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "last_seq",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const DataChannelInfo = /* @__PURE__ */proto3.makeMessageType("livekit.DataChannelInfo", () => [{
  no: 1,
  name: "label",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 3,
  name: "target",
  kind: "enum",
  T: proto3.getEnumType(SignalTarget)
}]);
const SimulateScenario = /* @__PURE__ */proto3.makeMessageType("livekit.SimulateScenario", () => [{
  no: 1,
  name: "speaker_update",
  kind: "scalar",
  T: 5,
  oneof: "scenario"
}, {
  no: 2,
  name: "node_failure",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}, {
  no: 3,
  name: "migration",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}, {
  no: 4,
  name: "server_leave",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}, {
  no: 5,
  name: "switch_candidate_protocol",
  kind: "enum",
  T: proto3.getEnumType(CandidateProtocol),
  oneof: "scenario"
}, {
  no: 6,
  name: "subscriber_bandwidth",
  kind: "scalar",
  T: 3,
  oneof: "scenario"
}, {
  no: 7,
  name: "disconnect_signal_on_resume",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}, {
  no: 8,
  name: "disconnect_signal_on_resume_no_messages",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}, {
  no: 9,
  name: "leave_request_full_reconnect",
  kind: "scalar",
  T: 8,
  oneof: "scenario"
}]);
const Ping = /* @__PURE__ */proto3.makeMessageType("livekit.Ping", () => [{
  no: 1,
  name: "timestamp",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 2,
  name: "rtt",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}]);
const Pong = /* @__PURE__ */proto3.makeMessageType("livekit.Pong", () => [{
  no: 1,
  name: "last_ping_timestamp",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 2,
  name: "timestamp",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}]);
const RegionSettings = /* @__PURE__ */proto3.makeMessageType("livekit.RegionSettings", () => [{
  no: 1,
  name: "regions",
  kind: "message",
  T: RegionInfo,
  repeated: true
}]);
const RegionInfo = /* @__PURE__ */proto3.makeMessageType("livekit.RegionInfo", () => [{
  no: 1,
  name: "region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "distance",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}]);
const SubscriptionResponse = /* @__PURE__ */proto3.makeMessageType("livekit.SubscriptionResponse", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "err",
  kind: "enum",
  T: proto3.getEnumType(SubscriptionError)
}]);
const RequestResponse = /* @__PURE__ */proto3.makeMessageType("livekit.RequestResponse", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "reason",
  kind: "enum",
  T: proto3.getEnumType(RequestResponse_Reason)
}, {
  no: 3,
  name: "message",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "trickle",
  kind: "message",
  T: TrickleRequest,
  oneof: "request"
}, {
  no: 5,
  name: "add_track",
  kind: "message",
  T: AddTrackRequest,
  oneof: "request"
}, {
  no: 6,
  name: "mute",
  kind: "message",
  T: MuteTrackRequest,
  oneof: "request"
}, {
  no: 7,
  name: "update_metadata",
  kind: "message",
  T: UpdateParticipantMetadata,
  oneof: "request"
}, {
  no: 8,
  name: "update_audio_track",
  kind: "message",
  T: UpdateLocalAudioTrack,
  oneof: "request"
}, {
  no: 9,
  name: "update_video_track",
  kind: "message",
  T: UpdateLocalVideoTrack,
  oneof: "request"
}, {
  no: 10,
  name: "publish_data_track",
  kind: "message",
  T: PublishDataTrackRequest,
  oneof: "request"
}, {
  no: 11,
  name: "unpublish_data_track",
  kind: "message",
  T: UnpublishDataTrackRequest,
  oneof: "request"
}]);
const RequestResponse_Reason = /* @__PURE__ */proto3.makeEnum("livekit.RequestResponse.Reason", [{
  no: 0,
  name: "OK"
}, {
  no: 1,
  name: "NOT_FOUND"
}, {
  no: 2,
  name: "NOT_ALLOWED"
}, {
  no: 3,
  name: "LIMIT_EXCEEDED"
}, {
  no: 4,
  name: "QUEUED"
}, {
  no: 5,
  name: "UNSUPPORTED_TYPE"
}, {
  no: 6,
  name: "UNCLASSIFIED_ERROR"
}, {
  no: 7,
  name: "INVALID_HANDLE"
}, {
  no: 8,
  name: "INVALID_NAME"
}, {
  no: 9,
  name: "DUPLICATE_HANDLE"
}, {
  no: 10,
  name: "DUPLICATE_NAME"
}, {
  no: 11,
  name: "INVALID_REQUEST"
}]);
const TrackSubscribed = /* @__PURE__ */proto3.makeMessageType("livekit.TrackSubscribed", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const ConnectionSettings = /* @__PURE__ */proto3.makeMessageType("livekit.ConnectionSettings", () => [{
  no: 1,
  name: "auto_subscribe",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 2,
  name: "adaptive_stream",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "subscriber_allow_pause",
  kind: "scalar",
  T: 8,
  opt: true
}, {
  no: 4,
  name: "disable_ice_lite",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 5,
  name: "auto_subscribe_data_track",
  kind: "scalar",
  T: 8,
  opt: true
}]);
const JoinRequest = /* @__PURE__ */proto3.makeMessageType("livekit.JoinRequest", () => [{
  no: 1,
  name: "client_info",
  kind: "message",
  T: ClientInfo
}, {
  no: 2,
  name: "connection_settings",
  kind: "message",
  T: ConnectionSettings
}, {
  no: 3,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "participant_attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 5,
  name: "add_track_requests",
  kind: "message",
  T: AddTrackRequest,
  repeated: true
}, {
  no: 6,
  name: "publisher_offer",
  kind: "message",
  T: SessionDescription
}, {
  no: 7,
  name: "reconnect",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 8,
  name: "reconnect_reason",
  kind: "enum",
  T: proto3.getEnumType(ReconnectReason)
}, {
  no: 9,
  name: "participant_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 10,
  name: "sync_state",
  kind: "message",
  T: SyncState
}]);
const WrappedJoinRequest = /* @__PURE__ */proto3.makeMessageType("livekit.WrappedJoinRequest", () => [{
  no: 1,
  name: "compression",
  kind: "enum",
  T: proto3.getEnumType(WrappedJoinRequest_Compression)
}, {
  no: 2,
  name: "join_request",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}]);
const WrappedJoinRequest_Compression = /* @__PURE__ */proto3.makeEnum("livekit.WrappedJoinRequest.Compression", [{
  no: 0,
  name: "NONE"
}, {
  no: 1,
  name: "GZIP"
}]);
const MediaSectionsRequirement = /* @__PURE__ */proto3.makeMessageType("livekit.MediaSectionsRequirement", () => [{
  no: 1,
  name: "num_audios",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "num_videos",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const TokenSourceRequest = /* @__PURE__ */proto3.makeMessageType("livekit.TokenSourceRequest", () => [{
  no: 1,
  name: "room_name",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 2,
  name: "participant_name",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 3,
  name: "participant_identity",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 4,
  name: "participant_metadata",
  kind: "scalar",
  T: 9,
  opt: true
}, {
  no: 5,
  name: "participant_attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 6,
  name: "room_config",
  kind: "message",
  T: RoomConfiguration,
  opt: true
}]);
const TokenSourceResponse = /* @__PURE__ */proto3.makeMessageType("livekit.TokenSourceResponse", () => [{
  no: 1,
  name: "server_url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "participant_token",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);function getDefaultExportFromCjs (x) {
	return x && x.__esModule && Object.prototype.hasOwnProperty.call(x, 'default') ? x['default'] : x;
}
export { ConnectionQualityInfo, ConnectionQualityUpdate, ConnectionSettings, DataChannelInfo, DataChannelReceiveState, JoinRequest, MediaSectionsRequirement, Ping, Pong, RegionInfo, RegionSettings, RequestResponse, RequestResponse_Reason, RoomMovedResponse, RoomUpdate, SimulateScenario, SpeakersChanged, StreamStateInfo, StreamStateUpdate, SubscribedAudioCodecUpdate, SubscribedCodec, SubscribedQuality, SubscribedQualityUpdate, SubscriptionPermission, SubscriptionPermissionUpdate, SubscriptionResponse, SyncState, TokenSourceRequest, TokenSourceResponse, TrackPermission, TrackSubscribed, WrappedJoinRequest, WrappedJoinRequest_Compression, getDefaultExportFromCjs };
