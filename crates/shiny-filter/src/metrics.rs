//! Counters for the filter proxy.
//!
//! These exist so "the browser is efficient for this web app" is a measurable
//! statement rather than a claim: `benchmarks/` reads this snapshot before and
//! after a run and reports blocked ratio, bytes saved and per-type breakdown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::classify::RequestKind;

/// Live counters, cheap to clone (one `Arc`).
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    inner: Arc<Counters>,
}

#[derive(Debug, Default)]
struct Counters {
    requests: AtomicU64,
    allowed: AtomicU64,
    blocked: AtomicU64,
    failed: AtomicU64,
    rewritten: AtomicU64,
    bytes_out: AtomicU64,
    bytes_saved: AtomicU64,
    cosmetic_rules: AtomicU64,

    documents: AtomicU64,
    scripts: AtomicU64,
    stylesheets: AtomicU64,
    images: AtomicU64,
    media: AtomicU64,
    fonts: AtomicU64,
    xhr: AtomicU64,
    other: AtomicU64,

    /// Time spent inside the filter decision, in microseconds.
    filter_micros: AtomicU64,
    /// Time spent rewriting document bodies, in microseconds.
    rewrite_micros: AtomicU64,
    /// Worst single decision seen, in microseconds. A mean is misleading
    /// here: the engine's first calls populate internal regex caches and cost
    /// orders of magnitude more than steady state, so the mean reported
    /// ~17ms/request on a proxy that actually runs at ~4µs. Both are kept,
    /// and the max is what a latency claim should be checked against.
    filter_micros_max: AtomicU64,
    rewrite_micros_max: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one completed request. `src_bytes` is the body size we would
    /// have forwarded; `out_bytes` what we actually sent (0 for a block).
    pub fn record(&self, kind: RequestKind, verdict_blocked: bool, src_bytes: u64, out_bytes: u64) {
        self.inner.requests.fetch_add(1, Ordering::Relaxed);
        if verdict_blocked {
            self.inner.blocked.fetch_add(1, Ordering::Relaxed);
            // Everything we refused to fetch counts as saved.
            self.inner.bytes_saved.fetch_add(src_bytes, Ordering::Relaxed);
        } else {
            self.inner.allowed.fetch_add(1, Ordering::Relaxed);
            self.inner.bytes_out.fetch_add(out_bytes, Ordering::Relaxed);
            if src_bytes > out_bytes {
                self.inner
                    .bytes_saved
                    .fetch_add(src_bytes - out_bytes, Ordering::Relaxed);
            }
        }
        self.bump_kind(kind);
    }

    pub fn record_failure(&self) {
        self.inner.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rewrite(&self, micros: u64) {
        self.inner.rewritten.fetch_add(1, Ordering::Relaxed);
        self.inner.rewrite_micros.fetch_add(micros, Ordering::Relaxed);
        bump_max(&self.inner.rewrite_micros_max, micros);
    }

    pub fn record_filter_time(&self, micros: u64) {
        self.inner.filter_micros.fetch_add(micros, Ordering::Relaxed);
        bump_max(&self.inner.filter_micros_max, micros);
    }

    pub fn record_cosmetic_rules(&self, count: u64) {
        self.inner
            .cosmetic_rules
            .fetch_add(count, Ordering::Relaxed);
    }

    fn bump_kind(&self, kind: RequestKind) {
        let counter = match kind {
            RequestKind::Document | RequestKind::Subdocument => &self.inner.documents,
            RequestKind::Script => &self.inner.scripts,
            RequestKind::Stylesheet => &self.inner.stylesheets,
            RequestKind::Image => &self.inner.images,
            RequestKind::Media => &self.inner.media,
            RequestKind::Font => &self.inner.fonts,
            RequestKind::Xhr | RequestKind::Fetch | RequestKind::Ping => &self.inner.xhr,
            _ => &self.inner.other,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let c = &self.inner;
        let requests = c.requests.load(Ordering::Relaxed);
        let blocked = c.blocked.load(Ordering::Relaxed);
        MetricsSnapshot {
            requests,
            allowed: c.allowed.load(Ordering::Relaxed),
            blocked,
            failed: c.failed.load(Ordering::Relaxed),
            rewritten: c.rewritten.load(Ordering::Relaxed),
            bytes_out: c.bytes_out.load(Ordering::Relaxed),
            bytes_saved: c.bytes_saved.load(Ordering::Relaxed),
            cosmetic_rules: c.cosmetic_rules.load(Ordering::Relaxed),
            blocked_ratio: if requests == 0 {
                0.0
            } else {
                blocked as f64 / requests as f64
            },
            mean_filter_micros: mean(c.filter_micros.load(Ordering::Relaxed), requests),
            max_filter_micros: c.filter_micros_max.load(Ordering::Relaxed),
            mean_rewrite_micros: mean(c.rewrite_micros.load(Ordering::Relaxed), c.rewritten.load(Ordering::Relaxed)),
            by_kind: KindCounts {
                documents: c.documents.load(Ordering::Relaxed),
                scripts: c.scripts.load(Ordering::Relaxed),
                stylesheets: c.stylesheets.load(Ordering::Relaxed),
                images: c.images.load(Ordering::Relaxed),
                media: c.media.load(Ordering::Relaxed),
                fonts: c.fonts.load(Ordering::Relaxed),
                xhr: c.xhr.load(Ordering::Relaxed),
                other: c.other.load(Ordering::Relaxed),
            },
        }
    }
}

/// Raise `slot` to `value` if it is larger (lock-free, relaxed).
fn bump_max(slot: &AtomicU64, value: u64) {
    let mut current = slot.load(Ordering::Relaxed);
    while value > current {
        match slot.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn mean(total: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total as f64 / count as f64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub requests: u64,
    pub allowed: u64,
    pub blocked: u64,
    pub failed: u64,
    pub rewritten: u64,
    pub bytes_out: u64,
    pub bytes_saved: u64,
    pub cosmetic_rules: u64,
    pub blocked_ratio: f64,
    pub mean_filter_micros: f64,
    /// Worst single decision (includes warm-up). See the field note on the
    /// counter for why a mean alone is not a latency claim.
    pub max_filter_micros: u64,
    pub mean_rewrite_micros: f64,
    pub by_kind: KindCounts,
}

impl MetricsSnapshot {
    /// Compact one-line form for the proxy log and the shell's status bar.
    pub fn summary(&self) -> String {
        format!(
            "{} req · {} blocked ({:.1}%) · {} rewritten · {:.1} KiB saved · filter {:.1}µs avg / {}µs max",
            self.requests,
            self.blocked,
            self.blocked_ratio * 100.0,
            self.rewritten,
            self.bytes_saved as f64 / 1024.0,
            self.mean_filter_micros,
            self.max_filter_micros,
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KindCounts {
    pub documents: u64,
    pub scripts: u64,
    pub stylesheets: u64,
    pub images: u64,
    pub media: u64,
    pub fonts: u64,
    pub xhr: u64,
    pub other: u64,
}
