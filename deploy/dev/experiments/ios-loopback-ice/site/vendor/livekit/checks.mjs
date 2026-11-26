// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 37107-37876 行按职责切出，行为不变。
// This file was generated from JSON Schema using quicktype, do not modify it directly.
// The code generation lives at https://github.com/livekit/attribute-definitions
//
// To parse this data:
//
//   import { Convert, AgentAttributes, TranscriptionAttributes } from "./file";
//
//   const agentAttributes = Convert.toAgentAttributes(json);
//   const transcriptionAttributes = Convert.toTranscriptionAttributes(json);
// Converts JSON strings to/from your types
import { __awaiter } from './tslib.mjs';
import { ServerInfo_Edition } from './protobuf-messages-chat.mjs';
import { livekitLogger } from './loglevel.mjs';
import { eventsExports } from './events.mjs';
import { RoomEvent } from './event-enums.mjs';
import { detectSilence } from './util.mjs';
import { isCloud, sleep } from './track.mjs';
import { SignalClient } from './signal-client.mjs';
import { RegionUrlProvider } from './region-streams.mjs';
import { createLocalAudioTrack, createLocalVideoTrack } from './publications.mjs';
import { ConnectionState, Room } from './room.mjs';

class Convert {
  static toAgentAttributes(json) {
    return JSON.parse(json);
  }
  static agentAttributesToJson(value) {
    return JSON.stringify(value);
  }
  static toTranscriptionAttributes(json) {
    return JSON.parse(json);
  }
  static transcriptionAttributesToJson(value) {
    return JSON.stringify(value);
  }
}var attributeTypings=/*#__PURE__*/Object.freeze({__proto__:null,Convert:Convert});var CheckStatus;
(function (CheckStatus) {
  CheckStatus[CheckStatus["IDLE"] = 0] = "IDLE";
  CheckStatus[CheckStatus["RUNNING"] = 1] = "RUNNING";
  CheckStatus[CheckStatus["SKIPPED"] = 2] = "SKIPPED";
  CheckStatus[CheckStatus["SUCCESS"] = 3] = "SUCCESS";
  CheckStatus[CheckStatus["FAILED"] = 4] = "FAILED";
})(CheckStatus || (CheckStatus = {}));
class Checker extends eventsExports.EventEmitter {
  constructor(url, token) {
    let options = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : {};
    super();
    this.status = CheckStatus.IDLE;
    this.logs = [];
    this.options = {};
    this.url = url;
    this.token = token;
    this.name = this.constructor.name;
    this.room = new Room(options.roomOptions);
    this.connectOptions = options.connectOptions;
    this.options = options;
  }
  run(onComplete) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.status !== CheckStatus.IDLE) {
        throw Error('check is running already');
      }
      this.setStatus(CheckStatus.RUNNING);
      try {
        yield this.perform();
      } catch (err) {
        if (err instanceof Error) {
          if (this.options.errorsAsWarnings) {
            this.appendWarning(err.message);
          } else {
            this.appendError(err.message);
          }
        }
      }
      yield this.disconnect();
      // sleep for a bit to ensure disconnect
      yield new Promise(resolve => setTimeout(resolve, 500));
      // @ts-ignore
      if (this.status !== CheckStatus.SKIPPED) {
        this.setStatus(this.isSuccess() ? CheckStatus.SUCCESS : CheckStatus.FAILED);
      }
      if (onComplete) {
        onComplete();
      }
      return this.getInfo();
    });
  }
  isSuccess() {
    return !this.logs.some(l => l.level === 'error');
  }
  connect(url) {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.room.state === ConnectionState.Connected) {
        return this.room;
      }
      if (!url) {
        url = this.url;
      }
      yield this.room.connect(url, this.token, this.connectOptions);
      return this.room;
    });
  }
  disconnect() {
    return __awaiter(this, void 0, void 0, function* () {
      if (this.room && this.room.state !== ConnectionState.Disconnected) {
        yield this.room.disconnect();
        // wait for it to go through
        yield new Promise(resolve => setTimeout(resolve, 500));
      }
    });
  }
  skip() {
    this.setStatus(CheckStatus.SKIPPED);
  }
  switchProtocol(protocol) {
    return __awaiter(this, void 0, void 0, function* () {
      let hasReconnecting = false;
      let hasReconnected = false;
      this.room.on(RoomEvent.Reconnecting, () => {
        hasReconnecting = true;
      });
      this.room.once(RoomEvent.Reconnected, () => {
        hasReconnected = true;
      });
      this.room.simulateScenario("force-".concat(protocol));
      yield new Promise(resolve => setTimeout(resolve, 1000));
      if (!hasReconnecting) {
        // no need to wait for reconnection
        return;
      }
      // wait for 10 seconds for reconnection
      const timeout = Date.now() + 10000;
      while (Date.now() < timeout) {
        if (hasReconnected) {
          return;
        }
        yield sleep(100);
      }
      throw new Error("Could not reconnect using ".concat(protocol, " protocol after 10 seconds"));
    });
  }
  appendMessage(message) {
    this.logs.push({
      level: 'info',
      message
    });
    this.emit('update', this.getInfo());
  }
  appendWarning(message) {
    this.logs.push({
      level: 'warning',
      message
    });
    this.emit('update', this.getInfo());
  }
  appendError(message) {
    this.logs.push({
      level: 'error',
      message
    });
    this.emit('update', this.getInfo());
  }
  setStatus(status) {
    this.status = status;
    this.emit('update', this.getInfo());
  }
  get engine() {
    var _a;
    return (_a = this.room) === null || _a === void 0 ? void 0 : _a.engine;
  }
  getInfo() {
    return {
      logs: this.logs,
      name: this.name,
      status: this.status,
      description: this.description
    };
  }
}/**
 * Checks for connections quality to closests Cloud regions and determining the best quality
 */
