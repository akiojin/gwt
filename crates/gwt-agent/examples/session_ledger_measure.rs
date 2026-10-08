//! Read-only input, isolated before/after measurement for Issue #5025.
//!
//! cargo run -p gwt-agent --example session_ledger_measure -- <input> <new-output> [ticks]
//! Both loaders receive separate copies of the same TOML corpus. No production
//! directory is modified. The eager loader uses a harness mutex. The shared
//! cached loader emits cumulative actual cache-guard hold/wait and separate
//! wall sweep time. Metadata/readdir are outside the guards. Neither measures an
//! entire GUI tick or a PM/prefs file-lock acquisition.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, FileTimes},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use gwt_agent::{session_ledger::load_sessions, Session};
use serde::Serialize;
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

#[derive(Default, Serialize)]
struct LoadLogCounts {
    event_count: u64,
    errors: BTreeMap<String, u64>,
    levels: BTreeMap<String, u64>,
    paths: BTreeSet<String>,
    sweeps: Vec<Sweep>,
}

#[derive(Default, Clone, Serialize)]
struct Sweep {
    session_sweep_us: u64,
    session_scan_us: u64,
    cache_lock_wait_us: u64,
    parse_attempts: u64,
}

#[derive(Default)]
struct Fields {
    message: String,
    error: String,
    path: String,
    sweep: Sweep,
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "session_sweep_us" => self.sweep.session_sweep_us = value,
            "session_scan_us" => self.sweep.session_scan_us = value,
            "cache_lock_wait_us" => self.sweep.cache_lock_wait_us = value,
            "parse_attempts" => self.sweep.parse_attempts = value,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "error" => self.error = value.to_owned(),
            "path" => self.path = value.to_owned(),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{value:?}"));
    }
}

struct CountLoadLogs(Arc<Mutex<LoadLogCounts>>);

impl<S: Subscriber> Layer<S> for CountLoadLogs {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.message == "Session ledger sweep measured" {
            self.0.lock().unwrap().sweeps.push(fields.sweep);
            return;
        }
        if !fields.message.starts_with("Cannot load session") {
            return;
        }
        let mut counts = self.0.lock().unwrap();
        counts.event_count += 1;
        *counts.errors.entry(fields.error).or_default() += 1;
        *counts
            .levels
            .entry(event.metadata().level().to_string())
            .or_default() += 1;
        counts.paths.insert(fields.path);
    }
}

#[derive(Serialize)]
struct CorpusSize {
    toml_count: u64,
    toml_bytes: u64,
}

