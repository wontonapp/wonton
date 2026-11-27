// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 22202-22901 行按职责切出，行为不变。
/**
 * Two-watermark flow control for one data channel kind.
 *
 * Owns the per-kind headroom gate: senders proceed freely while the buffer is at or below the
 * high-water mark and otherwise block — serialized FIFO through a mutex — until the browser's
 * `bufferedamountlow` event (armed at the low-water mark) signals the buffer has drained. The
 * serialization prevents woken senders from all refilling at once and overflowing the SCTP send
 * buffer (see livekit/client-sdk-js#1995).
 *
 * Waiters are parked on the channel object captured at wait entry. If that object stops being
 * current — replaced or torn down — its events may never fire again, so the owner must call
 * {@link invalidateWaiters}, which aborts parked waiters (releasing the gate); the next waiter
 * gets a fresh controller. {@link attach} and {@link detach} do this, and close the released
 * handle, as part of handle turnover.
 */
import { _ } from './protobuf-helpers.mjs';
import { __awaiter } from './tslib.mjs';
import { livekitLogger } from './loglevel.mjs';
import { TypedPromise } from './browser.mjs';
import { CriticalTimers, UnexpectedConnectionState } from './errors.mjs';

class FlowControlledDataChannel {
  constructor(opts) {
    /** Last emitted low-water status; starts true (an empty buffer is below the mark). */
    this.bufferStatusLow = true;
    this.headroomLock = new _();
    /** Cancels parked headroom waiters when the handle is replaced or torn down. */
    this.waiterAbortController = new AbortController();
    this.kind = opts.kind;
    this.lowWaterMark = opts.lowWaterMark;
    this.highWaterMark = opts.highWaterMark;
    this.isEngineClosed = opts.isEngineClosed;
    this.onBufferStatusChanged = opts.onBufferStatusChanged;
  }
  /** The currently attached RTCDataChannel handle, if any. */
  get channelHandle() {
    return this.handle;
  }
  /**
   * Attaches the channel handle this wrapper controls. Replacing an existing handle closes it and
   * rejects parked waiters — their events would never fire again on the abandoned object — and
   * installs a fresh controller. Wrappers outlive their handles: this is the one place handle
   * turnover happens, so a handle the wrapper lets go of is never left open, and stranding a
   * waiter is structurally impossible.
   */
  attach(dc) {
    const old = this.handle;
    this.handle = dc;
    if (old && old !== dc) {
      this.release(old, 'data channel replaced');
    }
  }
  /** Detaches and closes the handle on teardown, rejecting parked waiters. */
  detach() {
    let reason = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : 'data channel torn down';
    const old = this.handle;
    this.handle = undefined;
    if (old) {
      this.release(old, reason);
    }
  }
  /**
   * Closes a handle the wrapper no longer controls. `close()` moves it to `closing` synchronously,
   * so senders still holding it (queued on the headroom lock) fail the open check once they get
   * the lock. Parked waiters are aborted directly instead of relying on the `close` event, which
   * fires asynchronously, if at all (the spec lets `pc.close()` skip it). Strip any handlers that
   * must not observe the close before calling this.
   */
  release(dc, reason) {
    this.invalidateWaiters(reason);
    dc.close();
  }
  getChannel() {
    return this.handle;
  }
  /**
   * Whether the send buffer has room to accept more data (the send gate). Senders proceed while
   * this is true and block once it goes false. Callers resolve the handle (and decide what an
   * absent one means) before asking.
   */
  isBelowHighWaterMark(dc) {
    // RTCDataChannel has no built-in high-water mark, so we compare against our static mark.
    return dc.bufferedAmount <= this.highWaterMark;
  }
  /**
   * Whether the send buffer has drained to its low-water mark. Drives the engine's public
   * DCBufferStatusChanged event.
   */
  isBelowLowWaterMark(dc) {
    // Read the channel's own threshold: it is tuned dynamically for the lossy channel.
    return dc.bufferedAmount <= dc.bufferedAmountLowThreshold;
  }
  /**
   * Acquires the headroom lock, resolving with the unlock function. Batch senders (the resume
   * replay) hold it across all of their sends so no other sender can interleave, calling
   * {@link waitForHeadroomWithoutLock} per message to respect flow control within the batch.
   */
  lockHeadroom() {
    return this.headroomLock.lock();
  }
  /**
   * Resolves once the caller may send on `dc`: immediately while its send buffer is at or below
   * the high-water mark, otherwise once the buffer has drained to the low-water mark (the
   * `bufferedamountlow` event). Callers are serialized through the headroom lock so that, when
   * the buffer drains, they refill it one at a time (up to the high-water mark) rather than all
   * sending at once and overflowing the SCTP send buffer (see livekit/client-sdk-js#1995). The
   * checks run inside the lock so queued callers proceed in FIFO order.
   *
   * The caller passes the handle it resolved before queueing, and the wait is against that one
   * handle throughout: if it was replaced meanwhile, it has been closed, so the wait rejects
   * rather than letting the caller send on the replacement out of turn.
   */
  waitForHeadroomWithLock(dc) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.lockHeadroom();
      try {
        yield this.waitForHeadroomWithoutLock(dc);
      } finally {
        unlock();
      }
    });
  }
  /** Core wait of {@link waitForHeadroomWithLock}. The caller must hold the headroom lock. */
  waitForHeadroomWithoutLock(dc) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.isEngineClosed()) {
        throw new UnexpectedConnectionState('engine closed');
      }
      if (dc.readyState !== 'open') {
        throw new UnexpectedConnectionState("DataChannel ".concat(this.kind, " is ").concat(dc.readyState));
      }
      if (this.isBelowHighWaterMark(dc)) {
        return;
      }
      const abortSignal = this.waiterAbortController.signal;
      yield new TypedPromise((resolve, reject) => {
        const onBufferedAmountLow = () => {
          cleanup();
          resolve();
        };
        const onDCClose = () => {
          cleanup();
          reject(new UnexpectedConnectionState("DataChannel ".concat(this.kind, " closed while draining the buffer")));
        };
        const onAbort = () => {
          cleanup();
          reject(new UnexpectedConnectionState("DataChannel ".concat(this.kind, " was replaced or torn down while waiting for headroom")));
        };
        const cleanup = () => {
          dc.removeEventListener('bufferedamountlow', onBufferedAmountLow);
          dc.removeEventListener('close', onDCClose);
          abortSignal.removeEventListener('abort', onAbort);
        };
        if (abortSignal.aborted) {
          onAbort();
          return;
        }
        dc.addEventListener('bufferedamountlow', onBufferedAmountLow);
        // Proxy along any error caused by the data channel closing while we wait.
        dc.addEventListener('close', onDCClose);
        // The channel object we're parked on can be abandoned without ever firing another event
        // (e.g. the engine recreating channels); the abort is our way out.
        abortSignal.addEventListener('abort', onAbort);
      });
    });
  }
  /** Rejects all parked headroom waiters; the next waiter gets a fresh controller. */
  invalidateWaiters(reason) {
    this.waiterAbortController.abort(reason);
    this.waiterAbortController = new AbortController();
  }
  /**
   * Recomputes whether the buffer has drained to the low-water mark and, if that changed since the
   * last check, notifies the status listener. Two independent triggers land here: a send (which
   * raises the buffer) and the `bufferedamountlow` drain event (which lowers it) — the latter has
   * no send to hang the work off, which is why this is a shared entry point rather than a tail of
   * `send`.
   */
  refreshBufferStatus() {
    var _a;
    const dc = this.getChannel();
    if (!dc) {
      return;
    }
    const isLow = this.isBelowLowWaterMark(dc);
    if (isLow !== this.bufferStatusLow) {
      this.bufferStatusLow = isLow;
      (_a = this.onBufferStatusChanged) === null || _a === void 0 ? void 0 : _a.call(this, isLow);
    }
  }
}/**
 * A lossy channel: flow control plus a per-instance full-buffer policy.
 *
 * Each instance owns its own byterate stat, drop counter, and (when tuning is started) the
 * dynamic `bufferedAmountLowThreshold` adjustment that keeps the drop gate at roughly 100ms of
 * buffered latency. Keeping these per instance is what prevents one channel's traffic from
 * steering another channel's policy.
 */
