use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct PendingBatch {
    first_seen: Instant,
    last_seen: Instant,
    events: usize,
    paths: BTreeSet<PathBuf>,
}

#[derive(Debug)]
pub struct ReadyBatch {
    pub project: usize,
    pub events: usize,
    pub paths: Vec<PathBuf>,
}

/// Groups noisy filesystem events into bounded bursts.
///
/// The quiet period grows during a burst, while `max_settle` guarantees that a
/// continuously changing tree is eventually processed.
pub struct Batcher {
    pending: HashMap<usize, PendingBatch>,
    settle: Duration,
    max_settle: Duration,
    burst_threshold: usize,
}

impl Batcher {
    pub fn new(settle: Duration, max_settle: Duration, burst_threshold: usize) -> Self {
        Self {
            pending: HashMap::new(),
            settle,
            max_settle,
            burst_threshold,
        }
    }

    pub fn ingest(&mut self, project: usize, paths: impl IntoIterator<Item = PathBuf>) {
        self.ingest_at(project, paths, Instant::now());
    }

    fn ingest_at(
        &mut self,
        project: usize,
        paths: impl IntoIterator<Item = PathBuf>,
        now: Instant,
    ) {
        let pending = self.pending.entry(project).or_insert_with(|| PendingBatch {
            first_seen: now,
            last_seen: now,
            events: 0,
            paths: BTreeSet::new(),
        });
        pending.last_seen = now;
        pending.events += 1;
        pending.paths.extend(paths);
    }

    pub fn drain_ready(&mut self) -> Vec<ReadyBatch> {
        self.drain_ready_at(Instant::now())
    }

    pub fn drain_all(&mut self) -> Vec<ReadyBatch> {
        let projects: Vec<_> = self.pending.keys().copied().collect();
        self.take(projects)
    }

    fn drain_ready_at(&mut self, now: Instant) -> Vec<ReadyBatch> {
        let ready = self
            .pending
            .iter()
            .filter_map(|(project, batch)| {
                let burst_ratio =
                    (batch.events as f64 / self.burst_threshold as f64).clamp(0.0, 1.0);
                let adaptive = self.settle.mul_f64(1.0 + burst_ratio);
                let quiet_deadline = batch.last_seen + adaptive.min(self.max_settle);
                let hard_deadline = batch.first_seen + self.max_settle;
                (now >= quiet_deadline || now >= hard_deadline).then_some(*project)
            })
            .collect();
        self.take(ready)
    }

    fn take(&mut self, projects: Vec<usize>) -> Vec<ReadyBatch> {
        projects
            .into_iter()
            .filter_map(|project| {
                self.pending.remove(&project).map(|batch| ReadyBatch {
                    project,
                    events: batch.events,
                    paths: batch.paths.into_iter().collect(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_paths_in_a_burst() {
        let now = Instant::now();
        let mut batcher = Batcher::new(Duration::from_secs(1), Duration::from_secs(8), 20);
        batcher.ingest_at(0, [PathBuf::from("src/lib.rs")], now);
        batcher.ingest_at(0, [PathBuf::from("src/lib.rs")], now);
        let batches = batcher.drain_ready_at(now + Duration::from_secs(2));
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].events, 2);
        assert_eq!(batches[0].paths.len(), 1);
    }

    #[test]
    fn hard_deadline_flushes_a_continuous_burst() {
        let now = Instant::now();
        let mut batcher = Batcher::new(Duration::from_secs(3), Duration::from_secs(5), 2);
        batcher.ingest_at(0, [PathBuf::from("src/a.rs")], now);
        batcher.ingest_at(0, [PathBuf::from("src/b.rs")], now + Duration::from_secs(4));
        assert_eq!(
            batcher.drain_ready_at(now + Duration::from_secs(5)).len(),
            1
        );
    }
}
