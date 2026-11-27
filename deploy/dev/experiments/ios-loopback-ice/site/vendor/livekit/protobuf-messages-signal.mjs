// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 6578-7588 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { AudioTrackFeature, BackupCodecPolicy$1, DisconnectReason, PacketTrailerFeature, TrackSource, TrackType, VideoQuality$1 } from './protobuf-messages-enums.mjs';
import { Codec, DataBlob, DataBlobKey, DataTrackFrameEncoding, DataTrackInfo$1, DataTrackSchemaId, DataTrackSubscriptionOptions, Encryption_Type, ParticipantInfo, Room$1, TrackInfo, VideoLayer, VideoLayer_Mode } from './protobuf-messages-room.mjs';
import { ClientConfiguration, ParticipantTracks, ServerInfo } from './protobuf-messages-chat.mjs';
import { ConnectionQualityUpdate, MediaSectionsRequirement, Ping, Pong, RegionSettings, RequestResponse, RoomMovedResponse, RoomUpdate, SimulateScenario, SpeakersChanged, StreamStateUpdate, SubscribedAudioCodecUpdate, SubscribedQualityUpdate, SubscriptionPermission, SubscriptionPermissionUpdate, SubscriptionResponse, SyncState, TrackSubscribed } from './protobuf-messages-signal2.mjs';


