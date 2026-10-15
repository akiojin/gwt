//! Incremental parsing of local provider JSONL files.

use std::{
    collections::HashMap,
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(crate) struct JsonlCache<T> {
    files: HashMap<PathBuf, CachedFile<T>>,
    bytes_read: u64,
}

impl<T> Default for JsonlCache<T> {
    fn default() -> Self {
        Self {
            files: HashMap::new(),
            bytes_read: 0,
        }
    }
}

impl<T: Default> JsonlCache<T> {
    /// Cache parsed state, retaining only the last unterminated line as text.
    /// Provider JSONL files are append-only; a changed length on the same file
    /// identity is treated as append unless the file became shorter.
    pub(crate) fn read(&mut self, path: &Path, mut apply: impl FnMut(&mut T, &str)) -> Option<&T> {
        // Remove first so failures cannot return an old aggregate. Open even
        // unchanged files to check that they remain readable.
        let previous = self.files.remove(path);
        let mut file = File::open(path).ok()?;
        let version = FileVersion::read(&file).ok()?;
        let mut cached = match previous {
            Some(previous) if previous.version == version && version.modified.is_some() => {
                self.files.insert(path.to_path_buf(), previous);
                return self.files.get(path).map(|cached| &cached.value);
            }
            Some(previous)
                if previous.version.same_file(&version) && version.len > previous.offset =>
            {
                previous
            }
            _ => CachedFile::new(version),
        };
        if !cached
            .read_to(&mut file, version.len, &mut self.bytes_read, &mut apply)
            .ok()?
        {
            // A previously valid JSON value without a newline was extended
            // (for example `1` became `12`). Rebuild rather than count both.
            cached = CachedFile::new(version);
            cached
                .read_to(&mut file, version.len, &mut self.bytes_read, &mut apply)
                .ok()?;
        }
        cached.version = version;
        self.files.insert(path.to_path_buf(), cached);
        self.files.get(path).map(|cached| &cached.value)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        self.files.retain(|path, _| keep(path));
    }

    pub(crate) fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileVersion {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    identity: (u64, u64),
}

impl FileVersion {
    fn read(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            identity: file_identity(file, &metadata)?,
        })
    }

    fn same_file(&self, other: &Self) -> bool {
        self.identity == other.identity && self.created == other.created
    }
}

