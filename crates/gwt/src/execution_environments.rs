//! Machine-wide execution environments (Issue #7741, SPEC #7007 items 77-82).
//!
//! An execution environment is a definition: an account, the tool agents
//! reach it through, and the runtime options a session starts with. Several
//! agents may use one environment at the same time; nothing is reserved.
//!
//! Which environments a project may use, and in what priority order, is
//! saved with that project's Issues settings
//! ([`crate::issue_monitor::ProjectExecutionEnvironment`]). This module owns
//! only the definitions.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const ADOPTABLE_TOOLS: [&str; 5] = ["claude", "codex", "grok", "gh", "agy"];

/// Runtime options a session starts with (item 252: model, skip permissions
/// and Fast mode; how a session is started is decided at launch).
///
/// An unset option (`model: None`, a `false` switch) leaves the launch
/// candidate's own value in place, so creating an environment never changes
/// an existing launch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub fast_mode: bool,
}

/// An account in its tool's pool. Store order defines launch priority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    /// Command id of the tool this account signs in to (`claude`, `codex`).
    pub tool: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<gwt_core::timestamp::UtcTimestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<AccountOrigin>,
    /// The login reported by Copilot itself. This is public account metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in_identity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountOrigin {
    Adopted,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEnvironment {
    pub id: String,
    pub name: String,
    pub account_id: String,
    /// Command id of the tool sessions are started with.
    pub via_tool: String,
    #[serde(default)]
    pub runtime: RuntimeOptions,
}

/// How an environment came to exist, which decides what can be done to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKind {
    /// The account's own tool. Created with the account, deleted with it.
    Set,
    /// The account reached through a tool that reads its own settings file.
    Via,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub environments: Vec<ExecutionEnvironment>,
    /// Issue #7732 (SPEC #7007 item 194): the Project Manager's own list of
    /// environment ids, in priority order. Empty while it was never saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pm_environment_ids: Vec<String>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            accounts: Vec::new(),
            environments: Vec::new(),
            pm_environment_ids: Vec::new(),
        }
    }
}

/// Why an operation was refused. The text is shown next to the disabled
/// control (item 80), so it says what the user can do instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<Refusal> for io::Error {
    fn from(refusal: Refusal) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, refusal.0)
    }
}

fn tool_command(raw: &str) -> Option<String> {
    gwt_agent::resolve_agent_id(raw).map(|agent| agent.command().to_ascii_lowercase())
}

fn tool_name(tool: &str) -> String {
    gwt_agent::resolve_agent_id(tool).map_or_else(
        || tool.to_string(),
        |agent| agent.display_name().to_string(),
    )
}

/// Tools that take their provider and model from their own settings file
/// (item 81). gwt shows those values and never writes them.
pub fn reads_own_settings(tool: &str) -> bool {
    matches!(
        gwt_agent::resolve_agent_id(tool),
        Some(
            gwt_agent::AgentId::Hermes
                | gwt_agent::AgentId::OpenClaw
                | gwt_agent::AgentId::OpenCode
        )
    )
}

/// Tools whose sign-in is their own account (Claude Code, Codex, ...).
pub fn carries_own_account(tool: &str) -> bool {
    matches!(
        gwt_agent::resolve_agent_id(tool),
        Some(
            gwt_agent::AgentId::ClaudeCode
                | gwt_agent::AgentId::Codex
                | gwt_agent::AgentId::GrokBuild
                | gwt_agent::AgentId::Antigravity
                | gwt_agent::AgentId::Copilot
        )
    )
}

