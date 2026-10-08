//! Free provider quota reset, guarded by one native user decision per attempt.
pub(crate) mod codex;
pub(crate) mod native;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use codex::{CodexResetClient, ResetOutcome, ResetSnapshot};

pub const CLAUDE_NOTICE: &str = "Claude の追加利用は課金を伴うため gwt からは実行しない。別プロバイダへの切り替えを提案します。";

/// The audit identifies an attempt, never an authorization reusable by a caller.
#[derive(Debug, Serialize)]
pub(crate) struct ResetRequest {
    pub id: String,
    pub provider: String,
    pub window_id: String,
}

#[derive(Debug)]
pub(crate) struct ResetCompleted {
    pub audit_warning: Option<String>,
}

pub(crate) trait ResetBackend {
    fn read(&mut self) -> Result<ResetSnapshot, String>;
    fn consume(&mut self, key: &str, credit: &str) -> Result<ResetOutcome, String>;
}

impl ResetBackend for CodexResetClient {
    fn read(&mut self) -> Result<ResetSnapshot, String> {
        self.read()
    }

    fn consume(&mut self, key: &str, credit: &str) -> Result<ResetOutcome, String> {
        self.consume(key, credit)
    }
}

/// Consent is obtained inside this call and used once. There is deliberately no
/// approval flag/token parameter and no autonomous-mode shortcut.
pub(crate) fn execute(
    request: &ResetRequest,
    auth_root: &gwt_agent::CodexAuthRoot,
    backend: &mut impl ResetBackend,
    confirm: impl FnOnce(&str) -> Result<bool, String>,
    revalidate_target: impl FnOnce() -> Result<(), String>,
    release_hold: impl FnOnce() -> Result<(), String>,
    audit: &mut impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<ResetCompleted, String> {
    let result = (|| {
        if request.provider != "codex" {
            return Err(format!("unsupported free reset provider. {CLAUDE_NOTICE}"));
        }
        let before = backend.read()?;
        let credit = free_credit(&before)?;
        let prompt = format!(
            "Codex の無料リセットを1回使用します。\n対象 window: {}\nアカウント: {}\n認証ルート: {}\n由来: {}\n\
             このアカウントを共有する Codex の対象使用量枠がリセットされます。\n\
             無料リセット枠を1つ消費します。購入や有料追加利用は行いません。\n\
             成功と利用再開を確認した場合のみ gwt の hold を解除します。\n\n{CLAUDE_NOTICE}",
            request.window_id, before.account_id, auth_root.path.display(),
            serde_json::to_value(auth_root.origin).map_err(|_| "authentication origin encoding failed")?,
        );
        audit("proposed", &prompt)?;
        if !confirm(&prompt)? {
            audit("declined", "User did not approve this attempt")?;
            return Err("free reset declined; provider hold retained".into());
        }
        audit("approved", "Native confirmation for this attempt only")?;
        revalidate_target()?;
        let current = backend.read()?;
        if current.account_id != before.account_id
            || current.available_count == 0
            || !current.credits.iter().any(|c| {
                c.id == credit && c.reset_type == "codexRateLimits" && c.status == "available"
            })
        {
            return Err("Codex account or free credit changed after consent; propose again".into());
        }
        // Persist the execution boundary before touching the provider. A crash
        // never turns the durable audit into a replayable approval.
        audit("executing", &credit)?;
        let outcome = backend.consume(&request.id, &credit)?;
        audit("provider_result", &format!("{outcome:?}"))?;
        match outcome {
            ResetOutcome::Reset | ResetOutcome::AlreadyRedeemed => {}
            ResetOutcome::NoCredit => {
                return Err("no free reset credit; provider hold retained".into())
            }
            ResetOutcome::NothingToReset => {
                return Err("nothing to reset; provider hold retained".into())
            }
        }
        let after = backend.read()?;
        if after.account_id != before.account_id || after.ordinary_usage_allowed != Some(true) {
            return Err("reset returned success but account recovery is unconfirmed; provider hold retained".into());
        }
        release_hold()?;
        let audit_warning = audit(
            "completed",
            "Free reset and authoritative hold release confirmed",
        )
        .err();
        Ok(ResetCompleted { audit_warning })
    })();
    if let Err(reason) = &result {
        if let Err(audit_error) = audit("failed", reason) {
            return Err(format!("{reason}; audit failed: {audit_error}"));
        }
    }
    result
}

fn free_credit(snapshot: &ResetSnapshot) -> Result<String, String> {
    if snapshot.available_count > 0 {
        if let Some(credit) = snapshot.credits.iter().find(|credit| {
            !credit.id.trim().is_empty()
                && credit.reset_type == "codexRateLimits"
                && credit.status == "available"
        }) {
            return Ok(credit.id.clone());
        }
    }
    Err("no verified free Codex reset credit; provider hold retained".into())
}

#[derive(Debug, Serialize)]
pub struct ResetProposal {
    pub provider: String,
    pub reset_at: String,
    pub action: &'static str,
    pub message: String,
}

/// Read-only recommendations; even a free reset needs fresh availability and
/// explicit native consent when executed. No account calls happen while polling.
pub fn proposals(
    holds: &BTreeMap<String, String>,
    now: DateTime<Utc>,
    min_reset_wait_secs: u64,
) -> Vec<ResetProposal> {
    holds.iter().filter_map(|(provider, reset_at)| {
        let deadline = DateTime::parse_from_rfc3339(reset_at).ok()?;
        let remaining = deadline.signed_duration_since(now).num_seconds();
        if remaining <= 0 { return None; }
        let (action, message) = match provider.as_str() {
            "codex" if remaining as u64 >= min_reset_wait_secs => (
                "check_free_reset",
                "Codex の無料リセット枠を確認してください。provider.reset は対象 window を照合し、利用可能な無料枠と人間の明示承認がある場合だけ実行します。購入は行いません。".to_owned(),
            ),
            "claude" => ("switch_provider", CLAUDE_NOTICE.to_owned()),
            _ => return None,
        };
        // Issue #4908 AC-3: a hold with no stated reset reads `unknown`.
        let reset_at = crate::issue_monitor::provider_quota_reset_label(reset_at).to_owned();
        Some(ResetProposal { provider: provider.clone(), reset_at, action, message })
    }).collect()
}

#[cfg(test)]
mod tests;
