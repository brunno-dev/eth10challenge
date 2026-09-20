//! Aggregate completed-work metrics. No candidates, search inputs or target
//! addresses are included in this report.
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[derive(Default, Debug, Serialize)]
pub struct SearchMetrics {
    pub backend: String,
    pub completed_raw: u128,
    pub excluded: u128,
    pub pruned: u128,
    pub retained: u128,
    pub checksum_survivors: u128,
    pub completed_batches: u64,
    pub device_batches: u64,
    pub producer_seconds: f64,
    pub wait_seconds: f64,
    pub transfer_seconds: f64,
    pub filter_seconds: f64,
    pub derive_seconds: f64,
    pub checkpoint_seconds: f64,
    pub min_batch_size: Option<usize>,
    pub max_batch_size: Option<usize>,
    pub adaptive_changes: u64,
}

impl SearchMetrics {
    /// Record only a completed batch. Stage every checked sum before mutation,
    /// so invalid inputs or overflow cannot leave a partially updated report.
    pub fn record_batch(
        &mut self,
        raw: usize,
        retained: usize,
        pruned: usize,
        survivors: usize,
    ) -> Result<()> {
        let excluded = raw
            .checked_sub(retained)
            .context("retained count exceeds raw batch size")?;
        anyhow::ensure!(pruned <= excluded, "pruned count exceeds excluded count");
        anyhow::ensure!(
            survivors <= retained,
            "checksum survivors exceed retained count"
        );
        let completed_raw = self
            .completed_raw
            .checked_add(raw as u128)
            .context("completed raw metric overflow")?;
        let excluded_total = self
            .excluded
            .checked_add(excluded as u128)
            .context("excluded metric overflow")?;
        let pruned_total = self
            .pruned
            .checked_add(pruned as u128)
            .context("pruned metric overflow")?;
        let retained_total = self
            .retained
            .checked_add(retained as u128)
            .context("retained metric overflow")?;
        let checksum_survivors = self
            .checksum_survivors
            .checked_add(survivors as u128)
            .context("checksum survivor metric overflow")?;
        let completed_batches = self
            .completed_batches
            .checked_add(1)
            .context("completed batch metric overflow")?;

        self.completed_raw = completed_raw;
        self.excluded = excluded_total;
        self.pruned = pruned_total;
        self.retained = retained_total;
        self.checksum_survivors = checksum_survivors;
        self.completed_batches = completed_batches;
        Ok(())
    }

    pub fn observe_batch_size(&mut self, size: usize) {
        self.min_batch_size = Some(self.min_batch_size.map_or(size, |old| old.min(size)));
        self.max_batch_size = Some(self.max_batch_size.map_or(size, |old| old.max(size)));
    }

    /// Create a new report without replacing an existing file. Serialization
    /// happens before creation; an I/O error removes only our newly created file.
    pub fn write_new(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).context("serializing search metrics")?;
        write_new_bytes(path, &bytes, |file, bytes| file.write_all(bytes))
    }

    pub fn print(&self) {
        let backend = if self.backend.is_empty() {
            "unspecified"
        } else {
            &self.backend
        };
        println!(
            "Metrics ({backend}; completed batches only): {} raw, {} excluded ({} pruned), {} retained, {} derivation candidates; {} batches, {} device batches.",
            self.completed_raw,
            self.excluded,
            self.pruned,
            self.retained,
            self.checksum_survivors,
            self.completed_batches,
            self.device_batches,
        );
        println!(
            "Stage seconds: producer {:.3}, wait {:.3}, transfer {:.3}, filter {:.3}, derive {:.3}, checkpoint {:.3}. Producer and GPU work overlap; these times are not additive.",
            self.producer_seconds,
            self.wait_seconds,
            self.transfer_seconds,
            self.filter_seconds,
            self.derive_seconds,
            self.checkpoint_seconds,
        );
        println!(
            "CPU derive includes checksum and derivation together. GPU stages are host wall-clock intervals including synchronization, not exclusive kernel times from CUDA events. A batch containing a hit is not counted."
        );
        if let (Some(min), Some(max)) = (self.min_batch_size, self.max_batch_size) {
            println!(
                "Requested batch sizes: {min}..{max}; {} adaptive changes.",
                self.adaptive_changes
            );
        }
    }
}

