// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 5560-6578 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { AudioCodec } from './protobuf-messages-metrics.mjs';
import { ImageCodec, VideoCodec } from './protobuf-messages-enums.mjs';
import { Encryption_Type } from './protobuf-messages-room.mjs';
import { DataStream_OperationType } from './protobuf-messages-chat.mjs';


const DataStream_CompressionType = /* @__PURE__ */proto3.makeEnum("livekit.DataStream.CompressionType", [{
  no: 0,
  name: "NONE"
}, {
  no: 1,
  name: "DEFLATE_RAW"
}]);
const DataStream_TextHeader = /* @__PURE__ */proto3.makeMessageType("livekit.DataStream.TextHeader", () => [{
  no: 1,
  name: "operation_type",
  kind: "enum",
  T: proto3.getEnumType(DataStream_OperationType)
}, {
  no: 2,
  name: "version",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 3,
  name: "reply_to_stream_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "attached_stream_ids",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 5,
  name: "generated",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}], {
  localName: "DataStream_TextHeader"
});
const DataStream_ByteHeader = /* @__PURE__ */proto3.makeMessageType("livekit.DataStream.ByteHeader", () => [{
  no: 1,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}], {
  localName: "DataStream_ByteHeader"
});
const DataStream_Header = /* @__PURE__ */proto3.makeMessageType("livekit.DataStream.Header", () => [{
  no: 1,
  name: "stream_id",
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
  name: "topic",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "mime_type",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "total_length",
  kind: "scalar",
  T: 4,
  opt: true
}, {
  no: 7,
  name: "encryption_type",
  kind: "enum",
  T: proto3.getEnumType(Encryption_Type)
}, {
  no: 8,
  name: "attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 9,
  name: "text_header",
  kind: "message",
  T: DataStream_TextHeader,
  oneof: "content_header"
}, {
  no: 10,
  name: "byte_header",
  kind: "message",
  T: DataStream_ByteHeader,
  oneof: "content_header"
}, {
  no: 11,
  name: "inline_content",
  kind: "scalar",
  T: 12,
  opt: true
}, {
  no: 12,
  name: "compression",
  kind: "enum",
  T: proto3.getEnumType(DataStream_CompressionType)
}], {
  localName: "DataStream_Header"
});
const DataStream_Chunk = /* @__PURE__ */proto3.makeMessageType("livekit.DataStream.Chunk", () => [{
  no: 1,
  name: "stream_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "chunk_index",
  kind: "scalar",
  T: 4
  /* ScalarType.UINT64 */
}, {
  no: 3,
  name: "content",
  kind: "scalar",
  T: 12
  /* ScalarType.BYTES */
}, {
  no: 4,
  name: "version",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 5,
  name: "iv",
  kind: "scalar",
  T: 12,
  opt: true
}], {
  localName: "DataStream_Chunk"
});
const DataStream_Trailer = /* @__PURE__ */proto3.makeMessageType("livekit.DataStream.Trailer", () => [{
  no: 1,
  name: "stream_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "reason",
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
}], {
  localName: "DataStream_Trailer"
});
const FilterParams = /* @__PURE__ */proto3.makeMessageType("livekit.FilterParams", () => [{
  no: 1,
  name: "include_events",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 2,
  name: "exclude_events",
  kind: "scalar",
  T: 9,
  repeated: true
}]);
const WebhookConfig = /* @__PURE__ */proto3.makeMessageType("livekit.WebhookConfig", () => [{
  no: 1,
  name: "url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "signing_key",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "filter_params",
  kind: "message",
  T: FilterParams
}]);
const SubscribedAudioCodec = /* @__PURE__ */proto3.makeMessageType("livekit.SubscribedAudioCodec", () => [{
  no: 1,
  name: "codec",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "enabled",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}]);
const JobRestartPolicy = /* @__PURE__ */proto3.makeEnum("livekit.JobRestartPolicy", [{
  no: 0,
  name: "JRP_ON_FAILURE"
}, {
  no: 1,
  name: "JRP_NEVER"
}]);
const RoomAgentDispatch = /* @__PURE__ */proto3.makeMessageType("livekit.RoomAgentDispatch", () => [{
  no: 1,
  name: "agent_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "restart_policy",
  kind: "enum",
  T: proto3.getEnumType(JobRestartPolicy)
}, {
  no: 4,
  name: "deployment",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "attributes",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}]);
