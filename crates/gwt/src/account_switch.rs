//! Resolve the execution account against the final host profile before spawn.

use std::{collections::HashMap, path::Path};

use gwt_agent::{
    AccountSwitch, AccountSwitchReason, AgentId, ExecutionAccount, LaunchConfig, Session,
};
use gwt_core::timestamp::UtcTimestamp;

use crate::{
    account_homes::{self, VerifiedCopilot, COPILOT_TOKEN_ENV},
    execution_environments::{self, Account, Store},
};

pub(crate) struct PoolAccountLaunch {
    pub(crate) account: ExecutionAccount,
    pub(crate) copilot: Option<VerifiedCopilot>,
}

/// Refusals observed in one pane during a switch. Existing sessions outside
/// this pane never inherit these exclusions.
#[derive(Default)]
pub(crate) struct PoolSwitchAttempt {
    held_accounts: HashMap<String, Option<UtcTimestamp>>,
}

impl PoolSwitchAttempt {
    pub(crate) fn record_limit(&mut self, account: &str, reset: Option<UtcTimestamp>) {
        self.held_accounts.insert(account.to_owned(), reset);
    }

    pub(crate) fn next_account<'a>(
        &self,
        pool: impl IntoIterator<Item = &'a str>,
        current: &str,
        now: UtcTimestamp,
    ) -> Option<&'a str> {
        let mut pool: Vec<_> = pool.into_iter().collect();
        if let Some(index) = pool.iter().position(|id| *id == current) {
            pool.rotate_left(index + 1);
        }
        pool.into_iter().find(|id| {
            self.held_accounts
                .get(*id)
                .is_none_or(|reset| reset.is_some_and(|at| at <= now))
        })
    }

    pub(crate) fn next_reset<'a>(
        &self,
        pool: impl IntoIterator<Item = &'a str>,
        now: UtcTimestamp,
    ) -> Option<UtcTimestamp> {
        pool.into_iter()
            .filter_map(|id| self.held_accounts.get(id).copied().flatten())
            .filter(|at| *at > now)
            .min()
    }
}

pub(crate) fn pool_account_for_launch(
    config: &mut LaunchConfig,
    predecessor: Option<&Session>,
    cwd: &Path,
    mut report: impl FnMut(String),
) -> Result<Option<PoolAccountLaunch>, String> {
    let switching = config.execution_account_switch_reason.is_some();
    if switching && config.execution_account_id.is_none() {
        return Err("An account switch requires an exact target account.".into());
    }
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return Ok(None);
    }
    let tool = config.agent_id.command();
    let Some(env_key) = account_homes::home_env_key(tool) else {
        return Ok(None);
    };
    if has_auth_override(config, env_key) {
        if switching {
            return Err("The configured authentication prevents an account switch.".into());
        }
        let message = format!(
            "Using the configured {} authentication instead of an account pool.",
            config.agent_id.display_name()
        );
        tracing::info!(
            agent = tool,
            "explicit authentication overrides execution account selection"
        );
        report(message);
        return Ok(None);
    }
    let store = match execution_environments::adopt_default_accounts(&[tool]) {
        Ok(store) => store,
        Err(error)
            if predecessor
                .and_then(|s| s.execution_account.as_ref())
                .is_none()
                && config.execution_account_id.is_none() =>
        {
            tracing::warn!(%error, "Could not read execution accounts; using the existing launch environment");
            return Ok(None);
        }
        Err(error) => return Err(format!("Could not restore the execution account: {error}")),
    };
    let previous = predecessor.and_then(|session| session.execution_account.as_ref());
    let preferred = config.execution_account_id.as_deref();
    let selected = choose_account(&store, tool, previous, preferred, |account| {
        if switching && preferred != Some(account.id.as_str()) {
            return None;
        }
        let home = account.credential_home.as_deref().map(Path::new);
        if home.is_some_and(|home| !account_homes::is_managed_home(tool, &account.id, home)) {
            tracing::info!(account_id = %account.id, "execution account home is unavailable");
            return None;
        }
        if config.agent_id == AgentId::Copilot {
            let identity = account.sign_in_identity.as_deref()?;
            return match account_homes::verified_copilot_launch(
                &account.id,
                home,
                identity,
                &config.env_vars,
                &config.remove_env,
                cwd,
            ) {
                Ok(copilot) => Some(Some(copilot)),
                Err(reason) => {
                    tracing::info!(account_id = %account.id, reason, "Copilot account skipped");
                    None
                }
            };
        }
        account_homes::sign_in_state(tool, home).map(|_| None)
    })?;
    let Some((account, copilot)) = selected else {
        return Ok(None);
    };
    let account = ExecutionAccount {
        account_id: account.id.clone(),
        env_key: env_key.to_owned(),
        credential_home: account.credential_home.clone(),
    };
    if let Some((key, value)) =
        account_homes::account_launch_env(tool, account.credential_home.as_deref().map(Path::new))
    {
        if config.agent_id == AgentId::Codex {
            config.codex_auth_root =
                dunce::canonicalize(&value)
                    .ok()
                    .map(|path| gwt_agent::CodexAuthRoot {
                        path,
                        origin: gwt_agent::CodexAuthRootOrigin::CallerEnv,
                    });
        }
        config.env_vars.insert(key.to_owned(), value);
    }
    Ok(Some(PoolAccountLaunch { account, copilot }))
}

