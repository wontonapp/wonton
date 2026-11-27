// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 19813-20917 行按职责切出，行为不变。
import { __awaiter, _slicedToArray } from './tslib.mjs';
import { SessionDescription } from './protobuf-messages-signal.mjs';
import { ConnectionSettings, JoinRequest, WrappedJoinRequest, WrappedJoinRequest_Compression } from './protobuf-messages-signal2.mjs';
import { isReactNative } from './track.mjs';
import { isCompressionStreamSupported } from './utils-misc.mjs';


function toProtoSessionDescription(rsd, id) {
  const sd = new SessionDescription({
    sdp: rsd.sdp,
    type: rsd.type,
    id
  });
  return sd;
}
function createConnectionParams(token, info, opts) {
  var _a;
  const params = new URLSearchParams();
  params.set('access_token', token);
  // opts
  if (opts.reconnect) {
    params.set('reconnect', '1');
    if (opts.sid) {
      params.set('sid', opts.sid);
    }
  }
  params.set('auto_subscribe', opts.autoSubscribe ? '1' : '0');
  // ClientInfo
  params.set('sdk', isReactNative() ? 'reactnative' : 'js');
  params.set('version', info.version);
  params.set('protocol', info.protocol.toString());
  params.set('client_protocol', info.clientProtocol.toString());
  if (info.deviceModel) {
    params.set('device_model', info.deviceModel);
  }
  if (info.os) {
    params.set('os', info.os);
  }
  if (info.osVersion) {
    params.set('os_version', info.osVersion);
  }
  if (info.browser) {
    params.set('browser', info.browser);
  }
  if (info.browserVersion) {
    params.set('browser_version', info.browserVersion);
  }
  if (opts.adaptiveStream) {
    params.set('adaptive_stream', '1');
  }
  if (opts.disableIceLite) {
    params.set('disable_ice_lite', '1');
  }
  if (typeof opts.subscriberAllowPause !== 'undefined') {
    params.set('subscriber_allow_pause', opts.subscriberAllowPause ? '1' : '0');
  }
  if (opts.reconnectReason) {
    params.set('reconnect_reason', opts.reconnectReason.toString());
  }
  // @ts-ignore
  if ((_a = navigator.connection) === null || _a === void 0 ? void 0 : _a.type) {
    // @ts-ignore
    params.set('network', navigator.connection.type);
  }
  return params;
}
function createJoinRequestConnectionParams(token, info, opts, publisherOffer) {
  return __awaiter(this, void 0, void 0, function* () {
    const params = new URLSearchParams();
    params.set('access_token', token);
    const joinRequest = new JoinRequest({
      clientInfo: info,
      connectionSettings: new ConnectionSettings({
        autoSubscribe: !!opts.autoSubscribe,
        adaptiveStream: !!opts.adaptiveStream,
        disableIceLite: !!opts.disableIceLite,
        subscriberAllowPause: opts.subscriberAllowPause
      }),
      reconnect: !!opts.reconnect,
      participantSid: opts.sid ? opts.sid : undefined,
      publisherOffer: publisherOffer
    });
    if (opts.reconnectReason) {
      joinRequest.reconnectReason = opts.reconnectReason;
    }
    const joinRequestBytes = joinRequest.toBinary();
    let requestBytes;
    let compression;
    if (isCompressionStreamSupported()) {
      const stream = new CompressionStream('gzip');
      const writer = stream.writable.getWriter();
      writer.write(new Uint8Array(joinRequestBytes));
      writer.close();
      const chunks = [];
      const reader = stream.readable.getReader();
      while (true) {
        const _yield$reader$read = yield reader.read(),
          done = _yield$reader$read.done,
          value = _yield$reader$read.value;
        if (done) break;
        chunks.push(value);
      }
      const totalLength = chunks.reduce((acc, chunk) => acc + chunk.length, 0);
      const result = new Uint8Array(totalLength);
      let offset = 0;
      for (const chunk of chunks) {
        result.set(chunk, offset);
        offset += chunk.length;
      }
      requestBytes = result;
      compression = WrappedJoinRequest_Compression.GZIP;
    } else {
      requestBytes = joinRequestBytes;
      compression = WrappedJoinRequest_Compression.NONE;
    }
    const wrappedJoinRequest = new WrappedJoinRequest({
      joinRequest: requestBytes,
      compression
    });
    const wrappedBytes = wrappedJoinRequest.toBinary();
    const bytesToBase64 = bytes => {
      const binString = Array.from(bytes, byte => String.fromCodePoint(byte)).join('');
      return btoa(binString);
    };
    params.set('join_request', bytesToBase64(wrappedBytes).replace(/\+/g, '-').replace(/\//g, '_'));
    return params;
  });
}class TTLMap {
  /**
   * @param ttl ttl of the key (ms)
   */
  constructor(ttl) {
    this._map = new Map();
    this._lastCleanup = 0;
    this.ttl = ttl;
  }
  set(key, value) {
    const now = Date.now();
    if (now - this._lastCleanup > this.ttl / 2) {
      this.cleanup();
    }
    const expiresAt = now + this.ttl;
    this._map.set(key, {
      value,
      expiresAt
    });
    return this;
  }
  get(key) {
    const entry = this._map.get(key);
    if (!entry) return undefined;
    if (entry.expiresAt < Date.now()) {
      this._map.delete(key);
      return undefined;
    }
    return entry.value;
  }
  has(key) {
    const entry = this._map.get(key);
    if (!entry) return false;
    if (entry.expiresAt < Date.now()) {
      this._map.delete(key);
      return false;
    }
    return true;
  }
  delete(key) {
    return this._map.delete(key);
  }
  clear() {
    this._map.clear();
  }
  cleanup() {
    const now = Date.now();
    for (const _ref of this._map.entries()) {
      var _ref2 = _slicedToArray(_ref, 2);
      const key = _ref2[0];
      const entry = _ref2[1];
      if (entry.expiresAt < now) {
        this._map.delete(key);
      }
    }
    this._lastCleanup = now;
  }
  get size() {
    this.cleanup();
    return this._map.size;
  }
  forEach(callback) {
    this.cleanup();
    for (const _ref3 of this._map.entries()) {
      var _ref4 = _slicedToArray(_ref3, 2);
      const key = _ref4[0];
      const entry = _ref4[1];
      if (entry.expiresAt >= Date.now()) {
        callback(entry.value, key, this.asValueMap());
      }
    }
  }
  map(callback) {
    this.cleanup();
    const result = [];
    const valueMap = this.asValueMap();
    for (const _ref5 of valueMap.entries()) {
      var _ref6 = _slicedToArray(_ref5, 2);
      const key = _ref6[0];
      const value = _ref6[1];
      result.push(callback(value, key, valueMap));
    }
    return result;
  }
  asValueMap() {
    const result = new Map();
    for (const _ref7 of this._map.entries()) {
      var _ref8 = _slicedToArray(_ref7, 2);
      const key = _ref8[0];
      const entry = _ref8[1];
      if (entry.expiresAt >= Date.now()) {
        result.set(key, entry.value);
      }
    }
    return result;
  }
}var lib = {};var parser = {};var grammar = {exports: {}};var hasRequiredGrammar;
function requireGrammar() {
  if (hasRequiredGrammar) return grammar.exports;
  hasRequiredGrammar = 1;
  var grammar$1 = grammar.exports = {
    v: [{
      name: 'version',
      reg: /^(\d*)$/
    }],
    o: [{
      // o=- 20518 0 IN IP4 203.0.113.1
      // NB: sessionId will be a String in most cases because it is huge
      name: 'origin',
      reg: /^(\S*) (\d*) (\d*) (\S*) IP(\d) (\S*)/,
      names: ['username', 'sessionId', 'sessionVersion', 'netType', 'ipVer', 'address'],
      format: '%s %s %d %s IP%d %s'
    }],
    // default parsing of these only (though some of these feel outdated)
    s: [{
      name: 'name'
    }],
    i: [{
      name: 'description'
    }],
    u: [{
      name: 'uri'
    }],
    e: [{
      name: 'email'
    }],
    p: [{
      name: 'phone'
    }],
    z: [{
      name: 'timezones'
    }],
    // TODO: this one can actually be parsed properly...
    r: [{
      name: 'repeats'
    }],
    // TODO: this one can also be parsed properly
    // k: [{}], // outdated thing ignored
    t: [{
      // t=0 0
      name: 'timing',
      reg: /^(\d*) (\d*)/,
      names: ['start', 'stop'],
      format: '%d %d'
    }],
    c: [{
      // c=IN IP4 10.47.197.26
      name: 'connection',
      reg: /^IN IP(\d) (\S*)/,
      names: ['version', 'ip'],
      format: 'IN IP%d %s'
    }],
    b: [{
      // b=AS:4000
      push: 'bandwidth',
      reg: /^(TIAS|AS|CT|RR|RS):(\d*)/,
      names: ['type', 'limit'],
      format: '%s:%s'
    }],
    m: [{
      // m=video 51744 RTP/AVP 126 97 98 34 31
      // NB: special - pushes to session
      // TODO: rtp/fmtp should be filtered by the payloads found here?
      reg: /^(\w*) (\d*) ([\w/]*)(?: (.*))?/,
      names: ['type', 'port', 'protocol', 'payloads'],
      format: '%s %d %s %s'
    }],
    a: [{
      // a=rtpmap:110 opus/48000/2
      push: 'rtp',
      reg: /^rtpmap:(\d*) ([\w\-.]*)(?:\s*\/(\d*)(?:\s*\/(\S*))?)?/,
      names: ['payload', 'codec', 'rate', 'encoding'],
      format: function (o) {
        return o.encoding ? 'rtpmap:%d %s/%s/%s' : o.rate ? 'rtpmap:%d %s/%s' : 'rtpmap:%d %s';
      }
    }, {
      // a=fmtp:108 profile-level-id=24;object=23;bitrate=64000
      // a=fmtp:111 minptime=10; useinbandfec=1
      push: 'fmtp',
      reg: /^fmtp:(\d*) ([\S| ]*)/,
      names: ['payload', 'config'],
      format: 'fmtp:%d %s'
    }, {
      // a=control:streamid=0
      name: 'control',
      reg: /^control:(.*)/,
      format: 'control:%s'
    }, {
      // a=rtcp:65179 IN IP4 193.84.77.194
      name: 'rtcp',
      reg: /^rtcp:(\d*)(?: (\S*) IP(\d) (\S*))?/,
      names: ['port', 'netType', 'ipVer', 'address'],
      format: function (o) {
        return o.address != null ? 'rtcp:%d %s IP%d %s' : 'rtcp:%d';
      }
    }, {
      // a=rtcp-fb:98 trr-int 100
      push: 'rtcpFbTrrInt',
      reg: /^rtcp-fb:(\*|\d*) trr-int (\d*)/,
      names: ['payload', 'value'],
      format: 'rtcp-fb:%s trr-int %d'
    }, {
      // a=rtcp-fb:98 nack rpsi
      push: 'rtcpFb',
      reg: /^rtcp-fb:(\*|\d*) ([\w-_]*)(?: ([\w-_]*))?/,
      names: ['payload', 'type', 'subtype'],
      format: function (o) {
        return o.subtype != null ? 'rtcp-fb:%s %s %s' : 'rtcp-fb:%s %s';
      }
    }, {
      // a=extmap:2 urn:ietf:params:rtp-hdrext:toffset
      // a=extmap:1/recvonly URI-gps-string
      // a=extmap:3 urn:ietf:params:rtp-hdrext:encrypt urn:ietf:params:rtp-hdrext:smpte-tc 25@600/24
      push: 'ext',
      reg: /^extmap:(\d+)(?:\/(\w+))?(?: (urn:ietf:params:rtp-hdrext:encrypt))? (\S*)(?: (\S*))?/,
      names: ['value', 'direction', 'encrypt-uri', 'uri', 'config'],
      format: function (o) {
        return 'extmap:%d' + (o.direction ? '/%s' : '%v') + (o['encrypt-uri'] ? ' %s' : '%v') + ' %s' + (o.config ? ' %s' : '');
      }
    }, {
      // a=extmap-allow-mixed
      name: 'extmapAllowMixed',
      reg: /^(extmap-allow-mixed)/
    }, {
      // a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR|2^20|1:32
      push: 'crypto',
      reg: /^crypto:(\d*) ([\w_]*) (\S*)(?: (\S*))?/,
      names: ['id', 'suite', 'config', 'sessionConfig'],
      format: function (o) {
        return o.sessionConfig != null ? 'crypto:%d %s %s %s' : 'crypto:%d %s %s';
      }
    }, {
      // a=setup:actpass
      name: 'setup',
      reg: /^setup:(\w*)/,
      format: 'setup:%s'
    }, {
      // a=connection:new
      name: 'connectionType',
      reg: /^connection:(new|existing)/,
      format: 'connection:%s'
    }, {
      // a=mid:1
      name: 'mid',
      reg: /^mid:([^\s]*)/,
      format: 'mid:%s'
    }, {
      // a=msid:0c8b064d-d807-43b4-b434-f92a889d8587 98178685-d409-46e0-8e16-7ef0db0db64a
      name: 'msid',
      reg: /^msid:(.*)/,
      format: 'msid:%s'
    }, {
      // a=ptime:20
      name: 'ptime',
      reg: /^ptime:(\d*(?:\.\d*)*)/,
      format: 'ptime:%d'
    }, {
      // a=maxptime:60
      name: 'maxptime',
      reg: /^maxptime:(\d*(?:\.\d*)*)/,
      format: 'maxptime:%d'
    }, {
      // a=sendrecv
      name: 'direction',
      reg: /^(sendrecv|recvonly|sendonly|inactive)/
    }, {
      // a=ice-lite
      name: 'icelite',
      reg: /^(ice-lite)/
    }, {
      // a=ice-ufrag:F7gI
      name: 'iceUfrag',
      reg: /^ice-ufrag:(\S*)/,
      format: 'ice-ufrag:%s'
    }, {
      // a=ice-pwd:x9cml/YzichV2+XlhiMu8g
      name: 'icePwd',
      reg: /^ice-pwd:(\S*)/,
      format: 'ice-pwd:%s'
    }, {
      // a=fingerprint:SHA-1 00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33
      name: 'fingerprint',
      reg: /^fingerprint:(\S*) (\S*)/,
      names: ['type', 'hash'],
      format: 'fingerprint:%s %s'
    }, {
      // a=candidate:0 1 UDP 2113667327 203.0.113.1 54400 typ host
      // a=candidate:1162875081 1 udp 2113937151 192.168.34.75 60017 typ host generation 0 network-id 3 network-cost 10
      // a=candidate:3289912957 2 udp 1845501695 193.84.77.194 60017 typ srflx raddr 192.168.34.75 rport 60017 generation 0 network-id 3 network-cost 10
      // a=candidate:229815620 1 tcp 1518280447 192.168.150.19 60017 typ host tcptype active generation 0 network-id 3 network-cost 10
      // a=candidate:3289912957 2 tcp 1845501695 193.84.77.194 60017 typ srflx raddr 192.168.34.75 rport 60017 tcptype passive generation 0 network-id 3 network-cost 10
      push: 'candidates',
      reg: /^candidate:(\S*) (\d*) (\S*) (\d*) (\S*) (\d*) typ (\S*)(?: raddr (\S*) rport (\d*))?(?: tcptype (\S*))?(?: generation (\d*))?(?: network-id (\d*))?(?: network-cost (\d*))?/,
      names: ['foundation', 'component', 'transport', 'priority', 'ip', 'port', 'type', 'raddr', 'rport', 'tcptype', 'generation', 'network-id', 'network-cost'],
      format: function (o) {
        var str = 'candidate:%s %d %s %d %s %d typ %s';
        str += o.raddr != null ? ' raddr %s rport %d' : '%v%v';

        // NB: candidate has three optional chunks, so %void middles one if it's missing
        str += o.tcptype != null ? ' tcptype %s' : '%v';
        if (o.generation != null) {
          str += ' generation %d';
        }
        str += o['network-id'] != null ? ' network-id %d' : '%v';
        str += o['network-cost'] != null ? ' network-cost %d' : '%v';
        return str;
      }
    }, {
      // a=end-of-candidates (keep after the candidates line for readability)
      name: 'endOfCandidates',
      reg: /^(end-of-candidates)/
    }, {
      // a=remote-candidates:1 203.0.113.1 54400 2 203.0.113.1 54401 ...
      name: 'remoteCandidates',
      reg: /^remote-candidates:(.*)/,
      format: 'remote-candidates:%s'
    }, {
      // a=ice-options:google-ice
      name: 'iceOptions',
      reg: /^ice-options:(\S*)/,
      format: 'ice-options:%s'
    }, {
      // a=ssrc:2566107569 cname:t9YU8M1UxTF8Y1A1
      push: 'ssrcs',
      reg: /^ssrc:(\d*) ([\w_-]*)(?::(.*))?/,
      names: ['id', 'attribute', 'value'],
      format: function (o) {
        var str = 'ssrc:%d';
        if (o.attribute != null) {
          str += ' %s';
          if (o.value != null) {
            str += ':%s';
          }
        }
        return str;
      }
    }, {
      // a=ssrc-group:FEC 1 2
      // a=ssrc-group:FEC-FR 3004364195 1080772241
      push: 'ssrcGroups',
      // token-char = %x21 / %x23-27 / %x2A-2B / %x2D-2E / %x30-39 / %x41-5A / %x5E-7E
      reg: /^ssrc-group:([\x21\x23\x24\x25\x26\x27\x2A\x2B\x2D\x2E\w]*) (.*)/,
      names: ['semantics', 'ssrcs'],
      format: 'ssrc-group:%s %s'
    }, {
      // a=msid-semantic: WMS Jvlam5X3SX1OP6pn20zWogvaKJz5Hjf9OnlV
      name: 'msidSemantic',
      reg: /^msid-semantic:\s?(\w*) (\S*)/,
      names: ['semantic', 'token'],
      format: 'msid-semantic: %s %s' // space after ':' is not accidental
    }, {
      // a=group:BUNDLE audio video
      push: 'groups',
      reg: /^group:(\w*) (.*)/,
      names: ['type', 'mids'],
      format: 'group:%s %s'
    }, {
      // a=rtcp-mux
      name: 'rtcpMux',
      reg: /^(rtcp-mux)/
    }, {
      // a=rtcp-rsize
      name: 'rtcpRsize',
      reg: /^(rtcp-rsize)/
    }, {
      // a=sctpmap:5000 webrtc-datachannel 1024
      name: 'sctpmap',
      reg: /^sctpmap:([\w_/]*) (\S*)(?: (\S*))?/,
      names: ['sctpmapNumber', 'app', 'maxMessageSize'],
      format: function (o) {
        return o.maxMessageSize != null ? 'sctpmap:%s %s %s' : 'sctpmap:%s %s';
      }
    }, {
      // a=x-google-flag:conference
      name: 'xGoogleFlag',
      reg: /^x-google-flag:([^\s]*)/,
      format: 'x-google-flag:%s'
    }, {
      // a=rid:1 send max-width=1280;max-height=720;max-fps=30;depend=0
      push: 'rids',
      reg: /^rid:([\d\w]+) (\w+)(?: ([\S| ]*))?/,
      names: ['id', 'direction', 'params'],
      format: function (o) {
        return o.params ? 'rid:%s %s %s' : 'rid:%s %s';
      }
    }, {
      // a=imageattr:97 send [x=800,y=640,sar=1.1,q=0.6] [x=480,y=320] recv [x=330,y=250]
      // a=imageattr:* send [x=800,y=640] recv *
      // a=imageattr:100 recv [x=320,y=240]
      push: 'imageattrs',
      reg: new RegExp(
      // a=imageattr:97
      '^imageattr:(\\d+|\\*)' +
      // send [x=800,y=640,sar=1.1,q=0.6] [x=480,y=320]
      '[\\s\\t]+(send|recv)[\\s\\t]+(\\*|\\[\\S+\\](?:[\\s\\t]+\\[\\S+\\])*)' +
      // recv [x=330,y=250]
      '(?:[\\s\\t]+(recv|send)[\\s\\t]+(\\*|\\[\\S+\\](?:[\\s\\t]+\\[\\S+\\])*))?'),
      names: ['pt', 'dir1', 'attrs1', 'dir2', 'attrs2'],
      format: function (o) {
        return 'imageattr:%s %s %s' + (o.dir2 ? ' %s %s' : '');
      }
    }, {
      // a=simulcast:send 1,2,3;~4,~5 recv 6;~7,~8
      // a=simulcast:recv 1;4,5 send 6;7
      name: 'simulcast',
      reg: new RegExp(
      // a=simulcast:
      '^simulcast:' +
      // send 1,2,3;~4,~5
      '(send|recv) ([a-zA-Z0-9\\-_~;,]+)' +
      // space + recv 6;~7,~8
      '(?:\\s?(send|recv) ([a-zA-Z0-9\\-_~;,]+))?' +
      // end
      '$'),
      names: ['dir1', 'list1', 'dir2', 'list2'],
      format: function (o) {
        return 'simulcast:%s %s' + (o.dir2 ? ' %s %s' : '');
      }
    }, {
      // old simulcast draft 03 (implemented by Firefox)
      //   https://tools.ietf.org/html/draft-ietf-mmusic-sdp-simulcast-03
      // a=simulcast: recv pt=97;98 send pt=97
      // a=simulcast: send rid=5;6;7 paused=6,7
      name: 'simulcast_03',
      reg: /^simulcast:[\s\t]+([\S+\s\t]+)$/,
      names: ['value'],
      format: 'simulcast: %s'
    }, {
      // a=framerate:25
      // a=framerate:29.97
      name: 'framerate',
      reg: /^framerate:(\d+(?:$|\.\d+))/,
      format: 'framerate:%s'
    }, {
      // RFC4570
      // a=source-filter: incl IN IP4 239.5.2.31 10.1.15.5
      name: 'sourceFilter',
      reg: /^source-filter: *(excl|incl) (\S*) (IP4|IP6|\*) (\S*) (.*)/,
      names: ['filterMode', 'netType', 'addressTypes', 'destAddress', 'srcList'],
      format: 'source-filter: %s %s %s %s %s'
    }, {
      // a=bundle-only
      name: 'bundleOnly',
      reg: /^(bundle-only)/
    }, {
      // a=label:1
      name: 'label',
      reg: /^label:(.+)/,
      format: 'label:%s'
    }, {
      // RFC version 26 for SCTP over DTLS
      // https://tools.ietf.org/html/draft-ietf-mmusic-sctp-sdp-26#section-5
      name: 'sctpPort',
      reg: /^sctp-port:(\d+)$/,
      format: 'sctp-port:%s'
    }, {
      // RFC version 26 for SCTP over DTLS
      // https://tools.ietf.org/html/draft-ietf-mmusic-sctp-sdp-26#section-6
      name: 'maxMessageSize',
      reg: /^max-message-size:(\d+)$/,
      format: 'max-message-size:%s'
    }, {
      // RFC7273
      // a=ts-refclk:ptp=IEEE1588-2008:39-A7-94-FF-FE-07-CB-D0:37
      push: 'tsRefClocks',
      reg: /^ts-refclk:([^\s=]*)(?:=(\S*))?/,
      names: ['clksrc', 'clksrcExt'],
      format: function (o) {
        return 'ts-refclk:%s' + (o.clksrcExt != null ? '=%s' : '');
      }
    }, {
      // RFC7273
      // a=mediaclk:direct=963214424
      name: 'mediaClk',
      reg: /^mediaclk:(?:id=(\S*))? *([^\s=]*)(?:=(\S*))?(?: *rate=(\d+)\/(\d+))?/,
      names: ['id', 'mediaClockName', 'mediaClockValue', 'rateNumerator', 'rateDenominator'],
      format: function (o) {
        var str = 'mediaclk:';
        str += o.id != null ? 'id=%s %s' : '%v%s';
        str += o.mediaClockValue != null ? '=%s' : '';
        str += o.rateNumerator != null ? ' rate=%s' : '';
        str += o.rateDenominator != null ? '/%s' : '';
        return str;
      }
    }, {
      // a=keywds:keywords
      name: 'keywords',
      reg: /^keywds:(.+)$/,
      format: 'keywds:%s'
    }, {
      // a=content:main
      name: 'content',
      reg: /^content:(.+)/,
      format: 'content:%s'
    },
    // BFCP https://tools.ietf.org/html/rfc4583
    {
      // a=floorctrl:c-s
      name: 'bfcpFloorCtrl',
      reg: /^floorctrl:(c-only|s-only|c-s)/,
      format: 'floorctrl:%s'
    }, {
      // a=confid:1
      name: 'bfcpConfId',
      reg: /^confid:(\d+)/,
      format: 'confid:%s'
    }, {
      // a=userid:1
      name: 'bfcpUserId',
      reg: /^userid:(\d+)/,
      format: 'userid:%s'
    }, {
      // a=floorid:1
      name: 'bfcpFloorId',
      reg: /^floorid:(.+) (?:m-stream|mstrm):(.+)/,
      names: ['id', 'mStream'],
      format: 'floorid:%s mstrm:%s'
    }, {
      // any a= that we don't understand is kept verbatim on media.invalid
      push: 'invalid',
      names: ['value']
    }]
  };

  // set sensible defaults to avoid polluting the grammar with boring details
  Object.keys(grammar$1).forEach(function (key) {
    var objs = grammar$1[key];
    objs.forEach(function (obj) {
      if (!obj.reg) {
        obj.reg = /(.*)/;
      }
      if (!obj.format) {
        obj.format = '%s';
      }
    });
  });
  return grammar.exports;
}var hasRequiredParser;
function requireParser() {
  if (hasRequiredParser) return parser;
  hasRequiredParser = 1;
  (function (exports) {
    var toIntIfInt = function (v) {
      return String(Number(v)) === v ? Number(v) : v;
    };
    var attachProperties = function (match, location, names, rawName) {
      if (rawName && !names) {
        location[rawName] = toIntIfInt(match[1]);
      } else {
        for (var i = 0; i < names.length; i += 1) {
          if (match[i + 1] != null) {
            location[names[i]] = toIntIfInt(match[i + 1]);
          }
        }
      }
    };
    var parseReg = function (obj, location, content) {
      var needsBlank = obj.name && obj.names;
      if (obj.push && !location[obj.push]) {
        location[obj.push] = [];
      } else if (needsBlank && !location[obj.name]) {
        location[obj.name] = {};
      }
      var keyLocation = obj.push ? {} :
      // blank object that will be pushed
      needsBlank ? location[obj.name] : location; // otherwise, named location or root

      attachProperties(content.match(obj.reg), keyLocation, obj.names, obj.name);
      if (obj.push) {
        location[obj.push].push(keyLocation);
      }
    };
    var grammar = requireGrammar();
    var validLine = RegExp.prototype.test.bind(/^([a-z])=(.*)/);
    exports.parse = function (sdp) {
      var session = {},
        media = [],
        location = session; // points at where properties go under (one of the above)

      // parse lines we understand
      sdp.split(/(\r\n|\r|\n)/).filter(validLine).forEach(function (l) {
        var type = l[0];
        var content = l.slice(2);
        if (type === 'm') {
          media.push({
            rtp: [],
            fmtp: []
          });
          location = media[media.length - 1]; // point at latest media line
        }
        for (var j = 0; j < (grammar[type] || []).length; j += 1) {
          var obj = grammar[type][j];
          if (obj.reg.test(content)) {
            return parseReg(obj, location, content);
          }
        }
      });
      session.media = media; // link it up
      return session;
    };
    var paramReducer = function (acc, expr) {
      var s = expr.split(/=(.+)/, 2);
      if (s.length === 2) {
        acc[s[0]] = toIntIfInt(s[1]);
      } else if (s.length === 1 && expr.length > 1) {
        acc[s[0]] = undefined;
      }
      return acc;
    };
    exports.parseParams = function (str) {
      return str.split(/;\s?/).reduce(paramReducer, {});
    };

    // For backward compatibility - alias will be removed in 3.0.0
    exports.parseFmtpConfig = exports.parseParams;
    exports.parsePayloads = function (str) {
      return str.toString().split(' ').map(Number);
    };
    exports.parseRemoteCandidates = function (str) {
      var candidates = [];
      var parts = str.split(' ').map(toIntIfInt);
      for (var i = 0; i < parts.length; i += 3) {
        candidates.push({
          component: parts[i],
          ip: parts[i + 1],
          port: parts[i + 2]
        });
      }
      return candidates;
    };
    exports.parseImageAttributes = function (str) {
      return str.split(' ').map(function (item) {
        return item.substring(1, item.length - 1).split(',').reduce(paramReducer, {});
      });
    };
    exports.parseSimulcastStreamList = function (str) {
      return str.split(';').map(function (stream) {
        return stream.split(',').map(function (format) {
          var scid,
            paused = false;
          if (format[0] !== '~') {
            scid = toIntIfInt(format);
          } else {
            scid = toIntIfInt(format.substring(1, format.length));
            paused = true;
          }
          return {
            scid: scid,
            paused: paused
          };
        });
      });
    };
  })(parser);
  return parser;
}var writer;
var hasRequiredWriter;
function requireWriter() {
  if (hasRequiredWriter) return writer;
  hasRequiredWriter = 1;
  var grammar = requireGrammar();

  // customized util.format - discards excess arguments and can void middle ones
  var formatRegExp = /%[sdv%]/g;
  var format = function (formatStr) {
    var i = 1;
    var args = arguments;
    var len = args.length;
    return formatStr.replace(formatRegExp, function (x) {
      if (i >= len) {
        return x; // missing argument
      }
      var arg = args[i];
      i += 1;
      switch (x) {
        case '%%':
          return '%';
        case '%s':
          return String(arg);
        case '%d':
          return Number(arg);
        case '%v':
          return '';
      }
    });
    // NB: we discard excess arguments - they are typically undefined from makeLine
  };
  var makeLine = function (type, obj, location) {
    var str = obj.format instanceof Function ? obj.format(obj.push ? location : location[obj.name]) : obj.format;
    var args = [type + '=' + str];
    if (obj.names) {
      for (var i = 0; i < obj.names.length; i += 1) {
        var n = obj.names[i];
        if (obj.name) {
          args.push(location[obj.name][n]);
        } else {
          // for mLine and push attributes
          args.push(location[obj.names[i]]);
        }
      }
    } else {
      args.push(location[obj.name]);
    }
    return format.apply(null, args);
  };

  // RFC specified order
  // TODO: extend this with all the rest
  var defaultOuterOrder = ['v', 'o', 's', 'i', 'u', 'e', 'p', 'c', 'b', 't', 'r', 'z', 'a'];
  var defaultInnerOrder = ['i', 'c', 'b', 'a'];
  writer = function (session, opts) {
    opts = opts || {};
    // ensure certain properties exist
    if (session.version == null) {
      session.version = 0; // 'v=0' must be there (only defined version atm)
    }
    if (session.name == null) {
      session.name = ' '; // 's= ' must be there if no meaningful name set
    }
    session.media.forEach(function (mLine) {
      if (mLine.payloads == null) {
        mLine.payloads = '';
      }
    });
    var outerOrder = opts.outerOrder || defaultOuterOrder;
    var innerOrder = opts.innerOrder || defaultInnerOrder;
    var sdp = [];

    // loop through outerOrder for matching properties on session
    outerOrder.forEach(function (type) {
      grammar[type].forEach(function (obj) {
        if (obj.name in session && session[obj.name] != null) {
          sdp.push(makeLine(type, obj, session));
        } else if (obj.push in session && session[obj.push] != null) {
          session[obj.push].forEach(function (el) {
            sdp.push(makeLine(type, obj, el));
          });
        }
      });
    });

    // then for each media line, follow the innerOrder
    session.media.forEach(function (mLine) {
      sdp.push(makeLine('m', grammar.m[0], mLine));
      innerOrder.forEach(function (type) {
        grammar[type].forEach(function (obj) {
          if (obj.name in mLine && mLine[obj.name] != null) {
            sdp.push(makeLine(type, obj, mLine));
          } else if (obj.push in mLine && mLine[obj.push] != null) {
            mLine[obj.push].forEach(function (el) {
              sdp.push(makeLine(type, obj, el));
            });
          }
        });
      });
    });
    return sdp.join('\r\n') + '\r\n';
  };
  return writer;
}var hasRequiredLib;
function requireLib() {
  if (hasRequiredLib) return lib;
  hasRequiredLib = 1;
  var parser = requireParser();
  var writer = requireWriter();
  var grammar = requireGrammar();
  lib.grammar = grammar;
  lib.write = writer;
  lib.parse = parser.parse;
  lib.parseParams = parser.parseParams;
  lib.parseFmtpConfig = parser.parseFmtpConfig; // Alias of parseParams().
  lib.parsePayloads = parser.parsePayloads;
  lib.parseRemoteCandidates = parser.parseRemoteCandidates;
  lib.parseImageAttributes = parser.parseImageAttributes;
  lib.parseSimulcastStreamList = parser.parseSimulcastStreamList;
  return lib;
}var libExports = requireLib();/*
 * Video codecs use a very low bitrate at the beginning and increase slowly by
 * the bandwidth estimator until they reach the target bitrate. The process commonly
 * costs more than 10 seconds causing subscribers to get blurry video at the first
 * few seconds. We use x-google-start-bitrate to hint the BWE to start higher.
 *
 * Why 90%: Gives ~10% headroom for bandwidth estimation while starting close to target.
 * Why same for all codecs: Target bitrate already accounts for codec efficiency
 * (e.g., users set lower targets for VP9/AV1 knowing they're more efficient).
 * Why cap at 1 Mbps: Prevents BWE from starting too aggressively on high bitrate tracks.
 * The cap is lowered further for connections that were slow to set up, see
 * `computeStartBitrateCap`, or that the browser reports as slow, see
 * `connectionEffectiveTypeStartBitrateCap`.
 */
const startBitrateMultiplier = 0.9;
/** Maximum x-google-start-bitrate in kbps. 1 Mbps prevents BWE from starting too aggressively. */
const maxStartBitrateKbps = 1000;
/**
 * Minimum x-google-start-bitrate in kbps: libwebrtc's own default starting estimate. A target
 * below this gets no hint, since seeding above the real capacity costs more than the ramp it
 * saves, and a slow connection is seeded no higher than this.
 */
const minTargetBitrateKbps = 300;
/**
 * Connection setup times (ms) bounding the ramp in `computeStartBitrateCap`: at or below the
 * first the hint is capped at `maxStartBitrateKbps`, at or above the second at
 * `minTargetBitrateKbps`. Measured on shaped links (CLT-3380): healthy links set up in under
 * 860 ms, a 1 Mbps link with 150 ms RTT took 1.1–1.7 s, a 500 kbps link ~2.3 s at the median,
 * and a 300 kbps link never under 3 s. The first sits above the 1 Mbps link, which a lower seed
 * only slows down, and the second above the 500 kbps link's median, so only links that cannot
 * carry more land at the floor.
 */
const setupTimeForMaxStartBitrateMs = 1500;
const setupTimeForMinStartBitrateMs = 3500;
/** Cap on the start bitrate hint, in kbps, when the browser reports a `3g` connection. */
const effectiveType3gStartBitrateKbps = 500;
const debounceInterval = 20;
/**
 * Codec payload for `codec` in this media section, when the section carries `cid`.
 *
 * Returns `undefined` when the section does not belong to the track, `0` when it does but
 * does not offer the requested codec, and the codec payload otherwise.
 *
 * @internal
 */
function findTrackCodecPayload(media, cid, codec) {
  var _a, _b, _c;
  if (!((_a = media.msid) === null || _a === void 0 ? void 0 : _a.includes(cid))) {
    return undefined;
  }
  return (_c = (_b = media.rtp.find(rtp => rtp.codec.toUpperCase() === codec.toUpperCase())) === null || _b === void 0 ? void 0 : _b.payload) !== null && _c !== void 0 ? _c : 0;
}
/**
 * Start bitrate hinted for a single track, or `undefined` when its target is too low to
 * be worth seeding.
 *
 * 90% of the target leaves ~10% headroom for the estimator to settle. The same multiplier
 * is used for every codec because the target already reflects the codec's efficiency.
 * Camera is capped at 1 Mbps so the estimator does not open too aggressively on a
 * high-bitrate track; screen share is exempt, because its content needs the bitrate
 * immediately to stay legible.
 *
 * The cap is lowered for a connection that was slow to set up (`computeStartBitrateCap`), and
 * for one the browser reports as slow (`connectionEffectiveTypeStartBitrateCap`), whichever is
 * lower. Screen share is exempt from both, however slow the connection.
 *
 * @internal
 */
function computeTrackStartBitrate(trackbr, connectionSetupTimeMs) {
  if (trackbr.maxbr < minTargetBitrateKbps) {
    return undefined;
  }
  const calculated = Math.min(Math.round(trackbr.maxbr * startBitrateMultiplier), trackbr.maxbr);
  if (trackbr.isScreenShare) {
    return calculated;
  }
  const cap = Math.min(computeStartBitrateCap(connectionSetupTimeMs), connectionEffectiveTypeStartBitrateCap());
  return Math.min(calculated, cap);
}
/**
 * Cap on the start bitrate hint, in kbps, from the browser's effective connection type
 * (`navigator.connection.effectiveType`, Chromium only): `slow-2g` and `2g` cap at
 * `minTargetBitrateKbps`, `3g` at `effectiveType3gStartBitrateKbps`.
 *
 * It catches links that have low bandwidth but low latency, which set up quickly and so slip
 * past the setup time ramp. `4g`, and browsers without the API, give no guidance and stay at the
 * `maxStartBitrateKbps` ceiling, as `computeStartBitrateCap` does without a setup time. The
 * connection RTT the API also reports is not used: it lags behind network changes, and the setup
 * time already reflects latency.
 */
function connectionEffectiveTypeStartBitrateCap() {
  // A `typeof` guard, since `globalThis` is missing from some supported browsers (Chrome < 71,
  // Firefox < 65, Safari < 12.1) and the build does not polyfill it.
  const connection = typeof navigator !== 'undefined' ? navigator.connection : undefined;
  switch (connection === null || connection === void 0 ? void 0 : connection.effectiveType) {
    case 'slow-2g':
    case '2g':
      return minTargetBitrateKbps;
    case '3g':
      return effectiveType3gStartBitrateKbps;
    default:
      return maxStartBitrateKbps;
  }
}
/**
 * Cap on the start bitrate hint, in kbps, from how long the connection took to set up.
 *
 * Connection setup time (signaling join plus ICE/DTLS) is the network signal every platform has
 * before the first video offer, since libwebrtc cannot probe the path until a video sender
 * exists. It grows with round-trip time and loss, which also mark the links where a 1 Mbps
 * seed overshoots, so the cap ramps linearly from `maxStartBitrateKbps` at
 * `setupTimeForMaxStartBitrateMs` down to `minTargetBitrateKbps` at
 * `setupTimeForMinStartBitrateMs`. A video offer built while the connection is still being set
 * up (publishing during connect) passes the time elapsed so far, a lower bound on the setup time.
 * Without any timing the cap stays at the 1 Mbps ceiling.
 */
function computeStartBitrateCap(connectionSetupTimeMs) {
  if (connectionSetupTimeMs === undefined) {
    return maxStartBitrateKbps;
  }
  const ramp = Math.min(Math.max((connectionSetupTimeMs - setupTimeForMaxStartBitrateMs) / (setupTimeForMinStartBitrateMs - setupTimeForMaxStartBitrateMs), 0), 1);
  return Math.round(maxStartBitrateKbps - ramp * (maxStartBitrateKbps - minTargetBitrateKbps));
}
/**
 * The single start bitrate for this peer connection: the largest hint among the video
 * m-sections of `media` that map to a published track.
 *
 * libwebrtc reads `x-google-start-bitrate` per m-section but applies it to the shared
 * `Call` (`WebRtcVideoSendChannel::ApplyChangedParams` -> `SetSdpBitrateParameters`), where
 * `RtpBitrateConfigurator` holds one config for the whole connection. Differing per-section
 * values are therefore last-writer-wins, decided by m-section order, so every video section
 * gets the same number instead.
 *
 * Only sections that can send local media are considered. `trackBitrates` is append-only and an
 * unpublished section keeps its `a=msid`, so matching on msid alone would still pair a stale
 * entry with its old section — letting an uncapped screen-share target seed a connection that
 * now carries only a camera, or consuming the one-shot hint on a section that sends nothing.
 *
 * The direction is the discriminator, as an exclusion rather than a match: `recvonly` and
 * `inactive` are the only directions that cannot carry local media, and they are exactly the
 * two a dead section lands on — unpublish sets `inactive` (explicitly in
 * `LocalParticipant.unpublishTrack`, and via `removeTrack`'s sendonly -> inactive transition for
 * the simulcast senders), `removeTrack` on a `sendrecv` transceiver leaves `recvonly`, and the
 * pre-populated placeholders are `recvonly`. Everything else sends: `sendonly` from the
 * `addTransceiver` path, `sendrecv` from the legacy `addTrack` fallback (which reuses a
 * transceiver rather than creating a sendonly one), and a section with no direction attribute,
 * which SDP defaults to `sendrecv`.
 *
 * @internal
 */
function computeConnectionStartBitrate(media, trackBitrates, connectionSetupTimeMs) {
  let connectionStartBitrate;
  for (const m of media) {
    if (m.type !== 'video' || m.direction === 'recvonly' || m.direction === 'inactive') {
      continue;
    }
    for (const trackbr of trackBitrates) {
      if (!trackbr.cid) {
        continue;
      }
      const codecPayload = findTrackCodecPayload(m, trackbr.cid, trackbr.codec);
      if (codecPayload === undefined) {
        continue;
      }
      const startBitrate = codecPayload > 0 ? computeTrackStartBitrate(trackbr, connectionSetupTimeMs) : undefined;
      if (startBitrate !== undefined && (connectionStartBitrate === undefined || startBitrate > connectionStartBitrate)) {
        connectionStartBitrate = startBitrate;
      }
      break;
    }
  }
  return connectionStartBitrate;
}
/**
 * Declares `x-google-start-bitrate` on `codecPayload`'s fmtp. This SDP munging is used for
 * a bitrate setting that cannot be applied through `RTCRtpEncodingParameters`.
 *
 * Returns whether the section now carries the hint.
 *
 * @internal
 */
function applyVideoStartBitrate(media, codecPayload, startBitrate) {
  const fmtp = media.fmtp.find(entry => entry.payload === codecPayload);
  if (fmtp) {
    // A payload type is shared across the bundle, so a value written for one section is
    // already the connection-level one; leave it rather than rewrite it.
    if (!fmtp.config.includes('x-google-start-bitrate')) {
      fmtp.config += ";x-google-start-bitrate=".concat(startBitrate);
    }
  } else {
    // VP8 and some codecs may not have an existing fmtp line.
    media.fmtp.push({
      payload: codecPayload,
      config: "x-google-start-bitrate=".concat(startBitrate)
    });
  }
  return true;
}
const PCEvents = {
  NegotiationStarted: 'negotiationStarted',
  NegotiationComplete: 'negotiationComplete',
  // Fired with the offerId for every successful publisher answer application,
  // including answers that immediately recurse into another offer via
  // `renegotiate`. Use this rather than NegotiationComplete to know that a
  // specific offer has been negotiated end-to-end.
  OfferAnswered: 'offerAnswered',
  RTPVideoPayloadTypes: 'rtpVideoPayloadTypes'
};
export { PCEvents, TTLMap, applyVideoStartBitrate, computeConnectionStartBitrate, computeStartBitrateCap, computeTrackStartBitrate, connectionEffectiveTypeStartBitrateCap, createConnectionParams, createJoinRequestConnectionParams, debounceInterval, effectiveType3gStartBitrateKbps, findTrackCodecPayload, grammar, hasRequiredGrammar, hasRequiredLib, hasRequiredParser, hasRequiredWriter, lib, libExports, maxStartBitrateKbps, minTargetBitrateKbps, parser, requireGrammar, requireLib, requireParser, requireWriter, setupTimeForMaxStartBitrateMs, setupTimeForMinStartBitrateMs, startBitrateMultiplier, toProtoSessionDescription, writer };
