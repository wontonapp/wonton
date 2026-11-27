// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 8627-8732 行按职责切出，行为不变。

/**
 * @internal
 *
 * Get a named logger. When `ctxFn` is supplied, every log call
 * automatically:
 * 1. prepends a `[key=value ...]` prefix derived from `ctxFn()` to the
 *    message string, so identifiers are visible in browser devtools
 *    without expanding the structured context object, and
 * 2. merges `ctxFn()` into the structured context passed to any
 *    `setLogExtension` consumer, so ingestion pipelines continue to
 *    receive the full metadata unchanged.
 */
import { LogLevel, LoggerNames, livekitLogger, livekitLoggers, loglevelExports } from './loglevel.mjs';

function getLogger(name, ctxFn) {
  const logger = loglevelExports.getLogger(name);
  logger.setDefaultLevel(livekitLogger.getLevel());
  if (!ctxFn) {
    return logger;
  }
  return wrapWithContext(logger, ctxFn);
}
function wrapWithContext(base, ctxFn) {
  // Resolve the underlying method on every call so that later
  // setLogExtension installations (which replace the base logger's
  // methods via loglevel's methodFactory) are picked up.
  const wrap = method => (msg, extra) => {
    const ctx = ctxFn();
    const merged = ctx || extra ? Object.assign(Object.assign({}, ctx), extra) : undefined;
    base[method](msg, merged);
  };
  const proxy = Object.create(base);
  proxy.trace = wrap('trace');
  proxy.debug = wrap('debug');
  proxy.info = wrap('info');
  proxy.warn = wrap('warn');
  proxy.error = wrap('error');
  return proxy;
}
function setLogLevel(level, loggerName) {
  if (loggerName) {
    loglevelExports.getLogger(loggerName).setLevel(level);
  } else {
    for (const logger of livekitLoggers) {
      logger.setLevel(level);
    }
  }
}
/**
 * use this to hook into the logging function to allow sending internal livekit logs to third party services
 * if set, the browser logs will lose their stacktrace information (see https://github.com/pimterry/loglevel#writing-plugins)
 */
function setLogExtension(extension, logger) {
  const loggers = logger ? [logger] : livekitLoggers;
  loggers.forEach(logR => {
    const originalFactory = logR.methodFactory;
    logR.methodFactory = (methodName, configLevel, loggerName) => {
      const rawMethod = originalFactory(methodName, configLevel, loggerName);
      const logLevel = LogLevel[methodName];
      const needLog = logLevel >= configLevel && logLevel < LogLevel.silent;
      return (msg, context) => {
        if (context) rawMethod(msg, context);else rawMethod(msg);
        if (needLog) {
          extension(logLevel, msg, context);
        }
      };
    };
    logR.setLevel(logR.getLevel());
  });
}
const workerLogger = loglevelExports.getLogger(LoggerNames.E2EE);
const workerLogLevelListeners = new Set();
const originalWorkerSetLevel = workerLogger.setLevel.bind(workerLogger);
workerLogger.setLevel = (level, persist) => {
  originalWorkerSetLevel(level, persist);
  const numeric = workerLogger.getLevel();
  workerLogLevelListeners.forEach(cb => cb(numeric));
};
/** @internal Subscribe to workerLogger level changes (so E2EE workers can be kept in sync). */
function onWorkerLogLevelChanged(cb) {
  workerLogLevelListeners.add(cb);
  return () => {
    workerLogLevelListeners.delete(cb);
  };
}const maxRetryDelay = 7000;
/**
 * Upper bound of the random delay added to the first retry, whose base delay is 0. It keeps a
 * brief network blip recovering quickly while spreading out clients that disconnect together.
 */
const maxFirstRetryJitterInMs = 500;
const DEFAULT_RETRY_DELAYS_IN_MS = [0, 300, 2 * 2 * 300, 3 * 3 * 300, 4 * 4 * 300, maxRetryDelay, maxRetryDelay, maxRetryDelay, maxRetryDelay, maxRetryDelay];
class DefaultReconnectPolicy {
  constructor(retryDelays) {
    this._retryDelays = retryDelays !== undefined ? [...retryDelays] : DEFAULT_RETRY_DELAYS_IN_MS;
  }
  nextRetryDelayInMs(context) {
    if (context.retryCount >= this._retryDelays.length) return null;
    const retryDelay = this._retryDelays[context.retryCount];
    // The first retry has no base delay to scale, so it gets a fixed window instead. A zero at a
    // later index is an explicit request for an immediate retry and passes through the formula as zero.
    if (context.retryCount === 0 && retryDelay === 0) {
      return Math.random() * maxFirstRetryJitterInMs;
    }
    // Jitter scales with the delay (+/-50%) so clients spread out further on later retries.
    return Math.round(retryDelay * (0.5 + Math.random()));
  }
}
export { DEFAULT_RETRY_DELAYS_IN_MS, DefaultReconnectPolicy, getLogger, maxFirstRetryJitterInMs, maxRetryDelay, onWorkerLogLevelChanged, originalWorkerSetLevel, setLogExtension, setLogLevel, workerLogLevelListeners, workerLogger, wrapWithContext };
