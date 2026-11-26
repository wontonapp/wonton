// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 17313-18616 行按职责切出，行为不变。
import { _ } from './protobuf-helpers.mjs';
import { __awaiter, _arrayLikeToArray, _defineProperty, _objectSpread2, _objectWithoutProperties, _toArray } from './tslib.mjs';
import { DataTrackInfo$1, Encryption_Type } from './protobuf-messages-room.mjs';
import { LivekitReasonedError } from './errors.mjs';
import { U16_MAX_SIZE } from './device.mjs';


function coerceToDataView(input) {
  if (input instanceof DataView) {
    return input;
  } else if (input instanceof ArrayBuffer) {
    return new DataView(input);
  } else if (input instanceof Uint8Array) {
    return new DataView(input.buffer, input.byteOffset, input.byteLength);
  } else {
    throw new Error("Error coercing ".concat(input, " to DataView - input was not DataView, ArrayBuffer, or Uint8Array."));
  }
}var DataTrackHandleErrorReason;
(function (DataTrackHandleErrorReason) {
  DataTrackHandleErrorReason[DataTrackHandleErrorReason["Reserved"] = 0] = "Reserved";
  DataTrackHandleErrorReason[DataTrackHandleErrorReason["TooLarge"] = 1] = "TooLarge";
})(DataTrackHandleErrorReason || (DataTrackHandleErrorReason = {}));
class DataTrackHandleError extends LivekitReasonedError {
  constructor(message, reason) {
    super(19, message);
    this.name = 'DataTrackHandleError';
    this.reason = reason;
    this.reasonName = DataTrackHandleErrorReason[reason];
  }
  isReason(reason) {
    return this.reason === reason;
  }
  static tooLarge() {
    return new DataTrackHandleError('Value too large to be a valid track handle', DataTrackHandleErrorReason.TooLarge);
  }
  static reserved(value) {
    return new DataTrackHandleError("0x".concat(value.toString(16), " is a reserved value."), DataTrackHandleErrorReason.Reserved);
  }
}
const DataTrackHandle = {
  fromNumber(raw) {
    if (raw === 0) {
      throw DataTrackHandleError.reserved(raw);
    }
    if (raw > U16_MAX_SIZE) {
      throw DataTrackHandleError.tooLarge();
    }
    return raw;
  }
};
/** Manage allocating new handles which don't conflict over the lifetime of the client. */
class DataTrackHandleAllocator {
  constructor() {
    this.value = 0;
  }
  /** Returns a unique track handle for the next publication, if one can be obtained. */
  get() {
    this.value += 1;
    if (this.value > U16_MAX_SIZE) {
      return null;
    }
    return this.value;
  }
  reset() {
    this.value = 0;
  }
}const DataTrackInfo = {
  from(protocolInfo) {
    return {
      sid: protocolInfo.sid,
      pubHandle: protocolInfo.pubHandle,
      name: protocolInfo.name,
      usesE2ee: protocolInfo.encryption !== Encryption_Type.NONE
    };
  },
  toProtobuf(info) {
    return new DataTrackInfo$1({
      sid: info.sid,
      pubHandle: info.pubHandle,
      name: info.name,
      encryption: info.usesE2ee ? Encryption_Type.GCM : Encryption_Type.NONE
    });
  }
};var QueueTaskStatus;
(function (QueueTaskStatus) {
  QueueTaskStatus[QueueTaskStatus["WAITING"] = 0] = "WAITING";
  QueueTaskStatus[QueueTaskStatus["RUNNING"] = 1] = "RUNNING";
  QueueTaskStatus[QueueTaskStatus["COMPLETED"] = 2] = "COMPLETED";
})(QueueTaskStatus || (QueueTaskStatus = {}));
class AsyncQueue {
  constructor() {
    this.pendingTasks = new Map();
    this.taskMutex = new _();
    this.nextTaskIndex = 0;
  }
  run(task) {
    return __awaiter(this, void 0, void 0, function* () {
      const taskInfo = {
        id: this.nextTaskIndex++,
        enqueuedAt: Date.now(),
        status: QueueTaskStatus.WAITING
      };
      this.pendingTasks.set(taskInfo.id, taskInfo);
      const unlock = yield this.taskMutex.lock();
      try {
        taskInfo.executedAt = Date.now();
        taskInfo.status = QueueTaskStatus.RUNNING;
        return yield task();
      } finally {
        taskInfo.status = QueueTaskStatus.COMPLETED;
        this.pendingTasks.delete(taskInfo.id);
        unlock();
      }
    });
  }
  flush() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.run(() => __awaiter(this, void 0, void 0, function* () {}));
    });
  }
  snapshot() {
    return Array.from(this.pendingTasks.values());
  }
}/**
 * Implementation of AbortSignal.any
 * Creates a signal that will be aborted when any of the given signals is aborted.
 * @link https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal/any
 */
function abortSignalAny(signals) {
  // Handle empty signals array
  if (signals.length === 0) {
    const controller = new AbortController();
    return controller.signal;
  }
  // Fast path for single signal
  if (signals.length === 1) {
    return signals[0];
  }
  // Check if any signal is already aborted
  for (const signal of signals) {
    if (signal.aborted) {
      return signal;
    }
  }
  // Create a new controller for the combined signal
  const controller = new AbortController();
  const unlisteners = Array(signals.length);
  // Function to clean up all event listeners
  const cleanup = () => {
    for (const unsubscribe of unlisteners) {
      unsubscribe();
    }
  };
  // Add event listeners to each signal
  signals.forEach((signal, index) => {
    const handler = () => {
      controller.abort(signal.reason);
      cleanup();
    };
    signal.addEventListener('abort', handler);
    unlisteners[index] = () => signal.removeEventListener('abort', handler);
  });
  return controller.signal;
}
/**
 * Implementation of AbortSignal.timeout
 * Creates a signal that will be aborted after the specified timeout.
 * @link https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal/timeout
 */
function abortSignalTimeout(ms) {
  const controller = new AbortController();
  setTimeout(() => {
    controller.abort(new DOMException("signal timed out after ".concat(ms, " ms"), 'TimeoutError'));
  }, ms);
  return controller.signal;
}/**
 * Opt-in registry that lets a development tool observe the state machines driving the connection
 * layer.
 *
 * The machines are private to the objects that own them and, more importantly, do not outlive them:
 * a full reconnect replaces the whole engine along with its `SignalClient`, so anything watching a
 * single machine reference goes blind exactly when the interesting part starts. Machines therefore
 * announce themselves here as they are constructed, and a subscriber sees the whole succession.
 *
 * Nothing is recorded until {@link enableMachineInspector} is called, which no shipping code does —
 * with the registry off, announcing is a comparison and a return.
 */
