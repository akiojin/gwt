//! Agent process-tree resource policy resolution (SPEC #1921 Phase 86,
//! Issue #3813).
//!
//! Pure functions only: the caller supplies the persisted
//! [`AgentResourceConfig`], the launch repository's Issue Monitor
//! `max_active`, and the host logical-core count. No config or prefs I/O
//! happens here so the preset formulas stay table-testable.
//!
//! The primary control is generic to the whole process tree (priority class
//! / nice inheritance plus the Windows Job CPU cap). Build parallelism is the
//! one knob that has to be *told* to tools, so it is handed to every build
//! tool that reads a job count from the environment; cargo is one consumer,
//! not the owner of the setting.

use std::collections::HashMap;

use gwt_config::{
    AgentProcessPriority, AgentResourceConfig, AgentResourceConfigError, AgentResourcePreset,
};
use gwt_terminal::pty::{ProcessPolicy, ProcessPriority};

/// Environment variable cargo reads for its default `-j` parallelism.
pub const CARGO_BUILD_JOBS_ENV: &str = "CARGO_BUILD_JOBS";
/// Environment variable GNU make (and cmake's Makefile generator) reads for
/// its default `-j` parallelism.
pub const MAKEFLAGS_ENV: &str = "MAKEFLAGS";

/// Policy resolved for one AgentBootstrap launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAgentResourcePolicy {
    /// Priority / CPU cap applied to the PTY tree before release. `None`
    /// keeps the direct spawn behaviour (isolation disabled).
    pub process_policy: Option<ProcessPolicy>,
    /// Build parallelism to hand to build tools whose launch environment does
    /// not already pin one.
    pub build_jobs: Option<u32>,
}

/// FR-239: automatic CPU hard cap, `max(1, 100 / max_active)`.
pub fn automatic_cpu_limit_percent(max_active: usize) -> u8 {
    let share = 100 / max_active.max(1);
    share.clamp(1, 100) as u8
}