fn choose_account<'a, T>(
    store: &'a Store,
    tool: &str,
    previous: Option<&ExecutionAccount>,
    explicit: Option<&str>,
    usable: impl FnMut(&Account) -> Option<T>,
) -> Result<Option<(&'a Account, T)>, String> {
    if explicit.is_some_and(|id| store.account(id).is_none_or(|account| account.tool != tool)) {
        return Err(format!(
            "The selected execution account is unavailable for {tool}."
        ));
    }
    let account = store.select_pool_account(tool, explicit, usable);
    if account.is_none()
        && (explicit.is_some()
            || previous.is_some()
            || store.accounts.iter().any(|account| {
                account.tool == tool
                    && (account.origin.is_some() || account.credential_home.is_some())
            }))
    {
        let hint = if tool == AgentId::Copilot.command() {
            " Restore the registered sign-in in GitHub Copilot CLI and try again."
        } else {
            " Sign in to the tool and try again."
        };
        return Err(format!(
            "No signed-in execution account is available for {tool}.{hint}"
        ));
    }
    Ok(account)
}

fn has_auth_override(config: &LaunchConfig, home_key: &str) -> bool {
    let provider_keys: &[&str] = match config.agent_id {
        AgentId::ClaudeCode => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
        ],
        AgentId::Codex => &[
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_API_BASE_URL",
            "CHATGPT_BASE_URL",
        ],
        AgentId::GrokBuild => &["XAI_API_KEY", "GROK_API_KEY"],
        AgentId::Copilot => &[
            COPILOT_TOKEN_ENV,
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "COPILOT_HOME",
            "GH_HOST",
            "COPILOT_GH_HOST",
            "COPILOT_PROVIDER_BASE_URL",
            "COPILOT_PROVIDER_API_KEY",
            "COPILOT_PROVIDERS_CONFIG",
        ],
        _ => &[],
    };
    let is_key = |actual: &str, expected: &str| {
        if cfg!(windows) {
            actual.eq_ignore_ascii_case(expected)
        } else {
            actual == expected
        }
    };
    config.env_vars.iter().any(|(key, value)| {
        !value.is_empty()
            && (is_key(key, home_key)
                || provider_keys.iter().any(|expected| is_key(key, expected))
                || (is_key(key, "HOME")
                    && gwt_config::home::user_home().is_some_and(|home| home != Path::new(value))))
    })
}

