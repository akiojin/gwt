use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::path_filter::{self, is_relative_denied, PathFilterError};

const DEFAULT_TEXT_LIMIT: u64 = 5 * 1024 * 1024; // 5 MiB
const DEFAULT_BINARY_CHUNK_LIMIT: u64 = 10 * 1024 * 1024; // 10 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    ShiftJis,
    EucJp,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::ShiftJis => "Shift-JIS",
            Encoding::EucJp => "EUC-JP",
        }
    }
}

/// Newline convention preserved across read/write round-trips. Files with
/// mixed newlines collapse to the dominant style on detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Newline {
    Lf,
    Crlf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileKind {
    Text { encoding: Encoding },
    Binary,
}

/// SPEC-2006 Phase 2 amendment FR-018: callers send back the metadata they
/// observed at read time so the write path can detect external mutations
/// before atomic rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedMetadata {
    pub mtime: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextResult {
    pub encoding: Encoding,
    pub text: String,
    pub total_size: u64,
    pub mtime: u64,
    pub has_bom: bool,
    pub newline: Newline,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryChunk {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub total_size: u64,
    pub mtime: u64,
    pub read_only: bool,
}

/// Result of a successful write. `encoding_fallback` is the number of source
/// characters that could not be represented in the target encoding (UTF-8
/// gets a `?` substitution); zero means a perfect round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOutcome {
    pub new_mtime: u64,
    pub new_size: u64,
    pub encoding_fallback: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileContentError {
    Denied,
    TooLarge {
        size: u64,
        limit: u64,
    },
    IoError(String),
    NotAFile,
    BinaryNotText,
    /// SPEC-2006 Phase 2: write path mtime/size mismatch.
    Conflict {
        current_mtime: u64,
        current_size: u64,
    },
    /// SPEC-2006 Phase 2: target file is read-only on disk.
    ReadOnly,
    /// SPEC-2006 Phase 2: hex byte write offset >= file size.
    OutOfRange {
        offset: u64,
        size: u64,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ContentLimits {
    pub text_max_bytes: u64,
    pub binary_chunk_max_bytes: u64,
}

impl Default for ContentLimits {
    fn default() -> Self {
        Self {
            text_max_bytes: DEFAULT_TEXT_LIMIT,
            binary_chunk_max_bytes: DEFAULT_BINARY_CHUNK_LIMIT,
        }
    }
}

struct ResolvedFile {
    path: PathBuf,
    size: u64,
    mtime: u64,
    read_only: bool,
}

fn resolve_file(root: &Path, relative: &Path) -> Result<ResolvedFile, FileContentError> {
    let canonical = path_filter::canonical_root(root);
    let normalized =
        path_filter::normalize_relative(relative).map_err(|_| FileContentError::Denied)?;

    // Deny rule applies before existence check, so .git/.gwt/etc. are
    // rejected even when the file exists.
    if is_relative_denied(&canonical, &normalized) {
        return Err(FileContentError::Denied);
    }

    let resolved = path_filter::safe_resolve(&canonical, &normalized).map_err(|err| match err {
        PathFilterError::Escape => FileContentError::Denied,
        PathFilterError::NotFound => {
            FileContentError::IoError(format!("file not found: {}", relative.display()))
        }
    })?;

    let metadata = std::fs::metadata(&resolved.canonical_path)
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    if !metadata.is_file() {
        return Err(FileContentError::NotAFile);
    }

    Ok(ResolvedFile {
        path: resolved.canonical_path,
        size: metadata.len(),
        mtime: system_time_to_secs(metadata.modified().unwrap_or(UNIX_EPOCH)),
        read_only: metadata.permissions().readonly(),
    })
}

fn system_time_to_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_all_bytes(path: &Path) -> Result<Vec<u8>, FileContentError> {
    std::fs::read(path).map_err(|err| FileContentError::IoError(err.to_string()))
}

/// Read a file as text, auto-detecting encoding among UTF-8 (BOM-aware),
/// UTF-16 LE/BE (BOM), Shift-JIS, and EUC-JP. Returns `BinaryNotText` when
/// the bytes contain a NUL or chardetng selects an unsupported encoding.
pub fn read_text_file(
    root: &Path,
    relative: &Path,
    limits: &ContentLimits,
) -> Result<TextResult, FileContentError> {
    let resolved = resolve_file(root, relative)?;
    if resolved.size > limits.text_max_bytes {
        return Err(FileContentError::TooLarge {
            size: resolved.size,
            limit: limits.text_max_bytes,
        });
    }

    let bytes = read_all_bytes(&resolved.path)?;

    if bytes.is_empty() {
        return Ok(TextResult {
            encoding: Encoding::Utf8,
            text: String::new(),
            total_size: 0,
            mtime: resolved.mtime,
            has_bom: false,
            newline: Newline::Lf,
            read_only: resolved.read_only,
        });
    }

    let (encoding, text, has_bom) = detect_and_decode(&bytes)?;
    let newline = detect_newline(&text);
    Ok(TextResult {
        encoding,
        text,
        total_size: resolved.size,
        mtime: resolved.mtime,
        has_bom,
        newline,
        read_only: resolved.read_only,
    })
}

/// Read a slice of bytes from a file for hex viewer rendering. The chunk's
/// requested `length` must fit within `binary_chunk_max_bytes`; `offset` and
/// `length` are clamped to the file size and never panic.
pub fn read_binary_chunk(
    root: &Path,
    relative: &Path,
    offset: u64,
    length: u64,
    limits: &ContentLimits,
) -> Result<BinaryChunk, FileContentError> {
    if length > limits.binary_chunk_max_bytes {
        return Err(FileContentError::TooLarge {
            size: length,
            limit: limits.binary_chunk_max_bytes,
        });
    }

    let resolved = resolve_file(root, relative)?;
    let clamped_offset = offset.min(resolved.size);
    let available = resolved.size.saturating_sub(clamped_offset);
    let read_len = length.min(available);

    let mut bytes = vec![0u8; read_len as usize];
    if read_len > 0 {
        let mut file = std::fs::File::open(&resolved.path)
            .map_err(|err| FileContentError::IoError(err.to_string()))?;
        file.seek(SeekFrom::Start(clamped_offset))
            .map_err(|err| FileContentError::IoError(err.to_string()))?;
        file.read_exact(&mut bytes)
            .map_err(|err| FileContentError::IoError(err.to_string()))?;
    }

    Ok(BinaryChunk {
        offset: clamped_offset,
        bytes,
        total_size: resolved.size,
        mtime: resolved.mtime,
        read_only: resolved.read_only,
    })
}

/// Inspect a file and decide whether it should be presented as text (with
/// detected encoding) or binary (cannot decode safely). Subject to the same
/// deny rule and size limits as `read_text_file`.
pub fn file_kind(
    root: &Path,
    relative: &Path,
    limits: &ContentLimits,
) -> Result<FileKind, FileContentError> {
    let resolved = resolve_file(root, relative)?;
    if resolved.size > limits.text_max_bytes {
        return Ok(FileKind::Binary);
    }
    let bytes = read_all_bytes(&resolved.path)?;
    if bytes.is_empty() {
        return Ok(FileKind::Text {
            encoding: Encoding::Utf8,
        });
    }
    match detect_and_decode(&bytes) {
        Ok((encoding, _, _)) => Ok(FileKind::Text { encoding }),
        Err(FileContentError::BinaryNotText) => Ok(FileKind::Binary),
        Err(other) => Err(other),
    }
}

/// SPEC-2006 Phase 2 FR-017: write a text file using atomic tmp+rename,
/// preserving the original encoding, BOM, and newline convention. The
/// `expected` metadata guards against external mutations; mismatch raises
/// `Conflict` and leaves the on-disk file untouched.
#[allow(clippy::too_many_arguments)]
pub fn write_text_file(
    root: &Path,
    relative: &Path,
    text: &str,
    encoding: Encoding,
    newline: Newline,
    has_bom: bool,
    expected: ExpectedMetadata,
    limits: &ContentLimits,
) -> Result<WriteOutcome, FileContentError> {
    let resolved = resolve_file(root, relative)?;
    if resolved.read_only {
        return Err(FileContentError::ReadOnly);
    }
    if resolved.size != expected.size || resolved.mtime != expected.mtime {
        return Err(FileContentError::Conflict {
            current_mtime: resolved.mtime,
            current_size: resolved.size,
        });
    }

    // Normalise the in-memory text to the desired newline. The viewer always
    // sends LF-separated content; we splice CRLF back on the write side so
    // Windows-authored files round-trip exactly.
    let normalised = match newline {
        Newline::Lf => text.replace("\r\n", "\n"),
        Newline::Crlf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
    };

    let (mut encoded, encoding_fallback) = encode_text(&normalised, encoding)?;
    if has_bom {
        prepend_bom(&mut encoded, encoding);
    }

    if encoded.len() as u64 > limits.text_max_bytes {
        return Err(FileContentError::TooLarge {
            size: encoded.len() as u64,
            limit: limits.text_max_bytes,
        });
    }

    atomic_write(&resolved.path, &encoded)?;

    let new_metadata = std::fs::metadata(&resolved.path)
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    Ok(WriteOutcome {
        new_mtime: system_time_to_secs(new_metadata.modified().unwrap_or(UNIX_EPOCH)),
        new_size: new_metadata.len(),
        encoding_fallback,
    })
}

/// SPEC-2006 Phase 2 FR-019: replace a single byte at `offset` in-place,
/// keeping the file size unchanged. Uses a direct seek+write because a full
/// atomic rewrite would dominate hex-edit cost on multi-MiB binaries; the
/// surface is intentionally narrow (a single byte) so partial-write damage
/// is bounded to that byte.
pub fn write_binary_byte(
    root: &Path,
    relative: &Path,
    offset: u64,
    byte: u8,
    expected: ExpectedMetadata,
) -> Result<WriteOutcome, FileContentError> {
    let resolved = resolve_file(root, relative)?;
    if resolved.read_only {
        return Err(FileContentError::ReadOnly);
    }
    if resolved.size != expected.size || resolved.mtime != expected.mtime {
        return Err(FileContentError::Conflict {
            current_mtime: resolved.mtime,
            current_size: resolved.size,
        });
    }
    if offset >= resolved.size {
        return Err(FileContentError::OutOfRange {
            offset,
            size: resolved.size,
        });
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&resolved.path)
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    file.write_all(&[byte])
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    file.sync_all()
        .map_err(|err| FileContentError::IoError(err.to_string()))?;

    let new_metadata = std::fs::metadata(&resolved.path)
        .map_err(|err| FileContentError::IoError(err.to_string()))?;
    Ok(WriteOutcome {
        new_mtime: system_time_to_secs(new_metadata.modified().unwrap_or(UNIX_EPOCH)),
        new_size: new_metadata.len(),
        encoding_fallback: 0,
    })
}

fn detect_and_decode(bytes: &[u8]) -> Result<(Encoding, String, bool), FileContentError> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let (encoding, text) = decode_with(encoding_rs::UTF_8, Encoding::Utf8, &bytes[3..])?;
        return Ok((encoding, text, true));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (encoding, text) = decode_with(encoding_rs::UTF_16LE, Encoding::Utf16Le, &bytes[2..])?;
        return Ok((encoding, text, true));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (encoding, text) = decode_with(encoding_rs::UTF_16BE, Encoding::Utf16Be, &bytes[2..])?;
        return Ok((encoding, text, true));
    }

    if bytes.contains(&0u8) {
        return Err(FileContentError::BinaryNotText);
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((Encoding::Utf8, text.to_owned(), false));
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    let detected = detector.guess(None, chardetng::Utf8Detection::Allow);
    let supported = supported_encoding(detected).ok_or(FileContentError::BinaryNotText)?;
    let (encoding, text) = decode_with(detected, supported, bytes)?;
    Ok((encoding, text, false))
}

fn decode_with(
    encoding: &'static encoding_rs::Encoding,
    label: Encoding,
    bytes: &[u8],
) -> Result<(Encoding, String), FileContentError> {
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(FileContentError::BinaryNotText);
    }
    Ok((label, decoded.into_owned()))
}

fn supported_encoding(detected: &'static encoding_rs::Encoding) -> Option<Encoding> {
    if detected == encoding_rs::UTF_8 {
        Some(Encoding::Utf8)
    } else if detected == encoding_rs::UTF_16LE {
        Some(Encoding::Utf16Le)
    } else if detected == encoding_rs::UTF_16BE {
        Some(Encoding::Utf16Be)
    } else if detected == encoding_rs::SHIFT_JIS {
        Some(Encoding::ShiftJis)
    } else if detected == encoding_rs::EUC_JP {
        Some(Encoding::EucJp)
    } else {
        None
    }
}

fn detect_newline(text: &str) -> Newline {
    // Count CRLF first because the LF inside a CRLF would otherwise be
    // double-counted. The dominant convention wins; an empty file defaults
    // to LF so round-trips remain stable on UNIX hosts.
    let crlf_count = text.matches("\r\n").count();
    let lf_only_count = text.matches('\n').count().saturating_sub(crlf_count);
    if crlf_count > lf_only_count {
        Newline::Crlf
    } else {
        Newline::Lf
    }
}

fn encode_text(text: &str, encoding: Encoding) -> Result<(Vec<u8>, u64), FileContentError> {
    let target = encoding_for(encoding);
    let (encoded, _, had_unmappable) = target.encode(text);
    let fallback = if had_unmappable {
        // encoding_rs replaces unmappable code points with HTML numeric
        // references (e.g. `&#1234;`) when encoding a non-Unicode target.
        // We surface the count so the GUI can warn the user — the count is
        // bounded by the number of times the substitution was emitted.
        count_html_numeric_substitutions(&encoded)
    } else {
        0
    };
    Ok((encoded.into_owned(), fallback))
}

fn encoding_for(encoding: Encoding) -> &'static encoding_rs::Encoding {
    match encoding {
        Encoding::Utf8 => encoding_rs::UTF_8,
        Encoding::Utf16Le => encoding_rs::UTF_16LE,
        Encoding::Utf16Be => encoding_rs::UTF_16BE,
        Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
        Encoding::EucJp => encoding_rs::EUC_JP,
    }
}

