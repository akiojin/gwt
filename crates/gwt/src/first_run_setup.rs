//! Issue #7731 (SPEC #7007 slice N-4): first-run setup.
//!
//! The setup on the Hub page covers the two things a machine needs before an
//! agent can work: a registered tool (an execution environment whose tool is
//! installed here) and the agent who serves as Project Manager.
//!
//! Whether the setup is still required is read from the machine's state, not
//! from a "has been shown" mark, so opening it again from Settings shows the
//! same screen with the current state.
//!
//! The Project Manager runs per project. The person chosen here is stored as
//! this machine's designation and becomes the PM of every project that has
//! not chosen one itself, at the moment that project first reserves its PM
//! (`agent_registry`). A project's own choice in the agent ledger always wins.

use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
/// Longest display name the setup accepts for a new agent.
pub const NAME_MAX_CHARS: usize = 40;

/// The agent designated as Project Manager on this machine.
///
/// `member_id` is absent while the agent has not joined the ledger yet: a new
/// agent is added under `name` when the first project reserves its PM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmDesignation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
    pub name: String,
}

impl PmDesignation {
    /// Whether this designation names the ledger member `(member_id, name)`.
    pub fn names(&self, member_id: &str, name: &str) -> bool {
        match &self.member_id {
            Some(id) => id == member_id,
            None => same_name(&self.name, name),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    schema_version: u32,
    #[serde(default)]
    pm: Option<PmDesignation>,
}

/// A tool the machine has registered for agents to run with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupToolView {
    /// The execution environment's name, e.g. `Claude Code 1`.
    pub name: String,
    pub tool: String,
    pub installed: bool,
}

/// An agent already in the ledger who can be designated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupMemberView {
    pub member_id: String,
    pub name: String,
}

/// Everything the first-run setup screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunSetupView {
    pub tools: Vec<SetupToolView>,
    pub members: Vec<SetupMemberView>,
    pub pm: Option<PmDesignation>,
    /// `true` while the machine cannot start work: no installed tool is
    /// registered, or nobody is in the ledger and no PM is designated.
    pub needs_setup: bool,
    pub name_max_chars: usize,
}

/// The PM the setup screen asked for: an agent already in the ledger, or a
/// new agent by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmChoice {
    Member(String),
    New(String),
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

/// Whether at least one registered tool is installed on this machine.
pub fn tool_registered(tools: &[SetupToolView]) -> bool {
    tools.iter().any(|tool| tool.installed)
}

pub fn view(
    tools: Vec<SetupToolView>,
    members: Vec<SetupMemberView>,
    pm: Option<PmDesignation>,
) -> FirstRunSetupView {
    // A designation whose member has left the ledger no longer names anyone.
    let pm = pm.filter(|pm| {
        pm.member_id.is_none()
            || members
                .iter()
                .any(|member| pm.names(&member.member_id, &member.name))
    });
    let needs_setup = !tool_registered(&tools) || (members.is_empty() && pm.is_none());
    FirstRunSetupView {
        tools,
        members,
        pm,
        needs_setup,
        name_max_chars: NAME_MAX_CHARS,
    }
}

/// Turn the screen's choice into a designation, or say why it is refused.
///
/// A PM cannot be designated before a tool is registered: the setup never
/// completes on a machine where no agent could run.
pub fn designate(
    tools: &[SetupToolView],
    members: &[SetupMemberView],
    choice: PmChoice,
) -> Result<PmDesignation, String> {
    if !tool_registered(tools) {
        return Err("Register a tool before choosing the Project Manager.".into());
    }
    match choice {
        PmChoice::Member(member_id) => members
            .iter()
            .find(|member| member.member_id == member_id)
            .map(|member| PmDesignation {
                member_id: Some(member.member_id.clone()),
                name: member.name.clone(),
            })
            .ok_or_else(|| "That agent is no longer in the ledger. Choose again.".to_string()),
        PmChoice::New(name) => {
            let name = name.trim();
            if name.is_empty() {
                return Err("Enter a name for the new agent.".into());
            }
            if name.chars().count() > NAME_MAX_CHARS {
                return Err(format!(
                    "Use {NAME_MAX_CHARS} characters or fewer for the name."
                ));
            }
            // A name that is already in the ledger is that agent.
            Ok(
                match members.iter().find(|member| same_name(&member.name, name)) {
                    Some(member) => PmDesignation {
                        member_id: Some(member.member_id.clone()),
                        name: member.name.clone(),
                    },
                    None => PmDesignation {
                        member_id: None,
                        name: name.to_string(),
                    },
                },
            )
        }
    }
}

