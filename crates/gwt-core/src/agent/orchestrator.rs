//! Event-driven orchestration loop for agent mode
//!
//! The OrchestratorLoop receives events via mpsc channel and drives
//! the full agent workflow: Spec Kit -> approval -> task execution -> completion.

use std::sync::mpsc::{self, Receiver, Sender};

use super::master::{MasterAgent, ParsedTask};
use super::session::{AgentSession, SessionStatus};
use super::task::{Task, TaskStatus};
use super::types::{SessionId, TaskId};

/// Events that drive the orchestration loop
#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    /// A new session has been started with a user request
    SessionStart {
        session_id: SessionId,
        user_request: String,
    },
    /// User input received (approval, new instruction, answer)
    UserInput { content: String },
    /// A sub-agent has completed its task
    SubAgentCompleted { task_id: TaskId, pane_id: String },
    /// A sub-agent has failed
    SubAgentFailed {
        task_id: TaskId,
        pane_id: String,
        reason: String,
    },
    /// Tests passed for a task
    TestPassed { task_id: TaskId },
    /// Tests failed for a task
    TestFailed { task_id: TaskId, output: String },
    /// Periodic progress tick (for status reporting)
    ProgressTick,
    /// User requested interruption (Esc key)
    InterruptRequested,
}

/// Messages sent from the orchestrator to the TUI
#[derive(Debug, Clone)]
pub enum OrchestratorMessage {
    /// Chat message to display
    ChatMessage { role: String, content: String },
    /// Session status updated
    StatusUpdate {
        session_name: Option<String>,
        llm_call_count: u64,
        estimated_tokens: u64,
    },
    /// Plan ready for approval
    PlanForApproval {
        spec_content: String,
        plan_content: String,
        tasks_content: String,
    },
    /// Session completed
    SessionCompleted,
    /// Error occurred
    Error(String),
}

/// The orchestration loop state
pub struct OrchestratorLoop {
    /// Sender for events (cloned for external use)
    event_tx: Sender<OrchestratorEvent>,
    /// Receiver for events
    event_rx: Receiver<OrchestratorEvent>,
    /// Sender for messages to TUI
    message_tx: Sender<OrchestratorMessage>,
    /// Current session
    session: Option<AgentSession>,
    /// Whether waiting for user approval
    awaiting_approval: bool,
    /// Whether waiting for answers to clarifying questions
    awaiting_question_answers: bool,
    /// Original user request (preserved across question phase)
    original_request: Option<String>,
    /// Generated spec/plan/tasks content (held during approval)
    pending_artifacts: Option<(String, String, String)>,
    /// Parsed tasks from the plan
    parsed_tasks: Vec<ParsedTask>,
}

impl OrchestratorLoop {
    /// Create a new orchestrator loop
    ///
    /// Returns the loop and a sender for posting events.
    pub fn new(message_tx: Sender<OrchestratorMessage>) -> (Self, Sender<OrchestratorEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        let tx_clone = event_tx.clone();

        let orchestrator = Self {
            event_tx,
            event_rx,
            message_tx,
            session: None,
            awaiting_approval: false,
            awaiting_question_answers: false,
            original_request: None,
            pending_artifacts: None,
            parsed_tasks: Vec::new(),
        };

        (orchestrator, tx_clone)
    }

    /// Get a clone of the event sender
    pub fn event_sender(&self) -> Sender<OrchestratorEvent> {
        self.event_tx.clone()
    }

    /// Run the event loop (blocking)
    ///
    /// Processes events until the session completes or an interrupt is received.
    pub fn run_loop(&mut self, master: &mut MasterAgent) {
        while let Ok(event) = self.event_rx.recv() {
            match event {
                OrchestratorEvent::SessionStart {
                    session_id,
                    user_request,
                } => {
                    self.handle_session_start(master, session_id, &user_request);
                }
                OrchestratorEvent::UserInput { content } => {
                    self.handle_user_input(master, &content);
                }
                OrchestratorEvent::SubAgentCompleted { task_id, pane_id } => {
                    self.handle_sub_agent_completed(&task_id, &pane_id);
                }
                OrchestratorEvent::SubAgentFailed {
                    task_id,
                    pane_id,
                    reason,
                } => {
                    self.handle_sub_agent_failed(&task_id, &pane_id, &reason);
                }
                OrchestratorEvent::TestPassed { task_id } => {
                    self.handle_test_passed(&task_id);
                }
                OrchestratorEvent::TestFailed { task_id, output } => {
                    self.handle_test_failed(&task_id, &output);
                }
                OrchestratorEvent::ProgressTick => {
                    // Progress reporting (Phase 5)
                }
                OrchestratorEvent::InterruptRequested => {
                    self.handle_interrupt();
                    return;
                }
            }

            // Check if session is complete
            if let Some(session) = &self.session {
                if session.status == SessionStatus::Completed {
                    let _ = self.message_tx.send(OrchestratorMessage::SessionCompleted);
                    return;
                }
            }
        }
    }

