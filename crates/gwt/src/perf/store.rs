use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

use chrono::{NaiveDate, Utc};
use gwt_core::{logging::housekeep::housekeep_at, paths::gwt_logs_dir};

use super::record::PerfRecord;

const PERF_FILE_NAME_PREFIX: &str = "perf-";
const PERF_DATE_SUFFIX_FORMAT: &str = "%Y-%m-%d.jsonl";

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
}

impl<C: UtcDateClock> PerfStore<C> {
    fn with_clock(retention_days: u32, clock: C) -> io::Result<Self> {
        let log_dir = gwt_logs_dir().join("perf");
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