impl Store {
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }

    pub fn environment(&self, id: &str) -> Option<&ExecutionEnvironment> {
        self.environments
            .iter()
            .find(|environment| environment.id == id)
    }

    pub fn kind(&self, environment: &ExecutionEnvironment) -> EnvironmentKind {
        match self.account(&environment.account_id) {
            Some(account) if account.tool == environment.via_tool => EnvironmentKind::Set,
            _ => EnvironmentKind::Via,
        }
    }

    /// The environment a launch candidate for `tool` runs on.
    pub fn environment_for_tool(&self, tool: &str) -> Option<&ExecutionEnvironment> {
        let tool = tool_command(tool)?;
        self.environments
            .iter()
            .find(|environment| environment.via_tool == tool)
    }

    /// Register the account of a tool that carries its own sign-in. The
    /// account's own environment is created with it (item 78). Registering a
    /// known tool again changes nothing.
    pub fn register_account(&mut self, tool: &str) -> Option<&Account> {
        let tool = tool_command(tool).filter(|tool| carries_own_account(tool))?;
        if !self.accounts.iter().any(|account| account.tool == tool) {
            self.insert_default_account(&tool);
        }
        self.accounts.iter().find(|account| account.tool == tool)
    }

    fn insert_default_account(&mut self, tool: &str) -> &mut Account {
        let name = format!("{} 1", tool_name(tool));
        self.accounts.push(Account {
            id: tool.to_owned(),
            tool: tool.to_owned(),
            name: name.clone(),
            credential_home: None,
            added_at: None,
            origin: None,
            sign_in_identity: None,
        });
        if self.environment_for_tool(tool).is_none() {
            self.environments.push(ExecutionEnvironment {
                id: tool.to_owned(),
                name,
                account_id: tool.to_owned(),
                via_tool: tool.to_owned(),
                runtime: RuntimeOptions::default(),
            });
        }
        self.accounts.last_mut().expect("just inserted account")
    }

    fn adopt_defaults(&mut self, mut probe: impl FnMut(&str) -> Option<Option<String>>) {
        for tool in ADOPTABLE_TOOLS {
            let Some(identity) = probe(tool) else {
                continue;
            };
            let existing = self
                .accounts
                .iter()
                .position(|account| account.tool == tool && account.credential_home.is_none());
            let account = match existing {
                Some(index) => &mut self.accounts[index],
                None => self.insert_default_account(tool),
            };
            if account.origin.is_none() {
                account.origin = Some(AccountOrigin::Adopted);
                account.added_at = Some(gwt_core::timestamp::UtcTimestamp::now());
                account.sign_in_identity = identity;
            }
        }
    }

    /// Try the pool in priority order, starting with the recorded account on
    /// resume. If it is gone, the current pool order supplies the fallback.
    pub fn select_pool_account<T>(
        &self,
        tool: &str,
        preferred: Option<&str>,
        mut usable: impl FnMut(&Account) -> Option<T>,
    ) -> Option<(&Account, T)> {
        let pool: Vec<_> = self
            .accounts
            .iter()
            .filter(|account| account.tool == tool)
            .collect();
        let start = preferred
            .and_then(|id| pool.iter().position(|account| account.id == id))
            .unwrap_or(0);
        pool[start..]
            .iter()
            .chain(&pool[..start])
            .find_map(|&account| usable(account).map(|launch| (account, launch)))
    }

    /// Keep a pool's own environment while it still has an account. Returns
    /// the ids of the deleted environments for removal from project lists.
    pub fn delete_account(&mut self, account_id: &str) -> Vec<String> {
        let Some(index) = self
            .accounts
            .iter()
            .position(|account| account.id == account_id)
        else {
            return Vec::new();
        };
        let removed = self.accounts.remove(index);
        if let Some(next) = self
            .accounts
            .iter()
            .find(|account| account.tool == removed.tool)
        {
            for environment in &mut self.environments {
                if environment.account_id == account_id && environment.via_tool == removed.tool {
                    environment.account_id.clone_from(&next.id);
                }
            }
        }
        let (deleted, kept) = std::mem::take(&mut self.environments)
            .into_iter()
            .partition(|environment| environment.account_id == account_id);
        self.environments = kept;
        let deleted: Vec<String> = deleted
            .into_iter()
            .map(|environment: ExecutionEnvironment| environment.id)
            .collect();
        self.pm_environment_ids.retain(|id| !deleted.contains(id));
        deleted
    }

    /// Why `account_id` cannot be registered through `via_tool`, if it cannot.
    pub fn via_refusal(&self, account_id: &str, via_tool: &str) -> Option<Refusal> {
        let Some(account) = self.account(account_id) else {
            return Some(Refusal("This account is no longer registered.".into()));
        };
        let Some(tool) = tool_command(via_tool) else {
            return Some(Refusal("Choose the tool to go through.".into()));
        };
        if tool == account.tool {
            return Some(Refusal(format!(
                "{} already has its own environment. It was created with the account.",
                account.name
            )));
        }
        if !reads_own_settings(&tool) {
            return Some(Refusal(format!(
                "{} signs in to its own account, so another account cannot be used through it.",
                tool_name(&tool)
            )));
        }
        // A tool that reads its own settings file uses the one provider that
        // file names, so two accounts through it could not be told apart.
        self.environment_for_tool(&tool).map(|existing| {
            Refusal(format!(
                "{} is already registered as \"{}\". It uses the provider set in its own settings.",
                tool_name(&tool),
                existing.name
            ))
        })
    }

    /// Register `account_id` reached through `via_tool` (item 78 (2)).
    pub fn register_via(
        &mut self,
        account_id: &str,
        via_tool: &str,
        name: Option<&str>,
    ) -> Result<&ExecutionEnvironment, Refusal> {
        if let Some(refusal) = self.via_refusal(account_id, via_tool) {
            return Err(refusal);
        }
        let tool = tool_command(via_tool).expect("validated by via_refusal");
        let account = self.account(account_id).expect("validated by via_refusal");
        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map_or_else(
                || format!("{} (via {})", account.name, tool_name(&tool)),
                str::to_string,
            );
        self.environments.push(ExecutionEnvironment {
            id: format!("{account_id}:{tool}"),
            name,
            account_id: account_id.to_string(),
            via_tool: tool,
            runtime: RuntimeOptions::default(),
        });
        Ok(self.environments.last().expect("just pushed"))
    }

    /// Why `environment_id` cannot be deleted on its own, if it cannot.
    pub fn delete_refusal(&self, environment_id: &str) -> Option<Refusal> {
        let environment = self.environment(environment_id)?;
        (self.kind(environment) == EnvironmentKind::Set).then(|| {
            Refusal("Created with the account. It is deleted when the account is deleted.".into())
        })
    }

    /// Delete one environment registered through a tool. The account and its
    /// other environments stay.
    pub fn delete_environment(&mut self, environment_id: &str) -> Result<(), Refusal> {
        if let Some(refusal) = self.delete_refusal(environment_id) {
            return Err(refusal);
        }
        let before = self.environments.len();
        self.environments
            .retain(|environment| environment.id != environment_id);
        if self.environments.len() == before {
            return Err(Refusal(
                "This execution environment no longer exists.".into(),
            ));
        }
        self.pm_environment_ids.retain(|id| id != environment_id);
        Ok(())
    }

    /// The Project Manager's execution environments in priority order, and
    /// whether that list was saved (item 194). A machine that never saved
    /// one shows each account's own environment, in the order registered.
    pub fn pm_list(&self) -> (bool, Vec<&ExecutionEnvironment>) {
        let saved: Vec<_> = self
            .pm_environment_ids
            .iter()
            .filter_map(|id| self.environment(id))
            .collect();
        if !saved.is_empty() {
            return (true, saved);
        }
        let own = self
            .environments
            .iter()
            .filter(|environment| self.kind(environment) == EnvironmentKind::Set)
            .collect();
        (false, own)
    }

    /// Replace the Project Manager's list. The last entry cannot be removed
    /// (items 220 and 233): an empty list is refused here, whatever the
    /// screen allowed.
    pub fn set_pm_list(&mut self, environment_ids: &[String]) -> Result<(), Refusal> {
        if environment_ids.is_empty() {
            return Err(Refusal(
                "Keep at least one execution environment. The Project Manager cannot start a session without one."
                    .into(),
            ));
        }
        for (index, id) in environment_ids.iter().enumerate() {
            if self.environment(id).is_none() {
                return Err(Refusal(
                    "An execution environment on the list no longer exists. Reload and try again."
                        .into(),
                ));
            }
            if environment_ids[..index].contains(id) {
                return Err(Refusal(
                    "An execution environment is on the list more than once. Keep one of them."
                        .into(),
                ));
            }
        }
        self.pm_environment_ids = environment_ids.to_vec();
        Ok(())
    }

    /// Why the runtime options of `environment_id` cannot be edited here.
    pub fn runtime_edit_refusal(&self, environment_id: &str) -> Option<Refusal> {
        let environment = self.environment(environment_id)?;
        reads_own_settings(&environment.via_tool).then(|| {
            Refusal(format!(
                "{} reads these from its own settings file. Change them there.",
                tool_name(&environment.via_tool)
            ))
        })
    }

    /// Rename an environment and, where the tool allows it, change its
    /// runtime options. The account and the tool never change (item 78 (3)).
    pub fn update_environment(
        &mut self,
        environment_id: &str,
        name: Option<&str>,
        runtime: Option<RuntimeOptions>,
    ) -> Result<&ExecutionEnvironment, Refusal> {
        if runtime.is_some() {
            if let Some(refusal) = self.runtime_edit_refusal(environment_id) {
                return Err(refusal);
            }
        }
        let environment = self
            .environments
            .iter_mut()
            .find(|environment| environment.id == environment_id)
            .ok_or_else(|| Refusal("This execution environment no longer exists.".into()))?;
        if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
            environment.name = name.to_string();
        }
        if let Some(mut runtime) = runtime {
            runtime.model = runtime
                .model
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty());
            environment.runtime = runtime;
        }
        Ok(environment)
    }
}

