use std::path::Path;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileKind {
    Text { encoding: Encoding },
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextResult {
    pub encoding: Encoding,
    pub text: String,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryChunk {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileContentError {
    Denied,
    TooLarge { size: u64, limit: u64 },
    IoError(String),
    NotAFile,
    BinaryNotText,
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

fn resolve_file(root: &Path, relative: &Path) -> Result<ResolvedFile, FileContentError> {
    // Path filter: must be inside the root and pass normalization checks.
    let canonical = path_filter::canonical_root(root);
    let normalized =
        path_filter::normalize_relative(relative).map_err(|_| FileContentError::Denied)?;

    // Deny rule (builtin skip + .gitignore): applies before existence check, so
    // attempts to read .git/.gwt/etc. are rejected even when the file exists.
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
    })
}

struct ResolvedFile {
    path: std::path::PathBuf,
    size: u64,
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
        });
    }

    let (encoding, text) = detect_and_decode(&bytes)?;
    Ok(TextResult {
        encoding,
        text,
        total_size: resolved.size,
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
        use std::io::{Read, Seek, SeekFrom};
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
        Ok((encoding, _)) => Ok(FileKind::Text { encoding }),
        Err(FileContentError::BinaryNotText) => Ok(FileKind::Binary),
        Err(other) => Err(other),
    }
}

fn detect_and_decode(bytes: &[u8]) -> Result<(Encoding, String), FileContentError> {
    // BOM checks take precedence — these are unambiguous.
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let stripped = &bytes[3..];
        return decode_with(encoding_rs::UTF_8, Encoding::Utf8, stripped);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let stripped = &bytes[2..];
        return decode_with(encoding_rs::UTF_16LE, Encoding::Utf16Le, stripped);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let stripped = &bytes[2..];
        return decode_with(encoding_rs::UTF_16BE, Encoding::Utf16Be, stripped);
    }

    if bytes.contains(&0u8) {
        return Err(FileContentError::BinaryNotText);
    }

    // Try strict UTF-8 first. Failing that, ask chardetng for a best guess
    // restricted to encodings we explicitly support; anything else is binary.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((Encoding::Utf8, text.to_owned()));
    }

    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let detected = detector.guess(None, true);
    let supported = supported_encoding(detected).ok_or(FileContentError::BinaryNotText)?;
    decode_with(detected, supported, bytes)
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
