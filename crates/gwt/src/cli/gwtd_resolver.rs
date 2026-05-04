use std::path::{Path, PathBuf};

use crate::native_app::{INTERNAL_DAEMON_BINARY_NAME, MACOS_APP_BUNDLE_NAME};

type PathLookup<'a> = dyn Fn(&str) -> Option<PathBuf> + 'a;
type PathPredicate<'a> = dyn Fn(&Path) -> bool + 'a;

pub struct GwtdResolutionInputs<'a> {
    pub explicit_bin_path: Option<PathBuf>,
    pub path_lookup: Box<PathLookup<'a>>,
    pub installed_candidates: Vec<PathBuf>,
    pub development_fallbacks: Vec<PathBuf>,
    pub is_file: Box<PathPredicate<'a>>,
}

pub fn resolve_gwtd_path() -> Option<PathBuf> {
    let explicit_bin_path = std::env::var_os(gwt_agent::session::GWT_BIN_PATH_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("GWT_HOOK_BIN").map(PathBuf::from));
    resolve_gwtd_path_with(GwtdResolutionInputs {
        explicit_bin_path,
        path_lookup: Box::new(|command| which::which(command).ok()),
        installed_candidates: default_installed_candidates(std::env::current_exe().ok().as_deref()),
        development_fallbacks: default_development_fallbacks(),
        is_file: Box::new(|path| path.is_file()),
    })
}

pub fn resolve_gwtd_path_with(inputs: GwtdResolutionInputs<'_>) -> Option<PathBuf> {
    let GwtdResolutionInputs {
        explicit_bin_path,
        path_lookup,
        installed_candidates,
        development_fallbacks,
        is_file,
    } = inputs;

    explicit_bin_path
        .filter(|path| is_file(path))
        .or_else(|| path_lookup(INTERNAL_DAEMON_BINARY_NAME).filter(|path| is_file(path)))
        .or_else(|| first_existing(installed_candidates, &is_file))
        .or_else(|| first_existing(development_fallbacks, &is_file))
}

pub fn default_installed_candidates(current_exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(current_exe) = current_exe {
        if is_named_gwtd_binary(current_exe) {
            candidates.push(current_exe.to_path_buf());
        } else if is_named_gwt_binary(current_exe) {
            candidates.push(current_exe.with_file_name(gwtd_exe_name_for(current_exe)));
        }
    }

    candidates.push(PathBuf::from(format!(
        "/Applications/{MACOS_APP_BUNDLE_NAME}/Contents/MacOS/{INTERNAL_DAEMON_BINARY_NAME}"
    )));
    candidates
}

pub fn default_development_fallbacks() -> Vec<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{INTERNAL_DAEMON_BINARY_NAME}.exe")
    } else {
        INTERNAL_DAEMON_BINARY_NAME.to_string()
    };
    vec![PathBuf::from("target").join("debug").join(exe_name)]
}

fn first_existing(candidates: Vec<PathBuf>, is_file: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    candidates.into_iter().find(|candidate| is_file(candidate))
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_file_stem(path).is_some_and(|stem| {
        stem.eq_ignore_ascii_case(crate::native_app::GUI_FRONT_DOOR_BINARY_NAME)
    })
}

fn is_named_gwtd_binary(path: &Path) -> bool {
    normalized_file_stem(path)
        .is_some_and(|stem| stem.eq_ignore_ascii_case(INTERNAL_DAEMON_BINARY_NAME))
}

fn normalized_file_stem(path: &Path) -> Option<String> {
    path.file_name().and_then(|name| name.to_str()).map(|name| {
        name.strip_suffix(".exe")
            .or_else(|| name.strip_suffix(".EXE"))
            .unwrap_or(name)
            .to_string()
    })
}

fn gwtd_exe_name_for(current_exe: &Path) -> String {
    match current_exe.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => {
            format!("{INTERNAL_DAEMON_BINARY_NAME}.exe")
        }
        _ => INTERNAL_DAEMON_BINARY_NAME.to_string(),
    }
}
