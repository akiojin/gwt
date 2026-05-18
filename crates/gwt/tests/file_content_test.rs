use std::path::Path;

use gwt::file_content::{
    file_kind, read_binary_chunk, read_text_file, write_binary_byte, write_text_file,
    ContentLimits, Encoding, ExpectedMetadata, FileContentError, FileKind, Newline,
};
use tempfile::tempdir;

fn write_at(root: &Path, rel: &str, bytes: &[u8]) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(target, bytes).expect("write file");
}

#[test]
fn read_text_file_decodes_utf8_with_and_without_bom() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "plain.txt", "hello, world\n".as_bytes());
    write_at(dir.path(), "bom.txt", b"\xEF\xBB\xBFhello with BOM\n");

    let limits = ContentLimits::default();
    let plain = read_text_file(dir.path(), Path::new("plain.txt"), &limits).expect("plain text");
    assert_eq!(plain.encoding, Encoding::Utf8);
    assert_eq!(plain.text, "hello, world\n");
    assert_eq!(plain.total_size, 13);

    let bom = read_text_file(dir.path(), Path::new("bom.txt"), &limits).expect("bom text");
    assert_eq!(bom.encoding, Encoding::Utf8);
    assert_eq!(bom.text, "hello with BOM\n");
}

#[test]
fn read_text_file_decodes_utf16_le_and_be_with_bom() {
    let dir = tempdir().expect("tempdir");

    // UTF-16 LE BOM (FF FE) + "hi" in UTF-16 LE
    let mut le = vec![0xFF, 0xFE];
    for ch in "hi\n".encode_utf16() {
        le.extend_from_slice(&ch.to_le_bytes());
    }
    write_at(dir.path(), "le.txt", &le);

    // UTF-16 BE BOM (FE FF) + "hi" in UTF-16 BE
    let mut be = vec![0xFE, 0xFF];
    for ch in "hi\n".encode_utf16() {
        be.extend_from_slice(&ch.to_be_bytes());
    }
    write_at(dir.path(), "be.txt", &be);

    let limits = ContentLimits::default();
    let le_result = read_text_file(dir.path(), Path::new("le.txt"), &limits).expect("le text");
    assert_eq!(le_result.encoding, Encoding::Utf16Le);
    assert_eq!(le_result.text, "hi\n");

    let be_result = read_text_file(dir.path(), Path::new("be.txt"), &limits).expect("be text");
    assert_eq!(be_result.encoding, Encoding::Utf16Be);
    assert_eq!(be_result.text, "hi\n");
}

#[test]
fn read_text_file_decodes_shift_jis_and_euc_jp() {
    let dir = tempdir().expect("tempdir");
    // "あいう\n" encoded in Shift-JIS
    let sjis = encoding_rs::SHIFT_JIS.encode("あいう\n").0.into_owned();
    write_at(dir.path(), "sjis.txt", &sjis);

    // "あいう\n" encoded in EUC-JP
    let eucjp = encoding_rs::EUC_JP.encode("あいう\n").0.into_owned();
    write_at(dir.path(), "euc.txt", &eucjp);

    let limits = ContentLimits::default();
    let sjis_result =
        read_text_file(dir.path(), Path::new("sjis.txt"), &limits).expect("sjis decode");
    assert_eq!(sjis_result.encoding, Encoding::ShiftJis);
    assert_eq!(sjis_result.text, "あいう\n");

    let euc_result =
        read_text_file(dir.path(), Path::new("euc.txt"), &limits).expect("eucjp decode");
    assert_eq!(euc_result.encoding, Encoding::EucJp);
    assert_eq!(euc_result.text, "あいう\n");
}

#[test]
fn read_text_file_returns_binary_when_nul_byte_present() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "bin.dat", b"some text\x00with NUL byte");

    let limits = ContentLimits::default();
    let err = read_text_file(dir.path(), Path::new("bin.dat"), &limits).expect_err("binary");
    match err {
        FileContentError::BinaryNotText => {}
        other => panic!("expected BinaryNotText, got {other:?}"),
    }

    let kind = file_kind(dir.path(), Path::new("bin.dat"), &limits).expect("file_kind");
    assert!(matches!(kind, FileKind::Binary));
}

#[test]
fn file_kind_returns_text_for_ascii_and_binary_for_random_bytes() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "ascii.txt", b"plain ascii text\n");
    write_at(
        dir.path(),
        "random.bin",
        &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        ],
    );

    let limits = ContentLimits::default();
    let ascii_kind = file_kind(dir.path(), Path::new("ascii.txt"), &limits).expect("ascii");
    assert!(matches!(ascii_kind, FileKind::Text { .. }));

    let png_kind = file_kind(dir.path(), Path::new("random.bin"), &limits).expect("png header");
    assert!(matches!(png_kind, FileKind::Binary));
}