/** Kept small: enough for a panel opened mid-session to see how the connection got where it is. */
/** Announces a machine to whatever is watching. A no-op unless the inspector was enabled. */
function announceMachine(label, machine) {
  {
    return;
  }
}const _excluded = ["client"];
//#region src/emitter.ts
/**
* Minimal typed event emitter used internally by Fsm and BehavioralFsm.
*
* Two listener categories exist at runtime: named listeners (keyed by event
* name) and wildcard listeners (keyed as `"*"`). The wildcard fires on EVERY
* emit, regardless of event name, and its callback receives both the event
* name and the data. Named listeners receive only the data.
*
* @typeParam TEventMap - Record mapping event names to their payload types.
*   Constrains `on()` and `emit()` to matching event/payload pairs.
*/
var Emitter = class Emitter {
  constructor() {
    _defineProperty(this, "listeners", /* @__PURE__ */new Map());
  }
  on(event, cb) {
    let set = this.listeners.get(event);
    if (!set) {
      set = /* @__PURE__ */new Set();
      this.listeners.set(event, set);
    }
    set.add(cb);
    return {
      off: () => {
        set.delete(cb);
      }
    };
  }
  /**
  * Emit a named event, notifying wildcard listeners first, then named listeners.
  *
  * Wildcard listeners receive `(eventName, data)`; named listeners receive
  * only `data`. The firing order (wildcards before named) is intentional —
  * it lets relay listeners (like Fsm's bfsm proxy) observe all events
  * before specific subscribers react.
  */
  emit(event, data) {
    const wildcards = this.listeners.get("*");
    if (wildcards) for (const cb of wildcards) cb(event, data);
    const named = this.listeners.get(event);
    if (named) for (const cb of named) cb(data);
  }
  /**
  * Remove all listeners (named and wildcard).
  *
  * Existing `Subscription` objects remain valid — their `off()` closures
  * hold a reference to the now-empty Set, so calling them is harmless.
  * Called by `dispose()` to prevent memory retention after an FSM shuts down.
  */
  clear() {
    this.listeners.clear();
  }
};

//#endregion
//#region src/json-safe.ts
/**
* Thrown internally when the walk hits a value that can't survive a
* serialization boundary. Callers catch this to attach FSM/input context
* before re-throwing a fully descriptive error — see `path`/`label` below.
*/
var NonSerializableValueError = class extends Error {
  constructor(path, label) {
    super("non-serializable value at ".concat(path, " (").concat(label, ")"));
    this.path = path;
    this.label = label;
  }
};
/**
* Deep-clones `value`, throwing `NonSerializableValueError` the moment it
* finds anything that isn't `null`, a boolean, a finite number, a string, a
* plain array, or a plain object. `rootPath` seeds the path used in error
* messages — e.g. `"args"` so a nested failure reads as `args[1].onComplete`.
*
* Cloning (rather than just validating) is what keeps a returned snapshot
* from aliasing live FSM state: mutating the snapshot afterward — or the
* original value passed into a deferred `handle()` call — can't reach back
* into the FSM's internal deferred queue.
*/
const cloneJsonSafe = (value, rootPath) => {
  return cloneNode(value, rootPath, /* @__PURE__ */new Set());
};
const cloneNode = (value, path, ancestors) => {
  if (value === null) return null;
  switch (typeof value) {
    case "string":
    case "boolean":
      return value;
    case "number":
      if (!Number.isFinite(value)) throw new NonSerializableValueError(path, describeNonFiniteNumber(value));
      return value;
    case "object":
      return cloneObject(value, path, ancestors);
    default:
      throw new NonSerializableValueError(path, typeof value);
  }
};
const cloneObject = (obj, path, ancestors) => {
  var _obj$constructor$name, _obj$constructor;
  if (ancestors.has(obj)) throw new NonSerializableValueError(path, "circular reference");
  if (Array.isArray(obj)) {
    if (obj.length !== Object.keys(obj).length) throw new NonSerializableValueError(path, "sparse array or array with non-index properties");
    ancestors.add(obj);
    const cloned = obj.map((item, i) => cloneNode(item, "".concat(path, "[").concat(i, "]"), ancestors));
    ancestors.delete(obj);
    return cloned;
  }
  const proto = Object.getPrototypeOf(obj);
  if (proto !== Object.prototype && proto !== null) throw new NonSerializableValueError(path, (_obj$constructor$name = (_obj$constructor = obj.constructor) === null || _obj$constructor === void 0 ? void 0 : _obj$constructor.name) !== null && _obj$constructor$name !== void 0 ? _obj$constructor$name : "object");
  ancestors.add(obj);
  const cloned = {};
  for (const key of Object.keys(obj)) {
    const value = cloneNode(obj[key], "".concat(path, ".").concat(key), ancestors);
    Object.defineProperty(cloned, key, {
      value,
      enumerable: true,
      writable: true,
      configurable: true
    });
  }
  ancestors.delete(obj);
  return cloned;
};
const describeNonFiniteNumber = n => {
  if (Number.isNaN(n)) return "NaN";
  return n > 0 ? "Infinity" : "-Infinity";
};
/**
* Deep-clones `value` WITHOUT validating it — unlike `cloneJsonSafe`, this
* never throws. Plain objects/arrays are cloned recursively (so a caller
* can't alias state back into whatever holds the result); anything else
* (functions, `Date`/`Map`/class instances, symbols, non-finite numbers,
* etc.) is passed through by reference as-is.
*
* This is `rehydrate()`'s side of the aliasing guarantee: `dehydrate()`
* validates-and-clones on the way OUT (via `cloneJsonSafe`), but `rehydrate()`
* trusts the snapshot it's given is already valid data — re-validating on
* the way IN would be a redundant, unwanted asymmetry (see the build plan's
* disclosed known gaps). We still need the clone so mutating the caller's
* snapshot object after `rehydrate()` returns can't reach into the live
* FSM's internal deferred queue.
*
* Cycle-safe: a value that contains itself is returned as-is (by reference)
* rather than cloned infinitely — there's no validation step here to make
* that throw, so silently keeping the shared reference is the only option
* that doesn't hang.
*/
const cloneDeep = function (value) {
  let ancestors = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : /* @__PURE__ */new Set();
  if (value === null || typeof value !== "object") return value;
  if (ancestors.has(value)) return value;
  if (Array.isArray(value)) {
    ancestors.add(value);
    const cloned = value.map(item => cloneDeep(item, ancestors));
    ancestors.delete(value);
    return cloned;
  }
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) return value;
  ancestors.add(value);
  const cloned = {};
  for (const key of Object.keys(value)) {
    const clonedValue = cloneDeep(value[key], ancestors);
    Object.defineProperty(cloned, key, {
      value: clonedValue,
      enumerable: true,
      writable: true,
      configurable: true
    });
  }
  ancestors.delete(value);
  return cloned;
};