pub fn path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/execution-environments.json")
}

/// Read the saved definitions. A machine that has none yet reads as empty.
pub fn load() -> io::Result<Store> {
    load_from(&path())
}

fn load_from(path: &Path) -> io::Result<Store> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(error) => return Err(error),
    };
    let store: Store = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if store.schema_version > SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "execution environments were saved by a newer gwt",
        ));
    }
    Ok(store)
}

fn lock() -> io::Result<gwt_core::operation_deadline::NamedFileLock> {
    let path = path();
    let lock_path = path.with_extension("lock");
    gwt_config::home::assert_test_path(&path);
    gwt_config::home::assert_test_path(&lock_path);
    fs::create_dir_all(path.parent().expect("machine state parent"))?;
    gwt_core::operation_deadline::NamedFileLock::acquire(&lock_path, "execution environments")
}

/// Change the saved definitions under the cross-process lock. Nothing is
/// written when the change is refused or leaves the definitions as they were.
pub fn mutate<T>(change: impl FnOnce(&mut Store) -> io::Result<T>) -> io::Result<(Store, T)> {
    let _lock = lock()?;
    let mut store = load()?;
    let before = store.clone();
    let result = change(&mut store)?;
    if store != before {
        let bytes = serde_json::to_vec_pretty(&store).map_err(io::Error::other)?;
        crate::issue_monitor::durable_atomic_write(&path(), &bytes)?;
    }
    Ok((store, result))
}