#[cfg(unix)]
fn file_identity(_file: &File, metadata: &Metadata) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File, _metadata: &Metadata) -> io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the live File owns the handle and information is valid output storage.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(io::Error::other)?;
    Ok((
        u64::from(information.dwVolumeSerialNumber),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File, _metadata: &Metadata) -> io::Result<(u64, u64)> {
    Ok((0, 0))
}

struct CachedFile<T> {
    version: FileVersion,
    offset: u64,
    value: T,
    tail: Vec<u8>,
    tail_applied: bool,
}

impl<T: Default> CachedFile<T> {
    fn new(version: FileVersion) -> Self {
        Self {
            version,
            offset: 0,
            value: T::default(),
            tail: Vec::new(),
            tail_applied: false,
        }
    }

    /// Return false when an already applied final value needs rebuilding.
    fn read_to(
        &mut self,
        file: &mut File,
        end: u64,
        bytes_read: &mut u64,
        apply: &mut impl FnMut(&mut T, &str),
    ) -> io::Result<bool> {
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = file.take(end - self.offset);
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            *bytes_read += count as u64;
            self.offset += count as u64;
            for part in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
                if self.tail_applied && part.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    return Ok(false);
                }
                self.tail.extend_from_slice(part);
                if part.last() == Some(&b'\n') {
                    if !self.tail_applied {
                        let line = std::str::from_utf8(&self.tail)
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                        apply(&mut self.value, line.trim_end_matches(['\r', '\n']));
                    }
                    self.tail.clear();
                    self.tail_applied = false;
                }
            }
        }
        if self.offset != end {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "file truncated",
            ));
        }
        let tail = match std::str::from_utf8(&self.tail) {
            Ok(tail) => tail,
            // A provider can pause in the middle of a UTF-8 character too.
            Err(error) if error.error_len().is_none() => return Ok(true),
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        };
        // Existing provider readers accept a complete final JSON value even
        // without a newline. Defer incomplete JSON until later appended bytes.
        if !self.tail_applied && serde_json::from_str::<serde_json::Value>(tail).is_ok() {
            apply(&mut self.value, tail);
            self.tail_applied = true;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File, FileTimes, OpenOptions},
        io::Write,
        path::Path,
        time::{Duration, SystemTime},
    };

    use super::JsonlCache;

    fn append(path: &Path, text: impl AsRef<[u8]>) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_ref())
            .unwrap();
    }

    fn read(cache: &mut JsonlCache<Vec<u64>>, path: &Path) -> Option<Vec<u64>> {
        cache
            .read(path, |values, line| {
                if let Ok(value) = serde_json::from_str::<u64>(line) {
                    values.push(value);
                }
            })
            .cloned()
    }

    #[test]
    fn unchanged_files_read_no_text_and_appends_read_only_new_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, "1\n").unwrap();
        let mut cache = JsonlCache::default();

        assert_eq!(read(&mut cache, &path), Some(vec![1]));
        assert_eq!(cache.bytes_read(), 2);
        assert_eq!(read(&mut cache, &path), Some(vec![1]));
        assert_eq!(cache.bytes_read(), 2);

        append(&path, "2\n3");
        assert_eq!(read(&mut cache, &path), Some(vec![1, 2, 3]));
        assert_eq!(cache.bytes_read(), 5);
        append(&path, " \r\n4\n");
        assert_eq!(read(&mut cache, &path), Some(vec![1, 2, 3, 4]));
        assert_eq!(cache.bytes_read(), 10);
    }

    #[test]
    fn incomplete_tail_is_completed_once_without_rereading_old_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, b"1\n{\"n\":2,\"label\":\"\xc3").unwrap();
        let mut cache = JsonlCache::<Vec<u64>>::default();
        let apply = |values: &mut Vec<u64>, line: &str| {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                values.push(value.as_u64().or_else(|| value["n"].as_u64()).unwrap());
            }
        };

        assert_eq!(cache.read(&path, apply), Some(&vec![1]));
        append(&path, b"\xa9\"}");
        assert_eq!(cache.read(&path, apply), Some(&vec![1, 2]));
        assert_eq!(cache.read(&path, apply), Some(&vec![1, 2]));
        append(&path, "\n3\n");
        assert_eq!(cache.read(&path, apply), Some(&vec![1, 2, 3]));
        assert_eq!(cache.bytes_read(), fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn extending_an_applied_unterminated_value_rebuilds_the_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, "1").unwrap();
        let mut cache = JsonlCache::default();

        assert_eq!(read(&mut cache, &path), Some(vec![1]));
        append(&path, "2\n");
        assert_eq!(read(&mut cache, &path), Some(vec![12]));
    }

    #[test]
    fn truncation_same_size_rewrite_and_replacement_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, "1\n2\n").unwrap();
        let mut cache = JsonlCache::default();
        assert_eq!(read(&mut cache, &path), Some(vec![1, 2]));

        fs::write(&path, "3\n").unwrap();
        let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old_time))
            .unwrap();
        assert_eq!(read(&mut cache, &path), Some(vec![3]));

        fs::write(&path, "4\n").unwrap();
        assert_eq!(read(&mut cache, &path), Some(vec![4]));

        let replacement = dir.path().join("replacement.jsonl");
        fs::write(&replacement, "5\n").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        fs::remove_file(&path).unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(read(&mut cache, &path), Some(vec![5]));
    }

    #[test]
    fn missing_non_file_and_invalid_utf8_do_not_serve_stale_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, "1\n").unwrap();
        let mut cache = JsonlCache::default();
        assert_eq!(read(&mut cache, &path), Some(vec![1]));
        fs::remove_file(&path).unwrap();
        assert_eq!(read(&mut cache, &path), None);
        fs::create_dir(&path).unwrap();
        assert_eq!(read(&mut cache, &path), None);
        fs::remove_dir(&path).unwrap();
        fs::write(&path, "2\n").unwrap();
        assert_eq!(read(&mut cache, &path), Some(vec![2]));
        append(&path, "3\n");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[0xff])
            .unwrap();
        assert_eq!(read(&mut cache, &path), None);
    }
}