#[test]
fn read_text_file_rejects_files_exceeding_text_limit() {
    let dir = tempdir().expect("tempdir");
    let limits = ContentLimits {
        text_max_bytes: 16,
        binary_chunk_max_bytes: 32,
    };
    write_at(dir.path(), "big.txt", &b"a".repeat(32));

    let err = read_text_file(dir.path(), Path::new("big.txt"), &limits).expect_err("too large");
    match err {
        FileContentError::TooLarge { size, limit } => {
            assert_eq!(size, 32);
            assert_eq!(limit, 16);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn read_binary_chunk_rejects_chunk_exceeding_binary_limit() {
    let dir = tempdir().expect("tempdir");
    let limits = ContentLimits {
        text_max_bytes: 16,
        binary_chunk_max_bytes: 8,
    };
    write_at(dir.path(), "bin.dat", &b"\xFF".repeat(64));

    let err = read_binary_chunk(dir.path(), Path::new("bin.dat"), 0, 16, &limits)
        .expect_err("chunk too large");
    match err {
        FileContentError::TooLarge { size, limit } => {
            assert_eq!(size, 16);
            assert_eq!(limit, 8);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn read_text_file_denies_paths_excluded_by_deny_rule() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
    write_at(dir.path(), ".git/HEAD", b"ref: refs/heads/main\n");
    write_at(dir.path(), ".gitignore", b"secrets.env\n");
    write_at(dir.path(), "secrets.env", b"API_KEY=...\n");

    let limits = ContentLimits::default();
    let git_err =
        read_text_file(dir.path(), Path::new(".git/HEAD"), &limits).expect_err("git deny rule");
    assert!(matches!(git_err, FileContentError::Denied));

    let env_err = read_text_file(dir.path(), Path::new("secrets.env"), &limits)
        .expect_err("gitignore deny rule");
    assert!(matches!(env_err, FileContentError::Denied));
}

#[test]
fn read_binary_chunk_denies_paths_excluded_by_deny_rule() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".gwt")).expect("create .gwt");
    write_at(dir.path(), ".gwt/state.bin", &[0xFF, 0x00, 0x42]);

    let limits = ContentLimits::default();
    let err = read_binary_chunk(dir.path(), Path::new(".gwt/state.bin"), 0, 16, &limits)
        .expect_err("gwt deny rule");
    assert!(matches!(err, FileContentError::Denied));
}

#[test]
fn read_text_file_rejects_path_escape_attempts() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "inside.txt", b"ok");

    let limits = ContentLimits::default();
    let err =
        read_text_file(dir.path(), Path::new("../outside.txt"), &limits).expect_err("path escape");
    assert!(matches!(err, FileContentError::Denied));
}

#[test]
fn read_text_file_returns_not_a_file_for_directories() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("subdir")).expect("create subdir");

    let limits = ContentLimits::default();
    let err = read_text_file(dir.path(), Path::new("subdir"), &limits).expect_err("not a file");
    assert!(matches!(err, FileContentError::NotAFile));
}

#[test]
fn read_binary_chunk_normalizes_offset_and_length() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "data.bin", &(0u8..32).collect::<Vec<u8>>());

    let limits = ContentLimits::default();

    // Request from offset=10, length=100 → should return bytes [10, 32) (22 bytes), total_size=32
    let chunk =
        read_binary_chunk(dir.path(), Path::new("data.bin"), 10, 100, &limits).expect("chunk read");
    assert_eq!(chunk.offset, 10);
    assert_eq!(chunk.bytes.len(), 22);
    assert_eq!(chunk.bytes[0], 10);
    assert_eq!(chunk.bytes[21], 31);
    assert_eq!(chunk.total_size, 32);

    // Request from offset >= file size → empty chunk at clamped offset
    let empty = read_binary_chunk(dir.path(), Path::new("data.bin"), 64, 16, &limits)
        .expect("clamped chunk");
    assert_eq!(empty.offset, 32);
    assert!(empty.bytes.is_empty());
    assert_eq!(empty.total_size, 32);
}

#[test]
fn read_text_file_returns_empty_string_for_zero_byte_file() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "empty.txt", b"");

    let limits = ContentLimits::default();
    let result = read_text_file(dir.path(), Path::new("empty.txt"), &limits).expect("empty");
    assert_eq!(result.encoding, Encoding::Utf8);
    assert_eq!(result.text, "");
    assert_eq!(result.total_size, 0);
}

#[test]
fn read_binary_chunk_returns_empty_chunk_for_zero_byte_file() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "empty.bin", b"");

    let limits = ContentLimits::default();
    let chunk =
        read_binary_chunk(dir.path(), Path::new("empty.bin"), 0, 16, &limits).expect("empty chunk");
    assert_eq!(chunk.offset, 0);
    assert!(chunk.bytes.is_empty());
    assert_eq!(chunk.total_size, 0);
}