class CloudRegionCheck extends Checker {
  get description() {
    return 'Cloud regions';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      const regionProvider = new RegionUrlProvider(this.url, this.token);
      if (!regionProvider.isCloud()) {
        this.skip();
        return;
      }
      const regionStats = [];
      const seenUrls = new Set();
      for (let i = 0; i < 3; i++) {
        const regionUrl = yield regionProvider.getNextBestRegionUrl();
        if (!regionUrl) {
          break;
        }
        if (seenUrls.has(regionUrl)) {
          continue;
        }
        seenUrls.add(regionUrl);
        const stats = yield this.checkCloudRegion(regionUrl);
        this.appendMessage("".concat(stats.region, " RTT: ").concat(stats.rtt, "ms, duration: ").concat(stats.duration, "ms"));
        regionStats.push(stats);
      }
      regionStats.sort((a, b) => {
        return (a.duration - b.duration) * 0.5 + (a.rtt - b.rtt) * 0.5;
      });
      const bestRegion = regionStats[0];
      this.bestStats = bestRegion;
      this.appendMessage("best Cloud region: ".concat(bestRegion.region));
    });
  }
  getInfo() {
    const info = super.getInfo();
    info.data = this.bestStats;
    return info;
  }
  checkCloudRegion(url) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b;
      yield this.connect(url);
      if (this.options.protocol === 'tcp') {
        yield this.switchProtocol('tcp');
      }
      const region = (_a = this.room.serverInfo) === null || _a === void 0 ? void 0 : _a.region;
      if (!region) {
        throw new Error('Region not found');
      }
      const writer = yield this.room.localParticipant.streamText({
        topic: 'test'
      });
      const chunkSize = 1000; // each chunk is about 1000 bytes
      const totalSize = 1000000; // approximately 1MB of data
      const numChunks = totalSize / chunkSize; // will yield 1000 chunks
      const chunkData = 'A'.repeat(chunkSize); // create a string of 1000 'A' characters
      const startTime = Date.now();
      for (let i = 0; i < numChunks; i++) {
        yield writer.write(chunkData);
      }
      yield writer.close();
      const endTime = Date.now();
      const stats = yield (_b = this.room.engine.pcManager) === null || _b === void 0 ? void 0 : _b.publisher.getStats();
      const regionStats = {
        region: region,
        rtt: 10000,
        duration: endTime - startTime
      };
      stats === null || stats === void 0 ? void 0 : stats.forEach(stat => {
        if (stat.type === 'candidate-pair' && stat.nominated) {
          regionStats.rtt = stat.currentRoundTripTime * 1000;
        }
      });
      yield this.disconnect();
      return regionStats;
    });
  }
}const TEST_DURATION = 10000;
class ConnectionProtocolCheck extends Checker {
  get description() {
    return 'Connection via UDP vs TCP';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      const udpStats = yield this.checkConnectionProtocol('udp');
      const tcpStats = yield this.checkConnectionProtocol('tcp');
      this.bestStats = udpStats;
      // udp should is the better protocol typically. however, we'd prefer TCP when either of these conditions are true:
      // 1. the bandwidth limitation is worse on UDP by 500ms
      // 2. the packet loss is higher on UDP by 1%
      if (udpStats.qualityLimitationDurations.bandwidth - tcpStats.qualityLimitationDurations.bandwidth > 0.5 || (udpStats.packetsLost - tcpStats.packetsLost) / udpStats.packetsSent > 0.01) {
        this.appendMessage('best connection quality via tcp');
        this.bestStats = tcpStats;
      } else {
        this.appendMessage('best connection quality via udp');
      }
      const stats = this.bestStats;
      this.appendMessage("upstream bitrate: ".concat((stats.bitrateTotal / stats.count / 1000 / 1000).toFixed(2), " mbps"));
      this.appendMessage("RTT: ".concat((stats.rttTotal / stats.count * 1000).toFixed(2), " ms"));
      this.appendMessage("jitter: ".concat((stats.jitterTotal / stats.count * 1000).toFixed(2), " ms"));
      if (stats.packetsLost > 0) {
        this.appendWarning("packets lost: ".concat((stats.packetsLost / stats.packetsSent * 100).toFixed(2), "%"));
      }
      if (stats.qualityLimitationDurations.bandwidth > 1) {
        this.appendWarning("bandwidth limited ".concat((stats.qualityLimitationDurations.bandwidth / (TEST_DURATION / 1000) * 100).toFixed(2), "%"));
      }
      if (stats.qualityLimitationDurations.cpu > 0) {
        this.appendWarning("cpu limited ".concat((stats.qualityLimitationDurations.cpu / (TEST_DURATION / 1000) * 100).toFixed(2), "%"));
      }
    });
  }
  getInfo() {
    const info = super.getInfo();
    info.data = this.bestStats;
    return info;
  }
  checkConnectionProtocol(protocol) {
    return __awaiter(this, void 0, void 0, function* () {
      yield this.connect();
      if (protocol === 'tcp') {
        yield this.switchProtocol('tcp');
      } else {
        yield this.switchProtocol('udp');
      }
      // create a canvas with animated content
      const canvas = document.createElement('canvas');
      canvas.width = 1280;
      canvas.height = 720;
      const ctx = canvas.getContext('2d');
      if (!ctx) {
        throw new Error('Could not get canvas context');
      }
      let hue = 0;
      const animate = () => {
        hue = (hue + 1) % 360;
        ctx.fillStyle = "hsl(".concat(hue, ", 100%, 50%)");
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        requestAnimationFrame(animate);
      };
      animate();
      // create video track from canvas
      const stream = canvas.captureStream(30); // 30fps
      const videoTrack = stream.getVideoTracks()[0];
      // publish to room
      const pub = yield this.room.localParticipant.publishTrack(videoTrack, {
        simulcast: false,
        degradationPreference: 'maintain-resolution',
        videoEncoding: {
          maxBitrate: 2000000
        }
      });
      const track = pub.track;
      const protocolStats = {
        protocol,
        packetsLost: 0,
        packetsSent: 0,
        qualityLimitationDurations: {},
        rttTotal: 0,
        jitterTotal: 0,
        bitrateTotal: 0,
        count: 0
      };
      // gather stats once a second
      const interval = setInterval(() => __awaiter(this, void 0, void 0, function* () {
        const stats = yield track.getRTCStatsReport();
        stats === null || stats === void 0 ? void 0 : stats.forEach(stat => {
          if (stat.type === 'outbound-rtp') {
            protocolStats.packetsSent = stat.packetsSent;
            protocolStats.qualityLimitationDurations = stat.qualityLimitationDurations;
            protocolStats.bitrateTotal += stat.targetBitrate;
            protocolStats.count++;
          } else if (stat.type === 'remote-inbound-rtp') {
            protocolStats.packetsLost = stat.packetsLost;
            protocolStats.rttTotal += stat.roundTripTime;
            protocolStats.jitterTotal += stat.jitter;
          }
        });
      }), 1000);
      // wait a bit to gather stats
      yield new Promise(resolve => setTimeout(resolve, TEST_DURATION));
      clearInterval(interval);
      videoTrack.stop();
      canvas.remove();
      yield this.disconnect();
      return protocolStats;
    });
  }
}class PublishAudioCheck extends Checker {
  get description() {
    return 'Can publish audio';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const room = yield this.connect();
      const track = yield createLocalAudioTrack();
      const trackIsSilent = yield detectSilence(track, 1000);
      if (trackIsSilent) {
        throw new Error('unable to detect audio from microphone');
      }
      this.appendMessage('detected audio from microphone');
      room.localParticipant.publishTrack(track);
      // wait for a few seconds to publish
      yield new Promise(resolve => setTimeout(resolve, 3000));
      // verify RTC stats that it's publishing
      const stats = yield (_a = track.sender) === null || _a === void 0 ? void 0 : _a.getStats();
      if (!stats) {
        throw new Error('Could not get RTCStats');
      }
      let numPackets = 0;
      stats.forEach(stat => {
        if (stat.type === 'outbound-rtp' && (stat.kind === 'audio' || !stat.kind && stat.mediaType === 'audio')) {
          numPackets = stat.packetsSent;
        }
      });
      if (numPackets === 0) {
        throw new Error('Could not determine packets are sent');
      }
      this.appendMessage("published ".concat(numPackets, " audio packets"));
    });
  }
}class PublishVideoCheck extends Checker {
  get description() {
    return 'Can publish video';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const room = yield this.connect();
      const track = yield createLocalVideoTrack();
      // check if we have video from camera
      yield this.checkForVideo(track.mediaStreamTrack);
      room.localParticipant.publishTrack(track);
      // wait for a few seconds to publish
      yield new Promise(resolve => setTimeout(resolve, 5000));
      // verify RTC stats that it's publishing
      const stats = yield (_a = track.sender) === null || _a === void 0 ? void 0 : _a.getStats();
      if (!stats) {
        throw new Error('Could not get RTCStats');
      }
      let numPackets = 0;
      stats.forEach(stat => {
        if (stat.type === 'outbound-rtp' && (stat.kind === 'video' || !stat.kind && stat.mediaType === 'video')) {
          numPackets += stat.packetsSent;
        }
      });
      if (numPackets === 0) {
        throw new Error('Could not determine packets are sent');
      }
      this.appendMessage("published ".concat(numPackets, " video packets"));
    });
  }
  checkForVideo(track) {
    return __awaiter(this, void 0, void 0, function* () {
      const stream = new MediaStream();
      stream.addTrack(track.clone());
      // Create video element to check frames
      const video = document.createElement('video');
      video.srcObject = stream;
      video.muted = true;
      video.autoplay = true;
      video.playsInline = true;
      // For iOS Safari
      video.setAttribute('playsinline', 'true');
      document.body.appendChild(video);
      yield new Promise(resolve => {
        video.onplay = () => {
          setTimeout(() => {
            var _a, _b, _c, _d;
            const canvas = document.createElement('canvas');
            const settings = track.getSettings();
            const width = (_b = (_a = settings.width) !== null && _a !== void 0 ? _a : video.videoWidth) !== null && _b !== void 0 ? _b : 1280;
            const height = (_d = (_c = settings.height) !== null && _c !== void 0 ? _c : video.videoHeight) !== null && _d !== void 0 ? _d : 720;
            canvas.width = width;
            canvas.height = height;
            const ctx = canvas.getContext('2d');
            // Draw video frame to canvas
            ctx.drawImage(video, 0, 0);
            // Get image data and check if all pixels are black
            const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
            const data = imageData.data;
            let isAllBlack = true;
            for (let i = 0; i < data.length; i += 4) {
              if (data[i] !== 0 || data[i + 1] !== 0 || data[i + 2] !== 0) {
                isAllBlack = false;
                break;
              }
            }
            if (isAllBlack) {
              this.appendError('camera appears to be producing only black frames');
            } else {
              this.appendMessage('received video frames');
            }
            resolve();
          }, 1000);
        };
        video.play();
      });
      stream.getTracks().forEach(t => t.stop());
      video.remove();
    });
  }
}class ReconnectCheck extends Checker {
  get description() {
    return 'Resuming connection after interruption';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const room = yield this.connect();
      let reconnectingTriggered = false;
      let reconnected = false;
      let reconnectResolver;
      const reconnectTimeout = new Promise(resolve => {
        setTimeout(resolve, 5000);
        reconnectResolver = resolve;
      });
      const handleReconnecting = () => {
        reconnectingTriggered = true;
      };
      room.on(RoomEvent.SignalReconnecting, handleReconnecting).on(RoomEvent.Reconnecting, handleReconnecting).on(RoomEvent.Reconnected, () => {
        reconnected = true;
        reconnectResolver(true);
      });
      (_a = room.engine.client.ws) === null || _a === void 0 ? void 0 : _a.close();
      const onClose = room.engine.client.onClose;
      if (onClose) {
        onClose('');
      }
      yield reconnectTimeout;
      if (!reconnectingTriggered) {
        throw new Error('Did not attempt to reconnect');
      } else if (!reconnected || room.state !== ConnectionState.Connected) {
        this.appendWarning('reconnection is only possible in Redis-based configurations');
        throw new Error('Not able to reconnect');
      }
    });
  }
}class TURNCheck extends Checker {
  get description() {
    return 'Can connect via TURN';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      if (isCloud(new URL(this.url))) {
        this.appendMessage('Using region specific url');
        this.url = (_a = yield new RegionUrlProvider(this.url, this.token).getNextBestRegionUrl()) !== null && _a !== void 0 ? _a : this.url;
      }
      const signalClient = new SignalClient();
      const joinRes = yield signalClient.join(this.url, this.token, {
        autoSubscribe: true,
        maxRetries: 0,
        e2eeEnabled: false,
        websocketTimeout: 15000
      }, undefined, true);
      let hasTLS = false;
      let hasTURN = false;
      let hasSTUN = false;
      for (let iceServer of joinRes.iceServers) {
        for (let url of iceServer.urls) {
          if (url.startsWith('turn:')) {
            hasTURN = true;
            hasSTUN = true;
          } else if (url.startsWith('turns:')) {
            hasTURN = true;
            hasSTUN = true;
            hasTLS = true;
          }
          if (url.startsWith('stun:')) {
            hasSTUN = true;
          }
        }
      }
      if (!hasSTUN) {
        this.appendWarning('No STUN servers configured on server side.');
      } else if (hasTURN && !hasTLS) {
        this.appendWarning('TURN is configured server side, but TURN/TLS is unavailable.');
      }
      yield signalClient.close();
      if (((_c = (_b = this.connectOptions) === null || _b === void 0 ? void 0 : _b.rtcConfig) === null || _c === void 0 ? void 0 : _c.iceServers) || hasTURN) {
        yield this.room.connect(this.url, this.token, {
          rtcConfig: {
            iceTransportPolicy: 'relay'
          }
        });
      } else {
        this.appendWarning('No TURN servers configured.');
        this.skip();
        yield new Promise(resolve => setTimeout(resolve, 0));
      }
    });
  }
}class WebRTCCheck extends Checker {
  get description() {
    return 'Establishing WebRTC connection';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      let hasTcp = false;
      let hasIpv4Udp = false;
      this.room.on(RoomEvent.SignalConnected, () => {
        var _a;
        const prevTrickle = this.room.engine.client.onTrickle;
        this.room.engine.client.onTrickle = (sd, target) => {
          if (sd.candidate) {
            const candidate = new RTCIceCandidate(sd);
            let str = "".concat(candidate.protocol, " ").concat(candidate.address, ":").concat(candidate.port, " ").concat(candidate.type);
            if (candidate.address) {
              if (isIPPrivate(candidate.address)) {
                str += ' (private)';
              } else {
                if (candidate.protocol === 'tcp' && candidate.tcpType === 'passive') {
                  hasTcp = true;
                  str += ' (passive)';
                } else if (candidate.protocol === 'udp') {
                  hasIpv4Udp = true;
                }
              }
            }
            this.appendMessage(str);
          }
          if (prevTrickle) {
            prevTrickle(sd, target);
          }
        };
        if ((_a = this.room.engine.pcManager) === null || _a === void 0 ? void 0 : _a.subscriber) {
          this.room.engine.pcManager.subscriber.onIceCandidateError = ev => {
            if (ev instanceof RTCPeerConnectionIceErrorEvent) {
              this.appendWarning("error with ICE candidate: ".concat(ev.errorCode, " ").concat(ev.errorText, " ").concat(ev.url));
            }
          };
        }
      });
      try {
        yield this.connect();
        livekitLogger.info('now the room is connected');
      } catch (err) {
        this.appendWarning('ports need to be open on firewall in order to connect.');
        throw err;
      }
      if (!hasTcp) {
        this.appendWarning('Server is not configured for ICE/TCP');
      }
      if (!hasIpv4Udp) {
        this.appendWarning('No public IPv4 UDP candidates were found. Your server is likely not configured correctly');
      }
    });
  }
}
function isIPPrivate(address) {
  const parts = address.split('.');
  if (parts.length === 4) {
    if (parts[0] === '10') {
      return true;
    } else if (parts[0] === '192' && parts[1] === '168') {
      return true;
    } else if (parts[0] === '172') {
      const second = parseInt(parts[1], 10);
      if (second >= 16 && second <= 31) {
        return true;
      }
    }
  }
  return false;
}class WebSocketCheck extends Checker {
  get description() {
    return 'Connecting to signal connection via WebSocket';
  }
  perform() {
    return __awaiter(this, void 0, void 0, function* () {
      var _a, _b, _c;
      if (this.url.startsWith('ws:') || this.url.startsWith('http:')) {
        this.appendWarning('Server is insecure, clients may block connections to it');
      }
      let signalClient = new SignalClient();
      let joinRes;
      try {
        joinRes = yield signalClient.join(this.url, this.token, {
          autoSubscribe: true,
          maxRetries: 0,
          e2eeEnabled: false,
          websocketTimeout: 15000
        }, undefined, true);
      } catch (e) {
        if (isCloud(new URL(this.url))) {
          this.appendMessage("Initial connection failed with error ".concat(e.message, ". Retrying with region fallback"));
          const regionProvider = new RegionUrlProvider(this.url, this.token);
          const regionUrl = yield regionProvider.getNextBestRegionUrl();
          if (regionUrl) {
            joinRes = yield signalClient.join(regionUrl, this.token, {
              autoSubscribe: true,
              maxRetries: 0,
              e2eeEnabled: false,
              websocketTimeout: 15000
            }, undefined, true);
            this.appendMessage("Fallback to region worked. To avoid initial connections failing, ensure you're calling room.prepareConnection() ahead of time");
          }
        }
      }
      if (joinRes) {
        this.appendMessage("Connected to server, version ".concat(joinRes.serverVersion, "."));
        if (((_a = joinRes.serverInfo) === null || _a === void 0 ? void 0 : _a.edition) === ServerInfo_Edition.Cloud && ((_b = joinRes.serverInfo) === null || _b === void 0 ? void 0 : _b.region)) {
          this.appendMessage("LiveKit Cloud: ".concat((_c = joinRes.serverInfo) === null || _c === void 0 ? void 0 : _c.region));
        }
      } else {
        this.appendError("Websocket connection could not be established");
      }
      yield signalClient.close();
    });
  }
}class ConnectionCheck extends eventsExports.EventEmitter {
  constructor(url, token) {
    let options = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : {};
    super();
    this.options = {};
    this.checkResults = new Map();
    this.url = url;
    this.token = token;
    this.options = options;
  }
  getNextCheckId() {
    const nextId = this.checkResults.size;
    this.checkResults.set(nextId, {
      logs: [],
      status: CheckStatus.IDLE,
      name: '',
      description: ''
    });
    return nextId;
  }
  updateCheck(checkId, info) {
    this.checkResults.set(checkId, info);
    this.emit('checkUpdate', checkId, info);
  }
  isSuccess() {
    return Array.from(this.checkResults.values()).every(r => r.status !== CheckStatus.FAILED);
  }
  getResults() {
    return Array.from(this.checkResults.values());
  }
  createAndRunCheck(check) {
    return __awaiter(this, void 0, void 0, function* () {
      const checkId = this.getNextCheckId();
      const test = new check(this.url, this.token, this.options);
      const handleUpdate = info => {
        this.updateCheck(checkId, info);
      };
      test.on('update', handleUpdate);
      const result = yield test.run();
      test.off('update', handleUpdate);
      return result;
    });
  }
  checkWebsocket() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(WebSocketCheck);
    });
  }
  checkWebRTC() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(WebRTCCheck);
    });
  }
  checkTURN() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(TURNCheck);
    });
  }
  checkReconnect() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(ReconnectCheck);
    });
  }
  checkPublishAudio() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(PublishAudioCheck);
    });
  }
  checkPublishVideo() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(PublishVideoCheck);
    });
  }
  checkConnectionProtocol() {
    return __awaiter(this, void 0, void 0, function* () {
      const info = yield this.createAndRunCheck(ConnectionProtocolCheck);
      if (info.data && 'protocol' in info.data) {
        const stats = info.data;
        this.options.protocol = stats.protocol;
      }
      return info;
    });
  }
  checkCloudRegion() {
    return __awaiter(this, void 0, void 0, function* () {
      return this.createAndRunCheck(CloudRegionCheck);
    });
  }
}
export { CheckStatus, Checker, CloudRegionCheck, ConnectionCheck, ConnectionProtocolCheck, Convert, PublishAudioCheck, PublishVideoCheck, ReconnectCheck, TEST_DURATION, TURNCheck, WebRTCCheck, WebSocketCheck, attributeTypings, isIPPrivate };
