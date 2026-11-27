// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 3743-3888 行按职责切出，行为不变。
import { proto3 } from './protobuf-codegen.mjs';
import { Timestamp } from './protobuf-messages-google.mjs';

const MetricsBatch = /* @__PURE__ */proto3.makeMessageType("livekit.MetricsBatch", () => [{
  no: 1,
  name: "timestamp_ms",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 2,
  name: "normalized_timestamp",
  kind: "message",
  T: Timestamp
}, {
  no: 3,
  name: "str_data",
  kind: "scalar",
  T: 9,
  repeated: true
}, {
  no: 4,
  name: "time_series",
  kind: "message",
  T: TimeSeriesMetric,
  repeated: true
}, {
  no: 5,
  name: "events",
  kind: "message",
  T: EventMetric,
  repeated: true
}]);
const TimeSeriesMetric = /* @__PURE__ */proto3.makeMessageType("livekit.TimeSeriesMetric", () => [{
  no: 1,
  name: "label",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "participant_identity",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 3,
  name: "track_sid",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "samples",
  kind: "message",
  T: MetricSample,
  repeated: true
}, {
  no: 5,
  name: "rid",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const MetricSample = /* @__PURE__ */proto3.makeMessageType("livekit.MetricSample", () => [{
  no: 1,
  name: "timestamp_ms",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 2,
  name: "normalized_timestamp",
  kind: "message",
  T: Timestamp
}, {
  no: 3,
  name: "value",
  kind: "scalar",
  T: 2
  /* ScalarType.FLOAT */
}]);
const EventMetric = /* @__PURE__ */proto3.makeMessageType("livekit.EventMetric", () => [{
  no: 1,
  name: "label",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 2,
  name: "participant_identity",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 3,
  name: "track_sid",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}, {
  no: 4,
  name: "start_timestamp_ms",
  kind: "scalar",
  T: 3
  /* ScalarType.INT64 */
}, {
  no: 5,
  name: "end_timestamp_ms",
  kind: "scalar",
  T: 3,
  opt: true
}, {
  no: 6,
  name: "normalized_start_timestamp",
  kind: "message",
  T: Timestamp
}, {
  no: 7,
  name: "normalized_end_timestamp",
  kind: "message",
  T: Timestamp,
  opt: true
}, {
  no: 8,
  name: "metadata",
  kind: "scalar",
  T: 9
  /* ScalarType.STRING */
}, {
  no: 9,
  name: "rid",
  kind: "scalar",
  T: 13
  /* ScalarType.UINT32 */
}]);
const AudioCodec = /* @__PURE__ */proto3.makeEnum("livekit.AudioCodec", [{
  no: 0,
  name: "DEFAULT_AC"
}, {
  no: 1,
  name: "OPUS"
}, {
  no: 2,
  name: "AAC"
}, {
  no: 3,
  name: "AC_MP3"
}]);
export { AudioCodec, EventMetric, MetricSample, MetricsBatch, TimeSeriesMetric };
