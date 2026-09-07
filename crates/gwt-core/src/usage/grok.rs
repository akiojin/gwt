//! Grok Build local-session discovery used by exact conversation resume.
//!
//! Grok stores each native conversation at
//! `$GROK_HOME/sessions/<encoded-cwd>/<session-id>/` (falling back to
//! `~/.grok`). `summary.json` supplies the native identity and `updates.jsonl`
//! supplies the authoritative conversation history required by Resume.

use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokSessionMetadata {
    pub id: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrokSessionStoreAvailability {
    Present(PathBuf),
    Missing,
    Foreign,
}

pub fn grok_home() -> Option<PathBuf> {
    std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".grok")))
}

/// Resolve the store root from the environment that will actually be passed
/// to Grok, rather than from gwt's own process environment. Relative paths
/// use the same launch cwd that the Grok child process will receive.
pub fn grok_home_from_env(env: &HashMap<String, String>, launch_cwd: &Path) -> Option<PathBuf> {
    let home = env
        .get("GROK_HOME")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env.get("HOME")
                .or_else(|| env.get("USERPROFILE"))
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".grok"))
        })?;
    Some(if home.is_absolute() {
        home
    } else {
        launch_cwd.join(home)
    })
}

pub fn read_summary_metadata(path: &Path) -> io::Result<Option<GrokSessionMetadata>> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let Some(info) = value.get("info") else {
        return Ok(None);
    };
    let Some(id) = info
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let Some(cwd) = info
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some(GrokSessionMetadata {
        id: id.to_string(),
        cwd: PathBuf::from(cwd),
    }))
}