// SPEC-2006 Phase 2 amendment: read_text_file must expose mtime/has_bom/newline
// so the GUI can preserve them across a save round-trip.
#[test]
fn read_text_file_detects_crlf_lf_and_bom_metadata() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "lf.txt", b"alpha\nbeta\n");
    write_at(dir.path(), "crlf.txt", b"alpha\r\nbeta\r\n");
    write_at(dir.path(), "bom.txt", b"\xEF\xBB\xBFhello\n");

    let limits = ContentLimits::default();

    let lf = read_text_file(dir.path(), Path::new("lf.txt"), &limits).expect("lf");
    assert_eq!(lf.newline, Newline::Lf);
    assert!(!lf.has_bom);
    assert!(
        lf.mtime > 0,
        "mtime should be populated from filesystem metadata"
    );
    assert_eq!(lf.total_size, 11);

    let crlf = read_text_file(dir.path(), Path::new("crlf.txt"), &limits).expect("crlf");
    assert_eq!(crlf.newline, Newline::Crlf);
    assert!(!crlf.has_bom);

    let bom = read_text_file(dir.path(), Path::new("bom.txt"), &limits).expect("bom");
    assert!(bom.has_bom);
    assert_eq!(bom.encoding, Encoding::Utf8);
    assert_eq!(bom.newline, Newline::Lf);
}

#[test]
fn write_text_file_round_trips_utf8_lf_without_bom() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "note.txt", b"hello\n");

    let limits = ContentLimits::default();
    let before = read_text_file(dir.path(), Path::new("note.txt"), &limits).expect("read");
    let outcome = write_text_file(
        dir.path(),
        Path::new("note.txt"),
        "hello world\n",
        before.encoding,
        before.newline,
        before.has_bom,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
        &limits,
    )
    .expect("write");
    assert_eq!(outcome.encoding_fallback, 0);
    assert!(outcome.new_size >= "hello world\n".len() as u64);

    let after_bytes = std::fs::read(dir.path().join("note.txt")).expect("re-read");
    assert_eq!(after_bytes, b"hello world\n".to_vec());
}

#[test]
fn write_text_file_preserves_utf8_bom_and_crlf_round_trip() {
    let dir = tempdir().expect("tempdir");
    let original: &[u8] = b"\xEF\xBB\xBFalpha\r\nbeta\r\n";
    write_at(dir.path(), "crlf-bom.txt", original);

    let limits = ContentLimits::default();
    let before = read_text_file(dir.path(), Path::new("crlf-bom.txt"), &limits).expect("read");
    assert!(before.has_bom);
    assert_eq!(before.newline, Newline::Crlf);

    write_text_file(
        dir.path(),
        Path::new("crlf-bom.txt"),
        &before.text,
        before.encoding,
        before.newline,
        before.has_bom,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
        &limits,
    )
    .expect("write");

    let after_bytes = std::fs::read(dir.path().join("crlf-bom.txt")).expect("re-read");
    assert_eq!(
        after_bytes,
        original.to_vec(),
        "BOM + CRLF must survive a round-trip"
    );
}

#[test]
fn write_text_file_round_trips_shift_jis_and_euc_jp() {
    let dir = tempdir().expect("tempdir");
    let sjis = encoding_rs::SHIFT_JIS.encode("こんにちは\n").0.into_owned();
    write_at(dir.path(), "sjis.txt", &sjis);
    let euc = encoding_rs::EUC_JP.encode("こんばんは\n").0.into_owned();
    write_at(dir.path(), "euc.txt", &euc);

    let limits = ContentLimits::default();
    for (name, expected_enc) in [
        ("sjis.txt", Encoding::ShiftJis),
        ("euc.txt", Encoding::EucJp),
    ] {
        let before = read_text_file(dir.path(), Path::new(name), &limits).expect("read");
        assert_eq!(before.encoding, expected_enc);
        let mutated = format!("{}追記\n", before.text);
        let outcome = write_text_file(
            dir.path(),
            Path::new(name),
            &mutated,
            before.encoding,
            before.newline,
            before.has_bom,
            ExpectedMetadata {
                mtime: before.mtime,
                size: before.total_size,
            },
            &limits,
        )
        .expect("write");
        assert_eq!(
            outcome.encoding_fallback, 0,
            "Japanese text must encode without fallback"
        );

        let after = read_text_file(dir.path(), Path::new(name), &limits).expect("re-read");
        assert_eq!(after.encoding, expected_enc);
        assert_eq!(after.text, mutated);
    }
}

