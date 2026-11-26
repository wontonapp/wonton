// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 34011-34101 行按职责切出，行为不变。
/** An error which is thrown if a {@link DeferrableMap#getDeferred} call is aborted midway
 * through. */
import { __awaiter } from './tslib.mjs';
import { Future } from './utils-misc.mjs';

class DeferrableMapAbortError extends DOMException {
  constructor(message, reason) {
    super(message, 'AbortError');
    this.reason = reason;
  }
}
/**
 * A Map-like container keyed by unique strings that supports the ability to wait
 * for future keys to show up in the map.
 *
 * @example
 * // An already existing key:
 * const value = map.get("key");
 * // Wait for a key which will be added soon:
 * const value = await map.getDeferred("key");
 */
class DeferrableMap extends Map {
  constructor() {
    super(...arguments);
    this.pending = new Map();
  }
  set(key, value) {
    var _a, _b;
    super.set(key, value);
    // Resolve any futures waiting on this key.
    const futures = (_a = this.pending) === null || _a === void 0 ? void 0 : _a.get(key);
    if (futures) {
      for (const future of futures) {
        if (!future.isResolved) {
          (_b = future.resolve) === null || _b === void 0 ? void 0 : _b.call(future, value);
        }
      }
      this.pending.delete(key);
    }
    return this;
  }
  get [Symbol.toStringTag]() {
    return 'DeferrableMap';
  }
  getDeferred(key, signal) {
    return __awaiter(this, void 0, void 0, function* () {
      const existing = this.get(key);
      if (typeof existing !== 'undefined') {
        return existing;
      }
      // Bail out immediately if the signal is already aborted.
      if (signal === null || signal === void 0 ? void 0 : signal.aborted) {
        throw new DeferrableMapAbortError('The operation was aborted.', signal.reason);
      }
      const future = new Future(undefined, () => {
        // Clean up the pending list when the future settles.
        const futures = this.pending.get(key);
        if (!futures) {
          return;
        }
        const idx = futures.indexOf(future);
        if (idx !== -1) {
          futures.splice(idx, 1);
        }
        if (futures.length === 0) {
          this.pending.delete(key);
        }
      });
      const existingFutures = this.pending.get(key);
      if (existingFutures) {
        existingFutures.push(future);
      } else {
        this.pending.set(key, [future]);
      }
      // If a signal was provided, listen for abort and reject the future.
      if (signal) {
        const onAbort = () => {
          var _a;
          if (!future.isResolved) {
            (_a = future.reject) === null || _a === void 0 ? void 0 : _a.call(future, new DeferrableMapAbortError('The operation was aborted.', signal.reason));
          }
        };
        signal.addEventListener('abort', onAbort, {
          once: true
        });
        // Clean up the listener once the future settles (resolved or rejected).
        future.promise.finally(() => {
          signal.removeEventListener('abort', onAbort);
        });
      }
      return future.promise;
    });
  }
}
export { DeferrableMap, DeferrableMapAbortError };
