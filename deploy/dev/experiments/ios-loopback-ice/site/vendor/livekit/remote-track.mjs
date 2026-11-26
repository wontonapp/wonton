// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 15681-16206 行按职责切出，行为不变。
import { __awaiter } from './tslib.mjs';
import { CriticalTimers } from './errors.mjs';
import { TrackEvent } from './event-enums.mjs';
import { supportsSynchronizationSources } from './util.mjs';
import { Track, attachToElement, detachTrack, isWeb } from './track.mjs';
import { getDevicePixelRatio, getIntersectionObserver, getResizeObserver } from './utils-misc.mjs';
import { computeBitrate, debounce, monitorFrequency } from './e2ee-frame.mjs';

class RemoteTrack extends Track {
  constructor(mediaTrack, sid, kind, receiver, loggerOptions) {
    super(mediaTrack, kind, loggerOptions);
    this.timeSyncLoop = () => {
      var _a;
      if (this.listenerCount(TrackEvent.TimeSyncUpdate) === 0) {
        // nobody is listening anymore, pause the loop until a new listener subscribes
        this.timeSyncHandle = undefined;
        return;
      }
      this.timeSyncHandle = requestAnimationFrame(this.timeSyncLoop);
      const sources = (_a = this.receiver) === null || _a === void 0 ? void 0 : _a.getSynchronizationSources()[0];
      if (sources) {
        const timestamp = sources.timestamp,
          rtpTimestamp = sources.rtpTimestamp;
        if (rtpTimestamp && this.rtpTimestamp !== rtpTimestamp) {
          this.emit(TrackEvent.TimeSyncUpdate, {
            timestamp,
            rtpTimestamp
          });
          this.rtpTimestamp = rtpTimestamp;
        }
      }
    };
    this.onTimeSyncListenerAdded = event => {
      if (event === TrackEvent.TimeSyncUpdate && this.timeSyncHandle === undefined) {
        // `newListener` fires before the listener is registered, so schedule the
        // next frame instead of entering the loop (which would see a count of 0)
        this.timeSyncHandle = requestAnimationFrame(this.timeSyncLoop);
      }
    };
    this.sid = sid;
    this.receiver = receiver;
  }
  get isLocal() {
    return false;
  }
  /** @internal */
  setMuted(muted) {
    if (this.isMuted !== muted) {
      this.isMuted = muted;
      this._mediaStreamTrack.enabled = !muted;
      this.emit(muted ? TrackEvent.Muted : TrackEvent.Unmuted, this);
    }
  }
  /** @internal */
  setMediaStream(stream) {
    // this is needed to determine when the track is finished
    this.mediaStream = stream;
    const onRemoveTrack = event => {
      if (event.track === this._mediaStreamTrack) {
        stream.removeEventListener('removetrack', onRemoveTrack);
        if (this.receiver && 'playoutDelayHint' in this.receiver) {
          this.receiver.playoutDelayHint = undefined;
        }
        this.receiver = undefined;
        this._currentBitrate = 0;
        this.emit(TrackEvent.Ended, this);
      }
    };
    stream.addEventListener('removetrack', onRemoveTrack);
  }
  start() {
    this.startMonitor();
    // use `enabled` of track to enable re-use of transceiver
    super.enable();
  }
  stop() {
    this.stopMonitor();
    // use `enabled` of track to enable re-use of transceiver
    super.disable();
  }
  /**
   * Gets the RTCStatsReport for the RemoteTrack's underlying RTCRtpReceiver
   * See https://developer.mozilla.org/en-US/docs/Web/API/RTCStatsReport
   *
   * @returns Promise<RTCStatsReport> | undefined
   */
  getRTCStatsReport() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      if (!((_a = this.receiver) === null || _a === void 0 ? void 0 : _a.getStats)) {
        return;
      }
      const statsReport = yield this.receiver.getStats();
      return statsReport;
    });
  }
  /**
   * Allows to set a playout delay (in seconds) for this track.
   * A higher value allows for more buffering of the track in the browser
   * and will result in a delay of media being played back of `delayInSeconds`
   */
  setPlayoutDelay(delayInSeconds) {
    if (this.receiver) {
      if ('playoutDelayHint' in this.receiver) {
        this.receiver.playoutDelayHint = delayInSeconds;
      } else {
        this.log.warn('Playout delay not supported in this browser');
      }
    } else {
      this.log.warn('Cannot set playout delay, track already ended');
    }
  }
  /**
   * Returns the current playout delay (in seconds) of this track.
   */
  getPlayoutDelay() {
    if (this.receiver) {
      if ('playoutDelayHint' in this.receiver) {
        return this.receiver.playoutDelayHint;
      } else {
        this.log.warn('Playout delay not supported in this browser');
      }
    } else {
      this.log.warn('Cannot get playout delay, track already ended');
    }
    return 0;
  }
  /* @internal */
  startMonitor() {
    if (!this.monitorInterval) {
      this.monitorInterval = setInterval(() => this.monitorReceiver(), monitorFrequency);
    }
    if (supportsSynchronizationSources()) {
      this.registerTimeSyncUpdate();
    }
  }
  /* @internal */
  stopMonitor() {
    super.stopMonitor();
    this.off('newListener', this.onTimeSyncListenerAdded);
  }
  registerTimeSyncUpdate() {
    // `newListener` isn't part of the typed event map, hence the cast
    const emitter = this;
    emitter.off('newListener', this.onTimeSyncListenerAdded);
    emitter.on('newListener', this.onTimeSyncListenerAdded);
    if (this.timeSyncHandle === undefined) {
      this.timeSyncLoop();
    }
  }
}const REACTION_DELAY = 100;
class RemoteVideoTrack extends RemoteTrack {
  constructor(mediaTrack, sid, receiver, adaptiveStreamSettings, loggerOptions) {
    super(mediaTrack, sid, Track.Kind.Video, receiver, loggerOptions);
    this.elementInfos = [];
    this.monitorReceiver = () => __awaiter(this, void 0, void 0, function* () {
      if (!this.receiver) {
        this._currentBitrate = 0;
        return;
      }
      const stats = yield this.getReceiverStats();
      if (stats && this.prevStats && this.receiver) {
        this._currentBitrate = computeBitrate(stats, this.prevStats);
      }
      this.prevStats = stats;
    });
    this.debouncedHandleResize = debounce(() => {
      this.updateDimensions();
    }, REACTION_DELAY);
    this.adaptiveStreamSettings = adaptiveStreamSettings;
  }
  get isAdaptiveStream() {
    return this.adaptiveStreamSettings !== undefined;
  }
  /**
   * Look up frame-level metadata for a given RTP timestamp.
   * Use with the `TrackEvent.TimeSyncUpdate` event to correlate displayed frames
   * with their capture-time metadata.
   *
   * Requires the room to be configured with the `frameMetadata` worker option
   * and the publishing track to have frame metadata features enabled.
   *
   */
  lookupFrameMetadata(_ref) {
    let rtpTimestamp = _ref.rtpTimestamp;
    var _a;
    return (_a = this.frameMetadataExtractor) === null || _a === void 0 ? void 0 : _a.lookupMetadata(rtpTimestamp);
  }
  setStreamState(value) {
    super.setStreamState(value);
    this.log.debug('setStreamState', value);
    if (this.isAdaptiveStream && value === Track.StreamState.Active) {
      // update visibility for adaptive stream tracks when stream state received from server is active
      // this is needed to ensure the track is stopped when there's no element attached to it at all
      this.updateVisibility();
    }
  }
  /**
   * Note: When using adaptiveStream, you need to use remoteVideoTrack.attach() to add the track to a HTMLVideoElement, otherwise your video tracks might never start
   */
  get mediaStreamTrack() {
    return this._mediaStreamTrack;
  }
  /** @internal */
  setMuted(muted) {
    super.setMuted(muted);
    this.attachedElements.forEach(element => {
      // detach or attach
      if (muted) {
        detachTrack(this._mediaStreamTrack, element);
      } else {
        attachToElement(this._mediaStreamTrack, element);
      }
    });
  }
  attach(element) {
    if (!element) {
      element = super.attach();
    } else {
      super.attach(element);
    }
    // It's possible attach is called multiple times on an element. When that's
    // the case, we'd want to avoid adding duplicate elementInfos
    if (this.adaptiveStreamSettings && this.elementInfos.find(info => info.element === element) === undefined) {
      const elementInfo = new HTMLElementInfo(element);
      this.observeElementInfo(elementInfo);
    }
    return element;
  }
  /**
   * Observe an ElementInfo for changes when adaptive streaming.
   * @param elementInfo
   * @internal
   */
  observeElementInfo(elementInfo) {
    if (this.adaptiveStreamSettings && this.elementInfos.find(info => info === elementInfo) === undefined) {
      elementInfo.handleResize = () => {
        this.debouncedHandleResize();
      };
      elementInfo.handleVisibilityChanged = () => {
        this.updateVisibility();
      };
      this.elementInfos.push(elementInfo);
      elementInfo.observe();
      // trigger the first resize update cycle
      // if the tab is backgrounded, the initial resize event does not fire until
      // the tab comes into focus for the first time.
      this.debouncedHandleResize();
      this.updateVisibility();
    } else {
      this.log.warn('visibility resize observer not triggered', this.logContext);
    }
  }
  /**
   * Stop observing an ElementInfo for changes.
   * @param elementInfo
   * @internal
   */
  stopObservingElementInfo(elementInfo) {
    if (!this.isAdaptiveStream) {
      this.log.warn('stopObservingElementInfo ignored', this.logContext);
      return;
    }
    const stopElementInfos = this.elementInfos.filter(info => info === elementInfo);
    for (const info of stopElementInfos) {
      info.stopObserving();
    }
    this.elementInfos = this.elementInfos.filter(info => info !== elementInfo);
    this.updateVisibility();
    this.debouncedHandleResize();
  }
  detach(element) {
    if (element) {
      this.stopObservingElement(element);
      return super.detach(element);
    }
    const detachedElements = super.detach();
    for (const e of detachedElements) {
      this.stopObservingElement(e);
    }
    return detachedElements;
  }
  /** @internal */
  getDecoderImplementation() {
    var _a;
    return (_a = this.prevStats) === null || _a === void 0 ? void 0 : _a.decoderImplementation;
  }
  getReceiverStats() {
    return __awaiter(this, void 0, void 0, function* () {
      if (!this.receiver || !this.receiver.getStats) {
        return;
      }
      const stats = yield this.receiver.getStats();
      let receiverStats;
      let codecID = '';
      let codecs = new Map();
      stats.forEach(v => {
        if (v.type === 'inbound-rtp') {
          codecID = v.codecId;
          receiverStats = {
            type: 'video',
            streamId: v.id,
            framesDecoded: v.framesDecoded,
            framesDropped: v.framesDropped,
            framesReceived: v.framesReceived,
            packetsReceived: v.packetsReceived,
            packetsLost: v.packetsLost,
            frameWidth: v.frameWidth,
            frameHeight: v.frameHeight,
            pliCount: v.pliCount,
            firCount: v.firCount,
            nackCount: v.nackCount,
            jitter: v.jitter,
            timestamp: v.timestamp,
            bytesReceived: v.bytesReceived,
            decoderImplementation: v.decoderImplementation
          };
        } else if (v.type === 'codec') {
          codecs.set(v.id, v);
        }
      });
      if (receiverStats && codecID !== '' && codecs.get(codecID)) {
        receiverStats.mimeType = codecs.get(codecID).mimeType;
      }
      return receiverStats;
    });
  }
  stopObservingElement(element) {
    const stopElementInfos = this.elementInfos.filter(info => info.element === element);
    for (const info of stopElementInfos) {
      this.stopObservingElementInfo(info);
    }
  }
  handleAppVisibilityChanged() {
    const _super = Object.create(null, {
      handleAppVisibilityChanged: {
        get: () => super.handleAppVisibilityChanged
      }
    });
    return __awaiter(this, void 0, void 0, function* () {
      yield _super.handleAppVisibilityChanged.call(this);
      if (!this.isAdaptiveStream) return;
      this.updateVisibility();
    });
  }
  updateVisibility(forceEmit) {
    var _a, _b;
    const lastVisibilityChange = this.elementInfos.reduce((prev, info) => Math.max(prev, info.visibilityChangedAt || 0), 0);
    const backgroundPause = ((_b = (_a = this.adaptiveStreamSettings) === null || _a === void 0 ? void 0 : _a.pauseVideoInBackground) !== null && _b !== void 0 ? _b : true // default to true
    ) ? this.isInBackground : false;
    const isPiPMode = this.elementInfos.some(info => info.pictureInPicture);
    const isVisible = this.elementInfos.some(info => info.visible) && !backgroundPause || isPiPMode;
    if (this.lastVisible === isVisible && !forceEmit) {
      return;
    }
    if (!isVisible && Date.now() - lastVisibilityChange < REACTION_DELAY) {
      // delay hidden events
      CriticalTimers.setTimeout(() => {
        this.updateVisibility();
      }, REACTION_DELAY);
      return;
    }
    this.lastVisible = isVisible;
    this.emit(TrackEvent.VisibilityChanged, isVisible, this);
  }
  updateDimensions() {
    var _a, _b;
    let maxWidth = 0;
    let maxHeight = 0;
    const pixelDensity = this.getPixelDensity();
    for (const info of this.elementInfos) {
      const currentElementWidth = info.width() * pixelDensity;
      const currentElementHeight = info.height() * pixelDensity;
      if (currentElementWidth + currentElementHeight > maxWidth + maxHeight) {
        maxWidth = currentElementWidth;
        maxHeight = currentElementHeight;
      }
    }
    if (((_a = this.lastDimensions) === null || _a === void 0 ? void 0 : _a.width) === maxWidth && ((_b = this.lastDimensions) === null || _b === void 0 ? void 0 : _b.height) === maxHeight) {
      return;
    }
    this.lastDimensions = {
      width: maxWidth,
      height: maxHeight
    };
    this.emit(TrackEvent.VideoDimensionsChanged, this.lastDimensions, this);
  }
  getPixelDensity() {
    var _a;
    const pixelDensity = (_a = this.adaptiveStreamSettings) === null || _a === void 0 ? void 0 : _a.pixelDensity;
    if (pixelDensity === 'screen') {
      return getDevicePixelRatio();
    } else if (!pixelDensity) {
      // when unset, we'll pick a sane default here.
      // for higher pixel density devices (mobile phones, etc), we'll use 2
      // otherwise it defaults to 1
      const devicePixelRatio = getDevicePixelRatio();
      if (devicePixelRatio > 2) {
        return 2;
      } else {
        return 1;
      }
    }
    return pixelDensity;
  }
}
class HTMLElementInfo {
  get visible() {
    return this.isPiP || this.isIntersecting;
  }
  get pictureInPicture() {
    return this.isPiP;
  }
  constructor(element, visible) {
    this.onVisibilityChanged = entry => {
      var _a;
      const target = entry.target,
        isIntersecting = entry.isIntersecting;
      if (target === this.element) {
        this.isIntersecting = isIntersecting;
        this.isPiP = isElementInPiP(this.element);
        this.visibilityChangedAt = Date.now();
        (_a = this.handleVisibilityChanged) === null || _a === void 0 ? void 0 : _a.call(this);
      }
    };
    this.onEnterPiP = () => {
      var _a, _b;
      (_b = (_a = window.documentPictureInPicture) === null || _a === void 0 ? void 0 : _a.window) === null || _b === void 0 ? void 0 : _b.addEventListener('pagehide', this.onLeavePiP);
      // Document PiP: the browser may fire 'enter' before the app has appended its subtree into
      // documentPictureInPicture.window. Defer so pipWin.document.contains(video) is reliable.
      queueMicrotask(() => {
        requestAnimationFrame(() => {
          var _a;
          this.isPiP = isElementInPiP(this.element);
          (_a = this.handleVisibilityChanged) === null || _a === void 0 ? void 0 : _a.call(this);
        });
      });
    };
    this.onLeavePiP = () => {
      var _a;
      this.isPiP = isElementInPiP(this.element);
      (_a = this.handleVisibilityChanged) === null || _a === void 0 ? void 0 : _a.call(this);
    };
    this.element = element;
    this.isIntersecting = visible !== null && visible !== void 0 ? visible : isElementInViewport(element);
    this.isPiP = isWeb() && isElementInPiP(element);
    this.visibilityChangedAt = 0;
  }
  width() {
    return this.element.clientWidth;
  }
  height() {
    return this.element.clientHeight;
  }
  observe() {
    var _a, _b, _c;
    // make sure we update the current visible state once we start to observe
    this.isIntersecting = isElementInViewport(this.element);
    this.isPiP = isElementInPiP(this.element);
    this.element.handleResize = () => {
      var _a;
      (_a = this.handleResize) === null || _a === void 0 ? void 0 : _a.call(this);
    };
    this.element.handleVisibilityChanged = this.onVisibilityChanged;
    getIntersectionObserver().observe(this.element);
    getResizeObserver().observe(this.element);
    this.element.addEventListener('enterpictureinpicture', this.onEnterPiP);
    this.element.addEventListener('leavepictureinpicture', this.onLeavePiP);
    (_a = window.documentPictureInPicture) === null || _a === void 0 ? void 0 : _a.addEventListener('enter', this.onEnterPiP);
    (_c = (_b = window.documentPictureInPicture) === null || _b === void 0 ? void 0 : _b.window) === null || _c === void 0 ? void 0 : _c.addEventListener('pagehide', this.onLeavePiP);
  }
  stopObserving() {
    var _a, _b, _c, _d, _e;
    (_a = getIntersectionObserver()) === null || _a === void 0 ? void 0 : _a.unobserve(this.element);
    (_b = getResizeObserver()) === null || _b === void 0 ? void 0 : _b.unobserve(this.element);
    this.element.removeEventListener('enterpictureinpicture', this.onEnterPiP);
    this.element.removeEventListener('leavepictureinpicture', this.onLeavePiP);
    (_c = window.documentPictureInPicture) === null || _c === void 0 ? void 0 : _c.removeEventListener('enter', this.onEnterPiP);
    (_e = (_d = window.documentPictureInPicture) === null || _d === void 0 ? void 0 : _d.window) === null || _e === void 0 ? void 0 : _e.removeEventListener('pagehide', this.onLeavePiP);
  }
}
function isElementInPiP(el) {
  var _a;
  // Simple video PiP
  if (document.pictureInPictureElement === el) return true;
  // Document PiP: the element has to actually live in the PiP window, otherwise an
  // opener-document element whose coordinates happen to fall inside the PiP window's
  // bounds is reported as being in PiP.
  const pipWindow = (_a = window.documentPictureInPicture) === null || _a === void 0 ? void 0 : _a.window;
  if (pipWindow) {
    return isDocumentInPiPWindow(el.ownerDocument, pipWindow) && isElementInViewport(el, pipWindow);
  }
  return false;
}
// Whether the document is the Document PiP window's own document, or the document of a frame
// nested inside it. The spec allows frames in a PiP window, so an element in one is in PiP:
// https://wicg.github.io/document-picture-in-picture/#iframes
function isDocumentInPiPWindow(doc, pipWindow) {
  var _a, _b, _c;
  // A cross-origin frame anywhere between the element and the PiP window hides frameElement
  // from the walk below, but top stays readable across origins and identifies the top-level
  // window the element belongs to. For an opener element that is the opener, not the PiP window.
  if (((_a = doc.defaultView) === null || _a === void 0 ? void 0 : _a.top) === pipWindow) return true;
  let current = doc;
  while (current) {
    if (current === pipWindow.document) return true;
    try {
      // Reading frameElement across a cross-origin boundary throws, which means we cannot
      // establish the ancestry and must not claim the element is in PiP.
      current = (_c = (_b = current.defaultView) === null || _b === void 0 ? void 0 : _b.frameElement) === null || _c === void 0 ? void 0 : _c.ownerDocument;
    } catch (_d) {
      return false;
    }
  }
  return false;
}
// does not account for occlusion by other elements or opacity property
function isElementInViewport(el, win) {
  const viewportWindow = win || window;
  let top = el.offsetTop;
  let left = el.offsetLeft;
  const width = el.offsetWidth;
  const height = el.offsetHeight;
  const _el = el,
    hidden = _el.hidden;
  const _getComputedStyle = getComputedStyle(el),
    display = _getComputedStyle.display;
  while (el.offsetParent) {
    el = el.offsetParent;
    top += el.offsetTop;
    left += el.offsetLeft;
  }
  return top < viewportWindow.pageYOffset + viewportWindow.innerHeight && left < viewportWindow.pageXOffset + viewportWindow.innerWidth && top + height > viewportWindow.pageYOffset && left + width > viewportWindow.pageXOffset && !hidden && display !== 'none';
}
export { HTMLElementInfo, REACTION_DELAY, RemoteTrack, RemoteVideoTrack, isDocumentInPiPWindow, isElementInPiP, isElementInViewport };
