//! Shared path classification, not delivery authorization. Each gate keeps its
//! own provenance, commit, and verification requirements for these categories.

// Fingerprints exclude only gwt bookkeeping; TaskNotes remain verification
// inputs there even though matrix derivation excludes those local plans.
pub(crate) const BOOKKEEPING_GIT_EXCLUDE: &str = ":(exclude).gwt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryPath {
    Product,
    TaskNotes,
    Bookkeeping,
    WorkEventShard,
}

/// Git paths use forward slashes. Classification alone cannot prove that a
/// shard was written by a trusted producer or that its contents are immutable.
/// Unknown paths are product inputs, including crates/, web/, scripts/, CI,
/// manifests, and skills; an allowlist would silently miss new source roots.
pub(crate) fn classify_path(path: &[u8]) -> DeliveryPath {
    if path.starts_with(b".gwt/") {
        if is_canonical_work_event_shard(path) {
            DeliveryPath::WorkEventShard
        } else {
            DeliveryPath::Bookkeeping
        }
    } else if path.starts_with(b"tasks/") {
        DeliveryPath::TaskNotes
    } else {
        DeliveryPath::Product
    }
}

fn is_canonical_work_event_shard(path: &[u8]) -> bool {
    let Some(suffix) = path.strip_prefix(b".gwt/work/events/") else {
        return false;
    };
    let mut components = suffix.split(|byte| *byte == b'/');
    let (Some(bucket), Some(file_name)) = (components.next(), components.next()) else {
        return false;
    };
    let Some(digest) = file_name.strip_suffix(b".jsonl") else {
        return false;
    };
    components.next().is_none()
        && bucket.len() == 2
        && digest.len() == 64
        && digest
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
        && bucket == &digest[..2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_shards_have_a_distinct_class() {
        let digest = "ab".repeat(32);
        let canonical = format!(".gwt/work/events/ab/{digest}.jsonl");
        assert_eq!(
            classify_path(canonical.as_bytes()),
            DeliveryPath::WorkEventShard
        );
        for malformed in [
            format!(".gwt/work/events/cd/{digest}.jsonl"),
            format!(".gwt/work/events/ab/{digest}.jsonl/extra"),
            ".gwt/work/events/ab/short.jsonl".to_string(),
            format!(".gwt/work/events/AB/{}.jsonl", digest.to_uppercase()),
        ] {
            assert_eq!(
                classify_path(malformed.as_bytes()),
                DeliveryPath::Bookkeeping
            );
        }
    }

    #[test]
    fn bookkeeping_and_task_notes_do_not_hide_product_prefix_collisions() {
        assert_eq!(
            classify_path(b".gwt/work/events.jsonl"),
            DeliveryPath::Bookkeeping
        );
        assert_eq!(classify_path(b"tasks/todo.md"), DeliveryPath::TaskNotes);
        for product in [
            b".gwt-other/file".as_slice(),
            b"nested/.gwt/file",
            b"tasks.rs",
            b"tasks-other/file",
            b"src/main.rs",
        ] {
            assert_eq!(classify_path(product), DeliveryPath::Product);
        }
    }
}
