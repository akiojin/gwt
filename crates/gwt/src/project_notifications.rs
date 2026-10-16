//! Project-scoped notification qualification and delivery policy.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::{Duration, Instant},
};

use gwt_core::repo_hash::ProjectKey;
use serde::{Deserialize, Serialize, Serializer};

use crate::WindowProcessStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPermission {
    Granted,
    Denied,
    Default,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    Stopped,
    Error,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotificationIntent {
    pub transition_id: String,
    #[serde(serialize_with = "serialize_project_key")]
    pub project_key: ProjectKey,
    pub window_id: Option<String>,
    pub category: NotificationCategory,
    pub title: String,
    pub body: String,
    #[serde(skip)]
    generation: u64,
}

fn serialize_project_key<S: Serializer>(
    key: &ProjectKey,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(key.as_str())
}

#[derive(Debug, Clone)]
pub enum NotificationDelivery {
    Browser { client_id: String },
    Native { validity: Arc<AtomicBool> },
    Suppressed,
}

#[derive(Debug, Default)]
pub struct NotificationCoordinator {
    projects: HashMap<ProjectKey, ProjectNotifications>,
    pending: VecDeque<NotificationIntent>,
    attention_sequence: u64,
}

#[derive(Debug)]
struct ProjectNotifications {
    generation: u64,
    windows: HashMap<String, WindowHistory>,
    clients: HashMap<String, ClientAttention>,
    last_known_granted: bool,
    monitor_sources: HashSet<String>,
    routed_transitions: HashSet<String>,
    native_authorizations: Vec<Weak<AtomicBool>>,
}

impl ProjectNotifications {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            windows: HashMap::new(),
            clients: HashMap::new(),
            last_known_granted: false,
            monitor_sources: HashSet::new(),
            routed_transitions: HashSet::new(),
            native_authorizations: Vec::new(),
        }
    }

    fn invalidate_native_authorizations(&mut self) {
        for validity in self
            .native_authorizations
            .drain(..)
            .filter_map(|v| v.upgrade())
        {
            validity.store(false, Ordering::Release);
        }
    }
}

#[derive(Debug)]
struct WindowHistory {
    incarnation: u64,
    status: WindowProcessStatus,
    running_since: Option<Instant>,
}

#[derive(Debug)]
struct ClientAttention {
    visible: bool,
    focused: bool,
    permission: NotificationPermission,
    sequence: u64,
}

impl NotificationIntent {
    fn new(
        project_key: ProjectKey,
        generation: u64,
        window_id: Option<String>,
        category: NotificationCategory,
    ) -> Self {
        let (title, body) = match category {
            NotificationCategory::Stopped => (
                "gwt — Agent stopped",
                "An agent stopped after running for at least five minutes.",
            ),
            NotificationCategory::Error => (
                "gwt — Agent error",
                "An agent stopped with an error after running for at least five minutes.",
            ),
            NotificationCategory::NeedsHuman => {
                ("gwt — Attention needed", "An agent needs your attention.")
            }
        };
        Self {
            transition_id: uuid::Uuid::new_v4().to_string(),
            project_key,
            window_id,
            category,
            title: title.to_owned(),
            body: body.to_owned(),
            generation,
        }
    }
}

impl NotificationCoordinator {
    pub fn open_project(&mut self, key: ProjectKey, generation: u64) {
        if self
            .projects
            .get(&key)
            .is_some_and(|p| p.generation == generation)
        {
            return;
        }
        self.close_project(&key);
        self.projects
            .insert(key, ProjectNotifications::new(generation));
    }

    pub fn close_project(&mut self, key: &ProjectKey) {
        if let Some(mut project) = self.projects.remove(key) {
            project.invalidate_native_authorizations();
        }
        self.pending.retain(|intent| &intent.project_key != key);
    }

    pub fn retain_projects(&mut self, projects: &[(ProjectKey, u64)]) {
        let removed: Vec<_> = self
            .projects
            .keys()
            .filter(|key| !projects.iter().any(|(retained, _)| retained == *key))
            .cloned()
            .collect();
        for key in removed {
            self.close_project(&key);
        }
        for (key, generation) in projects {
            self.open_project(key.clone(), *generation);
        }
    }

