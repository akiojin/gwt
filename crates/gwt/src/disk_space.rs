//! Free-space observation for the host that runs every agent worktree
//! (Issue #4009 AC-4).
//!
//! On 2026-09-06 the host filled up silently: the verification coordinator
//! could no longer write its lease file, every `verify.run` on the host
//! failed with `No space left on device`, and nothing had warned anyone
//! beforehand. This module turns free space into a status field the PM reads
//! anyway (`issue.monitor.status`), so the warning appears before the
//! coordinator starts failing rather than after.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Warn while a volume has less than this many bytes free (20 GiB): about
/// one cold workspace rebuild, the amount a single blocked agent needs to
/// recover on its own.
pub const WARN_BELOW_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Warn while a volume has less than this percentage free.
pub const WARN_BELOW_PERCENT: u64 = 5;

/// One volume as seen from a path that lives on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskVolume {
    /// The path the volume was probed through.
    pub path: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// Free-space verdict over every probed volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSpaceStatus {
    pub volumes: Vec<DiskVolume>,
    pub warn_below_bytes: u64,
    pub warn_below_percent: u64,
    /// Present while any volume is below a threshold; names the volume and
    /// the operation that reclaims space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Probe each path's volume and evaluate the thresholds. A path that does
/// not exist or whose volume cannot be queried is skipped rather than
/// reported as empty: a missing directory must not read as a full disk.
/// (Windows resolves the volume of a missing path from its prefix, so the
/// existence check is what keeps the two platforms answering alike.)
pub fn probe(paths: &[&Path]) -> DiskSpaceStatus {
    let mut volumes = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let (Ok(free_bytes), Ok(total_bytes)) =
            (fs2::available_space(path), fs2::total_space(path))
        else {
            continue;
        };
        let volume = DiskVolume {
            path: path.display().to_string(),
            free_bytes,
            total_bytes,
        };
        if !volumes.contains(&volume) {
            volumes.push(volume);
        }
    }
    evaluate(volumes)
}

/// Apply the thresholds to already-measured volumes.
pub fn evaluate(volumes: Vec<DiskVolume>) -> DiskSpaceStatus {
    let low: Vec<String> = volumes
        .iter()
        .filter(|volume| is_low(volume))
        .map(|volume| {
            format!(
                "{} has {} free of {} ({:.2}%)",
                volume.path,
                format_bytes(volume.free_bytes),
                format_bytes(volume.total_bytes),
                free_percent(volume)
            )
        })
        .collect();
    let warning = (!low.is_empty()).then(|| {
        format!(
            "disk space low: {}; reclaim merged worktrees' build caches with \
             `worktree.gc_build_artifacts` before verification fails with \
             `No space left on device`",
            low.join("; ")
        )
    });
    DiskSpaceStatus {
        volumes,
        warn_below_bytes: WARN_BELOW_BYTES,
        warn_below_percent: WARN_BELOW_PERCENT,
        warning,
    }
}

fn is_low(volume: &DiskVolume) -> bool {
    volume.free_bytes < WARN_BELOW_BYTES
        || volume.total_bytes > 0
            && volume.free_bytes.saturating_mul(100) < volume.total_bytes * WARN_BELOW_PERCENT
}

fn free_percent(volume: &DiskVolume) -> f64 {
    if volume.total_bytes == 0 {
        return 0.0;
    }
    volume.free_bytes as f64 * 100.0 / volume.total_bytes as f64
}

/// Human-readable byte count (`1.5 GiB`).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn volume(path: &str, free_bytes: u64, total_bytes: u64) -> DiskVolume {
        DiskVolume {
            path: path.to_string(),
            free_bytes,
            total_bytes,
        }
    }

    /// Issue #4009 AC-4: a healthy volume carries no warning.
    #[test]
    fn healthy_volume_has_no_warning() {
        let status = evaluate(vec![volume("/work", 500 * GIB, 3600 * GIB)]);
        assert_eq!(status.warning, None);
        assert_eq!(status.warn_below_bytes, WARN_BELOW_BYTES);
        assert_eq!(status.warn_below_percent, WARN_BELOW_PERCENT);
        assert_eq!(status.volumes.len(), 1);
    }

    /// Issue #4009 AC-4: the 2026-09-06 state (357 MiB free of 3.6 TiB) warns
    /// and names the reclaim operation.
    #[test]
    fn volume_below_byte_threshold_warns_and_names_the_reclaim_operation() {
        let status = evaluate(vec![volume("/work", 357 * 1024 * 1024, 3600 * GIB)]);
        let warning = status.warning.expect("warning");
        assert!(warning.contains("/work"), "{warning}");
        assert!(warning.contains("357.0 MiB"), "{warning}");
        assert!(warning.contains("worktree.gc_build_artifacts"), "{warning}");
    }

    /// Issue #4009 AC-4: the percentage threshold catches a large volume
    /// that still has more than the byte floor free.
    #[test]
    fn volume_below_percent_threshold_warns() {
        // 4% free of 2 TiB is ~82 GiB: above the byte floor, below the percent floor.
        let status = evaluate(vec![volume("/work", 82 * GIB, 2048 * GIB)]);
        assert!(status.warning.is_some());
    }

    /// Issue #4009 AC-4: exactly at the thresholds is still healthy; the
    /// warning is for "below".
    #[test]
    fn volume_at_the_thresholds_does_not_warn() {
        let status = evaluate(vec![volume("/work", 20 * GIB, 100 * GIB)]);
        assert_eq!(status.warning, None);
    }

    /// Issue #4009 AC-4: the coordinator root may live on another volume than
    /// the worktrees; any low volume warns.
    #[test]
    fn any_low_volume_among_several_warns() {
        let status = evaluate(vec![
            volume("/work", 500 * GIB, 3600 * GIB),
            volume("/home/.gwt/runtime", GIB, 100 * GIB),
        ]);
        let warning = status.warning.expect("warning");
        assert!(warning.contains("/home/.gwt/runtime"), "{warning}");
        assert!(!warning.contains("/work has"), "{warning}");
    }

    #[test]
    fn probe_reports_the_volume_of_an_existing_path_and_skips_missing_ones() {
        let existing = std::env::temp_dir();
        let missing = existing.join("gwt-disk-space-missing-path-4009");
        let status = probe(&[existing.as_path(), missing.as_path()]);
        assert_eq!(status.volumes.len(), 1, "{status:?}");
        assert!(status.volumes[0].total_bytes > 0);
    }

    #[test]
    fn format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(20 * GIB), "20.0 GiB");
    }
}