fn count_html_numeric_substitutions(bytes: &[u8]) -> u64 {
    // Look for the pattern `&#NNN;` and count occurrences. UTF-8 / UTF-16
    // never go through this path because they are Unicode-complete.
    let s = String::from_utf8_lossy(bytes);
    let mut count = 0u64;
    let mut rest = s.as_ref();
    while let Some(pos) = rest.find("&#") {
        let after = &rest[pos + 2..];
        if let Some(end) = after.find(';') {
            if after[..end].chars().all(|c| c.is_ascii_digit()) {
                count += 1;
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    count
}

fn prepend_bom(bytes: &mut Vec<u8>, encoding: Encoding) {
    let bom: &[u8] = match encoding {
        Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoding::Utf16Le => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
        _ => return,
    };
    let mut prefixed = Vec::with_capacity(bom.len() + bytes.len());
    prefixed.extend_from_slice(bom);
    prefixed.extend_from_slice(bytes);
    *bytes = prefixed;
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), FileContentError> {
    let parent = target.parent().ok_or_else(|| {
        FileContentError::IoError(format!(
            "target has no parent directory: {}",
            target.display()
        ))
    })?;
    let basename = target
        .file_name()
        .ok_or_else(|| {
            FileContentError::IoError(format!("target has no file name: {}", target.display()))
        })?
        .to_string_lossy();
    let pid = std::process::id();
    let nonce = system_time_to_secs(SystemTime::now()).wrapping_mul(31);
    let tmp_name = format!(".{basename}.gwt-edit-tmp.{pid}.{nonce}");
    let tmp_path = parent.join(tmp_name);

    let write_result = std::fs::write(&tmp_path, bytes);
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(FileContentError::IoError(err.to_string()));
    }

    if let Err(err) = std::fs::rename(&tmp_path, target) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(FileContentError::IoError(err.to_string()));
    }
    Ok(())
}
