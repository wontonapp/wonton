// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 17026-17313 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { livekitLogger } from './loglevel.mjs';
import { extractProjectFromUrl, isSafari, sleep } from './track.mjs';

const CONNECTION_BACKOFF_MIN_MS = 500;
const CONNECTION_BACKOFF_MAX_MS = 15000;
/**
 * BackOffStrategy implements exponential backoff for connection failures.
 *
 * When severe connection failures occur (e.g., network issues, server unavailability),
 * this strategy introduces increasing delays between reconnection attempts to avoid
 * overwhelming the server and to give transient issues time to resolve.
 *
 * This strategy is only applied to LiveKit Cloud projects. It identifies
 * projects by extracting the project name from the connection URL and tracks failures
 * per project. Self-hosted deployments (URLs without a project identifier) are not
 * subject to backoff delays.
 *
 * The class is implemented as a singleton to maintain consistent backoff state across
 * the entire application lifecycle instead of room instance lifecycle.
 */
class BackOffStrategy {
  // eslint-disable-next-line @typescript-eslint/no-empty-function
  constructor() {
    this.failedConnectionAttempts = new Map();
    this.backOffPromises = new Map();
  }
  static getInstance() {
    if (!this._instance) {
      this._instance = new BackOffStrategy();
    }
    return this._instance;
  }
  addFailedConnectionAttempt(urlString) {
    var _a;
    const url = new URL(urlString);
    const projectName = extractProjectFromUrl(url);
    if (!projectName) {
      return;
    }
    let failureCount = (_a = this.failedConnectionAttempts.get(projectName)) !== null && _a !== void 0 ? _a : 0;
    this.failedConnectionAttempts.set(projectName, failureCount + 1);
    this.backOffPromises.set(projectName, sleep(Math.min(CONNECTION_BACKOFF_MIN_MS * Math.pow(2, failureCount), CONNECTION_BACKOFF_MAX_MS)));
  }
  getBackOffPromise(urlString) {
    const url = new URL(urlString);
    const projectName = url && extractProjectFromUrl(url);
    const backoffPromise = projectName && this.backOffPromises.get(projectName);
    return backoffPromise || Promise.resolve();
  }
  resetFailedConnectionAttempts(urlString) {
    const url = new URL(urlString);
    const projectName = url && extractProjectFromUrl(url);
    if (projectName) {
      this.failedConnectionAttempts.set(projectName, 0);
      this.backOffPromises.set(projectName, Promise.resolve());
    }
  }
  resetAll() {
    this.backOffPromises.clear();
    this.failedConnectionAttempts.clear();
  }
}
BackOffStrategy._instance = null;const defaultId = 'default';
class DeviceManager {
  constructor() {
    this._previousDevices = [];
  }
  static getInstance() {
    if (this.instance === undefined) {
      this.instance = new DeviceManager();
    }
    return this.instance;
  }
  get previousDevices() {
    return this._previousDevices;
  }
  getDevices(kind_1) {
    return __awaiter(this, arguments, void 0, function (kind) {
      var _this = this;
      let requestPermissions = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
      return function* () {
        var _a;
        if (((_a = DeviceManager.userMediaPromiseMap) === null || _a === void 0 ? void 0 : _a.size) > 0) {
          livekitLogger.debug('awaiting getUserMedia promise');
          try {
            if (kind) {
              yield DeviceManager.userMediaPromiseMap.get(kind);
            } else {
              yield Promise.all(DeviceManager.userMediaPromiseMap.values());
            }
          } catch (e) {
            livekitLogger.warn('error waiting for media permissons');
          }
        }
        let devices = yield navigator.mediaDevices.enumerateDevices();
        if (requestPermissions &&
        // for safari we need to skip this check, as otherwise it will re-acquire user media and fail on iOS https://bugs.webkit.org/show_bug.cgi?id=179363
        !(isSafari() && _this.hasDeviceInUse(kind))) {
          const isDummyDeviceOrEmpty = devices.filter(d => d.kind === kind).length === 0 || devices.some(device => {
            const noLabel = device.label === '';
            const isRelevant = kind ? device.kind === kind : true;
            return noLabel && isRelevant;
          });
          if (isDummyDeviceOrEmpty) {
            const permissionsToAcquire = {
              video: kind !== 'audioinput' && kind !== 'audiooutput',
              audio: kind !== 'videoinput' && {
                deviceId: {
                  ideal: 'default'
                }
              }
            };
            const stream = yield navigator.mediaDevices.getUserMedia(permissionsToAcquire);
            devices = yield navigator.mediaDevices.enumerateDevices();
            stream.getTracks().forEach(track => {
              track.stop();
            });
          }
        }
        _this._previousDevices = devices;
        if (kind) {
          devices = devices.filter(device => device.kind === kind);
        }
        return devices;
      }();
    });
  }
  normalizeDeviceId(kind, deviceId, groupId) {
    return __awaiter(this, void 0, void 0, function* () {
      if (deviceId !== defaultId) {
        return deviceId;
      }
      // resolve actual device id if it's 'default': Chrome returns it when no
      // device has been chosen
      const devices = yield this.getDevices(kind);
      const defaultDevice = devices.find(d => d.deviceId === defaultId);
      if (!defaultDevice) {
        livekitLogger.warn('could not reliably determine default device');
        return undefined;
      }
      const device = devices.find(d => d.deviceId !== defaultId && d.groupId === (groupId !== null && groupId !== void 0 ? groupId : defaultDevice.groupId));
      if (!device) {
        livekitLogger.warn('could not reliably determine default device');
        return undefined;
      }
      return device === null || device === void 0 ? void 0 : device.deviceId;
    });
  }
  hasDeviceInUse(kind) {
    return kind ? DeviceManager.userMediaPromiseMap.has(kind) : DeviceManager.userMediaPromiseMap.size > 0;
  }
}
DeviceManager.mediaDeviceKinds = ['audioinput', 'audiooutput', 'videoinput'];
DeviceManager.userMediaPromiseMap = new Map();const U16_MAX_SIZE = 0xffff;
const U32_MAX_SIZE = 0xffffffff;
/**
 * A number of fields withing the data tracks packet specification assume wrap around behavior when
 * an unsigned type is incremented beyond its max size (ie, the packet `sequence` field). This
 * wrapper type manually reimplements this wrap around behavior given javascript's lack of fixed
 * size integer types.
 */