    pub fn observe_runtime(
        &mut self,
        key: &ProjectKey,
        generation: u64,
        window_id: &str,
        incarnation: u64,
        status: WindowProcessStatus,
        now: Instant,
    ) {
        let Some(project) = self
            .projects
            .get_mut(key)
            .filter(|p| p.generation == generation)
        else {
            return;
        };
        let history = project
            .windows
            .entry(window_id.to_owned())
            .or_insert(WindowHistory {
                incarnation,
                status,
                running_since: (status == WindowProcessStatus::Running).then_some(now),
            });
        if history.incarnation != incarnation {
            *history = WindowHistory {
                incarnation,
                status,
                running_since: (status == WindowProcessStatus::Running).then_some(now),
            };
            return;
        }
        let qualified = history.status == WindowProcessStatus::Running
            && history.running_since.is_some_and(|since| {
                now.saturating_duration_since(since) >= Duration::from_secs(300)
            });
        let category = if qualified {
            match status {
                WindowProcessStatus::Stopped => Some(NotificationCategory::Stopped),
                WindowProcessStatus::Error => Some(NotificationCategory::Error),
                _ => None,
            }
        } else {
            None
        };
        if status != WindowProcessStatus::Running {
            history.running_since = None;
        } else if history.status != WindowProcessStatus::Running {
            history.running_since = Some(now);
        }
        history.status = status;
        if let Some(category) = category {
            self.pending.push_back(NotificationIntent::new(
                key.clone(),
                generation,
                Some(window_id.to_owned()),
                category,
            ));
        }
    }

    pub fn observe_needs_human(&mut self, key: &ProjectKey, generation: u64, source_id: &str) {
        let Some(project) = self
            .projects
            .get_mut(key)
            .filter(|p| p.generation == generation)
        else {
            return;
        };
        if !source_id.is_empty() && project.monitor_sources.insert(source_id.to_owned()) {
            self.pending.push_back(NotificationIntent::new(
                key.clone(),
                generation,
                None,
                NotificationCategory::NeedsHuman,
            ));
        }
    }

    pub fn drain_intents(&mut self) -> Vec<NotificationIntent> {
        self.pending.drain(..).collect()
    }

    pub fn report_attention(
        &mut self,
        key: &ProjectKey,
        generation: u64,
        client_id: &str,
        visible: bool,
        focused: bool,
        permission: NotificationPermission,
    ) {
        let Some(project) = self
            .projects
            .get_mut(key)
            .filter(|p| p.generation == generation)
        else {
            return;
        };
        project.invalidate_native_authorizations();
        self.attention_sequence = self.attention_sequence.saturating_add(1);
        project.last_known_granted = permission == NotificationPermission::Granted;
        project.clients.insert(
            client_id.to_owned(),
            ClientAttention {
                visible,
                focused,
                permission,
                sequence: self.attention_sequence,
            },
        );
    }

    pub fn route(
        &mut self,
        intent: &NotificationIntent,
        live_client_ids: &[String],
    ) -> NotificationDelivery {
        let Some(project) = self
            .projects
            .get_mut(&intent.project_key)
            .filter(|p| p.generation == intent.generation)
        else {
            return NotificationDelivery::Suppressed;
        };
        if !project
            .routed_transitions
            .insert(intent.transition_id.clone())
        {
            return NotificationDelivery::Suppressed;
        }
        project.clients.retain(|id, _| live_client_ids.contains(id));
        if project
            .clients
            .values()
            .any(|client| client.visible && client.focused)
        {
            return NotificationDelivery::Suppressed;
        }
        if let Some((client_id, _)) = project
            .clients
            .iter()
            .filter(|(_, client)| client.permission == NotificationPermission::Granted)
            .max_by_key(|(_, client)| client.sequence)
        {
            return NotificationDelivery::Browser {
                client_id: client_id.clone(),
            };
        }
        if !live_client_ids.is_empty() || !project.last_known_granted {
            return NotificationDelivery::Suppressed;
        }
        let validity = Arc::new(AtomicBool::new(true));
        project
            .native_authorizations
            .retain(|token| token.strong_count() > 0);
        project
            .native_authorizations
            .push(Arc::downgrade(&validity));
        NotificationDelivery::Native { validity }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::Ordering, time::Duration};