const EncodingOptionsPreset = /* @__PURE__ */proto3.makeEnum("livekit.EncodingOptionsPreset", [{
  no: 0,
  name: "H264_720P_30"
}, {
  no: 1,
  name: "H264_720P_60"
}, {
  no: 2,
  name: "H264_1080P_30"
}, {
  no: 3,
  name: "H264_1080P_60"
}, {
  no: 4,
  name: "PORTRAIT_H264_720P_30"
}, {
  no: 5,
  name: "PORTRAIT_H264_720P_60"
}, {
  no: 6,
  name: "PORTRAIT_H264_1080P_30"
}, {
  no: 7,
  name: "PORTRAIT_H264_1080P_60"
}, {
  no: 8,
  name: "PASSTHROUGH"
}]);
const EncodedFileType = /* @__PURE__ */proto3.makeEnum("livekit.EncodedFileType", [{
  no: 0,
  name: "DEFAULT_FILETYPE"
}, {
  no: 1,
  name: "MP4"
}, {
  no: 2,
  name: "OGG"
}, {
  no: 3,
  name: "MP3"
}]);
const StreamProtocol = /* @__PURE__ */proto3.makeEnum("livekit.StreamProtocol", [{
  no: 0,
  name: "DEFAULT_PROTOCOL"
}, {
  no: 1,
  name: "RTMP"
}, {
  no: 2,
  name: "SRT"
}, {
  no: 3,
  name: "WEBSOCKET"
}]);
const SegmentedFileProtocol = /* @__PURE__ */proto3.makeEnum("livekit.SegmentedFileProtocol", [{
  no: 0,
  name: "DEFAULT_SEGMENTED_FILE_PROTOCOL"
}, {
  no: 1,
  name: "HLS_PROTOCOL"
}]);
const SegmentedFileSuffix = /* @__PURE__ */proto3.makeEnum("livekit.SegmentedFileSuffix", [{
  no: 0,
  name: "INDEX"
}, {
  no: 1,
  name: "TIMESTAMP"
}]);
const ImageFileSuffix = /* @__PURE__ */proto3.makeEnum("livekit.ImageFileSuffix", [{
  no: 0,
  name: "IMAGE_SUFFIX_INDEX"
}, {
  no: 1,
  name: "IMAGE_SUFFIX_TIMESTAMP"
}, {
  no: 2,
  name: "IMAGE_SUFFIX_NONE_OVERWRITE"
}]);
const AudioMixing = /* @__PURE__ */proto3.makeEnum("livekit.AudioMixing", [{
  no: 0,
  name: "DEFAULT_MIXING"
}, {
  no: 1,
  name: "DUAL_CHANNEL_AGENT"
}, {
  no: 2,
  name: "DUAL_CHANNEL_ALTERNATE"
}]);
const EncodingOptions = /* @__PURE__ */proto3.makeMessageType("livekit.EncodingOptions", () => [{
  no: 1,
  name: "width",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 2,
  name: "height",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 3,
  name: "depth",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 4,
  name: "framerate",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 5,
  name: "audio_codec",
  kind: "enum",
  T: proto3.getEnumType(AudioCodec)
}, {
  no: 6,
  name: "audio_bitrate",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 7,
  name: "audio_frequency",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 8,
  name: "video_codec",
  kind: "enum",
  T: proto3.getEnumType(VideoCodec)
}, {
  no: 9,
  name: "video_bitrate",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 10,
  name: "key_frame_interval",
  kind: "scalar",
  T: 1
  /* ScalarType.DOUBLE */
}, {
  no: 11,
  name: "audio_quality",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 12,
  name: "video_quality",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}]);