    fn handle_session_start(
        &mut self,
        master: &mut MasterAgent,
        session_id: SessionId,
        user_request: &str,
    ) {
        let session = AgentSession::new(session_id, std::path::PathBuf::from("."));
        self.session = Some(session);

        // Send initial chat message
        let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
            role: "system".to_string(),
            content: format!("Starting session for: {}", user_request),
        });

        // Save original request for use after question phase
        self.original_request = Some(user_request.to_string());

        // Run question phase first to clarify requirements
        let needs_answers = self.run_question_phase(master, user_request);
        if needs_answers {
            self.awaiting_question_answers = true;
            return;
        }

        // Run Spec Kit workflow via master agent
        self.run_speckit_and_present(master, user_request);
    }

    fn handle_user_input(&mut self, master: &mut MasterAgent, content: &str) {
        if self.awaiting_approval {
            self.process_approval_response(master, content);
            return;
        }

        // If we were waiting for question answers, proceed to Spec Kit
        if self.awaiting_question_answers {
            self.awaiting_question_answers = false;
            // Feed answers to master context, then run Spec Kit
            let _ = master.send_message(content);
            if let Some(session) = &mut self.session {
                session.llm_call_count += 1;
            }
            let user_request = self
                .original_request
                .clone()
                .unwrap_or_else(|| content.to_string());
            self.run_speckit_and_present(master, &user_request);
            return;
        }

        // Forward to master agent for conversation
        match master.send_message(content) {
            Ok(response) => {
                let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                    role: "assistant".to_string(),
                    content: response,
                });
                if let Some(session) = &mut self.session {
                    session.llm_call_count += 1;
                    self.send_status_update();
                }
            }
            Err(e) => {
                let _ = self
                    .message_tx
                    .send(OrchestratorMessage::Error(format!("LLM error: {}", e)));
            }
        }
    }

    fn handle_sub_agent_completed(&mut self, task_id: &TaskId, _pane_id: &str) {
        if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                task.status = TaskStatus::Completed;
                task.completed_at = Some(chrono::Utc::now());
            }

            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!("Task {} completed", task_id.0),
            });

            // Check if all tasks are done
            let all_done = session
                .tasks
                .iter()
                .all(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Cancelled);

            if all_done && !session.tasks.is_empty() {
                session.status = SessionStatus::Completed;
            } else {
                // Launch next ready task
                self.launch_ready_tasks();
            }
        }
    }

    fn handle_sub_agent_failed(&mut self, task_id: &TaskId, _pane_id: &str, reason: &str) {
        if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                if task.retry_count < 3 {
                    task.retry_count += 1;
                    task.status = TaskStatus::Ready;
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!(
                            "Task {} failed (attempt {}/3): {}. Retrying...",
                            task_id.0, task.retry_count, reason
                        ),
                    });
                } else {
                    task.status = TaskStatus::Failed;
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!("Task {} failed after 3 attempts: {}", task_id.0, reason),
                    });
                }
            }
        }
    }

    fn handle_test_passed(&mut self, task_id: &TaskId) {
        let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
            role: "system".to_string(),
            content: format!("Tests passed for task {}", task_id.0),
        });
    }

    fn handle_test_failed(&mut self, task_id: &TaskId, output: &str) {
        let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
            role: "system".to_string(),
            content: format!("Tests failed for task {}: {}", task_id.0, output),
        });
    }

    fn handle_interrupt(&mut self) {
        if let Some(session) = &mut self.session {
            session.status = SessionStatus::Paused;
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: "Session paused by user interrupt (Esc)".to_string(),
            });
        }
    }

    /// Run Spec Kit workflow and present plan for approval
    fn run_speckit_and_present(&mut self, master: &mut MasterAgent, user_request: &str) {
        let scan_result = crate::agent::scanner::RepositoryScanner::new(".").scan();
        let claude_md = scan_result.claude_md.as_deref().unwrap_or("");
        let existing_specs = scan_result.existing_specs.join(", ");

        match master.run_speckit_workflow(
            user_request,
            &scan_result.directory_tree,
            claude_md,
            &existing_specs,
            &scan_result.directory_tree,
        ) {
            Ok((spec, plan, tasks)) => {
                self.present_plan_for_approval(&spec, &plan, &tasks);
            }
            Err(e) => {
                let _ = self.message_tx.send(OrchestratorMessage::Error(format!(
                    "Spec Kit workflow failed: {}",
                    e
                )));
            }
        }
    }

    /// Present the generated plan for user approval
    fn present_plan_for_approval(
        &mut self,
        spec_content: &str,
        plan_content: &str,
        tasks_content: &str,
    ) {
        self.awaiting_approval = true;
        self.pending_artifacts = Some((
            spec_content.to_string(),
            plan_content.to_string(),
            tasks_content.to_string(),
        ));

        let _ = self.message_tx.send(OrchestratorMessage::PlanForApproval {
            spec_content: spec_content.to_string(),
            plan_content: plan_content.to_string(),
            tasks_content: tasks_content.to_string(),
        });

        let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
            role: "system".to_string(),
            content: "Plan generated. Type 'y' or press Enter to approve, or provide feedback."
                .to_string(),
        });
    }

    /// Process user's approval or rejection of the plan
    fn process_approval_response(&mut self, master: &mut MasterAgent, content: &str) {
        let trimmed = content.trim().to_lowercase();
        let approved = trimmed.is_empty() || trimmed == "y" || trimmed == "yes";

        if approved {
            self.awaiting_approval = false;

            // Parse tasks and create Task objects
            if let Some((_, _, ref tasks_content)) = self.pending_artifacts {
                match master.parse_task_plan(tasks_content) {
                    Ok(parsed) => {
                        self.parsed_tasks = parsed;
                        self.create_tasks_from_parsed();
                        self.launch_ready_tasks();
                    }
                    Err(e) => {
                        let _ = self.message_tx.send(OrchestratorMessage::Error(format!(
                            "Failed to parse tasks: {}",
                            e
                        )));
                    }
                }
            }
            self.pending_artifacts = None;

            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: "Plan approved. Starting task execution...".to_string(),
            });
        } else {
            // User provided feedback - re-plan
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: "Re-planning based on feedback...".to_string(),
            });

            match master.send_message(&format!(
                "The user rejected the plan with this feedback: {}. Please revise.",
                content
            )) {
                Ok(response) => {
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "assistant".to_string(),
                        content: response,
                    });
                }
                Err(e) => {
                    let _ = self.message_tx.send(OrchestratorMessage::Error(format!(
                        "LLM error during re-planning: {}",
                        e
                    )));
                }
            }
        }
    }

    /// Create Task objects from parsed task data
    fn create_tasks_from_parsed(&mut self) {
        if let Some(session) = &mut self.session {
            for parsed in &self.parsed_tasks {
                let task = Task::new(
                    TaskId::new(),
                    parsed.name.clone(),
                    parsed.description.clone(),
                );
                session.tasks.push(task);
            }

            // Set first task(s) to Ready
            for task in &mut session.tasks {
                if task.dependencies.is_empty() {
                    task.status = TaskStatus::Ready;
                }
            }
        }
    }

    /// Launch tasks that are in Ready state
    fn launch_ready_tasks(&mut self) {
        if let Some(session) = &self.session {
            let ready_tasks: Vec<_> = session
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Ready)
                .collect();

            if ready_tasks.is_empty() {
                return;
            }

            // For now, launch one at a time (parallel execution will be added in Phase C)
            let task = &ready_tasks[0];
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!("Launching task: {}", task.name),
            });
        }

        // Mark first ready task as Running
        if let Some(session) = &mut self.session {
            if let Some(task) = session
                .tasks
                .iter_mut()
                .find(|t| t.status == TaskStatus::Ready)
            {
                task.status = TaskStatus::Running;
                task.started_at = Some(chrono::Utc::now());
            }
        }
    }

    /// Run the question phase to clarify requirements before planning (T045)
    ///
    /// Uses the MasterAgent to generate clarifying questions based on the
    /// user request and repository context, then waits for user answers.
    fn run_question_phase(&mut self, master: &mut MasterAgent, user_request: &str) -> bool {
        let scan_result = crate::agent::scanner::RepositoryScanner::new(".").scan();
        let repo_context = &scan_result.directory_tree;

        // Ask LLM to generate clarifying questions
        let prompt = format!(
            "Based on this user request and repository context, generate 1-3 brief clarifying questions \
            that would help produce a better implementation plan. If the request is already clear enough, \
            respond with exactly \"NO_QUESTIONS\".\n\n\
            User request: {}\n\nRepository context:\n{}",
            user_request, repo_context
        );

        match master.send_message(&prompt) {
            Ok(response) => {
                if let Some(session) = &mut self.session {
                    session.llm_call_count += 1;
                }
                self.send_status_update();

                if response.contains("NO_QUESTIONS") {
                    return false; // No questions needed
                }

                // Present questions to user
                let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                    role: "assistant".to_string(),
                    content: response,
                });
                true // Questions were asked, waiting for answers
            }
            Err(_) => false, // On error, skip questions
        }
    }

    /// Merge dependency commits from a completed task's branch into a dependent task's branch (T047)
    ///
    /// When task A completes and task B depends on A, merge A's branch into B's worktree.
    pub fn merge_dependency_commits(
        &self,
        source_branch: &str,
        target_worktree_path: &std::path::Path,
    ) -> std::result::Result<(), String> {
        let output = std::process::Command::new("git")
            .args(["merge", source_branch, "--no-edit"])
            .current_dir(target_worktree_path)
            .output()
            .map_err(|e| format!("Failed to run git merge: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Merge conflict: {}", stderr))
        }
    }

    fn send_status_update(&self) {
        if let Some(session) = &self.session {
            let _ = self.message_tx.send(OrchestratorMessage::StatusUpdate {
                session_name: session.spec_id.clone(),
                llm_call_count: session.llm_call_count,
                estimated_tokens: session.estimated_tokens,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_event_variants() {
        // Verify all event variants can be constructed
        let _ = OrchestratorEvent::SessionStart {
            session_id: SessionId::new(),
            user_request: "test".to_string(),
        };
        let _ = OrchestratorEvent::UserInput {
            content: "hello".to_string(),
        };
        let _ = OrchestratorEvent::SubAgentCompleted {
            task_id: TaskId::new(),
            pane_id: "%1".to_string(),
        };
        let _ = OrchestratorEvent::SubAgentFailed {
            task_id: TaskId::new(),
            pane_id: "%1".to_string(),
            reason: "error".to_string(),
        };
        let _ = OrchestratorEvent::TestPassed {
            task_id: TaskId::new(),
        };
        let _ = OrchestratorEvent::TestFailed {
            task_id: TaskId::new(),
            output: "fail".to_string(),
        };
        let _ = OrchestratorEvent::ProgressTick;
        let _ = OrchestratorEvent::InterruptRequested;
    }

    #[test]
    fn test_orchestrator_message_variants() {
        let _ = OrchestratorMessage::ChatMessage {
            role: "user".to_string(),
            content: "test".to_string(),
        };
        let _ = OrchestratorMessage::StatusUpdate {
            session_name: Some("test".to_string()),
            llm_call_count: 0,
            estimated_tokens: 0,
        };
        let _ = OrchestratorMessage::PlanForApproval {
            spec_content: "spec".to_string(),
            plan_content: "plan".to_string(),
            tasks_content: "tasks".to_string(),
        };
        let _ = OrchestratorMessage::SessionCompleted;
        let _ = OrchestratorMessage::Error("err".to_string());
    }

    #[test]
    fn test_merge_dependency_commits_nonexistent_dir() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let result =
            orchestrator.merge_dependency_commits("main", std::path::Path::new("/nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_orchestrator_creation() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (orchestrator, event_tx) = OrchestratorLoop::new(msg_tx);
        assert!(orchestrator.session.is_none());
        assert!(!orchestrator.awaiting_approval);

        // Event sender should be usable
        let _ = event_tx.send(OrchestratorEvent::InterruptRequested);
    }
}