/// Validate the complete provider receipt required by `grok --resume`.
///
/// Grok permits the same client-supplied native id under different encoded
/// cwd groups, so every exact-id candidate must be inspected before deciding
/// that the requested worktree is foreign. `summary.json` supplies identity;
/// the authoritative JSONL update log must carry that same native session id.
pub fn resumable_session_summary(
    home: &Path,
    session_id: &str,
    expected_cwd: &Path,
) -> io::Result<GrokSessionStoreAvailability> {
    let mut components = Path::new(session_id).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Ok(GrokSessionStoreAvailability::Missing);
    }
    let groups = match fs::read_dir(home.join("sessions")) {
        Ok(groups) => groups,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(GrokSessionStoreAvailability::Missing);
        }
        Err(error) => return Err(error),
    };
    let mut saw_foreign = false;
    let mut saw_matching_identity = false;
    let mut first_error = None;
    for group in groups {
        let group = match group {
            Ok(group) => group,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        let session_dir = group.path().join(session_id);
        let summary = session_dir.join("summary.json");
        if !summary.is_file() {
            continue;
        }
        let metadata = match read_summary_metadata(&summary) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                saw_foreign = true;
                continue;
            }
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        if metadata.id != session_id || !paths_match(&metadata.cwd, expected_cwd) {
            saw_foreign = true;
            continue;
        }
        saw_matching_identity = true;
        match has_resumable_updates(&session_dir.join("updates.jsonl"), session_id) {
            Ok(true) => return Ok(GrokSessionStoreAvailability::Present(summary)),
            Ok(false) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if saw_matching_identity {
        return Ok(GrokSessionStoreAvailability::Missing);
    }
    if saw_foreign {
        return Ok(GrokSessionStoreAvailability::Foreign);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(GrokSessionStoreAvailability::Missing)
}

fn has_resumable_updates(path: &Path, session_id: &str) -> io::Result<bool> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for line in io::BufReader::new(file).lines().take(64) {
        let line = line?;
        let trimmed = line.trim();
        let Ok(update) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if update
            .pointer("/params/sessionId")
            .and_then(serde_json::Value::as_str)
            == Some(session_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn paths_match(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn official_session_update(session_id: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "timestamp": 1_787_208_599_u64,
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {"sessionUpdate": "user_message_chunk"},
                },
            })
        )
    }

    #[test]
    fn grok_home_from_env_resolves_relative_override_against_launch_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let launch_cwd = temp.path().join("worktree");
        let env = HashMap::from([
            ("GROK_HOME".to_string(), "profile-store".to_string()),
            (
                "HOME".to_string(),
                temp.path().join("profile-home").display().to_string(),
            ),
        ]);

        assert_eq!(
            grok_home_from_env(&env, &launch_cwd),
            Some(launch_cwd.join("profile-store")),
        );
    }

    #[test]
    fn resume_receipt_requires_matching_authoritative_update_identity() {
        let home = tempfile::tempdir().expect("tempdir");
        let cwd = home.path().join("repo");
        fs::create_dir_all(&cwd).expect("create cwd");
        let session_id = "01a0195c-fbd7-7352-8d29-da6f6f755010";
        let other_session_id = "01a0195c-fbd7-7352-8d29-da6f6f755011";
        let summary = home
            .path()
            .join("sessions/%2Ftmp%2Frepo")
            .join(session_id)
            .join("summary.json");
        fs::create_dir_all(summary.parent().expect("summary parent"))
            .expect("create summary parent");
        fs::write(
            &summary,
            serde_json::json!({"info":{"id":session_id,"cwd":cwd}}).to_string(),
        )
        .expect("write summary");

        assert_eq!(
            resumable_session_summary(home.path(), session_id, &cwd)
                .expect("inspect incomplete store"),
            GrokSessionStoreAvailability::Missing,
        );

        let updates = summary
            .parent()
            .expect("summary parent")
            .join("updates.jsonl");
        fs::write(&updates, "{}\n").expect("write arbitrary JSON");
        assert_eq!(
            resumable_session_summary(home.path(), session_id, &cwd)
                .expect("reject arbitrary JSON"),
            GrokSessionStoreAvailability::Missing,
        );

        fs::write(&updates, official_session_update(other_session_id))
            .expect("write mismatched update stream");
        assert_eq!(
            resumable_session_summary(home.path(), session_id, &cwd)
                .expect("reject mismatched update stream"),
            GrokSessionStoreAvailability::Missing,
        );

        fs::write(&updates, official_session_update(session_id))
            .expect("write authoritative updates log");
        assert_eq!(
            resumable_session_summary(home.path(), session_id, &cwd)
                .expect("inspect complete store"),
            GrokSessionStoreAvailability::Present(summary.clone()),
        );
        assert_eq!(
            read_summary_metadata(&summary).expect("read metadata"),
            Some(GrokSessionMetadata {
                id: session_id.to_string(),
                cwd,
            }),
        );
        assert_eq!(
            resumable_session_summary(home.path(), &format!("../{session_id}"), home.path())
                .expect("reject traversal"),
            GrokSessionStoreAvailability::Missing,
        );
    }

    #[test]
    fn resume_receipt_selects_the_matching_cwd_when_native_ids_repeat() {
        let home = tempfile::tempdir().expect("tempdir");
        let expected_cwd = home.path().join("expected");
        let foreign_cwd = home.path().join("foreign");
        fs::create_dir_all(&expected_cwd).expect("create expected cwd");
        fs::create_dir_all(&foreign_cwd).expect("create foreign cwd");
        let session_id = "01a0195d-2af6-78d3-b478-b9ae0756d525";

        for (group, cwd) in [("00-foreign", &foreign_cwd), ("99-expected", &expected_cwd)] {
            let session_dir = home.path().join("sessions").join(group).join(session_id);
            fs::create_dir_all(&session_dir).expect("create session dir");
            fs::write(
                session_dir.join("summary.json"),
                serde_json::json!({"info":{"id":session_id,"cwd":cwd}}).to_string(),
            )
            .expect("write summary");
            fs::write(
                session_dir.join("updates.jsonl"),
                official_session_update(session_id),
            )
            .expect("write updates");
        }

        assert_eq!(
            resumable_session_summary(home.path(), session_id, &expected_cwd)
                .expect("locate expected session"),
            GrokSessionStoreAvailability::Present(
                home.path()
                    .join("sessions/99-expected")
                    .join(session_id)
                    .join("summary.json")
            ),
        );
    }
}
