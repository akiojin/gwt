//! Subscriber initialization and handle types.

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::EnvFilter,
    layer::SubscriberExt,
    reload::{self, Handle as TracingReloadHandle},
    util::SubscriberInitExt,
    Registry,
};

use super::{
    config::{LogLevel, LoggingConfig},
    fmt_layer, housekeep,
    ui_forwarder::UiForwarderLayer,
    writer, LogEvent,
};

/// Reload handle for the `EnvFilter` layer.
///
/// Exposed as its own type alias so that call sites can store it in
/// structs without naming the underlying `tracing_subscriber` generics.
pub type ReloadHandle = TracingReloadHandle<EnvFilter, Registry>;

/// All runtime handles produced by `init`.
///
/// The caller **must** keep this struct alive for the lifetime of the
/// process. Dropping it shuts down the non-blocking writer thread and
/// silently discards any remaining in-flight events.
pub struct LoggingHandles {
    /// Keeps the non-blocking writer thread alive. Do not drop until
    /// shutdown.
    pub guard: WorkerGuard,
    /// Handle for live level changes (Settings UI).
    pub reload_handle: ReloadHandle,
    /// Receiver side of the UI forwarder channel. `None` after the
    /// caller has taken ownership via [`LoggingHandles::take_ui_rx`].
    pub ui_rx: Option<UnboundedReceiver<LogEvent>>,
    /// Cloned sender for call sites that want to inject synthetic
    /// events directly (currently unused; reserved for future use).
    pub ui_tx: UnboundedSender<LogEvent>,
    /// The directory that logs are written to (canonicalised path
    /// after `create_dir_all`).
    pub log_dir: std::path::PathBuf,
}

impl LoggingHandles {
    /// Take the UI receiver. Subsequent calls return `None`.
    pub fn take_ui_rx(&mut self) -> Option<UnboundedReceiver<LogEvent>> {
        self.ui_rx.take()
    }

    /// Change the runtime log level by replacing the `EnvFilter`
    /// directive. Thin wrapper around [`apply_log_level_to_handle`]
    /// for callers that own a full `LoggingHandles`.
    pub fn set_level(&self, level: LogLevel) -> Result<(), String> {
        apply_log_level_to_handle(&self.reload_handle, level)
    }
}

/// Apply a `LogLevel` to a tracing-subscriber reload handle.
///
/// Shared by [`LoggingHandles::set_level`] and any caller that holds
/// only a cloned `ReloadHandle` (for example `gwt_tui::model::Model`,
/// which receives the handle via `set_log_reload_handle` and does not
/// own the rest of the `LoggingHandles` struct).
///
/// Returns a human-readable error string if the directive cannot be
/// parsed or the reload fails. Callers should surface failures via a
/// `tracing::warn!` event so the user has a signal that the level
/// change was ignored.
pub fn apply_log_level_to_handle(handle: &ReloadHandle, level: LogLevel) -> Result<(), String> {
    let directive = level.to_env_directive();
    let filter = EnvFilter::try_new(directive)
        .map_err(|err| format!("invalid filter directive {directive:?}: {err}"))?;
    handle
        .reload(filter)
        .map_err(|err| format!("reload failed: {err}"))
}

/// Initialize the global `tracing` subscriber.
///
/// Must be called exactly once at process startup, before any other
/// crate emits `tracing` events that should be persisted. Calling it a
/// second time will return an error because `Registry::init` installs
/// a global default.
///
/// Performs startup housekeeping (see `housekeep`) synchronously so
/// that the TUI does not have to wait for an async task.
pub fn init(config: LoggingConfig) -> Result<LoggingHandles, String> {
    // Startup housekeeping — best effort. Errors are returned inside
    // the report but never block initialization.
    let report = housekeep::housekeep(&config.log_dir, config.retention_days);
    if !report.errors.is_empty() {
        // We cannot emit a tracing event yet (the subscriber is not
        // installed). Swallow silently; the caller can inspect the
        // report if they care by calling `housekeep` themselves. A
        // future enhancement could return the report alongside the
        // handles.
    }

    let (non_blocking, guard) =
        writer::build(&config.log_dir).map_err(|e| format!("log writer init failed: {e}"))?;

    let (ui_tx, ui_rx) = unbounded_channel::<LogEvent>();

    let directive = config.initial_filter_directive();
    // Reviewer comment B6: capture the parse error so we can surface
    // it via `tracing::warn!` AFTER the subscriber is installed —
    // otherwise a malformed `RUST_LOG` / `config.toml` value silently
    // falls back to `default_level` and the user has no signal that
    // their override was ignored.
    let (env_filter, invalid_directive_error) = match EnvFilter::try_new(&directive) {
        Ok(filter) => (filter, None),
        Err(err) => {
            let fallback = EnvFilter::try_new(config.default_level.to_env_directive())
                .map_err(|fallback_err| format!("env filter init failed: {fallback_err}"))?;
            (fallback, Some(err.to_string()))
        }
    };
    let (reloadable_filter, reload_handle) = reload::Layer::new(env_filter);

    let fmt = fmt_layer::build(non_blocking);
    let ui = UiForwarderLayer::new(ui_tx.clone());

    Registry::default()
        .with(reloadable_filter)
        .with(fmt)
        .with(ui)
        .try_init()
        .map_err(|e| format!("subscriber init failed: {e}"))?;

    if let Some(err) = invalid_directive_error {
        tracing::warn!(
            target: "gwt_core::logging",
            raw = %directive,
            error = %err,
            fallback = %config.default_level,
            "invalid initial filter directive — fell back to default level"
        );
    }

    Ok(LoggingHandles {
        guard,
        reload_handle,
        ui_rx: Some(ui_rx),
        ui_tx,
        log_dir: config.log_dir,
    })
}