/// Register the account of every tool in `tools` that carries its own
/// sign-in. Run with the tools a project already launches and the tools
/// installed on this machine, this is the one-time carry-over of an existing
/// setup: afterwards each of them has its environment.
pub fn ensure_accounts(tools: &[String]) -> io::Result<Store> {
    let current = load()?;
    let missing = |store: &Store| {
        tools.iter().any(|tool| {
            tool_command(tool).is_some_and(|tool| {
                carries_own_account(&tool) && !store.accounts.iter().any(|a| a.tool == tool)
            })
        })
    };
    if !missing(&current) {
        return Ok(current);
    }
    mutate(|store| {
        for tool in tools {
            store.register_account(tool);
        }
        Ok(())
    })
    .map(|(store, ())| store)
}

/// Import accounts whose native sign-in is present under the default home.
/// Probe outside the store lock so slower providers cannot block writers.
pub fn adopt_signed_in_accounts() -> io::Result<Store> {
    adopt_default_accounts(&ADOPTABLE_TOOLS)
}

pub(crate) fn adopt_default_accounts(tools: &[&str]) -> io::Result<Store> {
    let current = load()?;
    let signed_in: Vec<_> = tools
        .iter()
        .filter(|&&tool| {
            !current.accounts.iter().any(|account| {
                account.tool == tool
                    && account.credential_home.is_none()
                    && account.origin.is_some()
            })
        })
        .filter_map(|&tool| {
            crate::account_homes::sign_in_state(tool, None).map(|state| (tool, state.identity))
        })
        .collect();
    if signed_in.is_empty() {
        return Ok(current);
    }
    mutate(|store| {
        store.adopt_defaults(|tool| {
            signed_in
                .iter()
                .find(|(candidate, _)| *candidate == tool)
                .map(|(_, identity)| identity.clone())
        });
        Ok(())
    })
    .map(|(store, ())| store)
}

/// Apply an environment's runtime options to the candidate about to start a
/// session (item 78 (3): a change takes effect from the next session).
///
/// A value set on the environment wins. An unset one keeps the candidate's
/// own value, so an environment nobody edited changes no launch. An automatic
/// tier chooses its own model, which the environment does not replace.
pub fn apply_runtime(
    profile: &mut crate::issue_monitor::IssueMonitorLaunchProfile,
    runtime: &RuntimeOptions,
    model_chosen_by_tier: bool,
) {
    if !model_chosen_by_tier {
        if let Some(model) = &runtime.model {
            profile.model = Some(model.clone());
        }
    }
    profile.skip_permissions |= runtime.skip_permissions;
    profile.fast_mode |= runtime.fast_mode;
}

/// A launch candidate as its session starts: the agent's fixed traits first
/// (Issue #7647), then the runtime options of the execution environment it
/// runs on, which win over the traits (item 189). Definitions that cannot be
/// read change nothing, so a damaged file never stops a launch.
pub fn candidate_as_started(
    prefs: &crate::issue_monitor::IssueMonitorPrefs,
    mut profile: crate::issue_monitor::IssueMonitorLaunchProfile,
    model_chosen_by_tier: bool,
) -> crate::issue_monitor::IssueMonitorLaunchProfile {
    crate::agent_names::apply_member_traits(prefs, &mut profile);
    match load() {
        Ok(store) => {
            if let Some(environment) =
                environment_for_candidate(&store, &prefs.execution_environments, &profile.agent_id)
            {
                apply_runtime(&mut profile, &environment.runtime, model_chosen_by_tier);
            }
        }
        Err(error) => tracing::warn!(
            error = %error,
            "execution environments are unreadable; the launch candidate keeps its own options"
        ),
    }
    profile
}

/// The environment a project's candidate for `tool` runs on: the one the
/// project's saved list names, else the one registered for the tool.
pub fn environment_for_candidate<'store>(
    store: &'store Store,
    project_list: &[crate::issue_monitor::ProjectExecutionEnvironment],
    tool: &str,
) -> Option<&'store ExecutionEnvironment> {
    let tool = tool_command(tool)?;
    match project_list.iter().find(|entry| entry.tool == tool) {
        Some(entry) => store.environment(&entry.environment_id),
        None => store.environment_for_tool(&tool),
    }
}

/// The project's available execution environments as its settings show
/// them, and whether that list was ever saved.
///
/// A project that never saved one shows the tools it launches today, each as
/// the environment registered for it. A tool with no registered environment
/// keeps its place under its own id, so saving the list never drops a tool
/// the project was already using.
pub fn project_list(
    store: &Store,
    prefs: &crate::issue_monitor::IssueMonitorPrefs,
) -> (bool, Vec<crate::issue_monitor::ProjectExecutionEnvironment>) {
    let saved = crate::issue_monitor::normalize_project_execution_environments(
        prefs.execution_environments.clone(),
    );
    if !saved.is_empty() {
        return (true, saved);
    }
    let derived = prefs
        .current_launch_tools()
        .into_iter()
        .map(|tool| crate::issue_monitor::ProjectExecutionEnvironment {
            environment_id: store
                .environment_for_tool(&tool)
                .map_or_else(|| tool.clone(), |environment| environment.id.clone()),
            tool,
        })
        .collect();
    (false, derived)
}