#[test]
fn write_text_file_rejects_conflict_when_mtime_mismatches() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "race.txt", b"original\n");

    let limits = ContentLimits::default();
    let before = read_text_file(dir.path(), Path::new("race.txt"), &limits).expect("read");

    // Simulate an external editor mutating the file after we read it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.path().join("race.txt"), b"external write\n").expect("external write");

    let err = write_text_file(
        dir.path(),
        Path::new("race.txt"),
        "our edit\n",
        before.encoding,
        before.newline,
        before.has_bom,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
        &limits,
    )
    .expect_err("conflict");
    match err {
        FileContentError::Conflict { current_size, .. } => {
            assert_eq!(current_size, b"external write\n".len() as u64);
        }
        other => panic!("expected Conflict, got {other:?}"),
    }

    // Disk content must remain the external write — atomic write never ran.
    let on_disk = std::fs::read(dir.path().join("race.txt")).expect("re-read");
    assert_eq!(on_disk, b"external write\n".to_vec());
}

#[test]
fn write_text_file_rejects_read_only_files() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "ro.txt", b"keep\n");
    let target = dir.path().join("ro.txt");
    let mut perms = std::fs::metadata(&target).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&target, perms).expect("chmod ro");

    let limits = ContentLimits::default();
    let before = read_text_file(dir.path(), Path::new("ro.txt"), &limits).expect("read");
    assert!(
        before.read_only,
        "TextResult.read_only must reflect permissions"
    );

    let err = write_text_file(
        dir.path(),
        Path::new("ro.txt"),
        "new\n",
        before.encoding,
        before.newline,
        before.has_bom,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
        &limits,
    )
    .expect_err("read-only");
    assert!(matches!(err, FileContentError::ReadOnly));

    // Restore writeable so tempdir cleanup succeeds.
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut perms = std::fs::metadata(&target).expect("metadata").permissions();
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(&target, perms);
    }
}

#[test]
fn write_text_file_rejects_deny_rule_targets() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
    write_at(dir.path(), ".git/HEAD", b"ref: refs/heads/main\n");

    let limits = ContentLimits::default();
    let err = write_text_file(
        dir.path(),
        Path::new(".git/HEAD"),
        "ref: refs/heads/feature\n",
        Encoding::Utf8,
        Newline::Lf,
        false,
        ExpectedMetadata { mtime: 0, size: 0 },
        &limits,
    )
    .expect_err("deny");
    assert!(matches!(err, FileContentError::Denied));
}

#[test]
fn write_binary_byte_replaces_a_single_byte_in_place() {
    let dir = tempdir().expect("tempdir");
    let payload: Vec<u8> = (0u8..16).collect();
    write_at(dir.path(), "bin.dat", &payload);

    let limits = ContentLimits::default();
    let before = read_binary_chunk(dir.path(), Path::new("bin.dat"), 0, 16, &limits).expect("read");

    let outcome = write_binary_byte(
        dir.path(),
        Path::new("bin.dat"),
        4,
        0xAB,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
    )
    .expect("write");
    assert_eq!(
        outcome.new_size, 16,
        "single-byte replace must not change file size"
    );

    let after = std::fs::read(dir.path().join("bin.dat")).expect("re-read");
    assert_eq!(after[4], 0xAB);
    assert_eq!(after.len(), 16);
    // Other bytes must be untouched.
    for i in [0usize, 3, 5, 15] {
        assert_eq!(after[i], i as u8);
    }
}

#[test]
fn write_binary_byte_returns_out_of_range_for_offset_at_or_beyond_size() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "tiny.dat", &[0xCAu8, 0xFE]);

    let limits = ContentLimits::default();
    let before = read_binary_chunk(dir.path(), Path::new("tiny.dat"), 0, 2, &limits).expect("read");

    let err = write_binary_byte(
        dir.path(),
        Path::new("tiny.dat"),
        2,
        0x00,
        ExpectedMetadata {
            mtime: before.mtime,
            size: before.total_size,
        },
    )
    .expect_err("oor");
    match err {
        FileContentError::OutOfRange { offset, size } => {
            assert_eq!(offset, 2);
            assert_eq!(size, 2);
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}

#[test]
fn write_binary_byte_rejects_conflict_and_deny() {
    let dir = tempdir().expect("tempdir");
    write_at(dir.path(), "data.bin", &[0u8, 1, 2, 3]);

    let conflict_err = write_binary_byte(
        dir.path(),
        Path::new("data.bin"),
        1,
        0xFF,
        ExpectedMetadata { mtime: 0, size: 99 },
    )
    .expect_err("conflict");
    assert!(matches!(conflict_err, FileContentError::Conflict { .. }));

    std::fs::create_dir_all(dir.path().join(".gwt")).expect("create .gwt");
    write_at(dir.path(), ".gwt/state.bin", &[0u8; 8]);
    let deny_err = write_binary_byte(
        dir.path(),
        Path::new(".gwt/state.bin"),
        0,
        0x42,
        ExpectedMetadata { mtime: 0, size: 0 },
    )
    .expect_err("deny");
    assert!(matches!(deny_err, FileContentError::Denied));
}
