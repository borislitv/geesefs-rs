//! Fixed Foyer gauge bridge. Scrapes never inspect regions, records or keys.
use mixtrics::{
    metrics::{
        BoxedCounterVec, BoxedGauge, BoxedGaugeVec, BoxedHistogramVec, GaugeOps, GaugeVecOps,
        RegistryOps,
    },
    registry::noop::NoopMetricsRegistry,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{borrow::Cow, sync::atomic::AtomicU64};

const STATES: [&str; 4] = ["clean", "writing", "evictable", "reclaiming"];
type Count = Gauge<u64, AtomicU64>;

/// Whole data-region ownership, not live indexed payload bytes. Independent
/// gauge updates can straddle a transition; this is not a transactional snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionStats {
    pub size_bytes: u64,
    pub total: u64,
    pub clean: u64,
    pub writing: u64,
    pub evictable: u64,
    pub reclaiming: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Regions {
    counts: [Count; 4],
    total: Count,
    size_bytes: Count,
}

impl Regions {
    /// Supported picker initialization runs before recovery and observes only
    /// actual data partitions, excluding the tombstone log and unusable tail.
    pub(crate) fn init(&self, count: usize, size_bytes: usize) {
        self.total.set(count as u64);
        self.size_bytes.set(size_bytes as u64);
    }

    pub(crate) fn snapshot(&self) -> RegionStats {
        RegionStats {
            size_bytes: self.size_bytes.get(),
            total: self.total.get(),
            clean: self.counts[0].get(),
            writing: self.counts[1].get(),
            evictable: self.counts[2].get(),
            reclaiming: self.counts[3].get(),
        }
    }
}

#[derive(Debug)]
struct RegionGauge(Count);

impl GaugeOps for RegionGauge {
    fn increase(&self, value: u64) {
        self.0.inc_by(value);
    }
    fn decrease(&self, value: u64) {
        self.0.dec_by(value);
    }
    fn absolute(&self, value: u64) {
        self.0.set(value);
    }
}

impl GaugeVecOps for Regions {
    fn gauge(&self, labels: &[Cow<'static, str>]) -> BoxedGauge {
        if let [_, state] = labels
            && let Some(index) = STATES.iter().position(|candidate| candidate == state)
        {
            return Box::new(RegionGauge(self.counts[index].clone()));
        }
        Box::new(NoopMetricsRegistry)
    }
}

impl RegistryOps for Regions {
    fn register_gauge_vec(
        &self,
        name: Cow<'static, str>,
        _: Cow<'static, str>,
        labels: &'static [&'static str],
    ) -> BoxedGaugeVec {
        if name == "foyer_storage_block_engine_block" && labels == ["name", "type"] {
            Box::new(self.clone())
        } else {
            Box::new(NoopMetricsRegistry)
        }
    }

    fn register_counter_vec(
        &self,
        name: Cow<'static, str>,
        description: Cow<'static, str>,
        labels: &'static [&'static str],
    ) -> BoxedCounterVec {
        NoopMetricsRegistry.register_counter_vec(name, description, labels)
    }

    fn register_histogram_vec(
        &self,
        name: Cow<'static, str>,
        description: Cow<'static, str>,
        labels: &'static [&'static str],
    ) -> BoxedHistogramVec {
        NoopMetricsRegistry.register_histogram_vec(name, description, labels)
    }

    fn register_histogram_vec_with_buckets(
        &self,
        name: Cow<'static, str>,
        description: Cow<'static, str>,
        labels: &'static [&'static str],
        buckets: Vec<f64>,
    ) -> BoxedHistogramVec {
        NoopMetricsRegistry.register_histogram_vec_with_buckets(name, description, labels, buckets)
    }
}
