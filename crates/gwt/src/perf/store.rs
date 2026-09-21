use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use chrono::{NaiveDate, Utc};
use fs2::FileExt;
use gwt_core::{logging::housekeep::housekeep_at, paths::gwt_logs_dir};

use super::{
    record::{PerfRecord, PerfViolationDetails},
    smoothing::ViolationSmoother,
};

const PERF_FILE_NAME_PREFIX: &str = "perf-";
const PERF_DATE_SUFFIX_FORMAT: &str = "%Y-%m-%d.jsonl";

/// Longest a sample waits for another collector's detector update before it
/// is written unmarked.
const DETECTOR_LOCK_WAIT: Duration = Duration::from_millis(25);

pub(crate) trait UtcDateClock {
    fn today_utc(&self) -> NaiveDate;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemUtcDateClock;

impl UtcDateClock for SystemUtcDateClock {
    fn today_utc(&self) -> NaiveDate {
        Utc::now().date_naive()
    }
}

pub(crate) struct PerfStore<C = SystemUtcDateClock> {
    log_dir: PathBuf,
    clock: C,
    open_date: Option<NaiveDate>,
    file: Option<File>,
}

impl PerfStore<SystemUtcDateClock> {
    pub(crate) fn new(retention_days: u32) -> io::Result<Self> {
        Self::with_clock(retention_days, SystemUtcDateClock)
    }

    /// Open the perf log only when it has already been established.
    ///
    /// Issue #4145: the GUI is the always-on collector and owns creating
    /// `~/.gwt/logs/perf/`. A short-lived `gwtd` invocation only appends to a
    /// log that already exists, so running an operation against a hermetic
    /// container HOME leaves that HOME byte-identical — the property
    /// `crates/gwt/tests/workspace_cli_test.rs` asserts for every forwarded
    /// `workspace.update`.
    pub(crate) fn open_established(retention_days: u32) -> io::Result<Option<Self>> {
        if !perf_log_dir().is_dir() {
            return Ok(None);
        }
        Self::new(retention_days).map(Some)
    }
}

fn perf_log_dir() -> PathBuf {
    gwt_logs_dir().join("perf")
}

impl<C: UtcDateClock> PerfStore<C> {
    fn with_clock(retention_days: u32, clock: C) -> io::Result<Self> {
        let log_dir = perf_log_dir();
        fs::create_dir_all(&log_dir)?;
        let today = clock.today_utc();
        let _housekeep_report = housekeep_at(
            &log_dir,
            retention_days,
            PERF_FILE_NAME_PREFIX,
            PERF_DATE_SUFFIX_FORMAT,
            today,
        );

        Ok(Self {
            log_dir,
            clock,
            open_date: None,
            file: None,
        })
    }

    pub(crate) fn append(&mut self, record: &PerfRecord) -> io::Result<()> {
        let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
        line.push(b'\n');

        let date = self.clock.today_utc();
        self.file_for_date(date)?.write_all(&line)
    }

    /// Share detection across GUI and short-lived CLI processes. A detector
    /// that stays busy past `DETECTOR_LOCK_WAIT`, or is unavailable, never
    /// holds the caller longer: its sample remains unmarked so readers can see
    /// that detection was not performed.
    pub(crate) fn append_budgeted(&mut self, sample: &PerfRecord, budget: f64) -> io::Result<()> {
        let Ok(_lock) = self.lock_detector() else {
            return self.append(sample);
        };

        let Ok(details) = self.observe_shared(sample, budget) else {
            return self.append(sample);
        };
        let sample = sample.clone().with_shared_detection();
        self.append(&sample)?;
        if let Some(details) = details {
            self.append(&sample.as_violation(details))?;
        }
        Ok(())
    }

    /// Contention is one small state read and write, or a child process forked
    /// while the lock was held and not yet exec'd; both clear within
    /// milliseconds, so a single non-blocking attempt would drop detection.
    fn lock_detector(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.log_dir.join("detector.lock"))?;
        let deadline = Instant::now() + DETECTOR_LOCK_WAIT;
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    fn observe_shared(
        &self,
        sample: &PerfRecord,
        budget: f64,
    ) -> io::Result<Option<PerfViolationDetails>> {
        let path = self.log_dir.join("detector-state.json");
        let mut bytes = Vec::new();
        match File::open(&path) {
            Ok(file) => {
                file.take(256 * 1024).read_to_end(&mut bytes)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut smoother = ViolationSmoother::restore(&bytes);
        let details = smoother.observe(&sample.target, sample.value, budget, sample.timestamp);
        let snapshot = smoother.snapshot().map_err(io::Error::other)?;
        // A separate lock survives atomic replacement of the state file. No
        // historical JSONL is replayed or changed on this hot path.
        if snapshot != bytes {
            let mut temp = tempfile::NamedTempFile::new_in(&self.log_dir)?;
            temp.write_all(&snapshot)?;
            temp.persist(path).map_err(|error| error.error)?;
        }
        Ok(details)
    }

    fn file_for_date(&mut self, date: NaiveDate) -> io::Result<&mut File> {
        if self.open_date != Some(date) {
            let file_name = format!(
                "{PERF_FILE_NAME_PREFIX}{}",
                date.format(PERF_DATE_SUFFIX_FORMAT)
            );
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_dir.join(file_name))?;
            self.file = Some(file);
            self.open_date = Some(date);
        }

        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("perf daily file was not opened"))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, path::Path, rc::Rc};

    use chrono::{DateTime, NaiveDate, Utc};
    use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};

    use super::{PerfStore, UtcDateClock};
    use crate::perf::record::{PerfRecord, PerfStream, PerfUnit};