/// One value a tool reports from its own settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSetting {
    pub label: String,
    pub value: String,
}

/// What a tool that reads its own settings is currently set to (item 81).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSettings {
    /// The file the values were read from, shown so the user can edit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub values: Vec<ToolSetting>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn tool_settings_path(tool: &str, home: &Path) -> Option<PathBuf> {
    match gwt_agent::resolve_agent_id(tool)? {
        gwt_agent::AgentId::Hermes => Some(home.join(".hermes/config.yaml")),
        gwt_agent::AgentId::OpenCode => Some(home.join(".config/opencode/opencode.json")),
        gwt_agent::AgentId::OpenClaw => Some(home.join(".openclaw/openclaw.json")),
        _ => None,
    }
}

fn setting(values: &mut Vec<ToolSetting>, label: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        values.push(ToolSetting {
            label: label.to_string(),
            value: value.to_string(),
        });
    }
}

/// A `provider/model` reference as OpenCode and OpenClaw write it.
fn provider_and_model(values: &mut Vec<ToolSetting>, reference: Option<&str>) {
    let Some(reference) = reference.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    match reference.split_once('/') {
        Some((provider, model)) => {
            setting(values, "Provider", Some(provider));
            setting(values, "Model", Some(model));
        }
        None => setting(values, "Model", Some(reference)),
    }
}

/// Read what `tool` is set to from its settings file under `home`. Only the
/// values the file states are returned; nothing is guessed for a missing or
/// unreadable file.
pub fn read_tool_settings(tool: &str, home: &Path) -> ToolSettings {
    let Some(path) = tool_settings_path(tool, home) else {
        return ToolSettings::default();
    };
    let mut values = Vec::new();
    if let Ok(text) = fs::read_to_string(&path) {
        if path
            .extension()
            .is_some_and(|extension| extension == "yaml")
        {
            if let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
                let model = root.get("model");
                let text_at = |key: &str| model?.get(key)?.as_str();
                setting(&mut values, "Provider", text_at("provider"));
                setting(&mut values, "Model", text_at("default"));
            }
        } else if let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) {
            let reference = root
                .get("model")
                .and_then(serde_json::Value::as_str)
                .or_else(|| root.pointer("/agents/defaults/model/primary")?.as_str())
                .or_else(|| root.pointer("/agents/defaults/model")?.as_str());
            provider_and_model(&mut values, reference);
        }
    }
    ToolSettings {
        path: Some(path.display().to_string()),
        values,
    }
}

