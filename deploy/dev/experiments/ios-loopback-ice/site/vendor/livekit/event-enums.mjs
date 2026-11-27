// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 12542-13195 行按职责切出，行为不变。
/**
 * Events are the primary way LiveKit notifies your application of changes.
 *
 * The following are events emitted by [[Room]], listen to room events like
 *
 * ```typescript
 * room.on(RoomEvent.TrackPublished, (track, publication, participant) => {})
 * ```
 */
import { _slicedToArray } from './tslib.mjs';

var RoomEvent;
(function (RoomEvent) {
  /**
   * When the connection to the server has been established
   */
  RoomEvent["Connected"] = "connected";
  /**
   * When the connection to the server has been interrupted and it's attempting
   * to reconnect.
   */
  RoomEvent["Reconnecting"] = "reconnecting";
  /**
   * When the signal connection to the server has been interrupted. This isn't noticeable to users most of the time.
   * It will resolve with a `RoomEvent.Reconnected` once the signal connection has been re-established.
   * If media fails additionally it an additional `RoomEvent.Reconnecting` will be emitted.
   */
  RoomEvent["SignalReconnecting"] = "signalReconnecting";
  /**
   * Fires when a reconnection has been successful.
   */
  RoomEvent["Reconnected"] = "reconnected";
  /**
   * When disconnected from room. This fires when room.disconnect() is called or
   * when an unrecoverable connection issue had occurred.
   *
   * DisconnectReason can be used to determine why the participant was disconnected. Notable reasons are
   * - DUPLICATE_IDENTITY: another client with the same identity has joined the room
   * - PARTICIPANT_REMOVED: participant was removed by RemoveParticipant API
   * - ROOM_DELETED: the room has ended via DeleteRoom API
   *
   * args: ([[DisconnectReason]])
   */
  RoomEvent["Disconnected"] = "disconnected";
  /**
   * Whenever the connection state of the room changes
   *
   * args: ([[ConnectionState]])
   */
  RoomEvent["ConnectionStateChanged"] = "connectionStateChanged";
  /**
   * When participant has been moved to a different room by the service request.
   * The behavior looks like the participant has been disconnected and reconnected to a different room
   * seamlessly without connection state transition.
   * A new token will be provided for reconnecting to the new room if needed.
   *
   * args: ([[room: string, token: string]])
   */
  RoomEvent["Moved"] = "moved";
  /**
   * When input or output devices on the machine have changed.
   */
  RoomEvent["MediaDevicesChanged"] = "mediaDevicesChanged";
  /**
   * When a [[RemoteParticipant]] joins *after* the local
   * participant. It will not emit events for participants that are already
   * in the room
   *
   * args: ([[RemoteParticipant]])
   */
  RoomEvent["ParticipantConnected"] = "participantConnected";
  /**
   * When a [[RemoteParticipant]] leaves *after* the local
   * participant has joined.
   *
   * args: ([[RemoteParticipant]], [[DisconnectReason]] | undefined)
   */
  RoomEvent["ParticipantDisconnected"] = "participantDisconnected";
  /**
   * When a new track is published to room *after* the local
   * participant has joined. It will not fire for tracks that are already published.
   *
   * A track published doesn't mean the participant has subscribed to it. It's
   * simply reflecting the state of the room.
   *
   * args: ([[RemoteTrackPublication]], [[RemoteParticipant]])
   */
  RoomEvent["TrackPublished"] = "trackPublished";
  /**
   * The [[LocalParticipant]] has subscribed to a new track. This event will **always**
   * fire as long as new tracks are ready for use.
   *
   * args: ([[RemoteTrack]], [[RemoteTrackPublication]], [[RemoteParticipant]])
   */
  RoomEvent["TrackSubscribed"] = "trackSubscribed";
  /**
   * Could not subscribe to a track
   *
   * args: (track sid, [[RemoteParticipant]])
   */
  RoomEvent["TrackSubscriptionFailed"] = "trackSubscriptionFailed";
  /**
   * A [[RemoteParticipant]] has unpublished a track
   *
   * args: ([[RemoteTrackPublication]], [[RemoteParticipant]])
   */
  RoomEvent["TrackUnpublished"] = "trackUnpublished";
  /**
   * A subscribed track is no longer available. Clients should listen to this
   * event and ensure they detach tracks.
   *
   * args: ([[Track]], [[RemoteTrackPublication]], [[RemoteParticipant]])
   */
  RoomEvent["TrackUnsubscribed"] = "trackUnsubscribed";
  /**
   * A track that was muted, fires on both [[RemoteParticipant]]s and [[LocalParticipant]]
   *
   * args: ([[TrackPublication]], [[Participant]])
   */
  RoomEvent["TrackMuted"] = "trackMuted";
  /**
   * A track that was unmuted, fires on both [[RemoteParticipant]]s and [[LocalParticipant]]
   *
   * args: ([[TrackPublication]], [[Participant]])
   */
  RoomEvent["TrackUnmuted"] = "trackUnmuted";
  /**
   * A local track was published successfully. This event is helpful to know
   * when to update your local UI with the newly published track.
   *
   * args: ([[LocalTrackPublication]], [[LocalParticipant]])
   */
  RoomEvent["LocalTrackPublished"] = "localTrackPublished";
  /**
   * A local track was unpublished. This event is helpful to know when to remove
   * the local track from your UI.
   *
   * When a user stops sharing their screen by pressing "End" on the browser UI,
   * this event will also fire.
   *
   * args: ([[LocalTrackPublication]], [[LocalParticipant]])
   */
  RoomEvent["LocalTrackUnpublished"] = "localTrackUnpublished";
  /**
   * When a local audio track is published the SDK checks whether there is complete silence
   * on that track and emits the LocalAudioSilenceDetected event in that case.
   * This allows for applications to show UI informing users that they might have to
   * reset their audio hardware or check for proper device connectivity.
   */
  RoomEvent["LocalAudioSilenceDetected"] = "localAudioSilenceDetected";
  /**
   * Active speakers changed. List of speakers are ordered by their audio level.
   * loudest speakers first. This will include the LocalParticipant too.
   *
   * Speaker updates are sent only to the publishing participant and their subscribers.
   *
   * args: (Array<[[Participant]]>)
   */
  RoomEvent["ActiveSpeakersChanged"] = "activeSpeakersChanged";
  /**
   * Participant metadata is a simple way for app-specific state to be pushed to
   * all users.
   * When RoomService.UpdateParticipantMetadata is called to change a participant's
   * state, *all*  participants in the room will fire this event.
   *
   * args: (prevMetadata: string, [[Participant]])
   *
   */
  RoomEvent["ParticipantMetadataChanged"] = "participantMetadataChanged";
  /**
   * Participant's display name changed
   *
   * args: (name: string, [[Participant]])
   *
   */
  RoomEvent["ParticipantNameChanged"] = "participantNameChanged";
  /**
   * Participant attributes is an app-specific key value state to be pushed to
   * all users.
   * When a participant's attributes changed, this event will be emitted with the changed attributes and the participant
   * args: (changedAttributes: [[Record<string, string]], participant: [[Participant]])
   */
  RoomEvent["ParticipantAttributesChanged"] = "participantAttributesChanged";
  /**
   * Emitted when the participant's state changes to ACTIVE and is ready to send/receive data messages
   *
   * args: (participant: [[Participant]])
   */
  RoomEvent["ParticipantActive"] = "participantActive";
  /**
   * Room metadata is a simple way for app-specific state to be pushed to
   * all users.
   * When RoomService.UpdateRoomMetadata is called to change a room's state,
   * *all*  participants in the room will fire this event.
   *
   * args: (string)
   */
  RoomEvent["RoomMetadataChanged"] = "roomMetadataChanged";
  /**
   * Data received from another participant.
   * Data packets provides the ability to use LiveKit to send/receive arbitrary payloads.
   * All participants in the room will receive the messages sent to the room.
   *
   * args: (payload: NonSharedUint8Array, participant: [[Participant]] | undefined, kind: [[DataPacket_Kind]], topic?: string, encryptionType?: Encryption_Type, participantIdentity?: string)
   */
  RoomEvent["DataReceived"] = "dataReceived";
  /**
   * SIP DTMF tones received from another participant.
   *
   * args: (participant: [[Participant]], dtmf: [[DataPacket_Kind]])
   */
  RoomEvent["SipDTMFReceived"] = "sipDTMFReceived";
  /**
   * Transcription received from a participant's track.
   * @beta
   */
  RoomEvent["TranscriptionReceived"] = "transcriptionReceived";
  /**
   * Connection quality was changed for a Participant. It'll receive updates
   * from the local participant, as well as any [[RemoteParticipant]]s that we are
   * subscribed to.
   *
   * args: (connectionQuality: [[ConnectionQuality]], participant: [[Participant]])
   */
  RoomEvent["ConnectionQualityChanged"] = "connectionQualityChanged";
  /**
   * StreamState indicates if a subscribed (remote) track has been paused by the SFU
   * (typically this happens because of subscriber's bandwidth constraints)
   *
   * When bandwidth conditions allow, the track will be resumed automatically.
   * TrackStreamStateChanged will also be emitted when that happens.
   *
   * args: (pub: [[RemoteTrackPublication]], streamState: [[Track.StreamState]],
   *        participant: [[RemoteParticipant]])
   */
  RoomEvent["TrackStreamStateChanged"] = "trackStreamStateChanged";
  /**
   * One of subscribed tracks have changed its permissions for the current
   * participant. If permission was revoked, then the track will no longer
   * be subscribed. If permission was granted, a TrackSubscribed event will
   * be emitted.
   *
   * args: (pub: [[RemoteTrackPublication]],
   *        status: [[TrackPublication.PermissionStatus]],
   *        participant: [[RemoteParticipant]])
   */
  RoomEvent["TrackSubscriptionPermissionChanged"] = "trackSubscriptionPermissionChanged";
  /**
   * One of subscribed tracks have changed its status for the current
   * participant.
   *
   * args: (pub: [[RemoteTrackPublication]],
   *        status: [[TrackPublication.SubscriptionStatus]],
   *        participant: [[RemoteParticipant]])
   */
  RoomEvent["TrackSubscriptionStatusChanged"] = "trackSubscriptionStatusChanged";
  /**
   * LiveKit will attempt to autoplay all audio tracks when you attach them to
   * audio elements. However, if that fails, we'll notify you via AudioPlaybackStatusChanged.
   * `Room.canPlaybackAudio` will indicate if audio playback is permitted.
   */
  RoomEvent["AudioPlaybackStatusChanged"] = "audioPlaybackChanged";
  /**
   * LiveKit will attempt to autoplay all video tracks when you attach them to
   * a video element. However, if that fails, we'll notify you via VideoPlaybackStatusChanged.
   * Calling `room.startVideo()` in a user gesture event handler will resume the video playback.
   */
  RoomEvent["VideoPlaybackStatusChanged"] = "videoPlaybackChanged";
  /**
   * When we have encountered an error while attempting to create a track.
   * The errors take place in getUserMedia().
   * Use MediaDeviceFailure.getFailure(error) to get the reason of failure.
   * [[LocalParticipant.lastCameraError]] and [[LocalParticipant.lastMicrophoneError]]
   * will indicate if it had an error while creating the audio or video track respectively.
   *
   * args: (error: Error)
   */
  RoomEvent["MediaDevicesError"] = "mediaDevicesError";
  /**
   * A participant's permission has changed.
   * args: (prevPermissions: [[ParticipantPermission]], participant: [[Participant]])
   */
  RoomEvent["ParticipantPermissionsChanged"] = "participantPermissionsChanged";
  /**
   * Signal connected, can publish tracks.
   */
  RoomEvent["SignalConnected"] = "signalConnected";
  /**
   * Recording of a room has started/stopped. Room.isRecording will be updated too.
   * args: (isRecording: boolean)
   */
  RoomEvent["RecordingStatusChanged"] = "recordingStatusChanged";
  RoomEvent["ParticipantEncryptionStatusChanged"] = "participantEncryptionStatusChanged";
  RoomEvent["EncryptionError"] = "encryptionError";
  /**
   * Emits whenever the current buffer status of a data channel changes
   * args: (isLow: boolean, kind: [[DataChannelKind]])
   */
  RoomEvent["DCBufferStatusChanged"] = "dcBufferStatusChanged";
  /**
   * Triggered by a call to room.switchActiveDevice
   * args: (kind: MediaDeviceKind, deviceId: string)
   */
  RoomEvent["ActiveDeviceChanged"] = "activeDeviceChanged";
  RoomEvent["ChatMessage"] = "chatMessage";
  /**
   * fired when the first remote participant has subscribed to the localParticipant's track
   */
  RoomEvent["LocalTrackSubscribed"] = "localTrackSubscribed";
  /**
   * fired when the client receives connection metrics from other participants
   */
  RoomEvent["MetricsReceived"] = "metricsReceived";
  /**
   * Emits when a new data track has been published by a downstream participant.
   */
  RoomEvent["DataTrackPublished"] = "dataTrackPublished";
  /**
   * Emits when a new data track has been unpublished by a downstream participant.
   */
  RoomEvent["DataTrackUnpublished"] = "dataTrackUnpublished";
  /**
   * Emits when a new data track has been published locally.
   */
  RoomEvent["LocalDataTrackPublished"] = "localDataTrackPublished";
  /**
   * Emits when a new data track has been unpublished locally.
   */
  RoomEvent["LocalDataTrackUnpublished"] = "localDataTrackUnpublished";
})(RoomEvent || (RoomEvent = {}));
var ParticipantEvent;
(function (ParticipantEvent) {
  /**
   * When a new track is published to room *after* the local
   * participant has joined. It will not fire for tracks that are already published.
   *
   * A track published doesn't mean the participant has subscribed to it. It's
   * simply reflecting the state of the room.
   *
   * args: ([[RemoteTrackPublication]])
   */
  ParticipantEvent["TrackPublished"] = "trackPublished";
  /**
   * Successfully subscribed to the [[RemoteParticipant]]'s track.
   * This event will **always** fire as long as new tracks are ready for use.
   *
   * args: ([[RemoteTrack]], [[RemoteTrackPublication]])
   */
  ParticipantEvent["TrackSubscribed"] = "trackSubscribed";
  /**
   * Could not subscribe to a track
   *
   * args: (track sid)
   */
  ParticipantEvent["TrackSubscriptionFailed"] = "trackSubscriptionFailed";
  /**
   * A [[RemoteParticipant]] has unpublished a track
   *
   * args: ([[RemoteTrackPublication]])
   */
  ParticipantEvent["TrackUnpublished"] = "trackUnpublished";
  /**
   * A subscribed track is no longer available. Clients should listen to this
   * event and ensure they detach tracks.
   *
   * args: ([[RemoteTrack]], [[RemoteTrackPublication]])
   */
  ParticipantEvent["TrackUnsubscribed"] = "trackUnsubscribed";
  /**
   * A track that was muted, fires on both [[RemoteParticipant]]s and [[LocalParticipant]]
   *
   * args: ([[TrackPublication]])
   */
  ParticipantEvent["TrackMuted"] = "trackMuted";
  /**
   * A track that was unmuted, fires on both [[RemoteParticipant]]s and [[LocalParticipant]]
   *
   * args: ([[TrackPublication]])
   */
  ParticipantEvent["TrackUnmuted"] = "trackUnmuted";
  /**
   * A local track was published successfully. This event is helpful to know
   * when to update your local UI with the newly published track.
   *
   * args: ([[LocalTrackPublication]])
   */
  ParticipantEvent["LocalTrackPublished"] = "localTrackPublished";
  /**
   * A local track was unpublished. This event is helpful to know when to remove
   * the local track from your UI.
   *
   * When a user stops sharing their screen by pressing "End" on the browser UI,
   * this event will also fire.
   *
   * args: ([[LocalTrackPublication]])
   */
  ParticipantEvent["LocalTrackUnpublished"] = "localTrackUnpublished";
  /**
   * A local track has been constrained by cpu.
   * This event is useful to know when to reduce the capture resolution of the track.
   *
   * This event is emitted on the local participant.
   *
   * args: ([[LocalVideoTrack]], [[LocalTrackPublication]])
   */
  ParticipantEvent["LocalTrackCpuConstrained"] = "localTrackCpuConstrained";
  /**
   * @internal
   */
  ParticipantEvent["LocalSenderCreated"] = "localSenderCreated";
  /**
   * Participant metadata is a simple way for app-specific state to be pushed to
   * all users.
   * When RoomService.UpdateParticipantMetadata is called to change a participant's
   * state, *all*  participants in the room will fire this event.
   * To access the current metadata, see [[Participant.metadata]].
   *
   * args: (prevMetadata: string)
   *
   */
  ParticipantEvent["ParticipantMetadataChanged"] = "participantMetadataChanged";
  /**
   * Participant's display name changed
   *
   * args: (name: string, [[Participant]])
   *
   */
  ParticipantEvent["ParticipantNameChanged"] = "participantNameChanged";
  /**
   * Data received from this participant as sender.
   * Data packets provides the ability to use LiveKit to send/receive arbitrary payloads.
   * All participants in the room will receive the messages sent to the room.
   *
   * args: (payload: NonSharedUint8Array, kind: [[DataPacket_Kind]])
   */
  ParticipantEvent["DataReceived"] = "dataReceived";
  /**
   * SIP DTMF tones received from this participant as sender.
   *
   * args: (dtmf: [[DataPacket_Kind]])
   */
  ParticipantEvent["SipDTMFReceived"] = "sipDTMFReceived";
  /**
   * Transcription received from this participant as data source.
   * @beta
   */
  ParticipantEvent["TranscriptionReceived"] = "transcriptionReceived";
  /**
   * Has speaking status changed for the current participant
   *
   * args: (speaking: boolean)
   */
  ParticipantEvent["IsSpeakingChanged"] = "isSpeakingChanged";
  /**
   * Connection quality was changed for a Participant. It'll receive updates
   * from the local participant, as well as any [[RemoteParticipant]]s that we are
   * subscribed to.
   *
   * args: (connectionQuality: [[ConnectionQuality]])
   */
  ParticipantEvent["ConnectionQualityChanged"] = "connectionQualityChanged";
  /**
   * StreamState indicates if a subscribed track has been paused by the SFU
   * (typically this happens because of subscriber's bandwidth constraints)
   *
   * When bandwidth conditions allow, the track will be resumed automatically.
   * TrackStreamStateChanged will also be emitted when that happens.
   *
   * args: (pub: [[RemoteTrackPublication]], streamState: [[Track.StreamState]])
   */
  ParticipantEvent["TrackStreamStateChanged"] = "trackStreamStateChanged";
  /**
   * One of subscribed tracks have changed its permissions for the current
   * participant. If permission was revoked, then the track will no longer
   * be subscribed. If permission was granted, a TrackSubscribed event will
   * be emitted.
   *
   * args: (pub: [[RemoteTrackPublication]],
   *        status: [[TrackPublication.SubscriptionStatus]])
   */
  ParticipantEvent["TrackSubscriptionPermissionChanged"] = "trackSubscriptionPermissionChanged";
  /**
   * One of the remote participants publications has changed its subscription status.
   *
   */
  ParticipantEvent["TrackSubscriptionStatusChanged"] = "trackSubscriptionStatusChanged";
  /**
   * a local track has been constrained by cpu
   */
  ParticipantEvent["TrackCpuConstrained"] = "trackCpuConstrained";
  // fired only on LocalParticipant
  /** @internal */
  ParticipantEvent["MediaDevicesError"] = "mediaDevicesError";
  // fired only on LocalParticipant
  /** @internal */
  ParticipantEvent["AudioStreamAcquired"] = "audioStreamAcquired";
  /**
   * A participant's permission has changed.
   * args: (prevPermissions: [[ParticipantPermission]])
   */
  ParticipantEvent["ParticipantPermissionsChanged"] = "participantPermissionsChanged";
  /** @internal */
  ParticipantEvent["PCTrackAdded"] = "pcTrackAdded";
  /**
   * Participant attributes is an app-specific key value state to be pushed to
   * all users.
   * When a participant's attributes changed, this event will be emitted with the changed attributes
   * args: (changedAttributes: [[Record<string, string]])
   */
  ParticipantEvent["AttributesChanged"] = "attributesChanged";
  /**
   * fired on local participant only, when the first remote participant has subscribed to the track specified in the payload
   */
  ParticipantEvent["LocalTrackSubscribed"] = "localTrackSubscribed";
  /** only emitted on local participant */
  ParticipantEvent["ChatMessage"] = "chatMessage";
  /**
   * Emitted when the participant's state changes to ACTIVE and is ready to send/receive data messages
   */
  ParticipantEvent["Active"] = "active";
})(ParticipantEvent || (ParticipantEvent = {}));
/** @internal */
var EngineEvent;
(function (EngineEvent) {
  EngineEvent["TransportsCreated"] = "transportsCreated";
  EngineEvent["Connected"] = "connected";
  EngineEvent["Disconnected"] = "disconnected";
  EngineEvent["Resuming"] = "resuming";
  EngineEvent["Resumed"] = "resumed";
  EngineEvent["Restarting"] = "restarting";
  EngineEvent["Restarted"] = "restarted";
  EngineEvent["SignalResumed"] = "signalResumed";
  EngineEvent["SignalRestarted"] = "signalRestarted";
  EngineEvent["Closing"] = "closing";
  EngineEvent["MediaTrackAdded"] = "mediaTrackAdded";
  EngineEvent["ActiveSpeakersUpdate"] = "activeSpeakersUpdate";
  EngineEvent["DataPacketReceived"] = "dataPacketReceived";
  EngineEvent["RTPVideoMapUpdate"] = "rtpVideoMapUpdate";
  EngineEvent["DCBufferStatusChanged"] = "dcBufferStatusChanged";
  EngineEvent["ParticipantUpdate"] = "participantUpdate";
  EngineEvent["RoomUpdate"] = "roomUpdate";
  EngineEvent["SpeakersChanged"] = "speakersChanged";
  EngineEvent["StreamStateChanged"] = "streamStateChanged";
  EngineEvent["ConnectionQualityUpdate"] = "connectionQualityUpdate";
  EngineEvent["SubscriptionError"] = "subscriptionError";
  EngineEvent["SubscriptionPermissionUpdate"] = "subscriptionPermissionUpdate";
  EngineEvent["RemoteMute"] = "remoteMute";
  EngineEvent["SubscribedQualityUpdate"] = "subscribedQualityUpdate";
  EngineEvent["LocalTrackUnpublished"] = "localTrackUnpublished";
  EngineEvent["LocalTrackSubscribed"] = "localTrackSubscribed";
  EngineEvent["Offline"] = "offline";
  EngineEvent["SignalRequestResponse"] = "signalRequestResponse";
  EngineEvent["SignalConnected"] = "signalConnected";
  EngineEvent["RoomMoved"] = "roomMoved";
  EngineEvent["PublishDataTrackResponse"] = "publishDataTrackResponse";
  EngineEvent["UnPublishDataTrackResponse"] = "unPublishDataTrackResponse";
  EngineEvent["DataTrackSubscriberHandles"] = "dataTrackSubscriberHandles";
  EngineEvent["DataTrackPacketReceived"] = "dataTrackPacketReceived";
  EngineEvent["Joined"] = "joined";
  EngineEvent["TokenRefreshed"] = "tokenRefreshed";
  EngineEvent["ServerRegionsReported"] = "serverRegionsReported";
  EngineEvent["RequestSubscribedCodecRefresh"] = "requestSubscribedCodecRefresh";
})(EngineEvent || (EngineEvent = {}));
var TrackEvent;
(function (TrackEvent) {
  TrackEvent["Message"] = "message";
  TrackEvent["Muted"] = "muted";
  TrackEvent["Unmuted"] = "unmuted";
  /**
   * Only fires on LocalTracks
   */
  TrackEvent["Restarted"] = "restarted";
  TrackEvent["Ended"] = "ended";
  TrackEvent["Subscribed"] = "subscribed";
  TrackEvent["Unsubscribed"] = "unsubscribed";
  TrackEvent["CpuConstrained"] = "cpuConstrained";
  /** @internal */
  TrackEvent["UpdateSettings"] = "updateSettings";
  /** @internal */
  TrackEvent["UpdateSubscription"] = "updateSubscription";
  /** @internal */
  TrackEvent["AudioPlaybackStarted"] = "audioPlaybackStarted";
  /** @internal */
  TrackEvent["AudioPlaybackFailed"] = "audioPlaybackFailed";
  /**
   * @internal
   * Only fires on LocalAudioTrack instances
   */
  TrackEvent["AudioSilenceDetected"] = "audioSilenceDetected";
  /** @internal */
  TrackEvent["VisibilityChanged"] = "visibilityChanged";
  /** @internal */
  TrackEvent["VideoDimensionsChanged"] = "videoDimensionsChanged";
  /** @internal */
  TrackEvent["VideoPlaybackStarted"] = "videoPlaybackStarted";
  /** @internal */
  TrackEvent["VideoPlaybackFailed"] = "videoPlaybackFailed";
  /** @internal */
  TrackEvent["ElementAttached"] = "elementAttached";
  /** @internal */
  TrackEvent["ElementDetached"] = "elementDetached";
  /**
   * @internal
   * Only fires on LocalTracks
   */
  TrackEvent["UpstreamPaused"] = "upstreamPaused";
  /**
   * @internal
   * Only fires on LocalTracks
   */
  TrackEvent["UpstreamResumed"] = "upstreamResumed";
  /**
   * @internal
   * Fires on RemoteTrackPublication
   */
  TrackEvent["SubscriptionPermissionChanged"] = "subscriptionPermissionChanged";
  /**
   * Fires on RemoteTrackPublication
   */
  TrackEvent["SubscriptionStatusChanged"] = "subscriptionStatusChanged";
  /**
   * Fires on RemoteTrackPublication
   */
  TrackEvent["SubscriptionFailed"] = "subscriptionFailed";
  /**
   * @internal
   */
  TrackEvent["TrackProcessorUpdate"] = "trackProcessorUpdate";
  /**
   * @internal
   */
  TrackEvent["AudioTrackFeatureUpdate"] = "audioTrackFeatureUpdate";
  /**
   * @beta
   */
  TrackEvent["TranscriptionReceived"] = "transcriptionReceived";
  /**
   * @experimental
   */
  TrackEvent["TimeSyncUpdate"] = "timeSyncUpdate";
  /**
   * @internal
   */
  TrackEvent["PreConnectBufferFlushed"] = "preConnectBufferFlushed";
})(TrackEvent || (TrackEvent = {}));function compact(summary) {
  const compacted = {};
  for (const _ref of Object.entries(summary)) {
    var _ref2 = _slicedToArray(_ref, 2);
    const key = _ref2[0];
    const value = _ref2[1];
    if (value !== undefined) {
      compacted[key] = value;
    }
  }
  return compacted;
}
export { EngineEvent, ParticipantEvent, RoomEvent, TrackEvent, compact };
