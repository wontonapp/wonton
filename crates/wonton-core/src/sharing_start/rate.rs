use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iroh::{
    EndpointId,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use wonton_tunnel::sharing::ShareProtocol;

const WINDOW: Duration = Duration::from_secs(60);
const MAX_ENDPOINTS: usize = 4096;

#[derive(Debug)]
pub(crate) struct LimitedShares {
    protocol: ShareProtocol,
    per_minute: u32,
    attempts: Mutex<HashMap<EndpointId, (Instant, u32)>>,
}

impl LimitedShares {
    pub(crate) fn new(protocol: ShareProtocol, per_minute: u32) -> Self {
        Self {
            protocol,
            per_minute,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    fn admit(&self, endpoint: EndpointId, now: Instant) -> bool {
        let mut attempts = self
            .attempts
            .lock()
            .expect("sharing rate limit lock poisoned");
        admit(&mut attempts, self.per_minute, endpoint, now)
    }
}

fn admit(
    attempts: &mut HashMap<EndpointId, (Instant, u32)>,
    per_minute: u32,
    endpoint: EndpointId,
    now: Instant,
) -> bool {
    attempts.retain(|_, (since, _)| now.saturating_duration_since(*since) < WINDOW);
    if !attempts.contains_key(&endpoint) && attempts.len() >= MAX_ENDPOINTS {
        return false;
    }
    let (_, used) = attempts.entry(endpoint).or_insert((now, 0));
    if *used >= per_minute {
        return false;
    }
    *used += 1;
    true
}

impl ProtocolHandler for LimitedShares {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        if !self.admit(connection.remote_id(), Instant::now()) {
            connection.close(1u32.into(), b"share connection rate exceeded");
            return Ok(());
        }
        self.protocol.accept(connection).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(index: u32) -> EndpointId {
        let mut key = [0; 32];
        key[..4].copy_from_slice(&index.to_be_bytes());
        iroh::SecretKey::from_bytes(&key).public()
    }

    #[test]
    fn attempts_are_per_endpoint_and_reopen_exactly_at_the_window_boundary() {
        let mut attempts = HashMap::new();
        let now = Instant::now();
        let alice = endpoint(1);
        let bob = endpoint(2);
        assert!(admit(&mut attempts, 2, alice, now));
        assert!(admit(&mut attempts, 2, alice, now));
        assert!(!admit(&mut attempts, 2, alice, now));
        assert!(admit(&mut attempts, 2, bob, now));
        assert!(!admit(
            &mut attempts,
            2,
            alice,
            now + WINDOW - Duration::from_nanos(1)
        ));
        assert!(admit(&mut attempts, 2, alice, now + WINDOW));
        assert!(admit(&mut attempts, 2, alice, now + WINDOW));
        assert!(!admit(&mut attempts, 2, alice, now + WINDOW));
    }

    #[test]
    fn capacity_does_not_evict_existing_endpoint_limits_and_expired_entries_release_it() {
        let mut attempts = HashMap::new();
        let now = Instant::now();
        for index in 0..MAX_ENDPOINTS as u32 {
            assert!(admit(&mut attempts, 2, endpoint(index), now));
        }
        let extra = endpoint(MAX_ENDPOINTS as u32);
        assert!(!admit(&mut attempts, 2, extra, now));
        assert!(admit(&mut attempts, 2, endpoint(0), now));
        assert!(!admit(&mut attempts, 2, endpoint(0), now));
        assert!(!admit(
            &mut attempts,
            2,
            extra,
            now + WINDOW - Duration::from_nanos(1)
        ));
        assert!(admit(&mut attempts, 2, extra, now + WINDOW));
    }
}
