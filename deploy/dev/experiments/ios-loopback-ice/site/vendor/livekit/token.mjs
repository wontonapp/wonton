// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 37876-38355 行按职责切出，行为不变。
/** A Fixed TokenSource is a token source that takes no parameters and returns a completely
 * independently derived value on each fetch() call.
 *
 * The most common downstream implementer is {@link TokenSourceLiteral}.
 */
import { _ } from './protobuf-helpers.mjs';
import { __awaiter, __rest, _defineProperty, _wrapAsyncGenerator } from './tslib.mjs';
import { RoomAgentDispatch, RoomConfiguration } from './protobuf-messages-stream.mjs';
import { TokenSourceRequest, TokenSourceResponse } from './protobuf-messages-signal2.mjs';

class TokenSourceFixed {}
/** A Configurable TokenSource is a token source that takes a
 * {@link TokenSourceFetchOptions} object as input and returns a deterministic
 * {@link TokenSourceResponseObject} output based on the options specified.
 *
 * For example, if options.participantName is set, it should be expected that
 * all tokens that are generated will have participant name field set to the
 * provided value.
 *
 * A few common downstream implementers are {@link TokenSourceEndpoint}
 * and {@link TokenSourceCustom}.
 */
class TokenSourceConfigurable {}new TextEncoder();
  const decoder = new TextDecoder(),
  strictDecoder = new TextDecoder("utf-8", {
    fatal: true
  });
