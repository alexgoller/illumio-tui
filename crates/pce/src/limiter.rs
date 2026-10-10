//! A token bucket shared by every request of one client. `acquire` waits
//! until a token is available. Uses tokio's clock so tests can pause time.
use std::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

pub struct RateLimiter {
    pub(crate) rate: f64,
    pub(crate) burst: f64,
    state: Mutex<State>,
}

struct State {
    tokens: f64,
    updated: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64, burst: u32) -> Self {
        let burst = f64::from(burst.max(1));
        Self {
            rate: requests_per_second.max(0.01),
            burst,
            state: Mutex::new(State {
                tokens: burst,
                updated: Instant::now(),
            }),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut s = self.state.lock().expect("limiter mutex poisoned");
                let now = Instant::now();
                let elapsed = now.duration_since(s.updated).as_secs_f64();
                s.tokens = (s.tokens + elapsed * self.rate).min(self.burst);
                s.updated = now;
                if s.tokens >= 1.0 {
                    s.tokens -= 1.0;
                    None
                } else {
                    Some(Duration::from_secs_f64((1.0 - s.tokens) / self.rate))
                }
            };
            match wait {
                None => return,
                Some(d) => sleep(d).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, Instant};

    #[tokio::test(start_paused = true)]
    async fn burst_is_immediate_then_throttled() {
        let l = RateLimiter::new(2.0, 5);
        let t0 = Instant::now();
        for _ in 0..5 {
            l.acquire().await;
        }
        assert_eq!(Instant::now() - t0, Duration::ZERO, "burst must not wait");
        l.acquire().await;
        let waited = Instant::now() - t0;
        assert!(
            waited >= Duration::from_millis(499),
            "6th request waited {waited:?}"
        );
        assert!(waited <= Duration::from_millis(520));
    }

    #[tokio::test(start_paused = true)]
    async fn tokens_refill_over_time() {
        let l = RateLimiter::new(1.0, 1);
        l.acquire().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        let t0 = Instant::now();
        l.acquire().await;
        assert_eq!(Instant::now() - t0, Duration::ZERO);
    }

    #[test]
    fn zero_rate_is_clamped() {
        let l = RateLimiter::new(0.0, 0);
        assert!(l.rate > 0.0);
        assert!(l.burst >= 1.0);
    }
}
