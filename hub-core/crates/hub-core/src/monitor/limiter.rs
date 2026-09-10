//! Per-host rate limiter. Hard requirement ported from the Crucix survey:
//! GDELT tolerates ≤1 request / 5s — cross-source budget for the same host
//! lives here (only GDELT uses it today, but the facility is generic).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    last: Mutex<HashMap<String, Instant>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(HashMap::new()),
        }
    }

    /// Sleep until at least `gap` has passed since the previous `wait` for
    /// `host`. Reservations are made under the lock so concurrent callers
    /// queue at multiples of `gap` instead of stampeding.
    pub async fn wait(&self, host: &str, gap: Duration) {
        let sleep_for = {
            let mut map = match self.last.lock() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            let now = Instant::now();
            match map.get(host) {
                Some(prev) => {
                    let elapsed = now.duration_since(*prev);
                    if elapsed >= gap {
                        map.insert(host.to_string(), now);
                        None
                    } else {
                        let wait = gap - elapsed;
                        map.insert(host.to_string(), now + wait);
                        Some(wait)
                    }
                }
                None => {
                    map.insert(host.to_string(), now);
                    None
                }
            }
        };
        if let Some(d) = sleep_for {
            tokio::time::sleep(d).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn second_call_waits() {
        let l = RateLimiter::new();
        l.wait("h", Duration::from_millis(120)).await;
        let t = Instant::now();
        l.wait("h", Duration::from_millis(120)).await;
        assert!(t.elapsed() >= Duration::from_millis(110));
        // different host is not throttled
        let t = Instant::now();
        l.wait("other", Duration::from_millis(120)).await;
        assert!(t.elapsed() < Duration::from_millis(60));
    }
}