fn write_new_bytes(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating new metrics report {}", path.display()))?;
    let result = write(&mut file, bytes).and_then(|()| file.sync_all());
    // Close the handle before cleanup, including on Windows.
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result.with_context(|| format!("writing metrics report {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "words-breaker-metrics-{}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ))
    }

    #[test]
    fn completed_metrics_conserve_counts_and_observe_batch_sizes() {
        let mut metrics = SearchMetrics {
            backend: "CUDA".into(),
            ..Default::default()
        };
        metrics.record_batch(100, 40, 30, 3).unwrap();
        metrics.record_batch(20, 0, 20, 0).unwrap();
        metrics.record_batch(8, 8, 0, 8).unwrap();
        assert_eq!(metrics.completed_raw, 128);
        assert_eq!(metrics.excluded, 80);
        assert_eq!(metrics.pruned, 50);
        assert_eq!(metrics.retained, 48);
        assert_eq!(metrics.checksum_survivors, 11);
        assert_eq!(metrics.completed_batches, 3);
        assert_eq!(metrics.device_batches, 0, "caller records actual launches");
        assert_eq!(metrics.completed_raw, metrics.excluded + metrics.retained);
        assert!(metrics.pruned <= metrics.excluded);
        assert!(metrics.checksum_survivors <= metrics.retained);
        for size in [64, 16, 128, 32] {
            metrics.observe_batch_size(size);
        }
        assert_eq!(metrics.min_batch_size, Some(16));
        assert_eq!(metrics.max_batch_size, Some(128));
        assert_eq!(metrics.adaptive_changes, 0);
    }

    #[test]
    fn invalid_metrics_and_overflow_do_not_partially_mutate() {
        let mut metrics = SearchMetrics::default();
        metrics.record_batch(10, 5, 3, 2).unwrap();
        let before = serde_json::to_vec(&metrics).unwrap();
        for (raw, retained, pruned, survivors) in [(1, 2, 0, 0), (5, 3, 3, 0), (5, 3, 0, 4)] {
            assert!(metrics
                .record_batch(raw, retained, pruned, survivors)
                .is_err());
            assert_eq!(serde_json::to_vec(&metrics).unwrap(), before);
        }
        metrics.completed_batches = u64::MAX;
        let before = serde_json::to_vec(&metrics).unwrap();
        assert!(metrics.record_batch(1, 1, 0, 1).is_err());
        assert_eq!(serde_json::to_vec(&metrics).unwrap(), before);
        metrics.completed_batches = 0;
        metrics.completed_raw = u128::MAX;
        let before = serde_json::to_vec(&metrics).unwrap();
        assert!(metrics.record_batch(1, 1, 0, 1).is_err());
        assert_eq!(serde_json::to_vec(&metrics).unwrap(), before);
    }

    #[test]
    fn report_is_new_only_and_failed_write_removes_its_partial_file() {
        let path = test_path();
        let mut metrics = SearchMetrics::default();
        metrics.record_batch(12, 5, 4, 1).unwrap();
        metrics.write_new(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["completed_raw"], 12);
        assert!(report.get("target").is_none());
        assert!(report.get("mnemonic").is_none());
        assert!(metrics.write_new(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(write_new_bytes(&path, b"replacement", |_, _| {
            panic!("existing report must be rejected before writing")
        })
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();

        let error = write_new_bytes(&path, b"partial report", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(error.is_err());
        assert!(!path.exists(), "failed creation left a partial report");
        // Cleanup must leave the path available for a subsequent valid report.
        metrics.write_new(&path).unwrap();
        fs::remove_file(path).unwrap();
    }
}
