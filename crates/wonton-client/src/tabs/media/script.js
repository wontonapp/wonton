(() => {
  'use strict';
  if (window !== window.top || window.__wontonPageMedia) return;
  const supported = ['play', 'pause', 'stop', 'previoustrack', 'nexttrack',
    'seekbackward', 'seekforward', 'seekto'];
  const handlers = new Map();
  const tracked = new WeakSet();
  const covers = new Map();
  let active = null;
  let positionState = null;
  let previous = '';
  let previousContent = '';
  let positionChanged = true;
  let pending = false;
  let coverUrl = null;
  let artwork = null;
  let lastPlayback = 'none';

  // WebView 124 on API 35 has neither MediaSession nor MediaMetadata.
  if (typeof window.MediaMetadata !== 'function') {
    window.MediaMetadata = class MediaMetadata {
      constructor(init = {}) {
        this.title = init.title || '';
        this.artist = init.artist || '';
        this.album = init.album || '';
        this.artwork = init.artwork || [];
      }
    };
  }
  if (!navigator.mediaSession) {
    class MediaSession {
      constructor() {
        this._metadata = null;
        this._playbackState = 'none';
      }
      get metadata() { return this._metadata; }
      set metadata(value) { this._metadata = value; }
      get playbackState() { return this._playbackState; }
      set playbackState(value) { this._playbackState = value; }
      setActionHandler() {}
      setPositionState() {}
    }
    Object.defineProperty(navigator, 'mediaSession', { value: new MediaSession() });
  }
  const session = navigator.mediaSession;
  const prototype = Object.getPrototypeOf(session);

  function schedule(updatePosition = false) {
    positionChanged ||= updatePosition;
    if (pending) return;
    pending = true;
    queueMicrotask(() => {
      pending = false;
      publish();
    });
  }

  for (const name of ['metadata', 'playbackState']) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    if (!descriptor?.get || !descriptor?.set || !descriptor.configurable) continue;
    Object.defineProperty(prototype, name, {
      ...descriptor,
      set(value) {
        descriptor.set.call(this, value);
        if (this === session) {
          if (name === 'playbackState') anchorPosition();
          schedule(name === 'playbackState');
        }
      },
    });
  }
  const setActionHandler = prototype.setActionHandler;
  prototype.setActionHandler = function(action, handler) {
    const result = setActionHandler.call(this, action, handler);
    if (this === session) {
      if (typeof handler === 'function') handlers.set(action, handler);
      else handlers.delete(action);
      schedule();
    }
    return result;
  };
  const setPositionState = prototype.setPositionState;
  prototype.setPositionState = function(state) {
    const result = setPositionState.call(this, state);
    if (this === session) {
      positionState = state && Number.isFinite(state.duration) && state.duration > 0
        ? { duration: state.duration, position: state.position ?? 0,
          rate: state.playbackRate ?? 1, at: performance.now() } : null;
      schedule(true);
    }
    return result;
  };

  function anchorPosition() {
    const next = playback();
    if (positionState) {
      const now = performance.now();
      if (lastPlayback === 'playing') {
        positionState.position = Math.min(positionState.duration,
          positionState.position + (now - positionState.at) / 1000 * positionState.rate);
      }
      positionState.at = now;
    }
    lastPlayback = next;
  }

  function playback() {
    if (session.playbackState !== 'none') return session.playbackState;
    if (!active) return 'none';
    return !active.paused && !active.ended ? 'playing' : 'paused';
  }

  function position(playing) {
    if (positionState) {
      const state = positionState;
      const elapsed = playing ? (performance.now() - state.at) / 1000 * state.rate : 0;
      return { duration_ms: milliseconds(state.duration),
        position_ms: milliseconds(Math.min(state.duration, state.position + elapsed)), rate: state.rate };
    }
    return { duration_ms: milliseconds(active?.duration),
      position_ms: milliseconds(active?.currentTime), rate: active?.playbackRate || 1 };
  }

  function milliseconds(seconds) {
    return Number.isFinite(seconds) && seconds >= 0 ? Math.round(seconds * 1000) : null;
  }

  function selectedCover(metadata) {
    const choices = Array.from(metadata?.artwork || []);
    const distance = entry => {
      const sizes = String(entry.sizes || '').split(/\s+/).map(size =>
        Math.max(...size.split('x').map(Number))).filter(size => size > 0);
      return sizes.length ? Math.min(...sizes.map(size => Math.abs(size - 512))) : Infinity;
    };
    choices.sort((left, right) => distance(left) - distance(right));
    if (!choices.length) return null;
    try {
      const url = new URL(choices[0].src, document.baseURI);
      return url.origin === location.origin ? url.href : null;
    } catch (_) { return null; }
  }

  async function loadCover(url) {
    try {
      const response = await fetch(url, { credentials: 'same-origin', mode: 'same-origin' });
      if (!response.ok) return null;
      const bitmap = await createImageBitmap(await response.blob());
      try {
        const scale = Math.min(1, 512 / Math.max(bitmap.width, bitmap.height));
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(bitmap.width * scale));
        canvas.height = Math.max(1, Math.round(bitmap.height * scale));
        canvas.getContext('2d').drawImage(bitmap, 0, 0, canvas.width, canvas.height);
        return canvas.toDataURL('image/jpeg', 0.85).split(',')[1];
      } finally { bitmap.close(); }
    } catch (_) { return null; }
  }

  function updateCover(metadata) {
    const url = selectedCover(metadata);
    if (url === coverUrl) return;
    coverUrl = url;
    artwork = null;
    if (!url) return;
    if (!covers.has(url)) covers.set(url, loadCover(url));
    covers.get(url).then(value => {
      if (coverUrl !== url) return;
      artwork = value;
      schedule();
    });
  }

  function title(metadata) {
    const given = metadata?.title || '';
    if (given || !active) return { title: given, artist: metadata?.artist || '' };
    const parts = document.title.split(' - ');
    return { title: parts[0].trim(), artist: (parts[1] || '').trim() };
  }

  function publish() {
    const metadata = session.metadata;
    updateCover(metadata);
    const state = playback();
    const actions = supported.filter(action => handlers.has(action));
    if (active) {
      for (const action of ['play', 'pause']) {
        if (!actions.includes(action)) actions.push(action);
      }
      if (Number.isFinite(active.duration) && active.duration > 0 && !actions.includes('seekto')) {
        actions.push('seekto');
      }
    }
    const value = { version: 1, playback: state, ...title(metadata),
      album: metadata?.album || '',
      artwork, ...position(state === 'playing'), actions };
    const content = JSON.stringify({ ...value, position_ms: null });
    if (!positionChanged && content === previousContent) return;
    positionChanged = false;
    const message = JSON.stringify(value);
    if (message === previous) return;
    previous = message;
    previousContent = content;
    window.wontonMedia.postMessage(message);
  }

  const events = ['play', 'playing', 'pause', 'ended', 'loadedmetadata',
    'durationchange', 'ratechange', 'seeked'];
  function observe(event) {
    const media = event.target;
    if (!(media instanceof HTMLMediaElement)) return;
    if (!tracked.has(media)) {
      tracked.add(media);
      for (const name of events) media.addEventListener(name, observe);
    }
    if (event.type === 'play' || event.type === 'playing' || !active) active = media;
    if (media === active) {
      anchorPosition();
      schedule(true);
    }
  }
  for (const name of events) document.addEventListener(name, observe, true);
  const play = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function(...args) {
    observe({ target: this, type: 'play' });
    return play.apply(this, args);
  };

  function command(action, seconds) {
    const handler = handlers.get(action);
    if (handler) {
      const details = { action };
      if (action === 'seekto' && seconds !== null) details.seekTime = seconds;
      if (action === 'seekbackward' || action === 'seekforward') details.seekOffset = seconds ?? 10;
      handler.call(session, details);
    } else if (active) {
      if (action === 'play') active.play().catch(() => {});
      else if (action === 'pause') active.pause();
      else if (action === 'seekto' && seconds !== null) active.currentTime = seconds;
    }
  }
  Object.defineProperty(window, '__wontonPageMedia', { value: { command } });
  window.addEventListener('pagehide', () => {
    window.wontonMedia.postMessage(JSON.stringify({ version: 1, playback: 'none', title: '',
      artist: '', album: '', artwork: null, duration_ms: null, position_ms: null, rate: 1, actions: [] }));
  });
  schedule();
})();
