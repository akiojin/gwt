//! Bounded, no-follow reads for security-sensitive local state.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::Path,
};

/// An already-opened regular file whose reads cannot cross `max_bytes`.
///
/// The handle is acquired with the platform no-follow flag before its type and
/// size are inspected. Keeping this handle through consumption removes the
/// path-check/open gap that otherwise permits a symlink or reparse-point swap.
pub struct BoundedRegularFile {
    file: File,
    remaining: u64,
    max_bytes: u64,
    byte_len: u64,
    description: String,
}

impl BoundedRegularFile {
    /// Open a regular file without following a symlink or reparse point.
    pub fn open(path: &Path, max_bytes: u64, description: &str) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{description} must be a regular file"),
            ));
        }
        if metadata.len() > max_bytes {
            return Err(size_limit_error(description, max_bytes));
        }
        Ok(Self {
            file,
            remaining: max_bytes,
            max_bytes,
            byte_len: metadata.len(),
            description: description.to_string(),
        })
    }

    /// Size observed from the opened handle before content consumption.
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Consume this exact handle into memory while retaining the open-time cap.
    pub fn read_all(self) -> io::Result<Vec<u8>> {
        let max_bytes = self.max_bytes;
        self.read_all_with_limit(max_bytes)
    }

    /// Consume this exact handle with a stricter late-bound aggregate cap.
    ///
    /// Callers can inspect several handles first, compute one aggregate
    /// budget, and then lower each handle's read allowance without reopening
    /// the path. Growth after the metadata preflight is caught by the bounded
    /// stream probe before allocation can cross `max_bytes`.
    pub fn read_all_with_limit(mut self, max_bytes: u64) -> io::Result<Vec<u8>> {
        let max_bytes = max_bytes.min(self.max_bytes);
        if self.byte_len > max_bytes {
            return Err(size_limit_error(&self.description, max_bytes));
        }
        self.max_bytes = max_bytes;
        self.remaining = max_bytes;
        let capacity = usize::try_from(self.byte_len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} size cannot be represented", self.description),
            )
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        self.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

impl Read for BoundedRegularFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0_u8; 1];
            return match self.file.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(size_limit_error(&self.description, self.max_bytes)),
            };
        }
        let allowed = usize::try_from(self.remaining.min(buffer.len() as u64))
            .expect("allowed read is bounded by the caller buffer");
        let read = self.file.read(&mut buffer[..allowed])?;
        self.remaining = self.remaining.saturating_sub(read as u64);
        Ok(read)
    }
}

/// Read one regular file through a single bounded no-follow handle.
pub fn read_bounded_regular_file(
    path: &Path,
    max_bytes: u64,
    description: &str,
) -> io::Result<Vec<u8>> {
    BoundedRegularFile::open(path, max_bytes, description)?.read_all()
}

fn size_limit_error(description: &str, max_bytes: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{description} exceeds the {max_bytes} byte size limit"),
    )
}