pub(crate) fn record_launch_account(
    session: &mut Session,
    predecessor: Option<&Session>,
    account: Option<ExecutionAccount>,
    reason: Option<AccountSwitchReason>,
) {
    session.execution_account = account;
    if let Some(predecessor) = predecessor {
        session
            .account_switches
            .clone_from(&predecessor.account_switches);
        if let Some((from, to)) = predecessor
            .execution_account
            .as_ref()
            .zip(session.execution_account.as_ref())
        {
            if from.account_id != to.account_id {
                tracing::info!(from = %from.account_id, to = %to.account_id, "resuming with a different execution account");
                session.account_switches.push(AccountSwitch {
                    from: from.account_id.clone(),
                    to: to.account_id.clone(),
                    reason: reason.unwrap_or(AccountSwitchReason::Manual),
                    at: UtcTimestamp::now(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_environments::AccountOrigin;

    #[test]
    fn quota_switch_selects_the_next_account_and_waits_for_the_earliest_reset() {
        let mut attempt = PoolSwitchAttempt::default();
        let now = UtcTimestamp::now();
        let reset_a = now + time::Duration::hours(2);
        let reset_b = now + time::Duration::hours(1);
        let pool = ["a", "b", "c"];

        attempt.record_limit("a", Some(reset_a));
        assert_eq!(attempt.next_account(pool, "a", now), Some("b"));
        attempt.record_limit("b", Some(reset_b));
        assert_eq!(attempt.next_account(pool, "b", now), Some("c"));
        attempt.record_limit("c", None);
        assert_eq!(attempt.next_account(pool, "c", now), None);
        assert_eq!(attempt.next_reset(pool, now), Some(reset_b));
        assert_eq!(attempt.next_account(pool, "c", reset_b), Some("b"));
    }

    #[test]
    fn a_resume_uses_current_pool_priority_and_records_the_account_change() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let register = |id| {
            let path = account_homes::prepare("grok", id).unwrap();
            std::fs::write(path.join("auth.json"), r#"{"profile":{"key":"dummy"}}"#).unwrap();
            path
        };
        let first = register("first");
        let second = register("second");
        execution_environments::mutate(|store| {
            for (id, path) in [("first", &first), ("second", &second)] {
                store.accounts.push(Account {
                    id: id.into(),
                    tool: "grok".into(),
                    name: id.into(),
                    credential_home: Some(path.to_str().unwrap().to_owned()),
                    added_at: None,
                    origin: Some(AccountOrigin::Added),
                    sign_in_identity: None,
                });
            }
            Ok(())
        })
        .unwrap();
        let fresh = || gwt_agent::AgentLaunchBuilder::new(AgentId::GrokBuild).build();
        let mut previous = Session::new(home.path(), "develop", AgentId::GrokBuild);
        previous.agent_session_id = Some("existing-conversation".into());
        let mut config = fresh();
        config.execution_account_id = Some("second".into());
        let launch = pool_account_for_launch(&mut config, None, home.path(), |_| {})
            .unwrap()
            .unwrap();
        record_launch_account(&mut previous, None, Some(launch.account), None);
        assert_eq!(config.env_vars["GROK_HOME"], second.to_str().unwrap());

        let mut config = crate::session_launch::launch_config_from_persisted_session(&previous);
        let launch = pool_account_for_launch(&mut config, Some(&previous), home.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(launch.account.account_id, "first");
        assert_eq!(config.env_vars["GROK_HOME"], first.to_str().unwrap());

        account_homes::remove("grok", "second").unwrap();
        let mut config = crate::session_launch::launch_config_from_persisted_session(&previous);
        config.execution_account_id = Some("first".into());
        config.execution_account_switch_reason = Some(AccountSwitchReason::RateLimit);
        let launch = pool_account_for_launch(&mut config, Some(&previous), home.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(config.env_vars["GROK_HOME"], first.to_str().unwrap());
        let mut resumed =
            crate::session_launch::initialize_launch_session(home.path(), home.path(), &config);
        record_launch_account(
            &mut resumed,
            Some(&previous),
            Some(launch.account),
            config.execution_account_switch_reason,
        );
        assert_eq!(
            resumed.exact_resume_session_id(),
            previous.exact_resume_session_id()
        );
        assert_eq!(resumed.execution_account.unwrap().account_id, "first");
        assert_eq!(resumed.account_switches[0].from, "second");
        assert_eq!(resumed.account_switches[0].to, "first");
        assert_eq!(
            resumed.account_switches[0].reason,
            AccountSwitchReason::RateLimit
        );
        let mut config = fresh();
        config.execution_account_id = Some("second".into());
        config.execution_account_switch_reason = Some(AccountSwitchReason::RateLimit);
        assert!(
            pool_account_for_launch(&mut config, Some(&previous), home.path(), |_| {}).is_err()
        );
    }

    #[test]
    fn explicit_homes_win_before_a_pool_is_read() {
        for (agent, key) in [
            (AgentId::ClaudeCode, "CLAUDE_CONFIG_DIR"),
            (AgentId::Codex, "CODEX_HOME"),
        ] {
            let mut config = gwt_agent::AgentLaunchBuilder::new(agent)
                .env(key, "configured-home")
                .build();
            config.execution_account_id = Some("otherwise-missing-account".into());
            let mut reports = Vec::new();
            let selected = pool_account_for_launch(&mut config, None, Path::new("."), |message| {
                reports.push(message)
            })
            .unwrap();
            assert!(selected.is_none());
            assert_eq!(config.env_vars[key], "configured-home");
            assert_eq!(reports.len(), 1);
        }
    }
}
