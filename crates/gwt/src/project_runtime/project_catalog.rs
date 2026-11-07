//! Candidate-only host catalog. Endpoint contents and process liveness stay fresh.

use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use gwt_core::daemon::{DaemonEndpoint, RuntimeTarget};
use serde::{Deserialize, Serialize};

const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Serialize)]
pub(crate) struct ProjectCatalogMetrics {
    pub cache_hit: bool,
    pub directory_reads: usize,
    pub entries_enumerated: usize,
    pub endpoints_read: usize,
    pub eligible_roots: usize,
    pub elapsed_ms: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DirectoryStamp {
    path: PathBuf,
    modified: SystemTime,
}

impl DirectoryStamp {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "catalog directory replaced",
            ));
        }
        Ok(Self {
            path: path.into(),
            modified: metadata.modified()?,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u32,
    created_at: SystemTime,
    projects_dir: PathBuf,
    directories: Vec<DirectoryStamp>,
    endpoints: Vec<PathBuf>,
}

impl Catalog {
    fn is_current(&self, projects_dir: &Path, now: SystemTime) -> bool {
        if self.schema_version != 1
            || self.projects_dir != projects_dir
            || !now
                .duration_since(self.created_at)
                .is_ok_and(|age| age < CACHE_TTL)
            || self
                .directories
                .first()
                .is_none_or(|stamp| stamp.path != projects_dir)
        {
            return false;
        }
        let watched: HashSet<&Path> = self
            .directories
            .iter()
            .map(|stamp| stamp.path.as_path())
            .collect();
        if watched.len() != self.directories.len()
            || self.endpoints.iter().any(|path| {
                !managed_relative_path(projects_dir, path, true)
                    || path.parent().is_none_or(|parent| !watched.contains(parent))
            })
        {
            return false;
        }
        self.directories.iter().all(|stamp| {
            (stamp.path == projects_dir || managed_relative_path(projects_dir, &stamp.path, false))
                && DirectoryStamp::read(&stamp.path)
                    .is_ok_and(|current| current.modified == stamp.modified)
        })
    }
}

fn managed_relative_path(projects_dir: &Path, path: &Path, endpoint: bool) -> bool {
    let Ok(relative) = path.strip_prefix(projects_dir) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    if components
        .iter()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let length = components.len();
    (if endpoint {
        length == 4
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
    } else {
        (1..=3).contains(&length)
    }) && (length < 2 || components[1].as_os_str() == "runtime")
        && (length < 3 || components[2].as_os_str() == "daemon")
}

/// Watch the deepest existing ancestor: a first daemon in an existing empty
/// store must invalidate the cache just as adding a new store does.
fn project_stamp(project: &Path) -> io::Result<Option<DirectoryStamp>> {
    for path in [
        project.join("runtime/daemon"),
        project.join("runtime"),
        project.into(),
    ] {
        match DirectoryStamp::read(&path) {
            Ok(stamp) => return Ok(Some(stamp)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn scan(
    projects_dir: &Path,
    now: SystemTime,
    metrics: &mut ProjectCatalogMetrics,
) -> io::Result<Catalog> {
    let mut catalog = Catalog {
        schema_version: 1,
        created_at: now,
        projects_dir: projects_dir.into(),
        directories: Vec::new(),
        endpoints: Vec::new(),
    };
    let root = match DirectoryStamp::read(projects_dir) {
        Ok(stamp) => stamp,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(catalog),
        Err(error) => return Err(error),
    };
    catalog.directories.push(root);
    metrics.directory_reads += 1;
    for entry in fs::read_dir(projects_dir)? {
        let entry = entry?;
        metrics.entries_enumerated += 1;
        if !entry.file_type()?.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Some(stamp) = project_stamp(&entry.path())? else {
            continue;
        };
        let has_daemons = stamp.path == entry.path().join("runtime/daemon");
        let daemon_dir = stamp.path.clone();
        catalog.directories.push(stamp);
        if !has_daemons {
            continue;
        }
        metrics.directory_reads += 1;
        match fs::read_dir(daemon_dir) {
            Ok(entries) => {
                for descriptor in entries {
                    let descriptor = descriptor?;
                    metrics.entries_enumerated += 1;
                    let path = descriptor.path();
                    if descriptor.file_type()?.is_file()
                        && path
                            .extension()
                            .is_some_and(|extension| extension == "json")
                    {
                        catalog.endpoints.push(path);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    catalog.endpoints.sort();
    Ok(catalog)
}

pub(crate) fn owner_host_project_catalog() -> io::Result<(Vec<PathBuf>, ProjectCatalogMetrics)> {
    let home = gwt_core::paths::gwt_home();
    host_project_catalog(
        &gwt_core::paths::gwt_projects_dir(),
        &home.join("cache/host-project-catalog.json"),
        SystemTime::now(),
        crate::process::is_process_alive,
    )
}

fn host_project_catalog(
    projects_dir: &Path,
    cache_path: &Path,
    now: SystemTime,
    is_alive: impl Fn(u32) -> bool,
) -> io::Result<(Vec<PathBuf>, ProjectCatalogMetrics)> {
    let started = Instant::now();
    let mut metrics = ProjectCatalogMetrics::default();
    let cached = fs::read(cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Catalog>(&bytes).ok())
        .filter(|catalog| catalog.is_current(projects_dir, now));
    metrics.cache_hit = cached.is_some();
    let catalog = match cached {
        Some(catalog) => catalog,
        None => {
            let catalog = scan(projects_dir, now, &mut metrics)?;
            // Cache IO is never authority: an unwritable/corrupt cache simply
            // costs another scan. Atomic publication permits concurrent readers.
            if let Ok(bytes) = serde_json::to_vec(&catalog) {
                let _ = gwt_github::cache::write_atomic_with_durability(
                    cache_path,
                    &bytes,
                    gwt_github::cache::Durability::RenameOnly,
                );
            }
            catalog
        }
    };
    let mut roots = Vec::new();
    for path in &catalog.endpoints {
        metrics.endpoints_read += 1;
        let Some(endpoint) = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<DaemonEndpoint>(&bytes).ok())
        else {
            continue;
        };
        // Never resolve identity for a dead fixture's missing root. Do not
        // reject valid live temporary clones just because of their path name.
        if endpoint.scope.target == RuntimeTarget::Host
            && endpoint.scope.project_root.is_dir()
            && is_alive(endpoint.pid)
        {
            roots.push(endpoint.scope.project_root);
        }
    }
    roots.sort();
    roots.dedup();
    metrics.eligible_roots = roots.len();
    metrics.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((roots, metrics))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::daemon::RuntimeScope;

    fn endpoint(project: &Path, root: &Path, pid: u32) -> PathBuf {
        let scope = RuntimeScope::new("fixture", "host", root.into(), RuntimeTarget::Host).unwrap();
        let path = project.join("runtime/daemon/host.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&DaemonEndpoint::new(
                scope,
                pid,
                "test".into(),
                "test".into(),
                "test".into(),
            ))
            .unwrap(),
        )
        .unwrap();
        path
    }

    #[test]
    fn catalog_cache_keeps_endpoint_contents_and_liveness_fresh() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let cache = temp.path().join("cache.json");
        let root = temp.path().join("root");
        let other = temp.path().join("other");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&other).unwrap();
        let path = endpoint(&projects.join("fixture"), &root, 7);
        let now = SystemTime::now();
        let (roots, cold) = host_project_catalog(&projects, &cache, now, |pid| pid == 7).unwrap();
        assert_eq!(roots, vec![dunce::canonicalize(&root).unwrap()]);
        assert!(!cold.cache_hit);
        let mut changed: DaemonEndpoint =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        changed.pid = 8;
        changed.scope.project_root = dunce::canonicalize(&other).unwrap();
        // In-place bytes can change without changing the directory entry set.
        fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let (roots, warm) = host_project_catalog(&projects, &cache, now, |pid| pid == 8).unwrap();
        assert!(warm.cache_hit);
        assert_eq!(warm.directory_reads, 0);
        assert_eq!(warm.entries_enumerated, 0);
        assert_eq!(roots, vec![dunce::canonicalize(&other).unwrap()]);
        let (roots, warm) = host_project_catalog(&projects, &cache, now, |_| false).unwrap();
        assert!(warm.cache_hit);
        assert!(
            roots.is_empty(),
            "a cached candidate cannot keep a dead Host alive"
        );
        let (_, expired) =
            host_project_catalog(&projects, &cache, now + CACHE_TTL, |_| true).unwrap();
        assert!(!expired.cache_hit);
        fs::write(&cache, b"{").unwrap();
        let (roots, repaired) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(!repaired.cache_hit);
        assert_eq!(roots, vec![dunce::canonicalize(other).unwrap()]);
    }

    #[test]
    fn catalog_detects_first_daemon_and_new_store_without_waiting_for_ttl() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let cache = temp.path().join("cache.json");
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(projects.join("empty")).unwrap();
        let now = SystemTime::now();
        let (roots, _) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(roots.is_empty());
        let cached: Catalog = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        assert!(cached
            .directories
            .iter()
            .any(|stamp| stamp.path == projects.join("empty")));
        endpoint(&projects.join("empty"), &root, 1);
        let (roots, refreshed) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(!refreshed.cache_hit);
        assert_eq!(roots, vec![dunce::canonicalize(&root).unwrap()]);
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        endpoint(&projects.join("new"), &other, 2);
        let (roots, refreshed) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(!refreshed.cache_hit);
        assert_eq!(roots.len(), 2);
    }

    #[test]
    fn thirteen_thousand_projects_reuse_listing_and_exclude_removed_test_roots() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let cache = temp.path().join("cache.json");
        fs::create_dir(&projects).unwrap();
        for index in 0..13_000 {
            let project = projects.join(format!("{index:016x}"));
            fs::create_dir(&project).unwrap();
            if index % 4 == 0 {
                endpoint(&project, &temp.path().join(format!("removed-{index}")), 1);
            }
        }
        let now = SystemTime::now();
        // The pre-fix catalog walk, retained only as a same-run benchmark.
        let before = Instant::now();
        let (mut directory_reads, mut entries, mut endpoints_read) = (1, 0, 0);
        let mut legacy_roots = Vec::new();
        for project in fs::read_dir(&projects).unwrap() {
            let project = project.unwrap();
            entries += 1;
            directory_reads += 1;
            let descriptors = match fs::read_dir(project.path().join("runtime/daemon")) {
                Ok(descriptors) => descriptors,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => panic!("legacy catalog fixture: {error}"),
            };
            for descriptor in descriptors {
                let path = descriptor.unwrap().path();
                entries += 1;
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                endpoints_read += 1;
                let endpoint: DaemonEndpoint =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                if endpoint.scope.target == RuntimeTarget::Host {
                    legacy_roots.push(endpoint.scope.project_root);
                }
            }
        }
        println!(
            "Issue #6809 catalog before: {}",
            serde_json::json!({
                "directory_reads": directory_reads, "entries_enumerated": entries,
                "endpoints_read": endpoints_read, "eligible_roots": legacy_roots.len(),
                "elapsed_ms": before.elapsed().as_secs_f64() * 1000.0,
            })
        );
        assert_eq!(legacy_roots.len(), 3_250);
        let (roots, cold) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(roots.is_empty());
        assert_eq!(cold.entries_enumerated, 13_000 + 3_250);
        assert_eq!(cold.endpoints_read, 3_250);
        let (roots, warm) = host_project_catalog(&projects, &cache, now, |_| true).unwrap();
        assert!(roots.is_empty());
        assert!(warm.cache_hit);
        assert_eq!(warm.directory_reads, 0);
        assert_eq!(warm.entries_enumerated, 0);
        assert_eq!(warm.endpoints_read, 3_250);
        assert_eq!(warm.eligible_roots, 0);
        println!(
            "Issue #6809 catalog cold: {}",
            serde_json::to_string(&cold).unwrap()
        );
        println!(
            "Issue #6809 catalog warm: {}",
            serde_json::to_string(&warm).unwrap()
        );
    }
}