//#endregion
//#region src/types.ts
/**
* Symbol used as a property key to identify machina FSM instances at runtime.
* Each class stamps itself with a MachinaType value so the ChildLink adapter
* can dispatch handle()/canHandle()/reset() correctly without circular imports.
*/
const MACHINA_TYPE = Symbol("machina.type");

//#endregion
//#region src/behavioral-fsm.ts
const MAX_TRANSITION_DEPTH = 20;
/**
* Defines FSM behavior (states + transitions) while tracking per-client state
* in a `WeakMap`. A single `BehavioralFsm` instance can drive any number of
* independent client objects simultaneously — each gets its own state,
* deferred queue, and lifecycle.
*
* Prefer `createBehavioralFsm()` over constructing this directly — the factory
* infers all generic parameters from the config object.
*
* All public methods silently no-op after `dispose()` is called.
*
* @typeParam TClient - The client object type. Must be an object (non-primitive)
*   so it can serve as a WeakMap key.
* @typeParam TStateNames - String literal union of valid state names.
* @typeParam TInputNames - String literal union of valid input names.
* @typeParam TBubbles - String literal union of inputs this FSM declares via
*   `bubbles`. Type-only — carried so `BubblesOfInstance` can extract it from
*   a constructed instance; nothing at runtime reads this generic.
*/
var BehavioralFsm = class BehavioralFsm {
  constructor(config) {
    _defineProperty(this, "id", void 0);
    _defineProperty(this, "initialState", void 0);
    _defineProperty(this, MACHINA_TYPE, "BehavioralFsm");
    _defineProperty(this, "states", void 0);
    _defineProperty(this, "emitter", new Emitter());
    _defineProperty(this, "clients", /* @__PURE__ */new WeakMap());
    _defineProperty(this, "knownClients", /* @__PURE__ */new Set());
    _defineProperty(this, "childSubscriptions", []);
    _defineProperty(this, "disposed", false);
    _defineProperty(this, "transitionDepth", 0);
    this.id = config.id;
    this.initialState = config.initialState;
    this.states = config.states;
    this.wrapChildLinks();
    this.setupChildSubscriptions();
  }
  /**
  * Dispatch an input to the given client's current state handler.
  *
  * Delegation order: if the current state has a `_child` FSM that can
  * handle the input, it is dispatched there. If the child emits `nohandler`,
  * the input bubbles up to this FSM's local handler. If no handler exists
  * here either, `nohandler` is emitted on this FSM's emitter.
  *
  * No-ops silently when disposed.
  */
  handle(client, inputName) {
    var _this$states$meta$sta;
    if (this.disposed) return;
    const meta = this.getOrCreateClientMeta(client);
    for (var _len = arguments.length, args = new Array(_len > 2 ? _len - 2 : 0), _key = 2; _key < _len; _key++) {
      args[_key - 2] = arguments[_key];
    }
    meta.currentActionArgs = args;
    const childLink = (_this$states$meta$sta = this.states[meta.state]) === null || _this$states$meta$sta === void 0 ? void 0 : _this$states$meta$sta._child;
    if (childLink) {
      if (childLink.canHandle(client, inputName)) {
        try {
          childLink.handle(client, inputName, ...args);
        } finally {
          meta.currentActionArgs = void 0;
        }
        return;
      }
    }
    this.handleLocally(client, inputName, args, meta);
  }
  /**
  * Returns true if the client's current state has a handler for `inputName`
  * (or a catch-all `"*"` handler), or if the current state's `_child` chain
  * can handle it — the check recurses to the same depth `handle()`'s
  * delegation can actually reach, so a grandchild-only input answers true
  * from the root. Does NOT initialize the client — no `_onEnter`, no
  * events, no side effects. Unseen clients are treated as if they were
  * already in `initialState`. Returns false when disposed.
  */
  canHandle(client, inputName) {
    var _this$clients$get$sta, _this$clients$get, _stateObj$inputName;
    if (this.disposed) return false;
    const state = (_this$clients$get$sta = (_this$clients$get = this.clients.get(client)) === null || _this$clients$get === void 0 ? void 0 : _this$clients$get.state) !== null && _this$clients$get$sta !== void 0 ? _this$clients$get$sta : this.initialState;
    const stateObj = this.states[state];
    if ((_stateObj$inputName = stateObj === null || stateObj === void 0 ? void 0 : stateObj[inputName]) !== null && _stateObj$inputName !== void 0 ? _stateObj$inputName : stateObj === null || stateObj === void 0 ? void 0 : stateObj["*"]) return true;
    const childLink = stateObj === null || stateObj === void 0 ? void 0 : stateObj._child;
    return childLink ? childLink.canHandle(client, inputName) : false;
  }
  /**
  * Transition the client back to `initialState`, firing `_onEnter` and
  * lifecycle events as if entering it fresh. No-ops when disposed.
  */
  reset(client) {
    if (this.disposed) return;
    this.transition(client, this.initialState);
  }
  /**
  * Returns the client's current state, or `undefined` if the client has
  * never been initialized (i.e. `handle()`, `transition()`, or `reset()`
  * have never been called for it). Does NOT trigger initialization.
  */
  currentState(client) {
    var _this$clients$get2;
    return (_this$clients$get2 = this.clients.get(client)) === null || _this$clients$get2 === void 0 ? void 0 : _this$clients$get2.state;
  }
  /**
  * Directly transition `client` to `toState`, running the full lifecycle:
  * `_onExit` for the current state → `transitioning` event → update state →
  * `_onEnter` for new state → `transitioned` event → child reset → deferred
  * queue replay → bounce (if `_onEnter` returned a state name).
  *
  * Same-state transitions are silently ignored. Transitions to unknown state
  * names emit `invalidstate` instead of throwing. Throws if the transition
  * depth exceeds `MAX_TRANSITION_DEPTH` (likely an `_onEnter` → transition loop).
  *
  * No-ops when disposed.
  */
  transition(client, toState) {
    if (this.disposed) return;
    const meta = this.getOrCreateClientMeta(client);
    const fromState = meta.state;
    if (toState === fromState) return;
    if (!Object.hasOwn(this.states, toState)) {
      this.emitter.emit("invalidstate", {
        stateName: toState,
        client
      });
      return;
    }
    this.transitionDepth++;
    if (this.transitionDepth > MAX_TRANSITION_DEPTH) {
      this.transitionDepth = 0;
      throw new Error("Max transition depth (".concat(MAX_TRANSITION_DEPTH, ") exceeded in FSM \"").concat(this.id, "\". Likely an infinite _onEnter \u2192 transition loop."));
    }
    try {
      const curStateObj = this.states[fromState];
      const newStateObj = this.states[toState];
      if (curStateObj !== null && curStateObj !== void 0 && curStateObj._onExit && typeof curStateObj._onExit === "function") {
        const exitArgs = this.buildHandlerArgs(client, "", meta);
        curStateObj._onExit(exitArgs);
      }
      meta.state = toState;
      const payload = {
        fromState,
        toState,
        client
      };
      this.emitter.emit("transitioning", payload);
      let bounceTarget = void 0;
      if (newStateObj !== null && newStateObj !== void 0 && newStateObj._onEnter && typeof newStateObj._onEnter === "function") {
        const enterArgs = this.buildHandlerArgs(client, "", meta);
        bounceTarget = newStateObj._onEnter(enterArgs);
      }
      this.emitter.emit("transitioned", payload);
      const childLink = newStateObj === null || newStateObj === void 0 ? void 0 : newStateObj._child;
      if (childLink) childLink.reset(client);
      this.processQueue(client, meta);
      if (typeof bounceTarget === "string" && meta.state === toState) this.transition(client, bounceTarget);
    } finally {
      this.transitionDepth--;
    }
  }
  /**
  * Returns the client's state as a dot-delimited path including any active
  * child FSM states (e.g. `"active.connecting.retrying"`). Returns just the
  * current state name when no child is active. Returns `""` for clients that
  * have never been initialized (unlike `currentState()` which returns `undefined`).
  */
  compositeState(client) {
    var _this$states$meta$sta2;
    const meta = this.clients.get(client);
    if (!meta) return "";
    const childLink = (_this$states$meta$sta2 = this.states[meta.state]) === null || _this$states$meta$sta2 === void 0 ? void 0 : _this$states$meta$sta2._child;
    if (childLink) {
      const childComposite = childLink.compositeState(client);
      if (childComposite) return "".concat(meta.state, ".").concat(childComposite);
    }
    return meta.state;
  }
  rehydrate(client, input) {
    if (this.disposed) return;
    if (typeof input === "string") {
      this.rehydrateCompositePath(client, input);
      return;
    }
    const writes = this.planSnapshotWrites(client, input);
    for (const write of writes) write();
  }
  /**
  * Snapshot everything machina tracks for `client`: current state, pending
  * deferred inputs, and — recursively — the same for every `_child` that has
  * ever seen this client, active or not. Feed the result to the object form
  * of `rehydrate()` to restore it later, deferrals included.
  *
  * Returns `undefined` for a client this FSM has never seen (mirrors
  * `currentState()`) — the call does NOT trigger initialization.
  *
  * Throws if any deferred input's args contain a non-serializable value
  * (function, undefined, symbol, bigint, non-finite number, Date/Map/class
  * instance, or a circular reference) — naming the input, its `until` target
  * if any, the FSM id, and the exact value path. Throws for an Fsm child
  * that's on `client`'s active path *relative to the true root* (consistent
  * with `rehydrate()`'s conditional throw) — an Fsm owns its own context, so
  * there's nothing per-client to snapshot. An Fsm child declared at a state
  * `client` never visited, OR nested under a `BehavioralFsm` child that is
  * itself off-path from the root, is skipped rather than throwing — neither
  * has any per-client state to lose, so one Fsm child anywhere in the
  * hierarchy doesn't disable `dehydrate()` for clients that never reach that
  * branch, no matter how deeply nested the Fsm child is.
  *
  * Meant for clients at rest between `handle()` calls — `currentActionArgs`
  * (the in-flight args mid-handler) has no meaning here and is excluded.
  *
  * @param isOnActivePath - @internal Whether this FSM itself is currently
  * reachable from the true root's active path. Defaults to `true` for the
  * public entry point (this FSM IS the root from its own perspective); the
  * `ChildLink` adapter passes `false` down when recursing into a nested
  * `BehavioralFsm` child that is itself off-path, so that child's own
  * Fsm-child checks don't recompute reachability from its dormant local
  * state alone.
  */
  dehydrate(client) {
    let isOnActivePath = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
    const meta = this.clients.get(client);
    if (!meta) return;
    const snapshot = {
      state: meta.state,
      deferred: meta.deferredQueue.map(item => this.snapshotDeferredInput(item))
    };
    const children = this.collectChildSnapshots(client, meta.state, isOnActivePath);
    if (children) snapshot.children = children;
    return snapshot;
  }
  /**
  * @internal
  * Validates `snapshot` against this level's state graph and recurses into
  * every declared child, returning write thunks to run only once the WHOLE
  * tree — every level — validates successfully. Nothing is written here.
  * Called by the object-form of `rehydrate()` and, recursively, by ChildLink
  * so a nested BehavioralFsm participates in the same validate-then-write
  * pass. Not part of the public persistence API — call `rehydrate()` instead.
  */
  planSnapshotWrites(client, snapshot) {
    if (this.disposed) return [];
    const state = snapshot.state,
      deferred = snapshot.deferred,
      children = snapshot.children;
    if (!Object.hasOwn(this.states, state)) throw new Error("rehydrate: unknown state \"".concat(state, "\" in FSM \"").concat(this.id, "\". Valid states: ").concat(Object.keys(this.states).join(", ")));
    const writes = [];
    if (children) for (const stateName of Object.keys(children)) {
      var _this$states$stateNam;
      if (!Object.hasOwn(this.states, stateName)) throw new Error("rehydrate: unknown state \"".concat(stateName, "\" in FSM \"").concat(this.id, "\" referenced by snapshot.children."));
      const childLink = (_this$states$stateNam = this.states[stateName]) === null || _this$states$stateNam === void 0 ? void 0 : _this$states$stateNam._child;
      if (!childLink) throw new Error("rehydrate: state \"".concat(stateName, "\" in FSM \"").concat(this.id, "\" has no _child, but the snapshot has a children[\"").concat(stateName, "\"] entry."));
      writes.push(...childLink.planRehydrate(client, children[stateName]));
    }
    const deferredQueue = deferred.map(item => _objectSpread2({
      inputName: item.inputName,
      args: cloneDeep(item.args)
    }, item.untilState !== void 0 ? {
      untilState: item.untilState
    } : {}));
    writes.push(() => {
      if (!this.clients.has(client)) this.knownClients.add(new WeakRef(client));
      this.clients.set(client, {
        state,
        deferredQueue
      });
    });
    return writes;
  }
  rehydrateCompositePath(client, compositeState) {
    const _compositeState$split = compositeState.split("."),
      _compositeState$split2 = _toArray(_compositeState$split),
      state = _compositeState$split2[0],
      rest = _arrayLikeToArray(_compositeState$split2).slice(1);
    if (!Object.hasOwn(this.states, state)) throw new Error("rehydrate: unknown state \"".concat(state, "\" in FSM \"").concat(this.id, "\". Valid states: ").concat(Object.keys(this.states).join(", ")));
    if (rest.length > 0) {
      var _this$states$state;
      const childPath = rest.join(".");
      const childLink = (_this$states$state = this.states[state]) === null || _this$states$state === void 0 ? void 0 : _this$states$state._child;
      if (!childLink) throw new Error("rehydrate: state \"".concat(state, "\" in FSM \"").concat(this.id, "\" has no _child, but composite path \"").concat(compositeState, "\" requires one."));
      childLink.rehydrate(client, childPath);
    }
    if (!this.clients.has(client)) this.knownClients.add(new WeakRef(client));
    this.clients.set(client, {
      state,
      deferredQueue: []
    });
  }
  /**
  * Walks every declared state's `_child`, dehydrating each one that has ever
  * seen `client`. The same child instance can be declared under multiple
  * state names (shared child) — it's dehydrated once and the result is
  * reused under every declaring state name, matching how the engine already
  * treats shared children elsewhere (dispose, event subscriptions).
  *
  * Declaring names are grouped by `childLink.instance` (the actual FSM
  * instance, not the `ChildLink` wrapper — `wrapChildLinks()` mints a fresh
  * wrapper per declaring state, so grouping by wrapper would never hit for a
  * shared child) BEFORE any `onPath` is computed or any child is dehydrated.
  * A shared child's `onPath` is the OR of `stateName === activeState` across
  * every declaring name in its group: at most one declaring name can ever
  * equal `activeState`, so this combined flag is correct and, critically,
  * independent of `Object.keys(this.states)` iteration order — computing
  * `onPath` per-declaring-name and caching whichever one happened to run
  * first would silently launder an on-path client's Fsm-child throw through
  * an unrelated off-path declaring name.
  *
  * An off-path Fsm child (declared only at states other than `activeState`,
  * OR nested anywhere under a `BehavioralFsm` child that is itself off-path
  * relative to the true root) is skipped entirely rather than dehydrated:
  * Fsm state isn't tracked per-client to begin with, so an off-path Fsm
  * child has nothing to lose by being skipped — unlike a BehavioralFsm
  * child's off-path meta, which is real per-client data. `isOnActivePath`
  * is the inherited "am I even reachable from the root" flag; combining it
  * with the group's `stateNames.includes(activeState)` check (rather than
  * using that check alone) is what keeps a nested Fsm grandchild from
  * throwing when its immediate BehavioralFsm parent is itself off-path —
  * the parent's own dormant `activeState` is irrelevant once the parent
  * isn't reachable. This keeps one Fsm child anywhere in the hierarchy from
  * disabling `dehydrate()` for every client, only for clients actually on
  * that branch.
  */
  collectChildSnapshots(client, activeState, isOnActivePath) {
    const declaringNamesByInstance = /* @__PURE__ */new Map();
    for (const stateName of Object.keys(this.states)) {
      var _this$states$stateNam2;
      const childLink = (_this$states$stateNam2 = this.states[stateName]) === null || _this$states$stateNam2 === void 0 ? void 0 : _this$states$stateNam2._child;
      if (!childLink) continue;
      const entry = declaringNamesByInstance.get(childLink.instance);
      if (entry) entry.stateNames.push(stateName);else declaringNamesByInstance.set(childLink.instance, {
        childLink,
        stateNames: [stateName]
      });
    }
    let children;
    for (const _ref of declaringNamesByInstance.values()) {
      const childLink = _ref.childLink;
      const stateNames = _ref.stateNames;
      const onPath = isOnActivePath && stateNames.includes(activeState);
      if (childLink.instance[MACHINA_TYPE] === "Fsm" && !onPath) continue;
      const childSnapshot = childLink.dehydrate(client, onPath);
      if (childSnapshot) {
        children !== null && children !== void 0 ? children : children = {};
        for (const stateName of stateNames) children[stateName] = childSnapshot;
      }
    }
    return children;
  }
  snapshotDeferredInput(item) {
    let clonedArgs;
    try {
      clonedArgs = cloneJsonSafe(item.args, "args");
    } catch (err) {
      if (!(err instanceof NonSerializableValueError)) throw err;
      const untilPart = item.untilState ? " (until \"".concat(item.untilState, "\")") : "";
      throw new Error("dehydrate: deferred input \"".concat(item.inputName, "\"").concat(untilPart, " in FSM \"").concat(this.id, "\" has a non-serializable value at ").concat(err.path, " (").concat(err.label, ")"));
    }
    const snapshot = {
      inputName: item.inputName,
      args: clonedArgs
    };
    if (item.untilState !== void 0) snapshot.untilState = item.untilState;
    return snapshot;
  }
  on(eventName, callback) {
    if (this.disposed) return {
      off() {}
    };
    return this.emitter.on(eventName, callback);
  }
  /**
  * Emit a custom event through the FSM. Built-in lifecycle events are
  * emitted automatically — this is for user-defined events from handlers.
  * No-ops when disposed.
  */
  emit(eventName, data) {
    if (this.disposed) return;
    this.emitter.emit(eventName, data);
  }
  /**
  * Permanently shut down this FSM. Irreversible — all subsequent method
  * calls become silent no-ops. Tears down child subscriptions, clears all
  * listeners, and cascades disposal to child FSMs (unless `preserveChildren`
  * is set). The same child appearing in multiple states is disposed once.
  */
  dispose(options) {
    this.disposed = true;
    for (const sub of this.childSubscriptions) sub.off();
    if (!(options !== null && options !== void 0 && options.preserveChildren)) {
      const seen = /* @__PURE__ */new Set();
      for (const stateName of Object.keys(this.states)) {
        var _this$states$stateNam3;
        const childLink = (_this$states$stateNam3 = this.states[stateName]) === null || _this$states$stateNam3 === void 0 ? void 0 : _this$states$stateNam3._child;
        if (childLink && !seen.has(childLink.instance)) {
          seen.add(childLink.instance);
          childLink.dispose();
        }
      }
    }
    this.emitter.clear();
  }
  /**
  * Walks all states at construction time, detects raw FSM instances assigned
  * to _child, and wraps them into ChildLink adapters via createChildLink().
  * Must run BEFORE setupChildSubscriptions() so the subscriptions see
  * ChildLink objects, not raw FSM instances.
  */
  wrapChildLinks() {
    for (const stateName of Object.keys(this.states)) {
      const stateObj = this.states[stateName];
      const rawChild = stateObj === null || stateObj === void 0 ? void 0 : stateObj._child;
      if (!rawChild) continue;
      if (typeof rawChild !== "object") throw new Error("State \"".concat(stateName, "\"._child: expected an Fsm or BehavioralFsm instance, got ").concat(String(rawChild)));
      if (!(MACHINA_TYPE in rawChild)) throw new Error("State \"".concat(stateName, "\"._child: expected an Fsm or BehavioralFsm instance, got a plain object"));
      stateObj._child = createChildLink(rawChild);
    }
  }
  /**
  * Walks all states at construction time, finds states with _child, and
  * subscribes once to each unique child's wildcard events. Subscriptions are
  * stored for cleanup in dispose(). We deduplicate by `childLink.instance`
  * (the underlying Fsm/BehavioralFsm instance), not the `ChildLink` wrapper —
  * `wrapChildLinks()` mints a fresh wrapper per declaring state, so a child
  * shared across states would otherwise get one subscription PER declaring
  * state, each independently walking known clients and relaying events. That
  * silently double-fires client-less relays (Fsm-child events, or a
  * BehavioralFsm child's custom `emit()` with no `client` in the payload)
  * whenever two different clients are active on two different declaring
  * names at once — the same wrapper-vs-instance identity bug
  * `collectChildSnapshots()` had before its #184 fix.
  */
  setupChildSubscriptions() {
    const seenInstances = /* @__PURE__ */new Set();
    for (const stateName of Object.keys(this.states)) {
      var _this$states$stateNam4;
      const childLink = (_this$states$stateNam4 = this.states[stateName]) === null || _this$states$stateNam4 === void 0 ? void 0 : _this$states$stateNam4._child;
      if (!childLink || seenInstances.has(childLink.instance)) continue;
      seenInstances.add(childLink.instance);
      const sub = childLink.onAny((eventName, data) => {
        if (eventName === "nohandler") {
          var _payload$args;
          const payload = data;
          if (payload.client !== void 0) this.bubbleNohandler(payload.client, childLink, payload.inputName, (_payload$args = payload.args) !== null && _payload$args !== void 0 ? _payload$args : []);else for (const ref of this.knownClients) {
            var _payload$args2;
            const client = ref.deref();
            if (client === void 0) {
              this.knownClients.delete(ref);
              continue;
            }
            this.bubbleNohandler(client, childLink, payload.inputName, (_payload$args2 = payload.args) !== null && _payload$args2 !== void 0 ? _payload$args2 : []);
          }
          return;
        }
        const payload = data;
        if (payload && typeof payload === "object" && "client" in payload) {
          if (this.isChildActiveForClient(payload.client, childLink)) this.emitter.emit(eventName, data);
        } else for (const ref of this.knownClients) {
          const client = ref.deref();
          if (!client) {
            this.knownClients.delete(ref);
            continue;
          }
          if (this.isChildActiveForClient(client, childLink)) {
            this.emitter.emit(eventName, data);
            break;
          }
        }
      });
      this.childSubscriptions.push(sub);
    }
  }
  /**
  * Bubbles a child nohandler to the parent for the given client.
  * Only fires if the client is currently in a state that has this childLink.
  * Extracted from the lambda in setupChildSubscriptions to keep it readable.
  */
  bubbleNohandler(client, childLink, inputName, args) {
    if (!this.isChildActiveForClient(client, childLink)) return;
    const meta = this.clients.get(client);
    meta.currentActionArgs = args;
    this.handleLocally(client, inputName, args, meta);
  }
  /**
  * Returns true if the given client is currently in a parent state whose
  * _child resolves to the same underlying instance as childLink. Returns
  * false if the client has no meta (never initialized) or is in a state
  * with a different (or no) child.
  *
  * Compares `.instance`, not the `ChildLink` wrapper itself — setupChildSubscriptions()
  * dedupes subscriptions by instance and keeps only ONE representative wrapper
  * per shared child, so the client's actual active declaring state may hold a
  * DIFFERENT wrapper for that same instance (wrapChildLinks() mints one per
  * declaring state). Comparing wrappers directly would only ever match the one
  * declaring state whose wrapper happened to be kept for the subscription,
  * silently breaking relay for every other declaring name of a shared child.
  */
  isChildActiveForClient(client, childLink) {
    var _this$states$meta$sta3;
    const meta = this.clients.get(client);
    if (!meta) return false;
    return ((_this$states$meta$sta3 = this.states[meta.state]) === null || _this$states$meta$sta3 === void 0 || (_this$states$meta$sta3 = _this$states$meta$sta3._child) === null || _this$states$meta$sta3 === void 0 ? void 0 : _this$states$meta$sta3.instance) === childLink.instance;
  }
  /**
  * The inner handler dispatch — no delegation, no initialization side effects
  * beyond what getOrCreateClientMeta already did. Called by handle() after
  * the delegation check, and by the nohandler child listener for bubbling.
  */
  handleLocally(client, inputName, args, meta) {
    var _stateObj$inputName2;
    const stateObj = this.states[meta.state];
    const handler = (_stateObj$inputName2 = stateObj === null || stateObj === void 0 ? void 0 : stateObj[inputName]) !== null && _stateObj$inputName2 !== void 0 ? _stateObj$inputName2 : stateObj === null || stateObj === void 0 ? void 0 : stateObj["*"];
    if (!handler) {
      this.emitter.emit("nohandler", {
        inputName,
        args,
        client
      });
      meta.currentActionArgs = void 0;
      return;
    }
    try {
      this.emitter.emit("handling", {
        inputName,
        client
      });
      const handlerArgs = this.buildHandlerArgs(client, inputName, meta);
      let targetState = void 0;
      if (typeof handler === "string") targetState = handler;else if (typeof handler === "function") targetState = handler(handlerArgs, ...args);
      this.emitter.emit("handled", {
        inputName,
        client
      });
      if (typeof targetState === "string") this.transition(client, targetState);
    } finally {
      meta.currentActionArgs = void 0;
    }
  }
  getOrCreateClientMeta(client) {
    let meta = this.clients.get(client);
    if (meta) return meta;
    meta = {
      state: void 0,
      deferredQueue: []
    };
    this.clients.set(client, meta);
    this.knownClients.add(new WeakRef(client));
    this.transition(client, this.initialState);
    return meta;
  }
  buildHandlerArgs(client, inputName, meta) {
    return {
      ctx: client,
      inputName,
      defer: opts => {
        if (!meta.currentActionArgs) return;
        const deferred = {
          inputName,
          args: [...meta.currentActionArgs],
          untilState: opts === null || opts === void 0 ? void 0 : opts.until
        };
        meta.deferredQueue.push(deferred);
        this.emitter.emit("deferred", {
          inputName,
          client
        });
      },
      emit: (evtName, evtData) => {
        this.emitter.emit(evtName, evtData);
      }
    };
  }
  processQueue(client, meta) {
    const toReplay = [];
    const remaining = [];
    for (const item of meta.deferredQueue) if (item.untilState === void 0 || item.untilState === meta.state) toReplay.push(item);else remaining.push(item);
    meta.deferredQueue = remaining;
    for (const item of toReplay) this.handle(client, item.inputName, ...item.args);
  }
};
/**
* Internal factory called by `wrapChildLinks()` during construction.
* Wraps a raw Fsm or BehavioralFsm instance in a uniform `ChildLink` adapter
* so the parent engine doesn't need to know which type it's talking to.
*
* Users never call this directly — they assign an FSM instance to `_child`
* in their state config and `wrapChildLinks()` handles the wrapping.
*/
function createChildLink(child) {
  if (!child || typeof child !== "object") throw new Error("createChildLink: expected an Fsm or BehavioralFsm instance, got ".concat(String(child)));
  const childType = child[MACHINA_TYPE];
  if (childType === "BehavioralFsm") return {
    instance: child,
    canHandle(client, inputName) {
      return child.canHandle(client, inputName);
    },
    handle(client, inputName) {
      for (var _len2 = arguments.length, args = new Array(_len2 > 2 ? _len2 - 2 : 0), _key2 = 2; _key2 < _len2; _key2++) {
        args[_key2 - 2] = arguments[_key2];
      }
      child.handle(client, inputName, ...args);
    },
    reset(client) {
      child.transition(client, child.initialState);
    },
    onAny(callback) {
      return child.on("*", callback);
    },
    compositeState(client) {
      return child.compositeState(client);
    },
    rehydrate(client, compositeState) {
      child.rehydrate(client, compositeState);
    },
    dehydrate(client, isOnActivePath) {
      return child.dehydrate(client, isOnActivePath);
    },
    planRehydrate(client, snapshot) {
      return child.planSnapshotWrites(client, snapshot);
    },
    dispose() {
      child.dispose();
    }
  };
  if (childType === "Fsm") return {
    instance: child,
    canHandle(_client, inputName) {
      return child.canHandle(inputName);
    },
    handle(_client, inputName) {
      for (var _len3 = arguments.length, args = new Array(_len3 > 2 ? _len3 - 2 : 0), _key3 = 2; _key3 < _len3; _key3++) {
        args[_key3 - 2] = arguments[_key3];
      }
      child.handle(inputName, ...args);
    },
    reset(_client) {
      child.reset();
    },
    onAny(callback) {
      return child.on("*", callback);
    },
    compositeState(_client) {
      return child.compositeState();
    },
    rehydrate(_client, _compositeState) {
      throw new Error("rehydrate: cannot rehydrate an Fsm child. Fsm owns its own context; rehydrate is only valid for BehavioralFsm hierarchies.");
    },
    dehydrate(_client, _isOnActivePath) {
      throw new Error("dehydrate: cannot dehydrate an Fsm child. Fsm owns its own context; dehydrate is only valid for BehavioralFsm hierarchies.");
    },
    planRehydrate(_client, _snapshot) {
      throw new Error("rehydrate: cannot rehydrate an Fsm child. Fsm owns its own context; rehydrate is only valid for BehavioralFsm hierarchies.");
    },
    dispose() {
      child.dispose();
    }
  };
  throw new Error("createChildLink: expected an Fsm or BehavioralFsm instance, got [MACHINA_TYPE] = ".concat(String(childType !== null && childType !== void 0 ? childType : "undefined")));
}