const SignalTarget = /* @__PURE__ */proto3.makeEnum("livekit.SignalTarget", [{
  no: 0,
  name: "PUBLISHER"
}, {
  no: 1,
  name: "SUBSCRIBER"
}]);
const StreamState = /* @__PURE__ */proto3.makeEnum("livekit.StreamState", [{
  no: 0,
  name: "ACTIVE"
}, {
  no: 1,
  name: "PAUSED"
}]);
const CandidateProtocol = /* @__PURE__ */proto3.makeEnum("livekit.CandidateProtocol", [{
  no: 0,
  name: "UDP"
}, {
  no: 1,
  name: "TCP"
}, {
  no: 2,
  name: "TLS"
}]);
const SignalRequest = /* @__PURE__ */proto3.makeMessageType("livekit.SignalRequest", () => [{
  no: 1,
  name: "offer",
  kind: "message",
  T: SessionDescription,
  oneof: "message"
}, {
  no: 2,
  name: "answer",
  kind: "message",
  T: SessionDescription,
  oneof: "message"
}, {
  no: 3,
  name: "trickle",
  kind: "message",
  T: TrickleRequest,
  oneof: "message"
}, {
  no: 4,
  name: "add_track",
  kind: "message",
  T: AddTrackRequest,
  oneof: "message"
}, {
  no: 5,
  name: "mute",
  kind: "message",
  T: MuteTrackRequest,
  oneof: "message"
}, {
  no: 6,
  name: "subscription",
  kind: "message",
  T: UpdateSubscription,
  oneof: "message"
}, {
  no: 7,
  name: "track_setting",
  kind: "message",
  T: UpdateTrackSettings,
  oneof: "message"
}, {
  no: 8,
  name: "leave",
  kind: "message",
  T: LeaveRequest,
  oneof: "message"
}, {
  no: 10,
  name: "update_layers",
  kind: "message",
  T: UpdateVideoLayers,
  oneof: "message"
}, {
  no: 11,
  name: "subscription_permission",
  kind: "message",
  T: SubscriptionPermission,
  oneof: "message"
}, {
  no: 12,
  name: "sync_state",
  kind: "message",
  T: SyncState,
  oneof: "message"
}, {
  no: 13,
  name: "simulate",
  kind: "message",
  T: SimulateScenario,
  oneof: "message"
}, {
  no: 14,
  name: "ping",
  kind: "scalar",
  T: 3,
  oneof: "message"
}, {
  no: 15,
  name: "update_metadata",
  kind: "message",
  T: UpdateParticipantMetadata,
  oneof: "message"
}, {
  no: 16,
  name: "ping_req",
  kind: "message",
  T: Ping,
  oneof: "message"
}, {
  no: 17,
  name: "update_audio_track",
  kind: "message",
  T: UpdateLocalAudioTrack,
  oneof: "message"
}, {
  no: 18,
  name: "update_video_track",
  kind: "message",
  T: UpdateLocalVideoTrack,
  oneof: "message"
}, {
  no: 19,
  name: "publish_data_track_request",
  kind: "message",
  T: PublishDataTrackRequest,
  oneof: "message"
}, {
  no: 20,
  name: "unpublish_data_track_request",
  kind: "message",
  T: UnpublishDataTrackRequest,
  oneof: "message"
}, {
  no: 21,
  name: "update_data_subscription",
  kind: "message",
  T: UpdateDataSubscription,
  oneof: "message"
}, {
  no: 22,
  name: "store_data_blob_request",
  kind: "message",
  T: StoreDataBlobRequest,
  oneof: "message"
}, {
  no: 23,
  name: "get_data_blob_request",
  kind: "message",
  T: GetDataBlobRequest,
  oneof: "message"
}]);
const SignalResponse = /* @__PURE__ */proto3.makeMessageType("livekit.SignalResponse", () => [{
  no: 1,
  name: "join",
  kind: "message",
  T: JoinResponse,
  oneof: "message"
}, {
  no: 2,
  name: "answer",
  kind: "message",
  T: SessionDescription,
  oneof: "message"
}, {
  no: 3,
  name: "offer",
  kind: "message",
  T: SessionDescription,
  oneof: "message"
}, {
  no: 4,
  name: "trickle",
  kind: "message",
  T: TrickleRequest,
  oneof: "message"
}, {
  no: 5,
  name: "update",
  kind: "message",
  T: ParticipantUpdate,
  oneof: "message"
}, {
  no: 6,
  name: "track_published",
  kind: "message",
  T: TrackPublishedResponse,
  oneof: "message"
}, {
  no: 8,
  name: "leave",
  kind: "message",
  T: LeaveRequest,
  oneof: "message"
}, {
  no: 9,
  name: "mute",
  kind: "message",
  T: MuteTrackRequest,
  oneof: "message"
}, {
  no: 10,
  name: "speakers_changed",
  kind: "message",
  T: SpeakersChanged,
  oneof: "message"
}, {
  no: 11,
  name: "room_update",
  kind: "message",
  T: RoomUpdate,
  oneof: "message"
}, {
  no: 12,
  name: "connection_quality",
  kind: "message",
  T: ConnectionQualityUpdate,
  oneof: "message"
}, {
  no: 13,
  name: "stream_state_update",
  kind: "message",
  T: StreamStateUpdate,
  oneof: "message"
}, {
  no: 14,
  name: "subscribed_quality_update",
  kind: "message",
  T: SubscribedQualityUpdate,
  oneof: "message"
}, {
  no: 15,
  name: "subscription_permission_update",
  kind: "message",
  T: SubscriptionPermissionUpdate,
  oneof: "message"
}, {
  no: 16,
  name: "refresh_token",
  kind: "scalar",
  T: 9,
  oneof: "message"
}, {
  no: 17,
  name: "track_unpublished",
  kind: "message",
  T: TrackUnpublishedResponse,
  oneof: "message"
}, {
  no: 18,
  name: "pong",
  kind: "scalar",
  T: 3,
  oneof: "message"
}, {
  no: 19,
  name: "reconnect",
  kind: "message",
  T: ReconnectResponse,
  oneof: "message"
}, {
  no: 20,
  name: "pong_resp",
  kind: "message",
  T: Pong,
  oneof: "message"
}, {
  no: 21,
  name: "subscription_response",
  kind: "message",
  T: SubscriptionResponse,
  oneof: "message"
}, {
  no: 22,
  name: "request_response",
  kind: "message",
  T: RequestResponse,
  oneof: "message"
}, {
  no: 23,
  name: "track_subscribed",
  kind: "message",
  T: TrackSubscribed,
  oneof: "message"
}, {
  no: 24,
  name: "room_moved",
  kind: "message",
  T: RoomMovedResponse,
  oneof: "message"
}, {
  no: 25,
  name: "media_sections_requirement",
  kind: "message",
  T: MediaSectionsRequirement,
  oneof: "message"
}, {
  no: 26,
  name: "subscribed_audio_codec_update",
  kind: "message",
  T: SubscribedAudioCodecUpdate,
  oneof: "message"
}, {
  no: 27,
  name: "publish_data_track_response",
  kind: "message",
  T: PublishDataTrackResponse,
  oneof: "message"
}, {
  no: 28,
  name: "unpublish_data_track_response",
  kind: "message",
  T: UnpublishDataTrackResponse,
  oneof: "message"
}, {
  no: 29,
  name: "data_track_subscriber_handles",
  kind: "message",
  T: DataTrackSubscriberHandles,
  oneof: "message"
}, {
  no: 30,
  name: "store_data_blob_response",
  kind: "message",
  T: StoreDataBlobResponse,
  oneof: "message"
}, {
  no: 31,
  name: "get_data_blob_response",
  kind: "message",
  T: GetDataBlobResponse,
  oneof: "message"
}]);
const SimulcastCodec = /* @__PURE__ */proto3.makeMessageType("livekit.SimulcastCodec", () => [{
  no: 1,
  name: "codec",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
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
}]);
const AddTrackRequest = /* @__PURE__ */proto3.makeMessageType("livekit.AddTrackRequest", () => [{
  no: 1,
  name: "cid",
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
  name: "type",
  kind: "enum",
  T: proto3.getEnumType(TrackType)
}, {
  no: 4,
  name: "width",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 5,
  name: "height",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 6,
  name: "muted",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 7,
  name: "disable_dtx",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 8,
  name: "source",
  kind: "enum",
  T: proto3.getEnumType(TrackSource)
}, {
  no: 9,
  name: "layers",
  kind: "message",
  T: VideoLayer,
  repeated: true
}, {
  no: 10,
  name: "simulcast_codecs",
  kind: "message",
  T: SimulcastCodec,
  repeated: true
}, {
  no: 11,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 12,
  name: "stereo",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 13,
  name: "disable_red",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 14,
  name: "encryption",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 15,
  name: "stream",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 16,
  name: "backup_codec_policy",
  kind: "enum",
  T: proto3.getEnumType(BackupCodecPolicy$1)
}, {
  no: 17,
  name: "audio_features",
  kind: "enum",
  T: proto3.getEnumType(AudioTrackFeature),
  repeated: true
}, {
  no: 18,
  name: "packet_trailer_features",
  kind: "enum",
  T: proto3.getEnumType(PacketTrailerFeature),
  repeated: true
}]);
const PublishDataTrackRequest = /* @__PURE__ */proto3.makeMessageType("livekit.PublishDataTrackRequest", () => [{
  no: 1,
  name: "pub_handle",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "encryption",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 4,
  name: "frame_encoding",
  kind: "message",
  T: DataTrackFrameEncoding,
  opt: true
}, {
  no: 5,
  name: "schema",
  kind: "message",
  T: DataTrackSchemaId,
  opt: true
}]);
const PublishDataTrackResponse = /* @__PURE__ */proto3.makeMessageType("livekit.PublishDataTrackResponse", () => [{
  no: 1,
  name: "info",
  kind: "message",
  T: DataTrackInfo$1
}]);
const UnpublishDataTrackRequest = /* @__PURE__ */proto3.makeMessageType("livekit.UnpublishDataTrackRequest", () => [{
  no: 1,
  name: "pub_handle",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const UnpublishDataTrackResponse = /* @__PURE__ */proto3.makeMessageType("livekit.UnpublishDataTrackResponse", () => [{
  no: 1,
  name: "info",
  kind: "message",
  T: DataTrackInfo$1
}]);
const DataTrackSubscriberHandles = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackSubscriberHandles", () => [{
  no: 1,
  name: "sub_handles",
  kind: "map",
  K: 13,
  V: {
    kind: "message",
    T: DataTrackSubscriberHandles_PublishedDataTrack
  }
}]);
const DataTrackSubscriberHandles_PublishedDataTrack = /* @__PURE__ */proto3.makeMessageType("livekit.DataTrackSubscriberHandles.PublishedDataTrack", () => [{
  no: 1,
  name: "publisher_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "publisher_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}], {
  localName: "DataTrackSubscriberHandles_PublishedDataTrack"
});
const TrickleRequest = /* @__PURE__ */proto3.makeMessageType("livekit.TrickleRequest", () => [{
  no: 1,
  name: "candidateInit",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "target",
  kind: "enum",
  T: proto3.getEnumType(SignalTarget)
}, {
  no: 3,
  name: "final",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const MuteTrackRequest = /* @__PURE__ */proto3.makeMessageType("livekit.MuteTrackRequest", () => [{
  no: 1,
  name: "sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "muted",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const JoinResponse = /* @__PURE__ */proto3.makeMessageType("livekit.JoinResponse", () => [{
  no: 1,
  name: "room",
  kind: "message",
  T: Room$1
}, {
  no: 2,
  name: "participant",
  kind: "message",
  T: ParticipantInfo
}, {
  no: 3,
  name: "other_participants",
  kind: "message",
  T: ParticipantInfo,
  repeated: true
}, {
  no: 4,
  name: "server_version",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "ice_servers",
  kind: "message",
  T: ICEServer,
  repeated: true
}, {
  no: 6,
  name: "subscriber_primary",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 7,
  name: "alternative_url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 8,
  name: "client_configuration",
  kind: "message",
  T: ClientConfiguration
}, {
  no: 9,
  name: "server_region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 10,
  name: "ping_timeout",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 11,
  name: "ping_interval",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 12,
  name: "server_info",
  kind: "message",
  T: ServerInfo
}, {
  no: 13,
  name: "sif_trailer",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}, {
  no: 14,
  name: "enabled_publish_codecs",
  kind: "message",
  T: Codec,
  repeated: true
}, {
  no: 15,
  name: "fast_publish",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const ReconnectResponse = /* @__PURE__ */proto3.makeMessageType("livekit.ReconnectResponse", () => [{
  no: 1,
  name: "ice_servers",
  kind: "message",
  T: ICEServer,
  repeated: true
}, {
  no: 2,
  name: "client_configuration",
  kind: "message",
  T: ClientConfiguration
}, {
  no: 3,
  name: "server_info",
  kind: "message",
  T: ServerInfo
}, {
  no: 4,
  name: "last_message_seq",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const TrackPublishedResponse = /* @__PURE__ */proto3.makeMessageType("livekit.TrackPublishedResponse", () => [{
  no: 1,
  name: "cid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "track",
  kind: "message",
  T: TrackInfo
}]);
const TrackUnpublishedResponse = /* @__PURE__ */proto3.makeMessageType("livekit.TrackUnpublishedResponse", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const SessionDescription = /* @__PURE__ */proto3.makeMessageType("livekit.SessionDescription", () => [{
  no: 1,
  name: "type",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "sdp",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "mid_to_track_id",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}]);
const ParticipantUpdate = /* @__PURE__ */proto3.makeMessageType("livekit.ParticipantUpdate", () => [{
  no: 1,
  name: "participants",
  kind: "message",
  T: ParticipantInfo,
  repeated: true
}]);
const UpdateSubscription = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateSubscription", () => [{
  no: 1,
  name: "track_sids",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 2,
  name: "subscribe",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "participant_tracks",
  kind: "message",
  T: ParticipantTracks,
  repeated: true
}]);
const UpdateDataSubscription = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateDataSubscription", () => [{
  no: 1,
  name: "updates",
  kind: "message",
  T: UpdateDataSubscription_Update,
  repeated: true
}]);
const UpdateDataSubscription_Update = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateDataSubscription.Update", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "subscribe",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "options",
  kind: "message",
  T: DataTrackSubscriptionOptions
}], {
  localName: "UpdateDataSubscription_Update"
});
const StoreDataBlobRequest = /* @__PURE__ */proto3.makeMessageType("livekit.StoreDataBlobRequest", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "blob",
  kind: "message",
  T: DataBlob
}]);
const StoreDataBlobResponse = /* @__PURE__ */proto3.makeMessageType("livekit.StoreDataBlobResponse", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "key",
  kind: "message",
  T: DataBlobKey
}]);
const GetDataBlobRequest = /* @__PURE__ */proto3.makeMessageType("livekit.GetDataBlobRequest", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "participant_identity",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "key",
  kind: "message",
  T: DataBlobKey
}]);
const GetDataBlobResponse = /* @__PURE__ */proto3.makeMessageType("livekit.GetDataBlobResponse", () => [{
  no: 1,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "blob",
  kind: "message",
  T: DataBlob
}]);
const UpdateTrackSettings = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateTrackSettings", () => [{
  no: 1,
  name: "track_sids",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 3,
  name: "disabled",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 4,
  name: "quality",
  kind: "enum",
  T: proto3.getEnumType(VideoQuality$1)
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
  name: "fps",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 8,
  name: "priority",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const UpdateLocalAudioTrack = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateLocalAudioTrack", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "features",
  kind: "enum",
  T: proto3.getEnumType(AudioTrackFeature),
  repeated: true
}]);
const UpdateLocalVideoTrack = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateLocalVideoTrack", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
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
}]);
const LeaveRequest = /* @__PURE__ */proto3.makeMessageType("livekit.LeaveRequest", () => [{
  no: 1,
  name: "can_reconnect",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 2,
  name: "reason",
  kind: "enum",
  T: proto3.getEnumType(DisconnectReason)
}, {
  no: 3,
  name: "action",
  kind: "enum",
  T: proto3.getEnumType(LeaveRequest_Action)
}, {
  no: 4,
  name: "regions",
  kind: "message",
  T: RegionSettings
}]);
const LeaveRequest_Action = /* @__PURE__ */proto3.makeEnum("livekit.LeaveRequest.Action", [{
  no: 0,
  name: "DISCONNECT"
}, {
  no: 1,
  name: "RESUME"
}, {
  no: 2,
  name: "RECONNECT"
}]);
const UpdateVideoLayers = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateVideoLayers", () => [{
  no: 1,
  name: "track_sid",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "layers",
  kind: "message",
  T: VideoLayer,
  repeated: true
}]);
const UpdateParticipantMetadata = /* @__PURE__ */proto3.makeMessageType("livekit.UpdateParticipantMetadata", () => [{
  no: 1,
  name: "metadata",
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
  name: "attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 4,
  name: "request_id",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const ICEServer = /* @__PURE__ */proto3.makeMessageType("livekit.ICEServer", () => [{
  no: 1,
  name: "urls",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 2,
  name: "username",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "credential",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
export { AddTrackRequest, CandidateProtocol, DataTrackSubscriberHandles, DataTrackSubscriberHandles_PublishedDataTrack, GetDataBlobRequest, GetDataBlobResponse, ICEServer, JoinResponse, LeaveRequest, LeaveRequest_Action, MuteTrackRequest, ParticipantUpdate, PublishDataTrackRequest, PublishDataTrackResponse, ReconnectResponse, SessionDescription, SignalRequest, SignalResponse, SignalTarget, SimulcastCodec, StoreDataBlobRequest, StoreDataBlobResponse, StreamState, TrackPublishedResponse, TrackUnpublishedResponse, TrickleRequest, UnpublishDataTrackRequest, UnpublishDataTrackResponse, UpdateDataSubscription, UpdateDataSubscription_Update, UpdateLocalAudioTrack, UpdateLocalVideoTrack, UpdateParticipantMetadata, UpdateSubscription, UpdateTrackSettings, UpdateVideoLayers };