function decodeBase64(encoded) {
  let url = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
  if (Uint8Array.fromBase64) return Uint8Array.fromBase64(encoded, {
    alphabet: url ? "base64url" : "base64"
  });
  if (url) {
    if (encoded.includes("+") || encoded.includes("/")) throw new TypeError("Invalid base64url");
    encoded = encoded.replace(/-/g, "+").replace(/_/g, "/");
  }
  const binary = atob(encoded),
    bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}class JOSEError extends Error {
  constructor(message, options) {
    var _Error$captureStackTr;
    super(message, options), _defineProperty(this, "code", "ERR_JOSE_GENERIC"), this.name = this.constructor.name, (_Error$captureStackTr = Error.captureStackTrace) === null || _Error$captureStackTr === void 0 ? void 0 : _Error$captureStackTr.call(Error, this, this.constructor);
  }
}
_defineProperty(JOSEError, "code", "ERR_JOSE_GENERIC");
class JWTClaimValidationFailed extends JOSEError {
  constructor(message, payload) {
    let claim = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : "unspecified";
    let reason = arguments.length > 3 && arguments[3] !== undefined ? arguments[3] : "unspecified";
    super(message, {
      cause: {
        claim,
        reason,
        payload
      }
    }), _defineProperty(this, "code", "ERR_JWT_CLAIM_VALIDATION_FAILED"), _defineProperty(this, "claim", void 0), _defineProperty(this, "reason", void 0), _defineProperty(this, "payload", void 0), this.claim = claim, this.reason = reason, this.payload = payload;
  }
}
_defineProperty(JWTClaimValidationFailed, "code", "ERR_JWT_CLAIM_VALIDATION_FAILED");
class JWTExpired extends JOSEError {
  constructor(message, payload) {
    let claim = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : "unspecified";
    let reason = arguments.length > 3 && arguments[3] !== undefined ? arguments[3] : "unspecified";
    super(message, {
      cause: {
        claim,
        reason,
        payload
      }
    }), _defineProperty(this, "code", "ERR_JWT_EXPIRED"), _defineProperty(this, "claim", void 0), _defineProperty(this, "reason", void 0), _defineProperty(this, "payload", void 0), this.claim = claim, this.reason = reason, this.payload = payload;
  }
}
_defineProperty(JWTExpired, "code", "ERR_JWT_EXPIRED");
class JOSEAlgNotAllowed extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JOSE_ALG_NOT_ALLOWED");
  }
}
_defineProperty(JOSEAlgNotAllowed, "code", "ERR_JOSE_ALG_NOT_ALLOWED");
class JOSENotSupported extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JOSE_NOT_SUPPORTED");
  }
}
_defineProperty(JOSENotSupported, "code", "ERR_JOSE_NOT_SUPPORTED");
class JWEDecryptionFailed extends JOSEError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : "decryption operation failed";
    let options = arguments.length > 1 ? arguments[1] : undefined;
    super(message, options);
    _defineProperty(this, "code", "ERR_JWE_DECRYPTION_FAILED");
  }
}
_defineProperty(JWEDecryptionFailed, "code", "ERR_JWE_DECRYPTION_FAILED");
class JWEInvalid extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JWE_INVALID");
  }
}
_defineProperty(JWEInvalid, "code", "ERR_JWE_INVALID");
class JWSInvalid extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JWS_INVALID");
  }
}
_defineProperty(JWSInvalid, "code", "ERR_JWS_INVALID");
class JWTInvalid extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JWT_INVALID");
  }
}
_defineProperty(JWTInvalid, "code", "ERR_JWT_INVALID");
class JWKInvalid extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JWK_INVALID");
  }
}
_defineProperty(JWKInvalid, "code", "ERR_JWK_INVALID");
class JWKSInvalid extends JOSEError {
  constructor() {
    super(...arguments);
    _defineProperty(this, "code", "ERR_JWKS_INVALID");
  }
}
_defineProperty(JWKSInvalid, "code", "ERR_JWKS_INVALID");
class JWKSNoMatchingKey extends JOSEError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : "no applicable key found in the JSON Web Key Set";
    let options = arguments.length > 1 ? arguments[1] : undefined;
    super(message, options);
    _defineProperty(this, "code", "ERR_JWKS_NO_MATCHING_KEY");
  }
}
_defineProperty(JWKSNoMatchingKey, "code", "ERR_JWKS_NO_MATCHING_KEY");
class JWKSMultipleMatchingKeys extends JOSEError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : "multiple matching keys found in the JSON Web Key Set";
    let options = arguments.length > 1 ? arguments[1] : undefined;
    super(message, options);
    _defineProperty(this, Symbol.asyncIterator, _wrapAsyncGenerator(function* () {}));
    _defineProperty(this, "code", "ERR_JWKS_MULTIPLE_MATCHING_KEYS");
  }
}
_defineProperty(JWKSMultipleMatchingKeys, "code", "ERR_JWKS_MULTIPLE_MATCHING_KEYS");
class JWKSTimeout extends JOSEError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : "request timed out";
    let options = arguments.length > 1 ? arguments[1] : undefined;
    super(message, options);
    _defineProperty(this, "code", "ERR_JWKS_TIMEOUT");
  }
}
_defineProperty(JWKSTimeout, "code", "ERR_JWKS_TIMEOUT");
class JWSSignatureVerificationFailed extends JOSEError {
  constructor() {
    let message = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : "signature verification failed";
    let options = arguments.length > 1 ? arguments[1] : undefined;
    super(message, options);
    _defineProperty(this, "code", "ERR_JWS_SIGNATURE_VERIFICATION_FAILED");
  }
}
_defineProperty(JWSSignatureVerificationFailed, "code", "ERR_JWS_SIGNATURE_VERIFICATION_FAILED");const invalid = "The input to be decoded is not correctly encoded.";
function decode(input) {
  try {
    return decodeBase64(typeof input == "string" ? input : decoder.decode(input), !0);
  } catch (cause) {
    throw new TypeError(invalid, {
      cause
    });
  }
}function isObject(input) {
  if (typeof input != "object" || input === null || Object.prototype.toString.call(input) !== "[object Object]") return false;
  const prototype = Object.getPrototypeOf(input);
  return prototype === null || Object.getPrototypeOf(prototype) === null;
}function decodeJwt(jwt) {
  if (typeof jwt != "string") throw new JWTInvalid("JWTs must use Compact JWS serialization, JWT must be a string");
  const _jwt$split = jwt.split("."),
    payload = _jwt$split[1],
    length = _jwt$split.length;
  if (length === 5) throw new JWTInvalid("Only JWTs using Compact JWS serialization can be decoded");
  if (length !== 3) throw new JWTInvalid("Invalid JWT");
  if (!payload) throw new JWTInvalid("JWTs must contain a payload");
  let decoded;
  try {
    decoded = decode(payload);
  } catch (_unused) {
    throw new JWTInvalid("Failed to base64url decode the payload");
  }
  let result;
  try {
    result = JSON.parse(strictDecoder.decode(decoded));
  } catch (_unused2) {
    throw new JWTInvalid("Failed to parse the decoded payload as JSON");
  }
  if (!isObject(result)) throw new JWTInvalid("Invalid JWT Claims Set");
  return result;
}const ONE_SECOND_IN_MILLISECONDS = 1000;
const ONE_MINUTE_IN_MILLISECONDS = 60 * ONE_SECOND_IN_MILLISECONDS;
function isResponseTokenValid(response) {
  const jwtPayload = decodeTokenPayload(response.participantToken);
  // Missing exp: TokenSourceCached would otherwise return this response forever.
  // nbf is optional (RFC 7519); do not skip the exp check when it is absent.
  if (!(jwtPayload === null || jwtPayload === void 0 ? void 0 : jwtPayload.exp)) {
    return false;
  }
  const now = new Date();
  if (jwtPayload.nbf) {
    const nbfInMilliseconds = jwtPayload.nbf * ONE_SECOND_IN_MILLISECONDS;
    const nbfDate = new Date(nbfInMilliseconds);
    if (nbfDate > now) {
      return false;
    }
  }
  const expInMilliseconds = jwtPayload.exp * ONE_SECOND_IN_MILLISECONDS;
  const expDate = new Date(expInMilliseconds - ONE_MINUTE_IN_MILLISECONDS);
  return expDate > now;
}
/** Given a LiveKit generated participant token, decodes and returns the associated {@link TokenPayload} data. */
function decodeTokenPayload(token) {
  const payload = decodeJwt(token);
  payload.roomConfig;
    const rest = __rest(payload, ["roomConfig"]);
  const mappedPayload = Object.assign(Object.assign({}, rest), {
    roomConfig: payload.roomConfig ? RoomConfiguration.fromJson(payload.roomConfig, {
      ignoreUnknownFields: true
    }) : undefined
  });
  return mappedPayload;
}
/** Given two TokenSourceFetchOptions values, check to see if they are deep equal. */
function areTokenSourceFetchOptionsEqual(a, b) {
  const allKeysSet = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const key of allKeysSet) {
    switch (key) {
      case 'roomName':
      case 'participantName':
      case 'participantIdentity':
      case 'participantMetadata':
      case 'participantAttributes':
      case 'agentName':
      case 'agentMetadata':
      case 'deployment':
        if (a[key] !== b[key]) {
          return false;
        }
        break;
      default:
        // ref: https://stackoverflow.com/a/58009992
        const exhaustiveCheckedKey = key;
        throw new Error("Options key ".concat(exhaustiveCheckedKey, " not being checked for equality!"));
    }
  }
  return true;
}/** A TokenSourceCached is a TokenSource which caches the last {@link TokenSourceResponseObject} value and returns it
 * until a) it expires or b) the {@link TokenSourceFetchOptions} provided to .fetch(...) change. */
