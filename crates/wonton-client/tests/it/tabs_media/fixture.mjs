import vm from 'node:vm';
import readline from 'node:readline';
import assert from 'node:assert/strict';

const lines = readline.createInterface({ input: process.stdin });
for await (const line of lines) {
  const { script, commands } = JSON.parse(line);
  for (const native of [false, true]) {
    const messages = [];
    let now = 1000;
    const calls = [];
    class Media extends EventTarget {
      paused = true;
      ended = false;
      currentTime = 12;
      duration = 60;
      playbackRate = 1;
      play() { this.paused = false; this.dispatchEvent(new Event('playing')); return Promise.resolve(); }
      pause() { this.paused = true; this.dispatchEvent(new Event('pause')); }
    }
    class Session {
      _metadata = null;
      _state = 'none';
      get metadata() { return this._metadata; }
      set metadata(value) { this._metadata = value; }
      get playbackState() { return this._state; }
      set playbackState(value) { this._state = value; }
      setActionHandler() {}
      setPositionState() {}
    }
    const context = vm.createContext({ navigator: native ? {mediaSession:new Session()} : {},
      document: Object.assign(new EventTarget(), {baseURI:'https://plugin.example/', title:'Navidrome'}),
      location:{origin:'https://plugin.example'}, HTMLMediaElement:Media, Audio:Media,
      performance:{now:()=>now}, queueMicrotask, setTimeout, URL, Map, WeakSet, Event,
      wontonMedia:{postMessage:message=>messages.push(JSON.parse(message))},
      addEventListener() {}, calls });
    context.window = context;
    context.top = context;
    vm.runInContext(script, context);
    await Promise.resolve();
    vm.runInContext(`globalThis.audio = new Audio();
      navigator.mediaSession.metadata = new MediaMetadata({title:'Track',artist:'Artist',album:'Album'});
      navigator.mediaSession.setActionHandler('nexttrack', details=>calls.push(details));
      navigator.mediaSession.setActionHandler('seekto', details=>calls.push(details));
      audio.play();`, context);
    await Promise.resolve();
    const playing = messages.at(-1);
    assert.equal(playing.playback, 'playing');
    assert.equal(playing.title, 'Track');
    assert.equal(playing.artist, 'Artist');
    assert.equal(playing.album, 'Album');
    assert.equal(playing.position_ms, 12000);
    assert.equal(playing.duration_ms, 60000);
    assert.deepEqual(new Set(playing.actions), new Set(['nexttrack','seekto','play','pause']));
    for (const command of commands) vm.runInContext(command, context);
    await Promise.resolve();
    assert.equal(context.audio.paused, true);
    assert.equal(calls[0].action, 'nexttrack');
    assert.equal(calls[1].action, 'seekto');
    assert.equal(calls[1].seekTime, 24.5);
    assert.equal(messages.at(-1).playback, 'paused');
    vm.runInContext(`navigator.mediaSession.setActionHandler('seekto', null);`, context);
    vm.runInContext(commands[1], context);
    assert.equal(context.audio.currentTime, 24.5);
    vm.runInContext(`audio.play(); navigator.mediaSession.setPositionState({duration:60,position:10,playbackRate:2});`, context);
    await Promise.resolve();
    now = 6000;
    vm.runInContext('audio.pause()', context);
    await Promise.resolve();
    assert.equal(messages.at(-1).position_ms, 20000);
    assert.equal(messages.at(-1).rate, 2);
    now = 16000;
    vm.runInContext('audio.play()', context);
    await Promise.resolve();
    assert.equal(messages.at(-1).position_ms, 20000);
    vm.runInContext(`navigator.mediaSession.playbackState = 'paused';`, context);
    await Promise.resolve();
    assert.equal(messages.at(-1).playback, 'paused');
    vm.runInContext(`navigator.mediaSession.playbackState = 'none'; navigator.mediaSession.setPositionState();`, context);
    await Promise.resolve();
    assert.equal(messages.at(-1).playback, 'playing');
    assert.equal(messages.at(-1).position_ms, 24500);
    context.document.title = 'Song Name - Artist Name - Navidrome';
    vm.runInContext(`navigator.mediaSession.metadata = new MediaMetadata({});`, context);
    await Promise.resolve();
    assert.equal(messages.at(-1).title, 'Song Name');
    assert.equal(messages.at(-1).artist, 'Artist Name');
    context.document.title = 'Only A Song';
    vm.runInContext(`navigator.mediaSession.metadata = new MediaMetadata({});`, context);
    await Promise.resolve();
    assert.equal(messages.at(-1).title, 'Only A Song');
    assert.equal(messages.at(-1).artist, '');
  }
  console.log(JSON.stringify({passed:true}));
}
