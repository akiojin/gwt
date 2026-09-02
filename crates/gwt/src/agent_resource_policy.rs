//! Agent process-tree resource policy resolution (SPEC #1921 Phase 86,
//! Issue #3813).
//!
//! Pure functions only: the caller supplies the persisted
//! [`AgentResourceConfig`], the launch repository's Issue Monitor
//! `max_active`, and the host logical-core count. No config or prefs I/O
//! happens here so the budget formulas stay table-testable.

use std::collections::HashMap;

use gwt_config::{AgentProcessPriority, AgentResourceConfig, AgentResourceConfigError};
use gwt_terminal::pty::{ProcessPolicy, ProcessPriority};

/// Environment variable cargo reads for its default `-j` parallelism.
pub const CARGO_BUILD_JOBS_ENV: &str = "CARGO_BUILD_JOBS";

/// Policy resolved for one AgentBootstrap launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAgentResourcePolicy {
    /// Priority / CPU cap applied to the PTY tree before release. `None`
    /// keeps the direct spawn behaviour (isolation disabled).
    pub process_policy: Option<ProcessPolicy>,
    /// `CARGO_BUILD_JOBS` to inject when the launch environment lacks one.
    pub cargo_jobs: Option<u32>,
}

/// FR-239: automatic CPU hard cap, `max(1, 100 / max_active)`.
pub fn automatic_cpu_limit_percent(max_active: usize) -> u8 {
    let share = 100 / max_active.max(1);
    share.clamp(1, 100) as u8
}

/// FR-239: automatic cargo parallelism, `max(1, logical_cores / max_active)`.
pub fn automatic_cargo_jobs(logical_cores: usize, max_active: usize) -> u32 {
    let jobs = logical_cores.max(1) / max_active.max(1);
    u32::try_from(jobs.max(1)).unwrap_or(u32::MAX)
}

fn process_priority(priority: AgentProcessPriority) -> ProcessPriority {
    match priority {
        AgentProcessPriority::Normal => ProcessPriority::Normal,
        AgentProcessPriority::BelowNormal => ProcessPriority::BelowNormal,
        AgentProcessPriority::Idle => ProcessPriority::Idle,
    }
}

/// Resolve the launch policy. Invalid explicit values fail closed instead of
/// launching an ungoverned tree (AS-6 / NFR-015).
pub fn resolve_agent_resource_policy(
    config: &AgentResourceConfig,
    max_active: usize,
    logical_cores: usize,
) -> Result<ResolvedAgentResourcePolicy, AgentResourceConfigError> {
    config.validate()?;
    if !config.enabled {
        return Ok(ResolvedAgentResourcePolicy {
            process_policy: None,
            cargo_jobs: None,
        });
    }
    Ok(ResolvedAgentResourcePolicy {
        process_policy: Some(ProcessPolicy {
            priority: process_priority(config.priority),
            cpu_limit_percent: Some(
                config
                    .cpu_limit_percent
                    .unwrap_or_else(|| automatic_cpu_limit_percent(max_active)),
            ),
        }),
        cargo_jobs: Some(
            config
                .cargo_jobs
                .unwrap_or_else(|| automatic_cargo_jobs(logical_cores, max_active)),
        ),
    })
}

/// FR-240: insert `CARGO_BUILD_JOBS` only when the effective launch
/// environment does not already carry one. Returns whether a value was
/// inserted.
pub fn inject_cargo_build_jobs(env: &mut HashMap<String, String>, cargo_jobs: Option<u32>) -> bool {
    let Some(jobs) = cargo_jobs else {
        return false;
    };
    if env.contains_key(CARGO_BUILD_JOBS_ENV) {
        return false;
    }
    env.insert(CARGO_BUILD_JOBS_ENV.to_string(), jobs.to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_budget_divides_by_clamped_max_active() {
        assert_eq!(automatic_cpu_limit_percent(0), 100);
        assert_eq!(automatic_cpu_limit_percent(1), 100);
        assert_eq!(automatic_cpu_limit_percent(3), 33);
        assert_eq!(automatic_cpu_limit_percent(200), 1);

        assert_eq!(automatic_cargo_jobs(18, 3), 6);
        assert_eq!(automatic_cargo_jobs(18, 0), 18);
        assert_eq!(automatic_cargo_jobs(0, 3), 1);
        assert_eq!(automatic_cargo_jobs(2, 5), 1);
    }

    #[test]
    fn disabled_policy_resolves_to_nothing() {
        let config = AgentResourceConfig {
            enabled: false,
            cpu_limit_percent: Some(20),
            cargo_jobs: Some(2),
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        assert_eq!(resolved.process_policy, None);
        assert_eq!(resolved.cargo_jobs, None);
    }

    #[test]
    fn default_policy_uses_below_normal_and_automatic_budgets() {
        let resolved =
            resolve_agent_resource_policy(&AgentResourceConfig::default(), 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::BelowNormal);
        assert_eq!(policy.cpu_limit_percent, Some(33));
        assert_eq!(resolved.cargo_jobs, Some(6));
    }

    #[test]
    fn explicit_values_and_priority_win_over_automatic_budgets() {
        let config = AgentResourceConfig {
            enabled: true,
            priority: AgentProcessPriority::Idle,
            cpu_limit_percent: Some(50),
            cargo_jobs: Some(2),
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::Idle);
        assert_eq!(policy.cpu_limit_percent, Some(50));
        assert_eq!(resolved.cargo_jobs, Some(2));

        let normal = AgentResourceConfig {
            priority: AgentProcessPriority::Normal,
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&normal, 1, 4).expect("resolve");
        assert_eq!(
            resolved.process_policy.expect("policy").priority,
            ProcessPriority::Normal
        );
    }

    #[test]
    fn invalid_config_fails_closed() {
        let config = AgentResourceConfig {
            cpu_limit_percent: Some(0),
            ..AgentResourceConfig::default()
        };
        assert!(resolve_agent_resource_policy(&config, 3, 18).is_err());
    }

    #[test]
    fn cargo_build_jobs_is_injected_only_when_absent() {
        let mut env = HashMap::new();
        assert!(inject_cargo_build_jobs(&mut env, Some(4)));
        assert_eq!(env.get(CARGO_BUILD_JOBS_ENV).map(String::as_str), Some("4"));

        let mut explicit = HashMap::from([(CARGO_BUILD_JOBS_ENV.to_string(), "9".to_string())]);
        assert!(!inject_cargo_build_jobs(&mut explicit, Some(4)));
        assert_eq!(
            explicit.get(CARGO_BUILD_JOBS_ENV).map(String::as_str),
            Some("9")
        );

        let mut disabled = HashMap::new();
        assert!(!inject_cargo_build_jobs(&mut disabled, None));
        assert!(disabled.is_empty());
    }
}