/// [`read_tool_settings`] for the signed-in user's home directory.
pub fn tool_settings(tool: &str) -> ToolSettings {
    home_dir().map_or_else(ToolSettings::default, |home| {
        read_tool_settings(tool, &home)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(tools: &[&str]) -> Store {
        let mut store = Store::default();
        for tool in tools {
            store.register_account(tool);
        }
        store
    }

    #[test]
    fn registering_an_account_creates_its_own_environment_once() {
        let mut store = Store::default();
        store.register_account("Claude Code").expect("known tool");
        store.register_account("claude").expect("same tool");

        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.environments.len(), 1);
        let environment = &store.environments[0];
        assert_eq!(environment.name, "Claude Code 1");
        assert_eq!(environment.via_tool, "claude");
        assert_eq!(store.kind(environment), EnvironmentKind::Set);
        assert_eq!(environment.runtime, RuntimeOptions::default());
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn pm_ids(store: &Store) -> (bool, Vec<&str>) {
        let (saved, list) = store.pm_list();
        (saved, list.iter().map(|e| e.id.as_str()).collect())
    }

    #[test]
    fn the_project_manager_list_starts_from_the_accounts_and_keeps_its_saved_order() {
        let mut store = store_with(&["claude", "codex"]);
        store.register_via("claude", "hermes", None).unwrap();
        // Never saved: each account's own environment, in the order registered.
        assert_eq!(pm_ids(&store), (false, vec!["claude", "codex"]));

        store
            .set_pm_list(&ids(&["codex", "claude:hermes"]))
            .unwrap();
        assert_eq!(pm_ids(&store), (true, vec!["codex", "claude:hermes"]));

        // A machine that saved no list writes none, and an older file reads as "never saved".
        let saved = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<Store>(&saved).unwrap(), store);
        let never = serde_json::to_string(&store_with(&["claude"])).unwrap();
        assert!(!never.contains("pm_environment_ids"), "{never}");
        assert!(serde_json::from_str::<Store>(&never)
            .unwrap()
            .pm_environment_ids
            .is_empty());
    }

    #[test]
    fn the_last_project_manager_environment_cannot_be_removed() {
        let mut store = store_with(&["claude", "codex"]);
        store.set_pm_list(&ids(&["codex"])).unwrap();
        let before = store.clone();

        let empty = store.set_pm_list(&[]).unwrap_err();
        assert!(empty.0.contains("Keep at least one"), "{empty}");
        let gone = store.set_pm_list(&ids(&["codex", "gone"])).unwrap_err();
        assert!(gone.0.contains("no longer exists"), "{gone}");
        let twice = store.set_pm_list(&ids(&["codex", "codex"])).unwrap_err();
        assert!(twice.0.contains("more than once"), "{twice}");
        assert_eq!(store, before, "a refused save changes nothing");
    }

    #[test]
    fn deleting_an_account_or_environment_takes_it_off_the_project_manager_list() {
        let mut store = store_with(&["claude", "codex"]);
        store.register_via("claude", "hermes", None).unwrap();
        store
            .set_pm_list(&ids(&["claude:hermes", "codex"]))
            .unwrap();

        store.delete_environment("claude:hermes").unwrap();
        assert_eq!(pm_ids(&store), (true, vec!["codex"]));
        // Deleting the account behind the last entry is not stopped. The
        // list goes back to the one made from the remaining accounts.
        store.delete_account("codex");
        assert!(store.pm_environment_ids.is_empty());
        assert_eq!(pm_ids(&store), (false, vec!["claude"]));
    }

    #[test]
    fn a_tool_without_its_own_account_registers_nothing() {
        let mut store = Store::default();
        assert!(store.register_account("hermes").is_none());
        assert!(store.register_account("my-custom-agent").is_none());
        assert_eq!(store, Store::default());
    }

    #[test]
    fn an_account_can_be_registered_through_a_tool_that_reads_its_own_settings() {
        let mut store = store_with(&["claude"]);
        let environment = store
            .register_via("claude", "Hermes Agent", None)
            .expect("registers")
            .clone();

        assert_eq!(environment.name, "Claude Code 1 (via Hermes Agent)");
        assert_eq!(environment.via_tool, "hermes");
        assert_eq!(store.kind(&environment), EnvironmentKind::Via);
        // A different tool is a different environment of the same account.
        store
            .register_via("claude", "opencode", Some("  Claude on OpenCode "))
            .expect("second tool");
        assert_eq!(store.environments.len(), 3);
        assert_eq!(store.environments[2].name, "Claude on OpenCode");
    }

    #[test]
    fn registering_through_a_tool_is_refused_with_a_reason() {
        let mut store = store_with(&["claude", "codex"]);
        store.register_via("claude", "hermes", None).unwrap();

        let own = store.register_via("claude", "claude", None).unwrap_err();
        assert!(own.0.contains("already has its own environment"), "{own}");
        let other_account = store.register_via("claude", "codex", None).unwrap_err();
        assert!(other_account.0.contains("signs in to its own account"));
        let taken = store.register_via("codex", "hermes", None).unwrap_err();
        assert!(taken.0.contains("already registered"), "{taken}");
        let unknown = store.register_via("gone", "hermes", None).unwrap_err();
        assert!(unknown.0.contains("no longer registered"));
        assert_eq!(store.environments.len(), 3);
    }

    #[test]
    fn only_an_environment_registered_through_a_tool_can_be_deleted_on_its_own() {
        let mut store = store_with(&["claude"]);
        store.register_via("claude", "hermes", None).unwrap();

        let refusal = store.delete_environment("claude").unwrap_err();
        assert!(refusal.0.contains("deleted when the account is deleted"));
        store.delete_environment("claude:hermes").expect("deletes");
        assert_eq!(store.environments.len(), 1);
        assert_eq!(store.accounts.len(), 1, "the account stays");
        assert!(store.delete_environment("claude:hermes").is_err());
    }

    #[test]
    fn deleting_an_account_deletes_every_environment_that_uses_it() {
        let mut store = store_with(&["claude", "codex"]);
        store.register_via("claude", "hermes", None).unwrap();

        let deleted = store.delete_account("claude");
        assert_eq!(deleted, vec!["claude".to_string(), "claude:hermes".into()]);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.environments.len(), 1);
        assert_eq!(store.environments[0].id, "codex");
        assert!(store.delete_account("claude").is_empty());
    }

    #[test]
    fn runtime_options_are_editable_only_where_gwt_owns_them() {
        let mut store = store_with(&["claude"]);
        store.register_via("claude", "hermes", None).unwrap();
        let runtime = RuntimeOptions {
            model: Some("  opus ".into()),
            skip_permissions: true,
            fast_mode: true,
        };

        let saved = store
            .update_environment("claude", Some("Work"), Some(runtime.clone()))
            .expect("the account's own tool is editable");
        assert_eq!(saved.name, "Work");
        assert_eq!(saved.runtime.model.as_deref(), Some("opus"));
        assert!(saved.runtime.skip_permissions && saved.runtime.fast_mode);

        let refusal = store
            .update_environment("claude:hermes", None, Some(runtime))
            .unwrap_err();
        assert!(refusal.0.contains("its own settings file"), "{refusal}");
        // Renaming does not touch the tool's settings, so it is allowed.
        store
            .update_environment("claude:hermes", Some("Hermes"), None)
            .expect("rename");
        assert_eq!(store.environment("claude:hermes").unwrap().name, "Hermes");
        assert_eq!(
            store.environment("claude:hermes").unwrap().runtime,
            RuntimeOptions::default()
        );
    }

    #[test]
    fn an_empty_model_means_the_tool_default() {
        let mut store = store_with(&["codex"]);
        let runtime = RuntimeOptions {
            model: Some("   ".into()),
            ..RuntimeOptions::default()
        };
        let saved = store
            .update_environment("codex", None, Some(runtime))
            .unwrap();
        assert_eq!(saved.runtime.model, None);
    }

    #[test]
    fn old_account_definitions_and_pool_metadata_round_trip() {
        let mut account: Account =
            serde_json::from_str(r#"{"id":"codex","tool":"codex","name":"Codex 1"}"#).unwrap();
        assert_eq!(account.credential_home, None);
        assert_eq!(account.origin, None);
        assert_eq!(account.added_at, None);
        account.credential_home = Some("/accounts/codex/second".into());
        account.origin = Some(AccountOrigin::Added);
        account.added_at = Some(gwt_core::timestamp::UtcTimestamp::now());
        assert_eq!(
            serde_json::from_str::<Account>(&serde_json::to_string(&account).unwrap()).unwrap(),
            account
        );
    }

    #[test]
    fn adoption_uses_sign_in_evidence_and_keeps_one_environment_per_tool() {
        let mut store = Store::default();
        let probe = |tool: &str| match tool {
            "claude" | "codex" | "grok" => Some(None),
            "gh" => Some(Some("existing-user".to_string())),
            _ => None,
        };
        store.adopt_defaults(probe);
        store.adopt_defaults(probe);
        assert_eq!(store.accounts.len(), 4);
        assert_eq!(store.environments.len(), 4);
        assert_eq!(
            store.account("grok").unwrap().origin,
            Some(AccountOrigin::Adopted)
        );
        assert_eq!(
            store.account("gh").unwrap().sign_in_identity.as_deref(),
            Some("existing-user")
        );
        assert!(store.environment_for_tool("agy").is_none());
    }

    #[test]
    fn a_pool_keeps_its_environment_and_prefers_the_recorded_usable_account() {
        let mut store = store_with(&["codex"]);
        store.pm_environment_ids.push("codex".into());
        for id in ["second", "third"] {
            let mut account = store.account("codex").unwrap().clone();
            account.id = id.into();
            account.credential_home = Some(format!("/accounts/codex/{id}"));
            store.accounts.push(account);
        }
        let select = |store: &Store, previous, unavailable| {
            store
                .select_pool_account("codex", previous, |account| {
                    (account.id != unavailable).then(|| account.id.clone())
                })
                .map(|(_, id)| id)
        };
        assert_eq!(select(&store, None, "codex").as_deref(), Some("second"));
        assert_eq!(select(&store, Some("third"), "").as_deref(), Some("third"));
        assert_eq!(
            select(&store, Some("second"), "second").as_deref(),
            Some("third")
        );
        assert!(store.delete_account("codex").is_empty());
        assert_eq!(store.environment("codex").unwrap().account_id, "second");
        assert_eq!(store.pm_list().1.len(), 1);
        assert_eq!(select(&store, Some("codex"), "").as_deref(), Some("second"));
    }

    #[test]
    fn saved_definitions_round_trip_and_a_missing_file_reads_as_empty() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());

        assert_eq!(load().unwrap(), Store::default());
        assert!(!path().exists(), "reading creates nothing");

        let store = ensure_accounts(&["claude".into(), "hermes".into(), "Codex".into()]).unwrap();
        assert_eq!(store.accounts.len(), 2);
        assert_eq!(load().unwrap(), store);

        let (saved, ()) = mutate(|store| {
            store.register_via("codex", "opencode", None)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(load().unwrap(), saved);
        // A refused change writes nothing.
        let before = fs::read(path()).unwrap();
        assert!(mutate(|store| Ok(store.delete_environment("codex")?)).is_err());
        assert_eq!(fs::read(path()).unwrap(), before);
        // Carrying over an already known setup rewrites nothing either.
        ensure_accounts(&["claude".into()]).unwrap();
        assert_eq!(fs::read(path()).unwrap(), before);
    }

    #[test]
    fn definitions_from_a_newer_gwt_are_not_overwritten() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        fs::create_dir_all(path().parent().unwrap()).unwrap();
        fs::write(
            path(),
            r#"{"schema_version":2,"accounts":[],"environments":[]}"#,
        )
        .unwrap();

        assert_eq!(load().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(ensure_accounts(&["claude".into()]).is_err());
    }
    #[test]
    fn an_environment_changes_only_what_it_sets() {
        let mut profile: crate::issue_monitor::IssueMonitorLaunchProfile =
            serde_json::from_value(serde_json::json!({"agent_id": "claude", "model": "opus"}))
                .unwrap();
        let untouched = profile.clone();

        apply_runtime(&mut profile, &RuntimeOptions::default(), false);
        assert_eq!(
            profile, untouched,
            "an unedited environment changes nothing"
        );

        let runtime = RuntimeOptions {
            model: Some("sonnet".into()),
            skip_permissions: true,
            fast_mode: true,
        };
        apply_runtime(&mut profile, &runtime, false);
        assert_eq!(profile.model.as_deref(), Some("sonnet"));
        assert!(profile.skip_permissions && profile.fast_mode);

        let mut tiered = untouched.clone();
        apply_runtime(&mut tiered, &runtime, true);
        assert_eq!(tiered.model.as_deref(), Some("opus"), "the tier chose it");
        assert!(tiered.skip_permissions && tiered.fast_mode);
    }

    #[test]
    fn a_candidate_runs_on_the_environment_its_project_lists() {
        let mut store = store_with(&["claude"]);
        store.register_via("claude", "hermes", None).unwrap();
        let list = vec![crate::issue_monitor::ProjectExecutionEnvironment {
            environment_id: "gone".into(),
            tool: "hermes".into(),
        }];

        let unsaved = environment_for_candidate(&store, &[], "Hermes Agent").unwrap();
        assert_eq!(unsaved.id, "claude:hermes");
        assert!(
            environment_for_candidate(&store, &list, "hermes").is_none(),
            "a listed environment that was deleted applies nothing"
        );
        assert_eq!(
            environment_for_candidate(&store, &list, "claude")
                .unwrap()
                .id,
            "claude"
        );
        assert!(environment_for_candidate(&store, &[], "codex").is_none());
    }

    #[test]
    fn tool_settings_are_read_from_the_tools_own_file() {
        let home = tempfile::tempdir().unwrap();
        let write = |relative: &str, body: &str| {
            let path = home.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        };
        let values = |tool: &str| -> Vec<(String, String)> {
            read_tool_settings(tool, home.path())
                .values
                .into_iter()
                .map(|setting| (setting.label, setting.value))
                .collect()
        };
        let pair = |provider: &str, model: &str| {
            vec![
                ("Provider".to_string(), provider.to_string()),
                ("Model".to_string(), model.to_string()),
            ]
        };

        // Nothing is guessed before the tool has a settings file.
        let missing = read_tool_settings("hermes", home.path());
        assert!(missing.values.is_empty());
        assert!(missing.path.unwrap().ends_with("config.yaml"));

        write(
            ".hermes/config.yaml",
            "model:\n  provider: zai\n  default: glm-4.6\n",
        );
        assert_eq!(values("hermes"), pair("zai", "glm-4.6"));
        write(
            ".config/opencode/opencode.json",
            r#"{"model":"anthropic/claude-sonnet"}"#,
        );
        assert_eq!(values("opencode"), pair("anthropic", "claude-sonnet"));
        write(
            ".openclaw/openclaw.json",
            r#"{"agents":{"defaults":{"model":{"primary":"openai/gpt-6"}}}}"#,
        );
        assert_eq!(values("openclaw"), pair("openai", "gpt-6"));

        write(".config/opencode/opencode.json", "not json");
        assert!(values("opencode").is_empty());
        assert_eq!(
            read_tool_settings("claude", home.path()),
            ToolSettings::default()
        );
    }

    #[test]
    fn an_unsaved_project_list_is_what_the_project_launches_today() {
        let mut store = store_with(&["claude"]);
        store.register_via("claude", "hermes", None).unwrap();
        let candidate = |agent: &str| -> crate::issue_monitor::IssueMonitorLaunchProfile {
            serde_json::from_value(serde_json::json!({ "agent_id": agent })).unwrap()
        };
        let mut prefs = crate::issue_monitor::IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(vec![
            candidate("hermes"),
            candidate("claude"),
            candidate("my-custom-agent"),
        ]);
        let ids = |prefs: &crate::issue_monitor::IssueMonitorPrefs| {
            let (saved, list) = project_list(&store, prefs);
            let ids: Vec<_> = list.into_iter().map(|entry| entry.environment_id).collect();
            (saved, ids)
        };

        // The custom tool has no environment yet and keeps its place.
        assert_eq!(
            ids(&prefs),
            (
                false,
                vec![
                    "claude:hermes".to_string(),
                    "claude".into(),
                    "my-custom-agent".into()
                ]
            )
        );
        prefs.execution_environments = vec![crate::issue_monitor::ProjectExecutionEnvironment {
            environment_id: "claude".into(),
            tool: "claude".into(),
        }];
        assert_eq!(ids(&prefs), (true, vec!["claude".to_string()]));
    }
}
