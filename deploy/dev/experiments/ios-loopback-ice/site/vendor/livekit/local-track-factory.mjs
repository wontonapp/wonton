// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 23980-23989 行按职责切出，行为不变。
import { TrackInvalidError } from './errors.mjs';
import { LocalAudioTrack } from './local-track.mjs';
import { LocalVideoTrack } from './local-video-track.mjs';

/** @internal */
function mediaTrackToLocalTrack(mediaStreamTrack, constraints, loggerOptions) {
  switch (mediaStreamTrack.kind) {
    case 'audio':
      return new LocalAudioTrack(mediaStreamTrack, constraints, false, undefined, loggerOptions);
    case 'video':
      return new LocalVideoTrack(mediaStreamTrack, constraints, false, loggerOptions);
    default:
      throw new TrackInvalidError("unsupported track type: ".concat(mediaStreamTrack.kind));
  }
}
export { mediaTrackToLocalTrack };