//#endregion
//#region src/fsm.ts
/**
* Single-client FSM. Wraps a BehavioralFsm and uses the config's `context`
* object as the implicit client, so callers never pass a client argument.
*
* Prefer `createFsm()` over constructing this directly — the factory infers
* all generic parameters from the config object.
*
* All public methods silently no-op after `dispose()` is called.
*
* @typeParam TCtx - The context type, inferred from `config.context`.
* @typeParam TStateNames - String literal union of valid state names.
* @typeParam TInputNames - String literal union of valid input names.
* @typeParam TBubbles - String literal union of inputs this FSM declares via
*   `bubbles`. Type-only — carried so `BubblesOfInstance` can extract it from
*   a constructed instance; nothing at runtime reads this generic.
*/
var Fsm = class Fsm {
  constructor(config) {
    var _config$context;
    _defineProperty(this, "id", void 0);
    _defineProperty(this, "initialState", void 0);
    _defineProperty(this, MACHINA_TYPE, "Fsm");
    _defineProperty(this, "states", void 0);
    _defineProperty(this, "bfsm", void 0);
    _defineProperty(this, "context", void 0);
    _defineProperty(this, "emitter", new Emitter());
    _defineProperty(this, "disposed", false);
    this.id = config.id;
    this.initialState = config.initialState;
    this.context = (_config$context = config.context) !== null && _config$context !== void 0 ? _config$context : {};
    this.bfsm = new BehavioralFsm(config);
    this.states = config.states;
    this.bfsm.on("*", (eventName, data) => {
      if (data && typeof data === "object" && "client" in data) {
        data.client;
          const payload = _objectWithoutProperties(data, _excluded);
        this.emitter.emit(eventName, payload);
      } else this.emitter.emit(eventName, data);
    });
    this.bfsm.transition(this.context, config.initialState);
  }
  /**
  * Dispatch an input to the current state's handler.
  * If a `_child` FSM in the current state can handle it, delegation occurs
  * there first; unhandled inputs bubble up to the parent.
  * No-ops silently when disposed.
  */
  handle(inputName) {
    if (this.disposed) return;
    for (var _len4 = arguments.length, args = new Array(_len4 > 1 ? _len4 - 1 : 0), _key4 = 1; _key4 < _len4; _key4++) {
      args[_key4 - 1] = arguments[_key4];
    }
    this.bfsm.handle(this.context, inputName, ...args);
  }
  /**
  * Returns true if the current state has a handler for `inputName`
  * (or a catch-all `"*"` handler), or if the current state's `_child`
  * chain can handle it (checked recursively, matching `handle()`'s
  * delegation reach). Does not trigger initialization or any side
  * effects. Returns false when disposed.
  */
  canHandle(inputName) {
    if (this.disposed) return false;
    return this.bfsm.canHandle(this.context, inputName);
  }
  /**
  * Transition back to `initialState`, firing `_onEnter` and lifecycle
  * events as if entering it fresh. No-ops silently when disposed.
  */
  reset() {
    if (this.disposed) return;
    this.bfsm.reset(this.context);
  }
  /**
  * Returns the current state name. Always defined — Fsm eagerly
  * initializes into `initialState` during construction.
  */
  currentState() {
    return this.bfsm.currentState(this.context);
  }
  /**
  * Directly transition to `toState`, firing `_onExit`, `_onEnter`, and
  * lifecycle events. Same-state transitions are silently ignored.
  * No-ops when disposed.
  */
  transition(toState) {
    if (this.disposed) return;
    this.bfsm.transition(this.context, toState);
  }
  /**
  * Returns the current state as a dot-delimited path that includes
  * any active child FSM states (e.g. `"active.connecting.retrying"`).
  * Returns just the current state name when no child is active.
  */
  compositeState() {
    return this.bfsm.compositeState(this.context);
  }
  on(eventName, callback) {
    if (this.disposed) return {
      off() {}
    };
    return this.emitter.on(eventName, callback);
  }
  /**
  * Emit a custom event through the FSM. Built-in lifecycle events are
  * emitted automatically — this is for user-defined events from handlers.
  * Routes through the BehavioralFsm so all relay paths are consistent.
  * No-ops when disposed.
  */
  emit(eventName, data) {
    if (this.disposed) return;
    this.bfsm.emit(eventName, data);
  }
  /**
  * Permanently shut down this FSM. Irreversible — all subsequent method
  * calls become silent no-ops. Clears all listeners and cascades disposal
  * to child FSMs (unless `preserveChildren` is set).
  */
  dispose(options) {
    this.disposed = true;
    this.bfsm.dispose(options);
    this.emitter.clear();
  }
};
/**
* Create a single-client FSM from a config object.
*
* Generic parameters are inferred automatically:
* - `TCtx` comes from `config.context` (defaults to `{}` if omitted).
* - `TStates` is captured with `const` inference to preserve string literal
*   types, enabling compile-time validation of transition targets and `handle()`
*   input names.
*
* State names, input names, and all handler signatures derive from `TStates`.
*
* @example
* ```ts
* const light = createFsm({
*   id: "traffic-light",
*   initialState: "green",
*   context: { tickCount: 0 },
*   states: {
*     green:  { timeout: "yellow" },
*     yellow: { timeout: "red" },
*     red:    { timeout: "green" },
*   },
* });
*
* light.handle("timeout"); // transitions green → yellow
* ```
*/
function createFsm(config) {
  return new Fsm(config);
}/**
 * Declares a handler for one input, restoring the payload typing that machina's `...unknown[]`
 * handler arguments give up.
 */