class WrapAroundUnsignedInt {
  static u16(raw) {
    return new WrapAroundUnsignedInt(raw, U16_MAX_SIZE);
  }
  static u32(raw) {
    return new WrapAroundUnsignedInt(raw, U32_MAX_SIZE);
  }
  constructor(raw, maxSize) {
    this.value = raw;
    if (raw < 0) {
      throw new Error('WrapAroundUnsignedInt: cannot faithfully represent an integer smaller than 0');
    }
    if (maxSize > Number.MAX_SAFE_INTEGER) {
      throw new Error('WrapAroundUnsignedInt: cannot faithfully represent an integer bigger than MAX_SAFE_INTEGER.');
    }
    this.maxSize = maxSize;
    this.clamp();
  }
  /** Manually clamp the given containing value according to the wrap around max size bounds. Use
   * this after out of bounds modification to the contained value by external code. */
  clamp() {
    while (this.value > this.maxSize) {
      this.value -= this.maxSize + 1;
    }
    while (this.value < 0) {
      this.value += this.maxSize + 1;
    }
  }
  clone() {
    return new WrapAroundUnsignedInt(this.value, this.maxSize);
  }
  /** When called, maps the containing value to a new containing value. After mapping, the wrap
   * around external max size bounds are applied. Note that this is a mutative operation. */
  update(updateFn) {
    this.value = updateFn(this.value);
    this.clamp();
  }
  /** Increments the given `n` to the inner value. Note that this is a mutative operation. */
  increment() {
    let n = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 1;
    this.update(value => value + n);
  }
  /** Decrements the given `n` from the inner value. Note that this is a mutative operation. */
  decrement() {
    let n = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 1;
    this.update(value => value - n);
  }
  getThenIncrement() {
    const previousValue = this.value;
    this.increment();
    return new WrapAroundUnsignedInt(previousValue, this.maxSize);
  }
  /** Returns true if {@link this} is before the passed other {@link WrapAroundUnsignedInt}. */
  isBefore(other) {
    const a = this.value >>> 0;
    const b = other.value >>> 0;
    const diff = b - a >>> 0;
    return diff !== 0 && diff < this.maxSize + 1;
  }
}
class DataTrackTimestamp {
  static fromRtpTicks(rtpTicks) {
    return new DataTrackTimestamp(rtpTicks, 90000);
  }
  /** Generates a timestamp initialized to a non cryptographically secure random value, so that
   * different streams are more difficult to correlate in packet capture. */
  static rtpRandom() {
    const randomValue = Math.round(Math.random() * U32_MAX_SIZE);
    return DataTrackTimestamp.fromRtpTicks(randomValue);
  }
  constructor(raw, rateInHz) {
    this.timestamp = WrapAroundUnsignedInt.u32(raw);
    this.rateInHz = rateInHz;
  }
  asTicks() {
    return this.timestamp.value;
  }
  clone() {
    return new DataTrackTimestamp(this.timestamp.value, this.rateInHz);
  }
  wrappingAdd(n) {
    this.timestamp.increment(n);
  }
  /** Returns true if {@link this} is before the passed other {@link DataTrackTimestamp}. */
  isBefore(other) {
    return this.timestamp.isBefore(other.timestamp);
  }
}
class DataTrackClock {
  constructor(rateInHz, epoch, base) {
    this.epoch = epoch;
    this.base = base;
    this.previous = base.clone();
    this.rateInHz = rateInHz;
  }
  static startingNow(base, rateInHz) {
    return new DataTrackClock(rateInHz, new Date(), base);
  }
  static startingAtTime(epoch, base, rateInHz) {
    return new DataTrackClock(rateInHz, epoch, base);
  }
  static rtpStartingNow(base) {
    return DataTrackClock.startingNow(base, 90000);
  }
  static rtpStartingAtTime(epoch, base) {
    return DataTrackClock.startingAtTime(epoch, base, 90000);
  }
  now() {
    return this.at(new Date());
  }
  at(timestamp) {
    let elapsedMs = timestamp.getTime() - this.epoch.getTime();
    let durationTicks = DataTrackClock.durationInMsToTicks(elapsedMs, this.rateInHz);
    let result = this.base.clone();
    result.wrappingAdd(durationTicks);
    // Enforce monotonicity in RTP wraparound space
    if (result.isBefore(this.previous)) {
      result = this.previous;
    }
    this.previous = result.clone();
    return result.clone();
  }
  /** Convert a duration since the epoch into clock ticks. */
  static durationInMsToTicks(durationMilliseconds, rateInHz) {
    // round(nanos * rate_hz / 1e9)
    let durationNanoseconds = durationMilliseconds * 1e6;
    let ticks = (durationNanoseconds * rateInHz + 500000000) / 1000000000;
    return Math.round(ticks);
  }
}
export { BackOffStrategy, CONNECTION_BACKOFF_MAX_MS, CONNECTION_BACKOFF_MIN_MS, DataTrackClock, DataTrackTimestamp, DeviceManager, U16_MAX_SIZE, U32_MAX_SIZE, WrapAroundUnsignedInt, defaultId };