class LossyDataChannel extends FlowControlledDataChannel {
  constructor(opts) {
    super(opts);
    this.statCurrentBytes = 0;
    this.statByterate = 0;
    this.dropCount = 0;
    this.bufferFullBehavior = opts.bufferFullBehavior;
    this.shouldSkipSends = opts.shouldSkipSends;
  }
  /** Sends prepared bytes with this channel's full-buffer policy (drop or wait). */
  send(msg) {
    return __awaiter(this, void 0, void 0, function* () {
      const dc = this.getChannel();
      if (!dc) {
        return;
      }
      // Depending on the channel's policy, either drop or wait for the buffer to drain below the
      // high-water mark before continuing.
      switch (this.bufferFullBehavior) {
        case 'wait':
          if (!this.isBelowHighWaterMark(dc)) {
            try {
              yield this.waitForHeadroomWithLock(dc);
            } catch (error) {
              if (this.isEngineClosed()) {
                throw error;
              }
              // The channel closed or was replaced while this send waited. The replacement may
              // still be connecting, so drop the packet, as sends during a reconnect are.
              return;
            }
          }
          break;
        case 'drop':
          // We check against the actual threshold on the DC here, as it is tuned dynamically.
          if (!this.isBelowLowWaterMark(dc)) {
            // Drop messages to reduce latency
            this.dropCount += 1;
            if (this.dropCount % 100 === 0) {
              livekitLogger.warn("dropping lossy data channel messages, total dropped: ".concat(this.dropCount));
            }
            return;
          }
      }
      this.statCurrentBytes += msg.byteLength;
      if (this.shouldSkipSends()) {
        return;
      }
      try {
        dc.send(msg);
        this.refreshBufferStatus();
      } catch (error) {
        // Preserve prior surface behaviour: a send that fails because the channel is closing is
        // logged, not thrown, so lossy/data-track sends don't reject during teardown windows.
        if (error instanceof TypeError) {
          livekitLogger.error(error);
        } else {
          throw error;
        }
      }
    });
  }
  /**
   * Starts the once-per-second adjustment of the channel's `bufferedAmountLowThreshold` to the
   * observed byterate, keeping the drop gate at roughly 100ms of buffered latency (clamped to
   * the watermarks). Restarts cleanly if already running.
   */
  startThresholdTuning() {
    this.stopThresholdTuning();
    this.statInterval = CriticalTimers.setInterval(() => {
      this.statByterate = this.statCurrentBytes;
      this.statCurrentBytes = 0;
      const dc = this.getChannel();
      if (dc) {
        // control buffered latency to ~100ms
        const threshold = this.statByterate / 10;
        dc.bufferedAmountLowThreshold = Math.min(Math.max(threshold, this.lowWaterMark), this.highWaterMark);
      }
    }, 1000);
  }
  /** Stops the threshold tuning and resets the stats and drop counter. */
  stopThresholdTuning() {
    this.statByterate = 0;
    this.statCurrentBytes = 0;
    if (this.statInterval) {
      CriticalTimers.clearInterval(this.statInterval);
      this.statInterval = undefined;
    }
    this.dropCount = 0;
  }
}class DataPacketBuffer {
  constructor() {
    this.buffer = [];
    this._totalSize = 0;
    this._sentSize = 0;
  }
  push(item) {
    this.buffer.push(item);
    this._totalSize += item.data.byteLength;
    if (item.sent) {
      this._sentSize += item.data.byteLength;
    }
  }
  pop() {
    const item = this.buffer.shift();
    if (item) {
      this._totalSize -= item.data.byteLength;
      if (item.sent) {
        this._sentSize -= item.data.byteLength;
      }
    }
    return item;
  }
  getAll() {
    return this.buffer.slice();
  }
  /** Every queued packet not yet handed to the channel, in sequence order. */
  getUnsent() {
    return this.buffer.filter(item => !item.sent);
  }
  /** Marks a single queued packet as handed to the channel. */
  markSent(item) {
    if (!item.sent) {
      item.sent = true;
      this._sentSize += item.data.byteLength;
    }
  }
  /**
   * Marks every queued packet as not-yet-sent. Used at the start of a resume replay: whatever is
   * still buffered was sent on the previous channel (or deferred) and must be re-handed to the
   * current one, so none of it counts as sent until the replay actually transmits it.
   */
  markAllUnsent() {
    for (const item of this.buffer) {
      item.sent = false;
    }
    this._sentSize = 0;
  }
  popToSequence(sequence) {
    while (this.buffer.length > 0) {
      const first = this.buffer[0];
      if (first.sequence <= sequence) {
        this.pop();
      } else {
        break;
      }
    }
  }
  alignBufferedAmount(bufferedAmount) {
    while (this.buffer.length > 0) {
      const first = this.buffer[0];
      // Unsent packets aren't part of the channel's bufferedAmount and are still awaiting the
      // resume replay — trimming them would silently lose them.
      if (!first.sent) {
        break;
      }
      if (this._sentSize - first.data.byteLength <= bufferedAmount) {
        break;
      }
      this.pop();
    }
  }
  get length() {
    return this.buffer.length;
  }
}/**
 * The reliable channel: flow control plus delivery-across-resume semantics.
 *
 * Every packet gets a monotonic sequence (stamped into the protobuf by the caller before
 * serialization, via {@link nextSequence}) and is retained in a replay buffer until the channel's
 * `bufferedAmount` confirms it has been handed to the transport. Sends that land in a reconnect
 * window — or whose headroom wait is torn down transiently — are queued unsent and resolve;
 * {@link replay} delivers them (plus any unacked packets) after a resume. Only an engine close
 * rejects, because no replay is coming after that.
 */