class TokenSourceCached extends TokenSourceConfigurable {
  constructor() {
    super(...arguments);
    this.cachedFetchOptions = null;
    this.cachedResponse = null;
    this.fetchMutex = new _();
  }
  isSameAsCachedFetchOptions(options) {
    if (!this.cachedFetchOptions) {
      return false;
    }
    return areTokenSourceFetchOptionsEqual(options, this.cachedFetchOptions);
  }
  shouldReturnCachedValueFromFetch(fetchOptions) {
    if (!this.cachedResponse) {
      return false;
    }
    if (!isResponseTokenValid(this.cachedResponse)) {
      return false;
    }
    if (!this.isSameAsCachedFetchOptions(fetchOptions)) {
      return false;
    }
    return true;
  }
  getCachedResponseJwtPayload() {
    if (!this.cachedResponse) {
      return null;
    }
    return decodeTokenPayload(this.cachedResponse.participantToken);
  }
  fetch(options, force) {
    return __awaiter(this, void 0, void 0, function* () {
      const unlock = yield this.fetchMutex.lock();
      try {
        if (force) {
          this.cachedResponse = null;
        }
        if (this.shouldReturnCachedValueFromFetch(options)) {
          return this.cachedResponse.toJson();
        }
        this.cachedFetchOptions = options;
        const tokenResponse = yield this.update(options);
        this.cachedResponse = tokenResponse;
        return tokenResponse.toJson();
      } finally {
        unlock();
      }
    });
  }
}
class TokenSourceLiteral extends TokenSourceFixed {
  constructor(literalOrFn) {
    super();
    this.literalOrFn = literalOrFn;
  }
  fetch() {
    return __awaiter(this, void 0, void 0, function* () {
      if (typeof this.literalOrFn === 'function') {
        return this.literalOrFn();
      } else {
        return this.literalOrFn;
      }
    });
  }
}
class TokenSourceCustom extends TokenSourceCached {
  constructor(customFn) {
    super();
    this.customFn = customFn;
  }
  update(options) {
    return __awaiter(this, void 0, void 0, function* () {
      const resultMaybePromise = this.customFn(options);
      let result;
      if (resultMaybePromise instanceof Promise) {
        result = yield resultMaybePromise;
      } else {
        result = resultMaybePromise;
      }
      return TokenSourceResponse.fromJson(result, {
        // NOTE: it could be possible that the response body could contain more fields than just
        // what's in TokenSourceResponse depending on the implementation
        ignoreUnknownFields: true
      });
    });
  }
}
class TokenSourceEndpoint extends TokenSourceCached {
  constructor(url) {
    let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    super();
    this.url = url;
    this.endpointOptions = options;
  }
  createRequestFromOptions(options) {
    var _a, _b, _c, _d;
    const request = new TokenSourceRequest();
    for (const key of Object.keys(options)) {
      switch (key) {
        case 'roomName':
        case 'participantName':
        case 'participantIdentity':
        case 'participantMetadata':
          request[key] = options[key];
          break;
        case 'participantAttributes':
          request.participantAttributes = (_a = options.participantAttributes) !== null && _a !== void 0 ? _a : {};
          break;
        case 'agentName':
          request.roomConfig = (_b = request.roomConfig) !== null && _b !== void 0 ? _b : new RoomConfiguration();
          if (request.roomConfig.agents.length === 0) {
            request.roomConfig.agents.push(new RoomAgentDispatch());
          }
          request.roomConfig.agents[0].agentName = options.agentName;
          break;
        case 'agentMetadata':
          request.roomConfig = (_c = request.roomConfig) !== null && _c !== void 0 ? _c : new RoomConfiguration();
          if (request.roomConfig.agents.length === 0) {
            request.roomConfig.agents.push(new RoomAgentDispatch());
          }
          request.roomConfig.agents[0].metadata = options.agentMetadata;
          break;
        case 'deployment':
          request.roomConfig = (_d = request.roomConfig) !== null && _d !== void 0 ? _d : new RoomConfiguration();
          if (request.roomConfig.agents.length === 0) {
            request.roomConfig.agents.push(new RoomAgentDispatch());
          }
          request.roomConfig.agents[0].deployment = options.deployment;
          break;
        default:
          // ref: https://stackoverflow.com/a/58009992
          const exhaustiveCheckedKey = key;
          throw new Error("Options key ".concat(exhaustiveCheckedKey, " not being included in forming request!"));
      }
    }
    return request;
  }
  update(options) {
    return __awaiter(this, void 0, void 0, function* () {
      var _a;
      const request = this.createRequestFromOptions(options);
      const response = yield fetch(this.url, Object.assign(Object.assign({}, this.endpointOptions), {
        method: (_a = this.endpointOptions.method) !== null && _a !== void 0 ? _a : 'POST',
        headers: Object.assign({
          'Content-Type': 'application/json'
        }, this.endpointOptions.headers),
        body: request.toJsonString({
          useProtoFieldName: true
        })
      }));
      if (!response.ok) {
        throw new Error("Error generating token from endpoint ".concat(this.url, ": received ").concat(response.status, " / ").concat(yield response.text()));
      }
      const body = yield response.json();
      return TokenSourceResponse.fromJson(body, {
        // NOTE: it could be possible that the response body could contain more fields than just
        // what's in TokenSourceResponse depending on the implementation (ie, DevelopmentTokenServer)
        ignoreUnknownFields: true
      });
    });
  }
}
class TokenSourceDevelopmentTokenServer extends TokenSourceEndpoint {
  constructor(tokenServerId, options) {
    const _options$baseUrl = options.baseUrl,
      baseUrl = _options$baseUrl === void 0 ? 'https://cloud-api.livekit.io' : _options$baseUrl,
      rest = __rest(options, ["baseUrl"]);
    super("".concat(baseUrl, "/api/v2/sandbox/connection-details"), Object.assign(Object.assign({}, rest), {
      headers: {
        'X-Sandbox-ID': tokenServerId
      }
    }));
  }
}
/** @deprecated Use {@link TokenSourceDevelopmentTokenServer} instead */
class TokenSourceSandboxTokenServer extends TokenSourceDevelopmentTokenServer {}
const TokenSource = {
  /** TokenSource.literal contains a single, literal set of {@link TokenSourceResponseObject}
   * credentials, either provided directly or returned from a provided function. */
  literal(literalOrFn) {
    return new TokenSourceLiteral(literalOrFn);
  },
  /**
   * TokenSource.custom allows a user to define a manual function which generates new
   * {@link TokenSourceResponseObject} values on demand.
   *
   * Use this to get credentials from custom backends / etc.
   */
  custom(customFn) {
    return new TokenSourceCustom(customFn);
  },
  /**
   * TokenSource.endpoint creates a token source that fetches credentials from a given URL using
   * the standard endpoint format:
   * @see https://docs.livekit.io/frontends/build/authentication/endpoint/
   */
  endpoint(url) {
    let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    return new TokenSourceEndpoint(url, options);
  },
  /**
   * @deprecated Use {@link TokenSource.developmentTokenServer} instead
   */
  sandboxTokenServer(sandboxId) {
    let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    return new TokenSourceSandboxTokenServer(sandboxId, options);
  },
  /**
   * TokenSource.developmentTokenServer queries a development token server for credentials,
   * which supports quick prototyping / getting started types of use cases.
   *
   * This token provider is INSECURE and should NOT be used in production.
   *
   * For more info:
   * @see https://docs.livekit.io/frontends/build/authentication/sandbox-token-server/
   */
  developmentTokenServer(tokenServerId) {
    let options = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : {};
    return new TokenSourceDevelopmentTokenServer(tokenServerId, options);
  }
};
export { JOSEAlgNotAllowed, JOSEError, JOSENotSupported, JWEDecryptionFailed, JWEInvalid, JWKInvalid, JWKSInvalid, JWKSMultipleMatchingKeys, JWKSNoMatchingKey, JWKSTimeout, JWSInvalid, JWSSignatureVerificationFailed, JWTClaimValidationFailed, JWTExpired, JWTInvalid, ONE_MINUTE_IN_MILLISECONDS, ONE_SECOND_IN_MILLISECONDS, TokenSource, TokenSourceCached, TokenSourceConfigurable, TokenSourceCustom, TokenSourceDevelopmentTokenServer, TokenSourceEndpoint, TokenSourceFixed, TokenSourceLiteral, TokenSourceSandboxTokenServer, areTokenSourceFetchOptionsEqual, decode, decodeBase64, decodeJwt, decodeTokenPayload, decoder, invalid, isObject, isResponseTokenValid, strictDecoder };
