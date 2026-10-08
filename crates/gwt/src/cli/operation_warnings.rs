//! Issue #4850: structured, non-fatal warnings an operation attaches to a
//! successful reply.
//!
//! An operation that already committed its external effect (a GitHub PATCH,
//! say) and then could not finish a best-effort local step must still answer
//! `ok:true` for the effect — and must say, in a machine-readable place, what
//! it skipped and why. Operations run synchronously on the thread that
//! renders their reply, so a thread-local sink is enough: the operation pushes
//! here, the JSON envelope drains into `warnings[]`, and the text renderer
//! prints one `warning:` line per entry.

use std::cell::RefCell;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationWarning {
    /// Stable machine-readable identifier (`snake_case`).
    pub code: String,
    /// Human-readable detail, including the cause.
    pub message: String,
}

thread_local! {
    static WARNINGS: RefCell<Vec<OperationWarning>> = const { RefCell::new(Vec::new()) };
}

/// Attach a warning to the operation running on this thread.
pub fn push(code: &str, message: impl Into<String>) {
    let warning = OperationWarning {
        code: code.to_string(),
        message: message.into(),
    };
    tracing::warn!(code = %warning.code, message = %warning.message, "operation warning");
    WARNINGS.with(|warnings| warnings.borrow_mut().push(warning));
}

/// The warnings attached so far, left in place for the reply renderer.
pub fn snapshot() -> Vec<OperationWarning> {
    WARNINGS.with(|warnings| warnings.borrow().clone())
}

/// Drain the warnings attached so far. The envelope calls this before an
/// operation (so a previous operation on a reused thread leaves nothing
/// behind) and after it (to render `warnings[]`).
pub fn take() -> Vec<OperationWarning> {
    WARNINGS.with(|warnings| std::mem::take(&mut *warnings.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_are_thread_local_and_drained_by_take() {
        take();
        push("one", "first");
        push("two", "second");
        assert_eq!(
            snapshot()
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(take().len(), 2);
        assert!(take().is_empty(), "take drains");
        push("three", "third");
        assert!(
            std::thread::spawn(|| snapshot().is_empty())
                .join()
                .expect("thread"),
            "another thread sees its own empty sink"
        );
        take();
    }
}
