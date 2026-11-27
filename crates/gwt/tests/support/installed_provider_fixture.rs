use std::{io, path::Path};

pub fn remove_posix_shim(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[test]
fn missing_posix_sibling_preserves_windows_entrypoint() {
    let temp = tempfile::tempdir().unwrap();
    let sibling = temp.path().join("codex");
    let entrypoint = temp.path().join("codex.cmd");
    std::fs::write(&sibling, "POSIX shim").unwrap();
    std::fs::write(&entrypoint, "Windows shim").unwrap();
    std::fs::remove_file(&sibling).unwrap();

    remove_posix_shim(&sibling).expect("an already absent POSIX sibling is valid");

    assert!(entrypoint.is_file());
}
