//! Passive throughput estimate shared by the native player and downloads.
//! No speed tests or extra requests; stale observations expire after two minutes.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Slow,
    Standard,
    Fast,
}
struct Estimate {
    bytes_per_s: f64,
    samples: u32,
    at: Instant,
}
static ESTIMATE: Mutex<Option<Estimate>> = Mutex::new(None);

pub fn observe(bytes: u64, elapsed: Duration) {
    if bytes < 16 * 1024 || elapsed.as_millis() < 10 {
        return;
    }
    let rate = bytes as f64 / elapsed.as_secs_f64();
    let mut state = ESTIMATE.lock().unwrap();
    let old = state
        .as_ref()
        .filter(|s| s.at.elapsed() < Duration::from_secs(120));
    let (rate, samples) = match old {
        Some(s) => (
            if rate < s.bytes_per_s {
                rate
            } else {
                s.bytes_per_s * 0.75 + rate * 0.25
            },
            s.samples.saturating_add(1),
        ),
        None => (rate, 1),
    };
    *state = Some(Estimate {
        bytes_per_s: rate,
        samples,
        at: Instant::now(),
    });
}

// Track outstanding reads too: a stalled request must trigger Auto before its
// eventual timeout. Each guard belongs to one transfer; concurrent reads do not
// erase another transfer's stall signal.
static TRANSFERS: Mutex<Vec<(u64, Instant)>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub struct Transfer(u64);
impl Transfer {
    pub fn start() -> Self {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        TRANSFERS.lock().unwrap().push((id, Instant::now()));
        Self(id)
    }
    pub fn progress(&self) {
        if let Some((_, at)) = TRANSFERS
            .lock()
            .unwrap()
            .iter_mut()
            .find(|(id, _)| *id == self.0)
        {
            *at = Instant::now();
        }
    }
}
impl Drop for Transfer {
    fn drop(&mut self) {
        TRANSFERS.lock().unwrap().retain(|(id, _)| *id != self.0);
    }
}
pub fn class() -> Class {
    if TRANSFERS
        .lock()
        .unwrap()
        .iter()
        .any(|(_, at)| at.elapsed() >= Duration::from_secs(3))
    {
        return Class::Slow;
    }
    let state = ESTIMATE.lock().unwrap();
    let Some(s) = state
        .as_ref()
        .filter(|s| s.at.elapsed() < Duration::from_secs(120))
    else {
        return Class::Standard;
    };
    classify(s.bytes_per_s, s.samples)
}

fn classify(rate: f64, samples: u32) -> Class {
    if rate < 64.0 * 1024.0 {
        Class::Slow
    } else if samples >= 3 && rate >= 512.0 * 1024.0 {
        Class::Fast
    } else {
        Class::Standard
    }
}

pub fn chunk_bytes() -> u64 {
    let state = ESTIMATE.lock().unwrap();
    state
        .as_ref()
        .filter(|s| s.at.elapsed() < Duration::from_secs(120))
        .map(|s| (s.bytes_per_s * 2.0).clamp(32.0 * 1024.0, 384.0 * 1024.0) as u64)
        .unwrap_or(64 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_downgrades_immediately_but_fast_needs_evidence() {
        assert_eq!(classify(32.0 * 1024.0, 1), Class::Slow);
        assert_eq!(classify(1024.0 * 1024.0, 1), Class::Standard);
        assert_eq!(classify(1024.0 * 1024.0, 3), Class::Fast);
    }
}
