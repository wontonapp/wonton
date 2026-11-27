// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 12217-12326 行按职责切出，行为不变。
var _a, _b;


class TypedPromise extends (_b = Promise) {
  // eslint-disable-next-line @typescript-eslint/no-useless-constructor
  constructor(executor) {
    super(executor);
  }
  catch(onrejected) {
    return super.catch(onrejected);
  }
  static reject(reason) {
    return super.reject(reason);
  }
  static all(values) {
    return super.all(values);
  }
  static race(values) {
    return super.race(values);
  }
}
_a = TypedPromise;
TypedPromise.resolve = value => {
  return Reflect.get(_b, "resolve", _a).call(_a, value);
};// tiny, simplified version of https://github.com/lancedikson/bowser/blob/master/src/parser-browsers.js
// reduced to only differentiate Chrome(ium) based browsers / Firefox / Safari
const commonVersionIdentifier = /version\/(\d+(\.?_?\d+)+)/i;
let browserDetails;
/**
 * @internal
 */
function getBrowser(userAgent) {
  let force = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : true;
  if (typeof userAgent === 'undefined' && typeof navigator === 'undefined') {
    return;
  }
  const ua = (userAgent !== null && userAgent !== void 0 ? userAgent : navigator.userAgent).toLowerCase();
  if (browserDetails === undefined || force) {
    const browser = browsersList.find(_ref => {
      let test = _ref.test;
      return test.test(ua);
    });
    browserDetails = browser === null || browser === void 0 ? void 0 : browser.describe(ua);
  }
  return browserDetails;
}
const browsersList = [{
  test: /firefox|iceweasel|fxios/i,
  describe(ua) {
    const browser = {
      name: 'Firefox',
      version: getMatch(/(?:firefox|iceweasel|fxios)[\s/](\d+(\.?_?\d+)+)/i, ua),
      os: ua.toLowerCase().includes('fxios') ? 'iOS' : undefined,
      osVersion: getOSVersion(ua)
    };
    return browser;
  }
}, {
  test: /chrom|crios|crmo/i,
  describe(ua) {
    const browser = {
      name: 'Chrome',
      version: getMatch(/(?:chrome|chromium|crios|crmo)\/(\d+(\.?_?\d+)+)/i, ua),
      os: ua.toLowerCase().includes('crios') ? 'iOS' : undefined,
      osVersion: getOSVersion(ua)
    };
    return browser;
  }
}, /* Safari */
{
  test: /safari|applewebkit/i,
  describe(ua) {
    const browser = {
      name: 'Safari',
      version: getMatch(commonVersionIdentifier, ua),
      os: ua.includes('mobile/') ? 'iOS' : 'macOS',
      osVersion: getOSVersion(ua)
    };
    return browser;
  }
}];
function getMatch(exp, ua) {
  let id = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : 1;
  const match = ua.match(exp);
  return match && match.length >= id && match[id] || '';
}
function getOSVersion(ua) {
  return ua.includes('mac os') ? getMatch(/\(.+?(\d+_\d+(:?_\d+)?)/, ua, 1).replace(/_/g, '.') : undefined;
}var version$1 = "2.23.0";const version = version$1;
const protocolVersion = 17;
/** Initial client protocol. */
const CLIENT_PROTOCOL_DEFAULT = 0;
/** Replaces RPC v1 protocol with a v2 data streams based one to support unlimited request /
 * response payload length. */
const CLIENT_PROTOCOL_DATA_STREAM_RPC = 1;
/** "Data streams v2": the client knows how to receive a single-packet data stream (a stream whose
 * entire payload is smuggled into the header packet, with no chunk/trailer packets). Senders only
 * use the single-packet optimization when the recipient advertises at least this protocol. */
const CLIENT_PROTOCOL_DATA_STREAM_V2 = 2;
/** The client protocol version indicates what level of support that the client has for
 * client <-> client api interactions. */
const clientProtocol = CLIENT_PROTOCOL_DATA_STREAM_V2;/** Base error that all LiveKit specific custom errors inherit from. */
class LivekitError extends Error {
  constructor(code, message, options) {
    super(message || 'an error has occurred');
    this.name = 'LiveKitError';
    this.code = code;
    if (typeof (options === null || options === void 0 ? void 0 : options.cause) !== 'undefined') {
      this.cause = options === null || options === void 0 ? void 0 : options.cause;
    }
  }
}
export { CLIENT_PROTOCOL_DATA_STREAM_RPC, CLIENT_PROTOCOL_DATA_STREAM_V2, CLIENT_PROTOCOL_DEFAULT, LivekitError, TypedPromise, _a, _b, browserDetails, browsersList, clientProtocol, commonVersionIdentifier, getBrowser, getMatch, getOSVersion, protocolVersion, version, version$1 };