class ReliableDataChannel extends FlowControlledDataChannel {
  constructor(opts) {
    super(opts);
    this.messageBuffer = new DataPacketBuffer();
    this.sequence = 1;
    this.isDeferringSends = opts.isDeferringSends;
  }
  /**
   * Claims the next packet sequence. The caller stamps it into the packet before serialization,
   * then passes it back to {@link send} so the replay buffer stays keyed by wire sequence.
   */
  nextSequence() {
    const sequence = this.sequence;
    this.sequence += 1;
    return sequence;
  }
  /**
   * Sends prepared bytes with reliable semantics. Resolves once the packet has either been handed
   * to the channel or queued for the resume replay; throws only when the engine is closed.
   */
  send(msg, sequence) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.isDeferringSends()) {
        // A reconnect is already underway — queue for the resume replay instead of parking on a
        // channel that is being torn down. The send resolves; delivery is deferred to the replay.
        this.messageBuffer.push({
          data: msg,
          sequence,
          sent: false
        });
        return;
      }
      const dc = this.getChannel();
      if (!dc) {
        return;
      }
      try {
        yield this.waitForHeadroomWithLock(dc);
      } catch (error) {
        if (this.isEngineClosed()) {
          // No replay is coming after an engine close — surface the failure.
          throw error;
        }
        // Transient teardown (the channel closed or was replaced while we waited): the reliable
        // channel promises delivery across resume, so queue the packet for the replay instead of
        // rejecting a send the app can't meaningfully retry. This also keeps the order: a send
        // queued behind one aborted by a replacement fails the same way (the old handle is closed),
        // so it can't reach the replacement ahead of the lower sequence the replay will deliver.
        this.messageBuffer.push({
          data: msg,
          sequence,
          sent: false
        });
        return;
      }
      if (this.isDeferringSends()) {
        // A reconnect began while we waited for headroom — same deal as above.
        this.messageBuffer.push({
          data: msg,
          sequence,
          sent: false
        });
        return;
      }
      this.messageBuffer.push({
        data: msg,
        sequence,
        sent: true
      });
      dc.send(msg);
      this.refreshBufferStatus();
    });
  }
  /**
   * Replays the buffered backlog after a resume: drops everything the server acked
   * (`lastMessageSeq`), then re-sends the rest in order. The headroom lock is held across the
   * whole replay — releasing it between messages would let a concurrent send (whose newer
   * sequence was already assigned before it queued on the lock) hit the wire mid-replay, and
   * receivers would then discard the remaining lower-sequence resent messages as duplicates.
   */
  replay(lastMessageSeq) {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.getChannel()) {
        return;
      }
      this.messageBuffer.popToSequence(lastMessageSeq);
      const unlock = yield this.lockHeadroom();
      try {
        // Resolved only once the lock is held: the handle can be replaced while the replay waits
        // for it.
        const dc = this.getChannel();
        if (!dc) {
          throw new UnexpectedConnectionState("DataChannel not found, kind: ".concat(this.kind));
        }
        // Everything left after the ack cutoff must be re-handed to the current channel.
        this.messageBuffer.markAllUnsent();
        // Drain in passes, re-scanning the live buffer each time: a send that arrives (deferred,
        // sent:false) during our own awaits appends after this pass started, so we pick it up on
        // the next one. Mark each packet only once we've actually handed it to the channel — a
        // blanket "mark all sent" would flip such a late arrival to sent without transmitting it,
        // and a later alignBufferedAmount would then drop it for good. If the loop throws
        // mid-drain, unsent entries keep their flag and the next replay picks them up.
        for (let batch = this.messageBuffer.getUnsent(); batch.length > 0; batch = this.messageBuffer.getUnsent()) {
          for (const item of batch) {
            // Respect flow control on resume too, so a large resend doesn't overflow the buffer.
            // Against the handle resolved under the lock: if it is replaced mid-replay (another
            // resume), it has been closed, so this throws and that resume's replay picks up the
            // entries still flagged unsent.
            yield this.waitForHeadroomWithoutLock(dc);
            dc.send(item.data);
            this.messageBuffer.markSent(item);
          }
        }
      } finally {
        unlock();
      }
      this.refreshBufferStatus();
    });
  }
  /**
   * Before recomputing status, trim packets the transport has now delivered — a send or a drain
   * may have acked buffered packets, and the replay buffer is keyed off the channel's buffered
   * bytes.
   */
  refreshBufferStatus() {
    const dc = this.channelHandle;
    if (dc) {
      this.messageBuffer.alignBufferedAmount(dc.bufferedAmount);
    }
    super.refreshBufferStatus();
  }
  /**
   * Drops all replay state and restarts sequencing. Only valid on a full reconnect, where the
   * session (and the receivers' sequence tracking) starts over.
   */
  reset() {
    this.messageBuffer = new DataPacketBuffer();
    this.sequence = 1;
  }
}var DataChannelKind;
(function (DataChannelKind) {
  DataChannelKind[DataChannelKind["RELIABLE"] = 0] = "RELIABLE";
  DataChannelKind[DataChannelKind["LOSSY"] = 1] = "LOSSY";
  DataChannelKind[DataChannelKind["DATA_TRACK_LOSSY"] = 2] = "DATA_TRACK_LOSSY";
})(DataChannelKind || (DataChannelKind = {}));
// Two-watermark flow control for the reliable and data-track channels. Senders fill the buffer
// freely up to the high-water mark; once it's exceeded they block until the browser's
// `bufferedamountlow` event (which we arm at the low-water mark) signals the buffer has drained.
// The gap between the marks keeps the SCTP send buffer saturated while we refill, so throughput
// isn't starved, while the high-water mark bounds the buffer well below the level that would abort
// the channel (see livekit/client-sdk-js#1995).
const reliableDataChannelWaterMarkLow = 64 * 1024;
const reliableDataChannelWaterMarkHigh = 1024 * 1024;
const lossyDataChannelWaterMarkLow = 8 * 1024;
const lossyDataChannelWaterMarkHigh = 256 * 1024;
function dataChannelLowWaterMark(kind) {
  return kind === DataChannelKind.RELIABLE ? reliableDataChannelWaterMarkLow : lossyDataChannelWaterMarkLow;
}
function dataChannelHighWaterMark(kind) {
  return kind === DataChannelKind.RELIABLE ? reliableDataChannelWaterMarkHigh : lossyDataChannelWaterMarkHigh;
}const lossyDataChannelLabel = '_lossy';
const reliableDataChannelLabel = '_reliable';
const dataTrackDataChannelLabel = '_data_track';
/** Unhooks every handler the manager may have wired onto `dc`, ahead of closing it. */
function clearHandlers(dc) {
  dc.onbufferedamountlow = null;
  dc.onclose = null;
  dc.onclosing = null;
  dc.onerror = null;
  dc.onmessage = null;
  dc.onopen = null;
}
/**
 * Owns the engine's data channels: the three flow-controlled publisher channel wrappers (which
 * live for the engine's lifetime and have RTCDataChannel handles attached/detached as peer
 * connections come and go) plus the subscriber-side receive handles adopted by label.
 *
 * Handle turnover goes through {@link FlowControlledDataChannel.attach}/`detach`, which close the
 * released handle and reject parked headroom waiters as a built-in — there is no separate
 * invalidation step to forget.
 */
