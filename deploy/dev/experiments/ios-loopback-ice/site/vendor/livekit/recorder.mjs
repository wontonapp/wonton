// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 22901-22968 行按职责切出，行为不变。
// Check if MediaRecorder is available
import { __awaiter } from './tslib.mjs';

const isMediaRecorderAvailable = typeof MediaRecorder !== 'undefined';
// Fallback class for environments without MediaRecorder
class FallbackRecorder {
  constructor() {
    throw new Error('MediaRecorder is not available in this environment');
  }
}
// Use conditional inheritance to avoid parse-time errors
const RecorderBase = isMediaRecorderAvailable ? MediaRecorder : FallbackRecorder;
class LocalTrackRecorder extends RecorderBase {
  constructor(track, options) {
    if (!isMediaRecorderAvailable) {
      throw new Error('MediaRecorder is not available in this environment');
    }
    super(new MediaStream([track.mediaStreamTrack]), options);
    let dataListener;
    let streamController;
    const isClosed = () => streamController === undefined;
    const onStop = () => {
      this.removeEventListener('dataavailable', dataListener);
      this.removeEventListener('stop', onStop);
      this.removeEventListener('error', onError);
      streamController === null || streamController === void 0 ? void 0 : streamController.close();
      streamController = undefined;
    };
    const onError = event => {
      streamController === null || streamController === void 0 ? void 0 : streamController.error(event);
      this.removeEventListener('dataavailable', dataListener);
      this.removeEventListener('stop', onStop);
      this.removeEventListener('error', onError);
      streamController = undefined;
    };
    this.byteStream = new ReadableStream({
      start: controller => {
        streamController = controller;
        dataListener = event => __awaiter(this, void 0, void 0, function* () {
          let data;
          if (event.data.arrayBuffer) {
            const arrayBuffer = yield event.data.arrayBuffer();
            data = new Uint8Array(arrayBuffer);
            // @ts-expect-error react-native passes over Uint8Arrays directly
          } else if (event.data.byteArray) {
            // @ts-expect-error
            data = event.data.byteArray;
          } else {
            throw new Error('no data available!');
          }
          if (isClosed()) {
            return;
          }
          controller.enqueue(data);
        });
        this.addEventListener('dataavailable', dataListener);
      },
      cancel: () => {
        onStop();
      }
    });
    this.addEventListener('stop', onStop);
    this.addEventListener('error', onError);
  }
}
// Helper function to check if recording is supported
function isRecordingSupported() {
  return isMediaRecorderAvailable;
}const DEFAULT_DIMENSIONS_TIMEOUT = 1000;
const PRE_CONNECT_BUFFER_TIMEOUT = 10000;
export { DEFAULT_DIMENSIONS_TIMEOUT, FallbackRecorder, LocalTrackRecorder, PRE_CONNECT_BUFFER_TIMEOUT, RecorderBase, isMediaRecorderAvailable, isRecordingSupported };
