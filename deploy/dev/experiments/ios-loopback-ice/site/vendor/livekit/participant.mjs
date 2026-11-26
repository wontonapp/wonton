// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 31929-32196 行按职责切出，行为不变。
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { ParticipantInfo_Kind, ParticipantInfo_State } from './protobuf-messages-room.mjs';
import { TrackPermission } from './protobuf-messages-signal2.mjs';
import { LoggerNames, livekitLogger } from './loglevel.mjs';
import { getLogger } from './log.mjs';
import { eventsExports } from './events.mjs';
import { ParticipantEvent, TrackEvent } from './event-enums.mjs';
import { diffAttributes } from './util.mjs';
import { Track } from './track.mjs';
import { Future, isAudioTrack } from './utils-misc.mjs';
import { ConnectionQuality, qualityFromProto } from './connection-quality.mjs';


class Participant extends eventsExports.EventEmitter {
  get logContext() {
    var _a, _b;
    return Object.assign({}, (_b = (_a = this.loggerOptions) === null || _a === void 0 ? void 0 : _a.loggerContextCb) === null || _b === void 0 ? void 0 : _b.call(_a));
  }
  get isEncrypted() {
    return this.trackPublications.size > 0 && Array.from(this.trackPublications.values()).every(tr => tr.isEncrypted);
  }
  get isAgent() {
    var _a;
    return ((_a = this.permissions) === null || _a === void 0 ? void 0 : _a.agent) || this.kind === ParticipantInfo_Kind.AGENT;
  }
  get isActive() {
    var _a;
    return ((_a = this.participantInfo) === null || _a === void 0 ? void 0 : _a.state) === ParticipantInfo_State.ACTIVE;
  }
  get kind() {
    return this._kind;
  }
  /** participant attributes, similar to metadata, but as a key/value map */
  get attributes() {
    return Object.freeze(Object.assign({}, this._attributes));
  }
  /** @internal */
  constructor(sid, identity, name, metadata, attributes, loggerOptions) {
    let kind = arguments.length > 6 && arguments[6] !== undefined ? arguments[6] : ParticipantInfo_Kind.STANDARD;
    var _a;
    super();
    /** audio level between 0-1.0, 1 being loudest, 0 being softest */
    this.audioLevel = 0;
    /** if participant is currently speaking */
    this.isSpeaking = false;
    this._connectionQuality = ConnectionQuality.Unknown;
    this.log = livekitLogger;
    this.loggerOptions = loggerOptions;
    this.log = getLogger((_a = loggerOptions === null || loggerOptions === void 0 ? void 0 : loggerOptions.loggerName) !== null && _a !== void 0 ? _a : LoggerNames.Participant, () => this.logContext);
    this.setMaxListeners(100);
    this.sid = sid;
    this.identity = identity;
    this.name = name;
    this.metadata = metadata;
    this.audioTrackPublications = new Map();
    this.videoTrackPublications = new Map();
    this.trackPublications = new Map();
    this._kind = kind;
    this._attributes = attributes !== null && attributes !== void 0 ? attributes : {};
  }
  getTrackPublications() {
    return Array.from(this.trackPublications.values());
  }
  /**
   * Finds the first track that matches the source filter, for example, getting
   * the user's camera track with getTrackBySource(Track.Source.Camera).
   */
  getTrackPublication(source) {
    for (const _ref of this.trackPublications) {
      var _ref2 = _slicedToArray(_ref, 2);
      const pub = _ref2[1];
      if (pub.source === source) {
        return pub;
      }
    }
  }
  /**
   * Finds the first track that matches the track's name.
   */
  getTrackPublicationByName(name) {
    for (const _ref3 of this.trackPublications) {
      var _ref4 = _slicedToArray(_ref3, 2);
      const pub = _ref4[1];
      if (pub.trackName === name) {
        return pub;
      }
    }
  }
  /**
   * Waits until the participant is active and ready to receive data messages
   * @returns a promise that resolves when the participant is active
   */
  waitUntilActive() {
    if (this.isActive) {
      return Promise.resolve();
    }
    if (this.activeFuture) {
      return this.activeFuture.promise;
    }
    this.activeFuture = new Future();
    this.once(ParticipantEvent.Active, () => {
      var _a, _b;
      (_b = (_a = this.activeFuture) === null || _a === void 0 ? void 0 : _a.resolve) === null || _b === void 0 ? void 0 : _b.call(_a);
      this.activeFuture = undefined;
    });
    return this.activeFuture.promise;
  }
  get connectionQuality() {
    return this._connectionQuality;
  }
  get isCameraEnabled() {
    var _a;
    const track = this.getTrackPublication(Track.Source.Camera);
    return !((_a = track === null || track === void 0 ? void 0 : track.isMuted) !== null && _a !== void 0 ? _a : true);
  }
  get isMicrophoneEnabled() {
    var _a;
    const track = this.getTrackPublication(Track.Source.Microphone);
    return !((_a = track === null || track === void 0 ? void 0 : track.isMuted) !== null && _a !== void 0 ? _a : true);
  }
  get isScreenShareEnabled() {
    const track = this.getTrackPublication(Track.Source.ScreenShare);
    return !!track;
  }
  get isLocal() {
    return false;
  }
  /** when participant joined the room */
  get joinedAt() {
    if (this.participantInfo) {
      return new Date(Number.parseInt(this.participantInfo.joinedAt.toString()) * 1000);
    }
    return new Date();
  }
  /** @internal */
  updateInfo(info) {
    var _a;
    // it's possible the update could be applied out of order due to await
    // during reconnect sequences. when that happens, it's possible for server
    // to have sent more recent version of participant info while JS is waiting
    // to process the existing payload.
    // when the participant sid remains the same, and we already have a later version
    // of the payload, they can be safely skipped
    if (this.participantInfo && this.participantInfo.sid === info.sid && this.participantInfo.version > info.version) {
      return false;
    }
    this.identity = info.identity;
    this.sid = info.sid;
    this._setName(info.name);
    this._setMetadata(info.metadata);
    this._setAttributes(info.attributes);
    if (info.state === ParticipantInfo_State.ACTIVE && ((_a = this.participantInfo) === null || _a === void 0 ? void 0 : _a.state) !== ParticipantInfo_State.ACTIVE) {
      this.emit(ParticipantEvent.Active);
    }
    if (info.permission) {
      this.setPermissions(info.permission);
    }
    // set this last so setMetadata can detect changes
    this.participantInfo = info;
    return true;
  }
  /**
   * Updates metadata from server
   **/
  _setMetadata(md) {
    const changed = this.metadata !== md;
    const prevMetadata = this.metadata;
    this.metadata = md;
    if (changed) {
      this.emit(ParticipantEvent.ParticipantMetadataChanged, prevMetadata);
    }
  }
  _setName(name) {
    const changed = this.name !== name;
    this.name = name;
    if (changed) {
      this.emit(ParticipantEvent.ParticipantNameChanged, name);
    }
  }
  /**
   * Updates metadata from server
   **/
  _setAttributes(attributes) {
    const diff = diffAttributes(this.attributes, attributes);
    this._attributes = attributes;
    if (Object.keys(diff).length > 0) {
      this.emit(ParticipantEvent.AttributesChanged, diff);
    }
  }
  /** @internal */
  setPermissions(permissions) {
    var _a, _b, _c, _d, _e, _f;
    const prevPermissions = this.permissions;
    const changed = permissions.canPublish !== ((_a = this.permissions) === null || _a === void 0 ? void 0 : _a.canPublish) || permissions.canSubscribe !== ((_b = this.permissions) === null || _b === void 0 ? void 0 : _b.canSubscribe) || permissions.canPublishData !== ((_c = this.permissions) === null || _c === void 0 ? void 0 : _c.canPublishData) || permissions.hidden !== ((_d = this.permissions) === null || _d === void 0 ? void 0 : _d.hidden) || permissions.recorder !== ((_e = this.permissions) === null || _e === void 0 ? void 0 : _e.recorder) || permissions.canPublishSources.length !== this.permissions.canPublishSources.length || permissions.canPublishSources.some((value, index) => {
      var _a;
      return value !== ((_a = this.permissions) === null || _a === void 0 ? void 0 : _a.canPublishSources[index]);
    }) || permissions.canSubscribeMetrics !== ((_f = this.permissions) === null || _f === void 0 ? void 0 : _f.canSubscribeMetrics);
    this.permissions = permissions;
    if (changed) {
      this.emit(ParticipantEvent.ParticipantPermissionsChanged, prevPermissions);
    }
    return changed;
  }
  /** @internal */
  setIsSpeaking(speaking) {
    if (speaking === this.isSpeaking) {
      return;
    }
    this.isSpeaking = speaking;
    if (speaking) {
      this.lastSpokeAt = new Date();
    }
    this.emit(ParticipantEvent.IsSpeakingChanged, speaking);
  }
  /** @internal */
  setConnectionQuality(q) {
    const prevQuality = this._connectionQuality;
    this._connectionQuality = qualityFromProto(q);
    if (prevQuality !== this._connectionQuality) {
      this.emit(ParticipantEvent.ConnectionQualityChanged, this._connectionQuality);
    }
  }
  /**
   * @internal
   */
  setDisconnected() {
    var _a, _b;
    if (this.activeFuture) {
      (_b = (_a = this.activeFuture).reject) === null || _b === void 0 ? void 0 : _b.call(_a, new Error('Participant disconnected'));
      this.activeFuture = undefined;
    }
  }
  /**
   * @internal
   */
  setAudioContext(ctx) {
    return __awaiter(this, void 0, void 0, function* () {
      this.audioContext = ctx;
      yield Promise.all(Array.from(this.audioTrackPublications.values()).map(publication => publication.track).filter(isAudioTrack).map(track => track.setAudioContext(ctx)));
    });
  }
  addTrackPublication(publication) {
    this.log.debug("adding track publication", {
      trackSid: publication.trackSid,
      source: publication.source,
      kind: publication.kind
    });
    // forward publication driven events
    publication.on(TrackEvent.Muted, () => {
      this.emit(ParticipantEvent.TrackMuted, publication);
    });
    publication.on(TrackEvent.Unmuted, () => {
      this.emit(ParticipantEvent.TrackUnmuted, publication);
    });
    const pub = publication;
    if (pub.track) {
      pub.track.sid = publication.trackSid;
    }
    this.trackPublications.set(publication.trackSid, publication);
    switch (publication.kind) {
      case Track.Kind.Audio:
        this.audioTrackPublications.set(publication.trackSid, publication);
        break;
      case Track.Kind.Video:
        this.videoTrackPublications.set(publication.trackSid, publication);
        break;
    }
  }
}function trackPermissionToProto(perms) {
  var _a, _b, _c;
  if (!perms.participantSid && !perms.participantIdentity) {
    throw new Error('Invalid track permission, must provide at least one of participantIdentity and participantSid');
  }
  return new TrackPermission({
    participantIdentity: (_a = perms.participantIdentity) !== null && _a !== void 0 ? _a : '',
    participantSid: (_b = perms.participantSid) !== null && _b !== void 0 ? _b : '',
    allTracks: (_c = perms.allowAll) !== null && _c !== void 0 ? _c : false,
    trackSids: perms.allowedTrackSids || []
  });
}
export { Participant, trackPermissionToProto };