pub fn path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/first-run-setup.json")
}

/// Read this machine's designation. A machine without one reads as `None`.
pub fn load_designation() -> io::Result<Option<PmDesignation>> {
    let content = match fs::read_to_string(path()) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let stored: Stored = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if stored.schema_version > SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "first-run setup was saved by a newer gwt",
        ));
    }
    Ok(stored.pm)
}

pub fn save_designation(pm: &PmDesignation) -> io::Result<()> {
    let path = path();
    gwt_config::home::assert_test_path(&path);
    fs::create_dir_all(path.parent().expect("machine state parent"))?;
    let bytes = serde_json::to_vec_pretty(&Stored {
        schema_version: SCHEMA_VERSION,
        pm: Some(pm.clone()),
    })
    .map_err(io::Error::other)?;
    crate::issue_monitor::durable_atomic_write(&path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    fn tool(name: &str, installed: bool) -> SetupToolView {
        SetupToolView {
            name: name.into(),
            tool: "claude".into(),
            installed,
        }
    }

    fn member(id: &str, name: &str) -> SetupMemberView {
        SetupMemberView {
            member_id: id.into(),
            name: name.into(),
        }
    }

    /// AC-3: setup is required exactly while the tool pool or the ledger is
    /// not ready, and stops being required once both are.
    #[test]
    fn setup_is_needed_until_a_tool_is_registered_and_a_pm_is_known() {
        assert!(view(Vec::new(), Vec::new(), None).needs_setup);
        assert!(
            view(
                vec![tool("Claude Code 1", false)],
                vec![member("m1", "Aoi")],
                None
            )
            .needs_setup
        );
        assert!(view(vec![tool("Claude Code 1", true)], Vec::new(), None).needs_setup);
        let designated = PmDesignation {
            member_id: None,
            name: "Aoi".into(),
        };
        assert!(
            !view(
                vec![tool("Claude Code 1", true)],
                Vec::new(),
                Some(designated)
            )
            .needs_setup
        );
        // An existing installation is never sent back into setup.
        assert!(
            !view(
                vec![tool("Claude Code 1", true)],
                vec![member("m1", "Aoi")],
                None
            )
            .needs_setup
        );
    }

    #[test]
    fn a_designation_of_a_removed_member_is_not_shown() {
        let gone = PmDesignation {
            member_id: Some("gone".into()),
            name: "Aoi".into(),
        };
        let shown = view(
            vec![tool("Claude Code 1", true)],
            vec![member("m1", "Ren")],
            Some(gone),
        );
        assert_eq!(shown.pm, None);
    }

    /// AC-4: without a registered tool the PM cannot be designated, and the
    /// refusal says why.
    #[test]
    fn designation_is_refused_without_a_registered_tool() {
        let refusal = designate(
            &[tool("Claude Code 1", false)],
            &[],
            PmChoice::New("Aoi".into()),
        );
        assert_eq!(
            refusal,
            Err("Register a tool before choosing the Project Manager.".into())
        );
    }

    /// AC-2: the PM is always named — an existing member or a new name.
    #[test]
    fn designation_names_a_member_or_a_new_agent() {
        let tools = [tool("Claude Code 1", true)];
        let members = [member("m1", "Aoi")];
        assert_eq!(
            designate(&tools, &members, PmChoice::Member("m1".into())),
            Ok(PmDesignation {
                member_id: Some("m1".into()),
                name: "Aoi".into()
            })
        );
        assert_eq!(
            designate(&tools, &members, PmChoice::New("  Ren ".into())),
            Ok(PmDesignation {
                member_id: None,
                name: "Ren".into()
            })
        );
        // A new name that is already in the ledger is that member.
        assert_eq!(
            designate(&tools, &members, PmChoice::New("aoi".into())),
            Ok(PmDesignation {
                member_id: Some("m1".into()),
                name: "Aoi".into()
            })
        );
        assert!(designate(&tools, &members, PmChoice::Member("gone".into())).is_err());
        assert!(designate(&tools, &members, PmChoice::New("   ".into())).is_err());
        assert!(designate(
            &tools,
            &members,
            PmChoice::New("x".repeat(NAME_MAX_CHARS + 1))
        )
        .is_err());
    }

    #[test]
    fn the_designation_round_trips_through_machine_state() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        assert_eq!(load_designation().unwrap(), None);
        let pm = PmDesignation {
            member_id: None,
            name: "Aoi".into(),
        };
        save_designation(&pm).unwrap();
        assert_eq!(load_designation().unwrap(), Some(pm));

        fs::write(path(), br#"{"schema_version":99}"#).unwrap();
        assert_eq!(
            load_designation().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