const StreamOutput = /* @__PURE__ */proto3.makeMessageType("livekit.StreamOutput", () => [{
  no: 1,
  name: "protocol",
  kind: "enum",
  T: proto3.getEnumType(StreamProtocol)
}, {
  no: 2,
  name: "urls",
  kind: "scalar",
  T: 9,
  repeated: true
}]);
const SegmentedFileOutput = /* @__PURE__ */proto3.makeMessageType("livekit.SegmentedFileOutput", () => [{
  no: 1,
  name: "protocol",
  kind: "enum",
  T: proto3.getEnumType(SegmentedFileProtocol)
}, {
  no: 2,
  name: "filename_prefix",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "playlist_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 11,
  name: "live_playlist_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "segment_duration",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 10,
  name: "filename_suffix",
  kind: "enum",
  T: proto3.getEnumType(SegmentedFileSuffix)
}, {
  no: 8,
  name: "disable_manifest",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 5,
  name: "s3",
  kind: "message",
  T: S3Upload,
  oneof: "output"
}, {
  no: 6,
  name: "gcp",
  kind: "message",
  T: GCPUpload,
  oneof: "output"
}, {
  no: 7,
  name: "azure",
  kind: "message",
  T: AzureBlobUpload,
  oneof: "output"
}, {
  no: 9,
  name: "aliOSS",
  kind: "message",
  T: AliOSSUpload,
  oneof: "output"
}]);
const ImageOutput = /* @__PURE__ */proto3.makeMessageType("livekit.ImageOutput", () => [{
  no: 1,
  name: "capture_interval",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "width",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 3,
  name: "height",
  kind: "scalar",
  T: 5
  /* ScalarType.INT32 */
}, {
  no: 4,
  name: "filename_prefix",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "filename_suffix",
  kind: "enum",
  T: proto3.getEnumType(ImageFileSuffix)
}, {
  no: 6,
  name: "image_codec",
  kind: "enum",
  T: proto3.getEnumType(ImageCodec)
}, {
  no: 7,
  name: "disable_manifest",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 8,
  name: "s3",
  kind: "message",
  T: S3Upload,
  oneof: "output"
}, {
  no: 9,
  name: "gcp",
  kind: "message",
  T: GCPUpload,
  oneof: "output"
}, {
  no: 10,
  name: "azure",
  kind: "message",
  T: AzureBlobUpload,
  oneof: "output"
}, {
  no: 11,
  name: "aliOSS",
  kind: "message",
  T: AliOSSUpload,
  oneof: "output"
}]);
const S3Upload = /* @__PURE__ */proto3.makeMessageType("livekit.S3Upload", () => [{
  no: 1,
  name: "access_key",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "secret",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 11,
  name: "session_token",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 12,
  name: "assume_role_arn",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 13,
  name: "assume_role_external_id",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "endpoint",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "bucket",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "force_path_style",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 7,
  name: "metadata",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}, {
  no: 8,
  name: "tagging",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 9,
  name: "content_disposition",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 10,
  name: "proxy",
  kind: "message",
  T: ProxyConfig
}]);
const GCPUpload = /* @__PURE__ */proto3.makeMessageType("livekit.GCPUpload", () => [{
  no: 1,
  name: "credentials",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "bucket",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "proxy",
  kind: "message",
  T: ProxyConfig
}]);
const AzureBlobUpload = /* @__PURE__ */proto3.makeMessageType("livekit.AzureBlobUpload", () => [{
  no: 1,
  name: "account_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "account_key",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "container_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const AliOSSUpload = /* @__PURE__ */proto3.makeMessageType("livekit.AliOSSUpload", () => [{
  no: 1,
  name: "access_key",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "secret",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "region",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 4,
  name: "endpoint",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "bucket",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const ProxyConfig = /* @__PURE__ */proto3.makeMessageType("livekit.ProxyConfig", () => [{
  no: 1,
  name: "url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "username",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "password",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}]);
const AutoParticipantEgress = /* @__PURE__ */proto3.makeMessageType("livekit.AutoParticipantEgress", () => [{
  no: 1,
  name: "preset",
  kind: "enum",
  T: proto3.getEnumType(EncodingOptionsPreset),
  oneof: "options"
}, {
  no: 2,
  name: "advanced",
  kind: "message",
  T: EncodingOptions,
  oneof: "options"
}, {
  no: 3,
  name: "file_outputs",
  kind: "message",
  T: EncodedFileOutput,
  repeated: true
}, {
  no: 4,
  name: "segment_outputs",
  kind: "message",
  T: SegmentedFileOutput,
  repeated: true
}]);
const AutoTrackEgress = /* @__PURE__ */proto3.makeMessageType("livekit.AutoTrackEgress", () => [{
  no: 1,
  name: "filepath",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "disable_manifest",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 2,
  name: "s3",
  kind: "message",
  T: S3Upload,
  oneof: "output"
}, {
  no: 3,
  name: "gcp",
  kind: "message",
  T: GCPUpload,
  oneof: "output"
}, {
  no: 4,
  name: "azure",
  kind: "message",
  T: AzureBlobUpload,
  oneof: "output"
}, {
  no: 6,
  name: "aliOSS",
  kind: "message",
  T: AliOSSUpload,
  oneof: "output"
}]);
const RoomCompositeEgressRequest = /* @__PURE__ */proto3.makeMessageType("livekit.RoomCompositeEgressRequest", () => [{
  no: 1,
  name: "room_name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "layout",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 3,
  name: "audio_only",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 15,
  name: "audio_mixing",
  kind: "enum",
  T: proto3.getEnumType(AudioMixing)
}, {
  no: 4,
  name: "video_only",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 5,
  name: "custom_base_url",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "file",
  kind: "message",
  T: EncodedFileOutput,
  oneof: "output"
}, {
  no: 7,
  name: "stream",
  kind: "message",
  T: StreamOutput,
  oneof: "output"
}, {
  no: 10,
  name: "segments",
  kind: "message",
  T: SegmentedFileOutput,
  oneof: "output"
}, {
  no: 8,
  name: "preset",
  kind: "enum",
  T: proto3.getEnumType(EncodingOptionsPreset),
  oneof: "options"
}, {
  no: 9,
  name: "advanced",
  kind: "message",
  T: EncodingOptions,
  oneof: "options"
}, {
  no: 11,
  name: "file_outputs",
  kind: "message",
  T: EncodedFileOutput,
  repeated: true
}, {
  no: 12,
  name: "stream_outputs",
  kind: "message",
  T: StreamOutput,
  repeated: true
}, {
  no: 13,
  name: "segment_outputs",
  kind: "message",
  T: SegmentedFileOutput,
  repeated: true
}, {
  no: 14,
  name: "image_outputs",
  kind: "message",
  T: ImageOutput,
  repeated: true
}, {
  no: 16,
  name: "webhooks",
  kind: "message",
  T: WebhookConfig,
  repeated: true
}]);
const EncodedFileOutput = /* @__PURE__ */proto3.makeMessageType("livekit.EncodedFileOutput", () => [{
  no: 1,
  name: "file_type",
  kind: "enum",
  T: proto3.getEnumType(EncodedFileType)
}, {
  no: 2,
  name: "filepath",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 6,
  name: "disable_manifest",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 3,
  name: "s3",
  kind: "message",
  T: S3Upload,
  oneof: "output"
}, {
  no: 4,
  name: "gcp",
  kind: "message",
  T: GCPUpload,
  oneof: "output"
}, {
  no: 5,
  name: "azure",
  kind: "message",
  T: AzureBlobUpload,
  oneof: "output"
}, {
  no: 7,
  name: "aliOSS",
  kind: "message",
  T: AliOSSUpload,
  oneof: "output"
}]);
const RoomEgress = /* @__PURE__ */proto3.makeMessageType("livekit.RoomEgress", () => [{
  no: 1,
  name: "room",
  kind: "message",
  T: RoomCompositeEgressRequest
}, {
  no: 3,
  name: "participant",
  kind: "message",
  T: AutoParticipantEgress
}, {
  no: 2,
  name: "tracks",
  kind: "message",
  T: AutoTrackEgress
}]);
const RoomConfiguration = /* @__PURE__ */proto3.makeMessageType("livekit.RoomConfiguration", () => [{
  no: 1,
  name: "name",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 2,
  name: "empty_timeout",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 3,
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
  no: 11,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 5,
  name: "egress",
  kind: "message",
  T: RoomEgress
}, {
  no: 7,
  name: "min_playout_delay",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 8,
  name: "max_playout_delay",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 9,
  name: "sync_streams",
  kind: "scalar",
  T: 8
  /* ScalarType.BOOL */
}, {
  no: 10,
  name: "agents",
  kind: "message",
  T: RoomAgentDispatch,
  repeated: true
}, {
  no: 12,
  name: "tags",
  kind: "map",
  K: 9,
  V: {
    kind: "scalar",
    T: 9
    /* ScalarType.STRING */
  }
}]);
export { AliOSSUpload, AudioMixing, AutoParticipantEgress, AutoTrackEgress, AzureBlobUpload, DataStream_ByteHeader, DataStream_Chunk, DataStream_CompressionType, DataStream_Header, DataStream_TextHeader, DataStream_Trailer, EncodedFileOutput, EncodedFileType, EncodingOptions, EncodingOptionsPreset, FilterParams, GCPUpload, ImageFileSuffix, ImageOutput, JobRestartPolicy, ProxyConfig, RoomAgentDispatch, RoomCompositeEgressRequest, RoomConfiguration, RoomEgress, S3Upload, SegmentedFileOutput, SegmentedFileProtocol, SegmentedFileSuffix, StreamOutput, StreamProtocol, SubscribedAudioCodec, WebhookConfig };