/// FR-239: automatic build parallelism, `max(1, logical_cores / max_active)`.
pub fn automatic_build_jobs(logical_cores: usize, max_active: usize) -> u32 {
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

/// Resolve the launch policy from the selected preset. Invalid explicit
/// values fail closed instead of launching an ungoverned tree (AS-6 /
/// NFR-015).
pub fn resolve_agent_resource_policy(
    config: &AgentResourceConfig,
    max_active: usize,
    logical_cores: usize,
) -> Result<ResolvedAgentResourcePolicy, AgentResourceConfigError> {
    config.validate()?;
    if !config.enabled {
        return Ok(ResolvedAgentResourcePolicy {
            process_policy: None,
            build_jobs: None,
        });
    }
    let automatic_cpu = automatic_cpu_limit_percent(max_active);
    let automatic_jobs = automatic_build_jobs(logical_cores, max_active);
    let (priority, cpu_limit_percent, build_jobs) = match config.preset {
        AgentResourcePreset::Automatic => {
            (ProcessPriority::BelowNormal, automatic_cpu, automatic_jobs)
        }
        AgentResourcePreset::GuiResponsiveness => (
            ProcessPriority::Idle,
            (automatic_cpu / 2).max(1),
            (automatic_jobs / 2).max(1),
        ),
        AgentResourcePreset::BuildSpeed => (
            ProcessPriority::BelowNormal,
            100,
            u32::try_from(logical_cores.max(1)).unwrap_or(u32::MAX),
        ),
        AgentResourcePreset::Custom => (
            process_priority(config.priority),
            config.cpu_limit_percent.unwrap_or(automatic_cpu),
            config.build_jobs.unwrap_or(automatic_jobs),
        ),
    };
    Ok(ResolvedAgentResourcePolicy {
        process_policy: Some(ProcessPolicy {
            priority,
            cpu_limit_percent: Some(cpu_limit_percent),
        }),
        build_jobs: Some(build_jobs),
    })
}

/// FR-240: hand the build parallelism to every build tool that reads a job
/// count from the environment, inserting each variable only when the
/// effective launch environment does not already carry it. Returns whether
/// anything was inserted.
pub fn inject_build_parallelism_env(
    env: &mut HashMap<String, String>,
    build_jobs: Option<u32>,
) -> bool {
    let Some(jobs) = build_jobs else {
        return false;
    };
    let mut inserted = false;
    for (key, value) in [
        (CARGO_BUILD_JOBS_ENV, jobs.to_string()),
        (MAKEFLAGS_ENV, format!("-j{jobs}")),
    ] {
        if !env.contains_key(key) {
            env.insert(key.to_string(), value);
            inserted = true;
        }
    }
    inserted
}

#[cfg(test)]
mod tests {
    use gwt_config::{AgentProcessPriority, AgentResourceConfig, AgentResourcePreset};

    use super::*;

    #[test]
    fn automatic_budget_divides_by_clamped_max_active() {
        assert_eq!(automatic_cpu_limit_percent(0), 100);
        assert_eq!(automatic_cpu_limit_percent(1), 100);
        assert_eq!(automatic_cpu_limit_percent(3), 33);
        assert_eq!(automatic_cpu_limit_percent(200), 1);

        assert_eq!(automatic_build_jobs(18, 3), 6);
        assert_eq!(automatic_build_jobs(18, 0), 18);
        assert_eq!(automatic_build_jobs(0, 3), 1);
        assert_eq!(automatic_build_jobs(2, 5), 1);
    }

    #[test]
    fn disabled_policy_resolves_to_nothing() {
        let config = AgentResourceConfig {
            enabled: false,
            preset: AgentResourcePreset::Custom,
            cpu_limit_percent: Some(20),
            build_jobs: Some(2),
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        assert_eq!(resolved.process_policy, None);
        assert_eq!(resolved.build_jobs, None);
    }

    #[test]
    fn automatic_preset_uses_below_normal_and_divided_budgets() {
        let resolved =
            resolve_agent_resource_policy(&AgentResourceConfig::default(), 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::BelowNormal);
        assert_eq!(policy.cpu_limit_percent, Some(33));
        assert_eq!(resolved.build_jobs, Some(6));
    }

    #[test]
    fn gui_responsiveness_preset_lowers_priority_and_halves_budgets() {
        let config = AgentResourceConfig {
            preset: AgentResourcePreset::GuiResponsiveness,
            // Custom-only fields are ignored outside Custom.
            priority: AgentProcessPriority::Normal,
            cpu_limit_percent: Some(90),
            build_jobs: Some(16),
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::Idle);
        assert_eq!(policy.cpu_limit_percent, Some(16));
        assert_eq!(resolved.build_jobs, Some(3));

        let single = resolve_agent_resource_policy(&config, 1, 2).expect("resolve");
        assert_eq!(
            single.process_policy.expect("policy").cpu_limit_percent,
            Some(50)
        );
        assert_eq!(single.build_jobs, Some(1));
    }

    #[test]
    fn build_speed_preset_keeps_below_normal_and_lifts_budgets() {
        let config = AgentResourceConfig {
            preset: AgentResourcePreset::BuildSpeed,
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::BelowNormal);
        assert_eq!(policy.cpu_limit_percent, Some(100));
        assert_eq!(resolved.build_jobs, Some(18));
    }

    #[test]
    fn custom_preset_uses_explicit_values_and_automatic_fallbacks() {
        let config = AgentResourceConfig {
            enabled: true,
            preset: AgentResourcePreset::Custom,
            priority: AgentProcessPriority::Idle,
            cpu_limit_percent: Some(50),
            build_jobs: Some(2),
        };
        let resolved = resolve_agent_resource_policy(&config, 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("process policy");
        assert_eq!(policy.priority, ProcessPriority::Idle);
        assert_eq!(policy.cpu_limit_percent, Some(50));
        assert_eq!(resolved.build_jobs, Some(2));

        let partial = AgentResourceConfig {
            preset: AgentResourcePreset::Custom,
            priority: AgentProcessPriority::Normal,
            ..AgentResourceConfig::default()
        };
        let resolved = resolve_agent_resource_policy(&partial, 3, 18).expect("resolve");
        let policy = resolved.process_policy.expect("policy");
        assert_eq!(policy.priority, ProcessPriority::Normal);
        assert_eq!(policy.cpu_limit_percent, Some(33));
        assert_eq!(resolved.build_jobs, Some(6));
    }

    #[test]
    fn invalid_config_fails_closed() {
        let config = AgentResourceConfig {
            preset: AgentResourcePreset::Custom,
            cpu_limit_percent: Some(0),
            ..AgentResourceConfig::default()
        };
        assert!(resolve_agent_resource_policy(&config, 3, 18).is_err());
    }

    #[test]
    fn build_parallelism_env_is_injected_only_where_absent() {
        let mut env = HashMap::new();
        assert!(inject_build_parallelism_env(&mut env, Some(4)));
        assert_eq!(env.get(CARGO_BUILD_JOBS_ENV).map(String::as_str), Some("4"));
        assert_eq!(env.get(MAKEFLAGS_ENV).map(String::as_str), Some("-j4"));

        let mut explicit = HashMap::from([
            (CARGO_BUILD_JOBS_ENV.to_string(), "9".to_string()),
            (MAKEFLAGS_ENV.to_string(), "-j2 --silent".to_string()),
        ]);
        assert!(!inject_build_parallelism_env(&mut explicit, Some(4)));
        assert_eq!(
            explicit.get(CARGO_BUILD_JOBS_ENV).map(String::as_str),
            Some("9")
        );
        assert_eq!(
            explicit.get(MAKEFLAGS_ENV).map(String::as_str),
            Some("-j2 --silent")
        );

        let mut partial = HashMap::from([(CARGO_BUILD_JOBS_ENV.to_string(), "9".to_string())]);
        assert!(inject_build_parallelism_env(&mut partial, Some(4)));
        assert_eq!(
            partial.get(CARGO_BUILD_JOBS_ENV).map(String::as_str),
            Some("9")
        );
        assert_eq!(partial.get(MAKEFLAGS_ENV).map(String::as_str), Some("-j4"));

        let mut disabled = HashMap::new();
        assert!(!inject_build_parallelism_env(&mut disabled, None));
        assert!(disabled.is_empty());
    }
}