    use super::*;

    fn setup() -> (NotificationCoordinator, ProjectKey, Instant) {
        let key = ProjectKey::parse("0123456789abcdef").unwrap();
        let mut coordinator = NotificationCoordinator::default();
        coordinator.open_project(key.clone(), 1);
        (coordinator, key, Instant::now())
    }

    fn needs_human(
        coordinator: &mut NotificationCoordinator,
        key: &ProjectKey,
        source_id: &str,
    ) -> NotificationIntent {
        coordinator.observe_needs_human(key, 1, source_id);
        coordinator.drain_intents().pop().expect("immediate intent")
    }

    fn native_validity(delivery: NotificationDelivery) -> Arc<AtomicBool> {
        match delivery {
            NotificationDelivery::Native { validity } => validity,
            other => panic!("expected native delivery, got {other:?}"),
        }
    }

    #[test]
    fn running_five_minutes_qualifies_stopped_and_error_once() {
        let (mut coordinator, key, now) = setup();
        coordinator.observe_runtime(&key, 1, "short", 1, WindowProcessStatus::Running, now);
        coordinator.observe_runtime(
            &key,
            1,
            "short",
            1,
            WindowProcessStatus::Stopped,
            now + Duration::from_secs(299),
        );
        assert!(coordinator.drain_intents().is_empty());

        coordinator.observe_runtime(&key, 1, "stopped", 1, WindowProcessStatus::Running, now);
        coordinator.observe_runtime(
            &key,
            1,
            "stopped",
            1,
            WindowProcessStatus::Running,
            now + Duration::from_secs(200),
        );
        coordinator.observe_runtime(
            &key,
            1,
            "stopped",
            1,
            WindowProcessStatus::Stopped,
            now + Duration::from_secs(300),
        );
        coordinator.observe_runtime(
            &key,
            1,
            "stopped",
            1,
            WindowProcessStatus::Stopped,
            now + Duration::from_secs(301),
        );
        coordinator.observe_runtime(&key, 1, "error", 1, WindowProcessStatus::Running, now);
        coordinator.observe_runtime(
            &key,
            1,
            "error",
            1,
            WindowProcessStatus::Error,
            now + Duration::from_secs(300),
        );
        let intents = coordinator.drain_intents();
        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].category, NotificationCategory::Stopped);
        assert_eq!(intents[1].category, NotificationCategory::Error);
        assert_ne!(intents[0].transition_id, intents[1].transition_id);
    }

    #[test]
    fn idle_and_waiting_restart_the_running_interval() {
        let (mut coordinator, key, now) = setup();
        for (window, interruption) in [
            ("idle", WindowProcessStatus::Idle),
            ("waiting", WindowProcessStatus::Waiting),
        ] {
            coordinator.observe_runtime(&key, 1, window, 1, WindowProcessStatus::Running, now);
            coordinator.observe_runtime(
                &key,
                1,
                window,
                1,
                interruption,
                now + Duration::from_secs(200),
            );
            coordinator.observe_runtime(
                &key,
                1,
                window,
                1,
                WindowProcessStatus::Running,
                now + Duration::from_secs(201),
            );
            coordinator.observe_runtime(
                &key,
                1,
                window,
                1,
                WindowProcessStatus::Stopped,
                now + Duration::from_secs(400),
            );
            assert!(coordinator.drain_intents().is_empty());
            coordinator.observe_runtime(
                &key,
                1,
                window,
                1,
                WindowProcessStatus::Running,
                now + Duration::from_secs(401),
            );
            coordinator.observe_runtime(
                &key,
                1,
                window,
                1,
                WindowProcessStatus::Stopped,
                now + Duration::from_secs(701),
            );
            assert_eq!(coordinator.drain_intents().len(), 1);
        }
    }

    #[test]
    fn restored_status_and_new_incarnation_cannot_borrow_running_time() {
        let (mut coordinator, key, now) = setup();
        coordinator.observe_runtime(&key, 1, "restored", 1, WindowProcessStatus::Stopped, now);
        coordinator.observe_runtime(&key, 1, "restored", 1, WindowProcessStatus::Error, now);
        coordinator.observe_runtime(&key, 1, "replaced", 1, WindowProcessStatus::Running, now);
        coordinator.observe_runtime(
            &key,
            1,
            "replaced",
            2,
            WindowProcessStatus::Stopped,
            now + Duration::from_secs(600),
        );
        assert!(coordinator.drain_intents().is_empty());
        coordinator.observe_runtime(
            &key,
            1,
            "replaced",
            2,
            WindowProcessStatus::Running,
            now + Duration::from_secs(601),
        );
        coordinator.observe_runtime(
            &key,
            1,
            "replaced",
            2,
            WindowProcessStatus::Error,
            now + Duration::from_secs(901),
        );
        assert_eq!(coordinator.drain_intents().len(), 1);
    }

    #[test]
    fn needs_human_is_immediate_deduplicated_and_uses_safe_wire_text() {
        let (mut coordinator, key, _) = setup();
        let intent = needs_human(&mut coordinator, &key, "/private/raw-terminal-source");
        coordinator.observe_needs_human(&key, 1, "/private/raw-terminal-source");
        coordinator.observe_needs_human(&key, 1, "");
        assert!(coordinator.drain_intents().is_empty());
        assert!(uuid::Uuid::parse_str(&intent.transition_id).is_ok());
        let wire = serde_json::to_value(&intent).unwrap();
        assert_eq!(wire["project_key"], key.as_str());
        assert_eq!(wire["category"], "needs_human");
        assert!(wire["window_id"].is_null());
        assert!(wire.get("generation").is_none());
        assert_eq!(intent.title, "gwt — Attention needed");
        assert_eq!(intent.body, "An agent needs your attention.");
        for permission in ["granted", "denied", "default", "unsupported"] {
            assert!(serde_json::from_value::<NotificationPermission>(permission.into()).is_ok());
        }
    }

    #[test]
    fn current_visible_focused_browser_suppresses_without_retry() {
        let (mut coordinator, key, _) = setup();
        coordinator.report_attention(
            &key,
            1,
            "focused",
            true,
            true,
            NotificationPermission::Granted,
        );
        coordinator.report_attention(
            &key,
            1,
            "away",
            false,
            false,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "source");
        assert!(matches!(
            coordinator.route(&intent, &["focused".into(), "away".into()]),
            NotificationDelivery::Suppressed
        ));
        assert!(matches!(
            coordinator.route(&intent, &[]),
            NotificationDelivery::Suppressed
        ));
    }

    #[test]
    fn all_away_selects_latest_granted_report_from_live_project_clients() {
        let (mut coordinator, key, _) = setup();
        coordinator.report_attention(
            &key,
            1,
            "first",
            false,
            true,
            NotificationPermission::Granted,
        );
        coordinator.report_attention(
            &key,
            1,
            "second",
            true,
            false,
            NotificationPermission::Granted,
        );
        coordinator.report_attention(
            &key,
            1,
            "first",
            false,
            true,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "source");
        assert!(
            matches!(coordinator.route(&intent, &["first".into(), "second".into()]), NotificationDelivery::Browser { client_id } if client_id == "first")
        );
    }

    #[test]
    fn disconnected_attention_is_pruned_but_its_latest_grant_is_retained() {
        let (mut coordinator, key, _) = setup();
        coordinator.report_attention(&key, 1, "gone", true, true, NotificationPermission::Granted);
        coordinator.report_attention(
            &key,
            1,
            "live",
            false,
            false,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "live-source");
        assert!(
            matches!(coordinator.route(&intent, &["live".into()]), NotificationDelivery::Browser { client_id } if client_id == "live")
        );
        let intent = needs_human(&mut coordinator, &key, "closed-source");
        assert!(native_validity(coordinator.route(&intent, &[])).load(Ordering::Acquire));
    }

    #[test]
    fn connected_unreported_clients_prevent_native_and_latest_permission_revokes_grant() {
        let (mut coordinator, key, _) = setup();
        coordinator.report_attention(
            &key,
            1,
            "old",
            false,
            false,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "unreported");
        assert!(matches!(
            coordinator.route(&intent, &["unreported".into()]),
            NotificationDelivery::Suppressed
        ));
        for permission in [
            NotificationPermission::Denied,
            NotificationPermission::Default,
            NotificationPermission::Unsupported,
        ] {
            coordinator.report_attention(
                &key,
                1,
                "old",
                false,
                false,
                NotificationPermission::Granted,
            );
            coordinator.report_attention(&key, 1, "latest", false, false, permission);
            let intent = needs_human(&mut coordinator, &key, &format!("{permission:?}"));
            assert!(matches!(
                coordinator.route(&intent, &[]),
                NotificationDelivery::Suppressed
            ));
        }
    }

    #[test]
    fn later_attention_invalidates_native_authorization_without_second_channel() {
        let (mut coordinator, key, _) = setup();
        coordinator.report_attention(
            &key,
            1,
            "old",
            false,
            false,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "source");
        let validity = native_validity(coordinator.route(&intent, &[]));
        coordinator.report_attention(
            &key,
            1,
            "new",
            false,
            false,
            NotificationPermission::Granted,
        );
        assert!(!validity.load(Ordering::Acquire));
        assert!(matches!(
            coordinator.route(&intent, &["new".into()]),
            NotificationDelivery::Suppressed
        ));
    }

    #[test]
    fn project_generation_change_and_close_clear_intents_attention_and_tokens() {
        let (mut coordinator, key, now) = setup();
        coordinator.report_attention(
            &key,
            1,
            "old",
            false,
            false,
            NotificationPermission::Granted,
        );
        let intent = needs_human(&mut coordinator, &key, "source");
        let validity = native_validity(coordinator.route(&intent, &[]));
        coordinator.observe_needs_human(&key, 1, "queued");
        coordinator.observe_runtime(&key, 1, "window", 1, WindowProcessStatus::Running, now);
        coordinator.open_project(key.clone(), 2);
        assert!(!validity.load(Ordering::Acquire));
        assert!(coordinator.drain_intents().is_empty());
        assert!(matches!(
            coordinator.route(&intent, &[]),
            NotificationDelivery::Suppressed
        ));
        coordinator.report_attention(
            &key,
            1,
            "stale",
            true,
            true,
            NotificationPermission::Granted,
        );
        coordinator.observe_runtime(
            &key,
            2,
            "window",
            1,
            WindowProcessStatus::Stopped,
            now + Duration::from_secs(600),
        );
        assert!(coordinator.drain_intents().is_empty());
        coordinator.observe_needs_human(&key, 2, "source");
        let fresh = coordinator.drain_intents().pop().unwrap();
        assert!(matches!(
            coordinator.route(&fresh, &[]),
            NotificationDelivery::Suppressed
        ));
        coordinator.report_attention(
            &key,
            2,
            "fresh",
            false,
            false,
            NotificationPermission::Granted,
        );
        coordinator.observe_needs_human(&key, 2, "close");
        let closing = coordinator.drain_intents().pop().unwrap();
        let validity = native_validity(coordinator.route(&closing, &[]));
        coordinator.close_project(&key);
        assert!(!validity.load(Ordering::Acquire));
        assert!(matches!(
            coordinator.route(&closing, &[]),
            NotificationDelivery::Suppressed
        ));
    }

    #[test]
    fn retain_projects_clears_removed_project_intents() {
        let (mut coordinator, key, _) = setup();
        coordinator.observe_needs_human(&key, 1, "queued");
        coordinator.retain_projects(&[(key.clone(), 1)]);
        assert_eq!(coordinator.drain_intents().len(), 1);
        coordinator.observe_needs_human(&key, 1, "queued-two");
        coordinator.retain_projects(&[]);
        assert!(coordinator.drain_intents().is_empty());
        coordinator.observe_needs_human(&key, 1, "closed");
        assert!(coordinator.drain_intents().is_empty());
    }
}