    #[derive(Clone)]
    struct FixedUtcDateClock {
        today: Rc<Cell<NaiveDate>>,
    }

    impl FixedUtcDateClock {
        fn new(today: NaiveDate) -> Self {
            Self {
                today: Rc::new(Cell::new(today)),
            }
        }

        fn set(&self, today: NaiveDate) {
            self.today.set(today);
        }
    }

    impl UtcDateClock for FixedUtcDateClock {
        fn today_utc(&self) -> NaiveDate {
            self.today.get()
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    fn timestamp(day: NaiveDate) -> DateTime<Utc> {
        day.and_hms_opt(12, 0, 0)
            .expect("valid test timestamp")
            .and_utc()
    }

    fn sample(day: NaiveDate, target: &str, value: f64) -> PerfRecord {
        PerfRecord::sample(
            timestamp(day),
            PerfStream::Op,
            target,
            value,
            PerfUnit::Milliseconds,
        )
    }

    fn read_json_lines(path: &Path) -> Vec<serde_json::Value> {
        fs::read_to_string(path)
            .expect("read perf log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("valid JSONL record"))
            .collect()
    }

    #[test]
    fn an_in_budget_sample_in_another_store_resets_the_shared_run() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let day = date(2026, 9, 14);
        let path = gwt_logs_dir().join("perf/perf-2026-09-14.jsonl");
        for (step, value) in [5000.0, 10.0, 5000.0, 5000.0, 5000.0]
            .into_iter()
            .enumerate()
        {
            let mut store = PerfStore::with_clock(30, FixedUtcDateClock::new(day)).expect("store");
            store
                .append_budgeted(&sample(day, "route:search", value), 2000.0)
                .expect("record");
            let violations = read_json_lines(&path)
                .iter()
                .filter(|r| r["type"] == "violation")
                .count();
            assert_eq!(violations, usize::from(step == 4));
        }
    }

    /// Issue #4292 CI: a lock held for a moment (a concurrent collector, or a
    /// forked child that has not exec'd yet) must not drop detection.
    #[test]
    fn a_briefly_held_detector_still_evaluates_the_sample() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let day = date(2026, 9, 15);
        let mut store = PerfStore::with_clock(30, FixedUtcDateClock::new(day)).expect("store");
        let lock = fs::File::create(gwt_logs_dir().join("perf/detector.lock")).expect("lock file");
        fs2::FileExt::lock_exclusive(&lock).expect("hold detector");
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2));
            drop(lock);
        });

        store
            .append_budgeted(&sample(day, "route:search", 5000.0), 2000.0)
            .expect("record");
        release.join().expect("release the detector");

        let records = read_json_lines(&gwt_logs_dir().join("perf/perf-2026-09-15.jsonl"));
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]["detector_version"], 1,
            "a detector released within the wait bound must still evaluate the sample"
        );
    }

    #[test]
    fn appends_records_to_the_fixed_utc_daily_file() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let today = date(2026, 4, 10);
        let mut store =
            PerfStore::with_clock(30, FixedUtcDateClock::new(today)).expect("create store");

        store
            .append(&sample(today, "gwtd:issue.view", 12.5))
            .expect("append first record");
        store
            .append(&sample(today, "gwtd:issue.list", 8.0))
            .expect("append second record");

        let path = gwt_logs_dir().join("perf").join("perf-2026-04-10.jsonl");
        let records = read_json_lines(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["target"], "gwtd:issue.view");
        assert_eq!(records[1]["target"], "gwtd:issue.list");
    }

    #[test]
    fn switches_daily_files_when_the_injected_utc_date_changes() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let first_day = date(2026, 4, 10);
        let second_day = date(2026, 4, 11);
        let clock = FixedUtcDateClock::new(first_day);
        let mut store = PerfStore::with_clock(30, clock.clone()).expect("create store");

        store
            .append(&sample(first_day, "gwtd:issue.view", 12.5))
            .expect("append first-day record");
        clock.set(second_day);
        store
            .append(&sample(second_day, "gwtd:issue.view", 13.0))
            .expect("append second-day record");

        let perf_dir = gwt_logs_dir().join("perf");
        let first_records = read_json_lines(&perf_dir.join("perf-2026-04-10.jsonl"));
        let second_records = read_json_lines(&perf_dir.join("perf-2026-04-11.jsonl"));
        assert_eq!(first_records.len(), 1);
        assert_eq!(second_records.len(), 1);
        assert_eq!(first_records[0]["value"], 12.5);
        assert_eq!(second_records[0]["value"], 13.0);
    }

    #[test]
    fn runs_perf_housekeeping_once_when_the_store_starts() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let perf_dir = gwt_logs_dir().join("perf");
        fs::create_dir_all(&perf_dir).expect("create perf dir");
        fs::write(perf_dir.join("perf-2026-04-04.jsonl"), b"kept\n")
            .expect("write retention-boundary log");
        fs::write(perf_dir.join("perf-2026-04-03.jsonl"), b"expired\n").expect("write expired log");
        fs::write(perf_dir.join("unrelated-2026-04-03.jsonl"), b"unrelated\n")
            .expect("write unrelated log");

        let _store = PerfStore::with_clock(7, FixedUtcDateClock::new(date(2026, 4, 10)))
            .expect("create store");

        assert!(perf_dir.join("perf-2026-04-04.jsonl").exists());
        assert!(!perf_dir.join("perf-2026-04-03.jsonl").exists());
        assert!(perf_dir.join("unrelated-2026-04-03.jsonl").exists());
    }
}