class DataChannelManager {
  constructor(opts) {
    this.opts = opts;
    const flowControlOptions = kind => ({
      kind,
      lowWaterMark: dataChannelLowWaterMark(kind),
      highWaterMark: dataChannelHighWaterMark(kind),
      isEngineClosed: opts.isEngineClosed,
      onBufferStatusChanged: isLow => opts.onBufferStatusChanged(kind, isLow)
    });
    this.reliable = new ReliableDataChannel(Object.assign(Object.assign({}, flowControlOptions(DataChannelKind.RELIABLE)), {
      isDeferringSends: opts.isReconnecting
    }));
    this.lossy = new LossyDataChannel(Object.assign(Object.assign({}, flowControlOptions(DataChannelKind.LOSSY)), {
      // Classic lossy user data: a stale packet is worthless, so drop instead of queueing.
      bufferFullBehavior: 'drop',
      shouldSkipSends: opts.isReconnecting
    }));
    this.dataTrack = new LossyDataChannel(Object.assign(Object.assign({}, flowControlOptions(DataChannelKind.DATA_TRACK_LOSSY)), {
      // Data tracks backpressure the producer instead — it decides what to skip at frame
      // granularity rather than the engine dropping arbitrary chunks out of frames.
      bufferFullBehavior: 'wait',
      shouldSkipSends: opts.isReconnecting
    }));
  }
  /** The flow-control wrapper for `kind`. */
  channelFor(kind) {
    switch (kind) {
      case DataChannelKind.RELIABLE:
        return this.reliable;
      case DataChannelKind.LOSSY:
        return this.lossy;
      case DataChannelKind.DATA_TRACK_LOSSY:
        return this.dataTrack;
    }
  }
  /** The raw RTCDataChannel handle for `kind`, publisher side by default. */
  getHandle(kind) {
    let subscriber = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
    if (!subscriber) {
      return this.channelFor(kind).channelHandle;
    }
    switch (kind) {
      case DataChannelKind.RELIABLE:
        return this.reliableSub;
      case DataChannelKind.LOSSY:
        return this.lossySub;
      case DataChannelKind.DATA_TRACK_LOSSY:
        return this.dataTrackSub;
    }
  }
  get hasPublisherChannels() {
    return Boolean(this.reliable.channelHandle || this.lossy.channelHandle || this.dataTrack.channelHandle);
  }
  /**
   * Creates the three publisher data channels on the given transport, wires their handlers, and
   * attaches them to the wrappers — attaching closes any replaced channel objects and rejects the
   * waiters still parked on them.
   */
  createPublisherChannels(pcManager) {
    // Recreating (the Safari null-id path): attaching the new handles below closes the old ones,
    // so unhook them first — the old channel's close must not report an unexpected close.
    for (const channel of [this.lossy, this.reliable, this.dataTrack]) {
      const old = channel.channelHandle;
      if (old) {
        clearHandlers(old);
      }
    }
    const wire = (channel, dc, onMessage) => {
      // also handle messages over the pub channel, for backwards compatibility
      dc.onmessage = onMessage;
      // handle datachannel errors
      dc.onerror = this.opts.onDataError;
      // detect unexpected publisher data channel closes
      dc.onclose = () => this.opts.onChannelClose(channel.kind);
      // set up dc buffer threshold - if this is not set, it will default to 0
      dc.bufferedAmountLowThreshold = channel.lowWaterMark;
      // handle buffer amount low events
      dc.onbufferedamountlow = () => channel.refreshBufferStatus();
      channel.attach(dc);
    };
    wire(this.lossy, pcManager.createPublisherDataChannel(lossyDataChannelLabel, {
      ordered: false,
      maxRetransmits: 0
    }), this.opts.onDataMessage);
    wire(this.reliable, pcManager.createPublisherDataChannel(reliableDataChannelLabel, {
      ordered: true
    }), this.opts.onDataMessage);
    wire(this.dataTrack, pcManager.createPublisherDataChannel(dataTrackDataChannelLabel, {
      ordered: false,
      maxRetransmits: 0
    }), this.opts.onDataTrackMessage);
    this.lossy.startThresholdTuning();
  }
  /**
   * Adopts a subscriber-side data channel by label, wiring the matching receive handler.
   * Returns false for labels this manager doesn't own.
   */
  adoptSubscriberChannel(channel) {
    let handler;
    if (channel.label === reliableDataChannelLabel) {
      this.reliableSub = channel;
      handler = this.opts.onDataMessage;
    } else if (channel.label === lossyDataChannelLabel) {
      this.lossySub = channel;
      handler = this.opts.onDataMessage;
    } else if (channel.label === dataTrackDataChannelLabel) {
      this.dataTrackSub = channel;
      handler = this.opts.onDataTrackMessage;
    } else {
      return false;
    }
    channel.onmessage = handler;
    return true;
  }
  /**
   * Tears down all channels for a peer-connection cleanup: strips handlers, closes the handles,
   * and resets the reliable session state. Detaching the publisher handles closes them and
   * rejects parked waiters (the spec allows `pc.close()` to transition channels to 'closed'
   * without firing events, so waiting for browser close events is not an option).
   */
  teardown() {
    // Detach the data channel handlers before closing anything. Closing a peer connection tears
    // down the SCTP transport, which can dispatch `error`/`close` events on the still-open data
    // channels; if our handlers are still attached at that point, handleDataError logs a spurious
    // "Unknown DataChannel error" during an otherwise graceful disconnect. Removing the handlers
    // before dc.close()/pcManager.close() makes this deterministic regardless of how/when the
    // browser dispatches those teardown events. See livekit/client-sdk-js#1953.
    const publisherDataChannels = [this.lossy, this.reliable, this.dataTrack];
    const subscriberDataChannels = [this.lossySub, this.reliableSub, this.dataTrackSub];
    // Publisher handles are closed by detach, which also rejects parked waiters.
    for (const channel of publisherDataChannels) {
      const dc = channel.channelHandle;
      if (dc) {
        clearHandlers(dc);
      }
      channel.detach('peer connections cleaned up');
    }
    // Subscriber handles have no wrapper, so close them directly.
    for (const dc of subscriberDataChannels) {
      if (dc) {
        clearHandlers(dc);
        dc.close();
      }
    }
    this.lossySub = undefined;
    this.reliableSub = undefined;
    this.dataTrackSub = undefined;
    // Full teardown starts the session over: drop replay state and restart sequencing.
    this.reliable.reset();
  }
}
export { DataChannelKind, DataChannelManager, DataPacketBuffer, FlowControlledDataChannel, LossyDataChannel, ReliableDataChannel, clearHandlers, dataChannelHighWaterMark, dataChannelLowWaterMark, dataTrackDataChannelLabel, lossyDataChannelLabel, lossyDataChannelWaterMarkHigh, lossyDataChannelWaterMarkLow, reliableDataChannelLabel, reliableDataChannelWaterMarkHigh, reliableDataChannelWaterMarkLow };