function on(handler) {
  return handler;
}
/**
 * Whether a transport-originated input belongs to the attempt that currently owns the session.
 * Inputs from a superseded attempt are dropped: its transport is already being replaced, so it can
 * neither declare the session live nor take it down.
 */
function isCurrentAttempt(ctx, event) {
  return event.attemptId === ctx.attemptId;
}
const attemptEstablished = on((_ref, event) => {
  let ctx = _ref.ctx;
  if (!isCurrentAttempt(ctx, event)) {
    return;
  }
  return 'connected';
});
const startConnect = on(_ref2 => {
  let ctx = _ref2.ctx;
  ctx.attemptId += 1;
  ctx.lastError = undefined;
  return 'connecting';
});
const startReconnect = on(_ref3 => {
  let ctx = _ref3.ctx;
  ctx.attemptId += 1;
  ctx.lastError = undefined;
  return 'reconnecting';
});
const requestClose = on((_ref4, event) => {
  let ctx = _ref4.ctx;
  ctx.closeReason = event.reason;
  return 'disconnecting';
});
const signalStates = {
  new: {
    connect: startConnect,
    close: requestClose
  },
  connecting: {
    connectComplete: attemptEstablished,
    // An initial connect has no session to fall back on, so failure is terminal.
    connectFailed: on((_ref5, event) => {
      let ctx = _ref5.ctx;
      ctx.lastError = event.error;
      return 'closed';
    }),
    close: requestClose
  },
  connected: {
    reconnect: startReconnect,
    transportFailed: on((_ref6, event) => {
      let ctx = _ref6.ctx;
      if (!isCurrentAttempt(ctx, event)) {
        // a transport that has already been replaced, reporting its close late
        return;
      }
      ctx.lastError = event.reason;
      return 'offline';
    }),
    close: requestClose
  },
  offline: {
    connect: startConnect,
    reconnect: startReconnect,
    close: requestClose
  },
  reconnecting: {
    reconnectComplete: attemptEstablished,
    reconnectFailed: on((_ref7, event) => {
      let ctx = _ref7.ctx;
      ctx.lastError = event.error;
      return event.recoverable ? 'offline' : 'closed';
    }),
    close: requestClose
  },
  // Owns the transport until the close handshake settles. Every path into this state is followed
  // by a `closeComplete`, so it cannot become a trap.
  disconnecting: {
    closeComplete: 'closed'
  },
  // No transport, but the session may still be resumable: the engine resumes after an unexpected
  // close just as it does from `offline`.
  closed: {
    connect: startConnect,
    reconnect: startReconnect
  }
};
/**
 * Lifecycle model of the signal connection.
 *
 * The machine deliberately does not own connection attempts: `SignalClient` performs the
 * asynchronous work and reports the outcome. It also does not decide *whether* to reconnect —
 * that policy (backoff, resume vs. full reconnect, region failover, giving up) belongs to
 * `RTCEngine`, so transport loss lands in `offline` rather than starting a reconnect on its own.
 *
 * Each client gets its own instance: the context is mutable and per-connection.
 */
function createSignalMachine() {
  let initialState = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 'new';
  const context = {
    attemptId: 0
  };
  return createFsm({
    id: 'signal',
    initialState: initialState,
    context,
    states: signalStates
  });
}
export { AsyncQueue, BehavioralFsm, DataTrackHandle, DataTrackHandleAllocator, DataTrackHandleError, DataTrackHandleErrorReason, DataTrackInfo, Emitter, Fsm, MACHINA_TYPE, MAX_TRANSITION_DEPTH, NonSerializableValueError, QueueTaskStatus, _excluded, abortSignalAny, abortSignalTimeout, announceMachine, attemptEstablished, cloneDeep, cloneJsonSafe, cloneNode, cloneObject, coerceToDataView, createChildLink, createFsm, createSignalMachine, describeNonFiniteNumber, isCurrentAttempt, on, requestClose, signalStates, startConnect, startReconnect };
