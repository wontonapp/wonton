use std::sync::Arc;
use tokio::runtime::Handle;
use wonton_protocol::CoreError;
use wonton_tunnel::sharing::ProxyHub;

/// Scope construction can be called from either UniFFI or an async test/runtime.
/// Bind on a helper thread rather than nesting Tokio's block_on on the caller.
pub(crate) fn bind(runtime: Handle) -> Result<Arc<ProxyHub>, CoreError> {
    std::thread::spawn(move || runtime.block_on(ProxyHub::bind()))
        .join()
        .map_err(|_| CoreError::TransportUnavailable {
            reason: "proxy hub startup failed".into(),
        })?
        .map_err(|_| CoreError::TransportUnavailable {
            reason: "proxy hub bind failed".into(),
        })
}