fn toml_paths(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("toml")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn corpus_size(dir: &Path) -> io::Result<CorpusSize> {
    let paths = toml_paths(dir)?;
    let mut bytes = 0;
    for path in &paths {
        bytes += fs::metadata(path)?.len();
    }
    Ok(CorpusSize {
        toml_count: paths.len() as u64,
        toml_bytes: bytes,
    })
}

fn copy_corpus(input: &Path, output: &Path) -> io::Result<()> {
    fs::create_dir(output)?;
    for path in toml_paths(input)? {
        let destination = output.join(path.file_name().unwrap());
        fs::copy(&path, &destination)?;
        // Keep temporary-file age comparable; do not make old residue look new.
        File::options()
            .write(true)
            .open(destination)?
            .set_times(FileTimes::new().set_modified(fs::metadata(path)?.modified()?))?;
    }
    Ok(())
}

#[derive(Serialize)]
struct Tick {
    index: u64,
    sessions: usize,
    parse_attempts: u64,
    loader_tick_us: u128,
    cache_mutex_hold_us: u128,
    cache_mutex_wait_us: u128,
}

#[derive(Serialize)]
struct Measurement {
    mutex_scope: &'static str,
    initial: CorpusSize,
    final_size: CorpusSize,
    load_logs: LoadLogCounts,
    ticks: Vec<Tick>,
}

fn measure(dir: &Path, ticks: u64, cached: bool) -> io::Result<Measurement> {
    let counts = Arc::new(Mutex::new(LoadLogCounts::default()));
    let subscriber = tracing_subscriber::registry().with(CountLoadLogs(Arc::clone(&counts)));
    let initial = corpus_size(dir)?;
    let eager_mutex = Mutex::new(());
    let mut measurements = Vec::new();
    tracing::subscriber::with_default(subscriber, || -> io::Result<()> {
        for index in 0..ticks {
            let tick_started = Instant::now();
            let (sessions, parse_attempts, held_us, wait_us) = if cached {
                let sessions = load_sessions(dir)?;
                let sweep = counts
                    .lock()
                    .unwrap()
                    .sweeps
                    .last()
                    .cloned()
                    .ok_or_else(|| io::Error::other("shared loader measurement event missing"))?;
                (
                    sessions,
                    sweep.parse_attempts,
                    sweep.session_scan_us as u128,
                    sweep.cache_lock_wait_us as u128,
                )
            } else {
                let guard = eager_mutex.lock().unwrap();
                let wait_us = tick_started.elapsed().as_micros();
                let held_started = Instant::now();
                let paths = toml_paths(dir)?;
                let attempts = paths.len() as u64;
                let sessions = paths
                    .iter()
                    .filter_map(|path| Session::load_and_migrate(path).ok())
                    .collect::<Vec<_>>();
                let held_us = held_started.elapsed().as_micros();
                drop(guard);
                (sessions, attempts, held_us, wait_us)
            };
            measurements.push(Tick {
                index,
                sessions: sessions.len(),
                parse_attempts,
                loader_tick_us: tick_started.elapsed().as_micros(),
                cache_mutex_hold_us: held_us,
                cache_mutex_wait_us: wait_us,
            });
        }
        Ok(())
    })?;
    let load_logs = std::mem::take(&mut *counts.lock().unwrap());
    Ok(Measurement {
        mutex_scope: if cached {
            "Sum of actual shared-cache guard hold durations, including cache pruning; excludes metadata/readdir, final debug event and guard drop. Wait is summed per acquisition; session_sweep_us separately records wall sweep duration."
        } else {
            "Uncontended harness mutex around eager readdir and load_and_migrate; representative loader-held section only"
        },
        initial,
        final_size: corpus_size(dir)?,
        load_logs,
        ticks: measurements,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input =
        PathBuf::from(args.next().ok_or("missing input corpus directory")?).canonicalize()?;
    let output = PathBuf::from(args.next().ok_or("missing new output directory")?);
    let ticks: u64 = args
        .next()
        .map(|value| value.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(125);
    if ticks == 0 || args.next().is_some() {
        return Err("usage: session_ledger_measure <input> <new-output> [positive-ticks]".into());
    }
    let output_parent = output
        .parent()
        .ok_or("output must have a parent")?
        .canonicalize()?;
    let output = output_parent.join(output.file_name().ok_or("output must have a name")?);
    if output.starts_with(&input) {
        return Err("output must be outside the read-only input corpus".into());
    }
    // Refuse replacement of existing evidence and create only isolated copies.
    fs::create_dir(&output)?;
    let before_dir = output.join("before");
    let after_dir = output.join("after");
    copy_corpus(&input, &before_dir)?;
    copy_corpus(&input, &after_dir)?;
    let before = measure(&before_dir, ticks, false)?;
    let after = measure(&after_dir, ticks, true)?;
    let result = serde_json::json!({
        "input": input,
        "output": output,
        "ticks_per_phase": ticks,
        "scope": "Separate identical TOML snapshots; eager load_and_migrate versus shared load_sessions. Before uses an uncontended harness mutex; after records cumulative actual shared-cache guard hold/wait and separate wall sweep time. Metadata/readdir are outside after guards. No wall-clock PASS threshold; no claim about complete GUI ticks, PM/prefs locks or launch stalls.",
        "before": before,
        "after": after,
    });
    let encoded = serde_json::to_string_pretty(&result)?;
    fs::write(output.join("measurement.json"), &encoded)?;
    println!("{encoded}");
    Ok(())
}
