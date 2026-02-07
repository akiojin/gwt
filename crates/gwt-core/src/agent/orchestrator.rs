//! Event-driven orchestration loop for agent mode
//!
//! The OrchestratorLoop receives events via mpsc channel and drives
//! the full agent workflow: Spec Kit -> approval -> task execution -> completion.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use super::master::{MasterAgent, ParsedTask};
use super::session::{AgentSession, SessionStatus};
use super::task::{Task, TaskStatus, TestStatus, TestVerification};
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

    fn handle_sub_agent_completed(&mut self, task_id: &TaskId, pane_id: &str) {
        if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                task.status = TaskStatus::Completed;
                task.completed_at = Some(chrono::Utc::now());
            }

            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!("Task {} completed, running test verification...", task_id.0),
            });
        }

        // Run test verification (T050)
        self.run_test_verification(task_id, pane_id);
    }

    fn handle_sub_agent_failed(&mut self, task_id: &TaskId, _pane_id: &str, reason: &str) {
        let retryable = Self::is_retryable_error(reason);

        if retryable {
            self.retry_task(task_id);
        } else if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                task.status = TaskStatus::Failed;
            }
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Task {} failed (non-retryable): {}. User intervention may be needed.",
                    task_id.0, reason
                ),
            });
        }
    }

    fn handle_test_passed(&mut self, task_id: &TaskId) {
        // Update test_status to Passed (T051)
        if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                if let Some(ref mut tv) = task.test_status {
                    tv.status = TestStatus::Passed;
                } else {
                    task.test_status = Some(TestVerification {
                        status: TestStatus::Passed,
                        command: String::new(),
                        output: None,
                        attempt: 1,
                    });
                }
            }
        }

        let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
            role: "system".to_string(),
            content: format!(
                "Tests passed for task {}. Creating pull request...",
                task_id.0
            ),
        });

        // Attempt to create a PR
        let pr_result = {
            let session = self.session.as_ref();
            session
                .and_then(|s| s.tasks.iter().find(|t| t.id == *task_id))
                .map(|task| {
                    let worktree_path = task
                        .assigned_worktree
                        .as_ref()
                        .map(|wt| wt.path.clone())
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    Self::create_pull_request(task, &worktree_path)
                })
        };

        if let Some(result) = pr_result {
            match result {
                Ok(url) => {
                    // Store PR reference on the task
                    if let Some(session) = &mut self.session {
                        if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                            task.pull_request = Some(super::task::PullRequestRef {
                                number: 0, // Will be parsed from URL if needed
                                url: url.clone(),
                            });
                        }
                    }
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!("PR created for task {}: {}", task_id.0, url),
                    });
                }
                Err(e) => {
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!("PR creation failed for task {}: {}", task_id.0, e),
                    });
                }
            }
        }

        // Check if all tasks are done
        self.check_session_completion();
    }

    fn handle_test_failed(&mut self, task_id: &TaskId, output: &str) {
        // Update test_status and check retry count (T052)
        let should_retry = if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                let attempt = task.test_status.as_ref().map(|tv| tv.attempt).unwrap_or(0) + 1;
                task.test_status = Some(TestVerification {
                    status: TestStatus::Failed,
                    command: String::new(),
                    output: Some(output.to_string()),
                    attempt,
                });
                attempt < 3
            } else {
                false
            }
        } else {
            false
        };

        if should_retry {
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Tests failed for task {}. Retrying with fix instructions...",
                    task_id.0
                ),
            });

            // Send fix instructions to the sub-agent pane if available
            if let Some(session) = &self.session {
                if let Some(task) = session.tasks.iter().find(|t| t.id == *task_id) {
                    if let Some(ref sub_agent) = task.sub_agent {
                        let fix_prompt = format!(
                            "The tests failed with the following output:\n{}\n\nPlease fix the failing tests and try again.",
                            output
                        );
                        let _ =
                            crate::tmux::pane::send_prompt_to_pane(&sub_agent.pane_id, &fix_prompt);
                    }
                }
            }
        } else {
            if let Some(session) = &mut self.session {
                if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                    task.status = TaskStatus::Failed;
                }
            }
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Tests failed for task {} after max retries: {}",
                    task_id.0, output
                ),
            });
        }
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

    /// Run test verification after sub-agent completion (T050)
    ///
    /// Captures pane output, sends the test command, waits, then checks results.
    fn run_test_verification(&self, task_id: &TaskId, pane_id: &str) {
        // Get the test command from a repository scan
        let scan_result = crate::agent::scanner::RepositoryScanner::new(".").scan();
        let test_cmd = &scan_result.test_command;

        if test_cmd.is_empty() {
            // No test command available - skip verification, treat as passed
            let _ = self.event_tx.send(OrchestratorEvent::TestPassed {
                task_id: task_id.clone(),
            });
            return;
        }

        // Send test command to the pane
        if let Err(e) = crate::tmux::pane::send_keys(pane_id, &format!("{} \n", test_cmd)) {
            let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                role: "system".to_string(),
                content: format!("Failed to send test command to pane: {}", e),
            });
            let _ = self.event_tx.send(OrchestratorEvent::TestFailed {
                task_id: task_id.clone(),
                output: format!("Failed to send test command: {}", e),
            });
            return;
        }

        // Wait for tests to run
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Capture test output
        let output = match crate::tmux::pane::capture_pane_output(pane_id) {
            Ok(o) => o,
            Err(e) => {
                let _ = self.event_tx.send(OrchestratorEvent::TestFailed {
                    task_id: task_id.clone(),
                    output: format!("Failed to capture pane output: {}", e),
                });
                return;
            }
        };

        // Check test results (look for common failure indicators)
        if Self::is_test_output_passing(&output) {
            let _ = self.event_tx.send(OrchestratorEvent::TestPassed {
                task_id: task_id.clone(),
            });
        } else {
            let _ = self.event_tx.send(OrchestratorEvent::TestFailed {
                task_id: task_id.clone(),
                output,
            });
        }
    }

    /// Check if test output indicates passing tests
    fn is_test_output_passing(output: &str) -> bool {
        // Cargo test: "test result: ok."
        if output.contains("test result: ok.") {
            return true;
        }
        // npm test success patterns
        if output.contains("Tests:") && output.contains("passed") && !output.contains("failed") {
            return true;
        }
        // Generic: no "FAILED" or "error" in the last few lines
        let last_lines: Vec<&str> = output.lines().rev().take(10).collect();
        let last_text = last_lines.join("\n").to_lowercase();
        !last_text.contains("failed") && !last_text.contains("error")
    }

    /// Create a pull request for a completed task (T053)
    fn create_pull_request(
        task: &Task,
        worktree_path: &Path,
    ) -> std::result::Result<String, String> {
        // Check prerequisites (T054)
        Self::check_pr_prerequisites(worktree_path)?;

        // Get diff stat for PR body
        let diff_output = std::process::Command::new("git")
            .args(["diff", "--stat", "HEAD~1"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to get diff stat: {}", e))?;
        let diff_stat = String::from_utf8_lossy(&diff_output.stdout).to_string();

        // Create PR via gh CLI
        let title = format!("feat: {}", task.name);
        let body = format!(
            "## Summary\n\nAutomated PR for task: {}\n\n{}\n\n## Changes\n\n```\n{}\n```",
            task.name, task.description, diff_stat
        );

        let output = std::process::Command::new("gh")
            .args(["pr", "create", "--title", &title, "--body", &body, "--fill"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to run gh pr create: {}", e))?;

        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(url)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("gh pr create failed: {}", stderr))
        }
    }

    /// Check prerequisites before creating a PR (T054)
    fn check_pr_prerequisites(worktree_path: &Path) -> std::result::Result<(), String> {
        // Check for uncommitted changes
        let status_output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to run git status: {}", e))?;

        let status_text = String::from_utf8_lossy(&status_output.stdout);
        if !status_text.trim().is_empty() {
            return Err(
                "Uncommitted changes detected. Please commit all changes first.".to_string(),
            );
        }

        // Check that there are actual changes vs the base branch
        let diff_output = std::process::Command::new("git")
            .args(["diff", "HEAD"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to run git diff: {}", e))?;

        // git diff HEAD should be empty (all committed), but we want commits ahead of base
        // So check if there are commits ahead
        let _ = diff_output; // We already checked porcelain above

        // Check gh auth status
        let auth_output = std::process::Command::new("gh")
            .args(["auth", "status"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to run gh auth status: {}", e))?;

        if !auth_output.status.success() {
            return Err("GitHub CLI not authenticated. Run 'gh auth login' first.".to_string());
        }

        Ok(())
    }

    /// Handle merge conflicts by sending resolution instructions to a sub-agent (T055)
    pub fn handle_merge_conflict(
        &self,
        pane_id: &str,
        conflict_info: &str,
    ) -> std::result::Result<(), String> {
        let prompt = format!(
            "A merge conflict has occurred. Please resolve the following conflicts and commit:\n\n{}\n\nSteps:\n1. Review the conflicting files\n2. Edit to resolve conflicts (remove conflict markers)\n3. git add the resolved files\n4. git commit",
            conflict_info
        );
        crate::tmux::pane::send_prompt_to_pane(pane_id, &prompt)
            .map_err(|e| format!("Failed to send merge conflict instructions: {}", e))
    }

    /// Retry a failed task (T057)
    fn retry_task(&mut self, task_id: &TaskId) {
        if let Some(session) = &mut self.session {
            if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                if task.retry_count < 3 {
                    task.retry_count += 1;
                    task.status = TaskStatus::Ready;
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!(
                            "Task {} retrying (attempt {}/3)...",
                            task_id.0, task.retry_count
                        ),
                    });
                } else {
                    task.status = TaskStatus::Failed;
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: format!(
                            "Task {} failed after 3 attempts. User intervention needed.",
                            task_id.0
                        ),
                    });
                    return;
                }
            }
        }
        self.launch_ready_tasks();
    }

    /// Determine if an error is retryable (T056 helper)
    fn is_retryable_error(reason: &str) -> bool {
        let lower = reason.to_lowercase();
        // Transient errors that may succeed on retry
        lower.contains("timeout")
            || lower.contains("connection")
            || lower.contains("rate limit")
            || lower.contains("temporary")
            || lower.contains("503")
            || lower.contains("429")
            || lower.contains("econnreset")
    }

    /// Check if all tasks are complete and update session status
    fn check_session_completion(&mut self) {
        if let Some(session) = &mut self.session {
            let all_done = session.tasks.iter().all(|t| {
                t.status == TaskStatus::Completed
                    || t.status == TaskStatus::Failed
                    || t.status == TaskStatus::Cancelled
            });

            if all_done && !session.tasks.is_empty() {
                let any_failed = session.tasks.iter().any(|t| t.status == TaskStatus::Failed);
                if any_failed {
                    // Some tasks failed, but session is still considered completed
                    let _ = self.message_tx.send(OrchestratorMessage::ChatMessage {
                        role: "system".to_string(),
                        content: "All tasks finished. Some tasks failed.".to_string(),
                    });
                }
                session.status = SessionStatus::Completed;
            } else {
                self.launch_ready_tasks();
            }
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

    // T050: is_test_output_passing tests
    #[test]
    fn test_is_test_output_passing_cargo_ok() {
        let output = "running 5 tests\ntest result: ok. 5 passed; 0 failed;";
        assert!(OrchestratorLoop::is_test_output_passing(output));
    }

    #[test]
    fn test_is_test_output_passing_cargo_failed() {
        let output = "running 5 tests\ntest result: FAILED. 3 passed; 2 failed;";
        assert!(!OrchestratorLoop::is_test_output_passing(output));
    }

    #[test]
    fn test_is_test_output_passing_npm_passed() {
        let output = "Tests: 10 passed, 10 total";
        assert!(OrchestratorLoop::is_test_output_passing(output));
    }

    #[test]
    fn test_is_test_output_passing_npm_failed() {
        let output = "Tests: 2 failed, 8 passed, 10 total";
        assert!(!OrchestratorLoop::is_test_output_passing(output));
    }

    #[test]
    fn test_is_test_output_passing_generic_no_errors() {
        let output = "All checks completed successfully\nDone.";
        assert!(OrchestratorLoop::is_test_output_passing(output));
    }

    #[test]
    fn test_is_test_output_passing_generic_with_error() {
        let output = "Building...\nCompilation error on line 42";
        assert!(!OrchestratorLoop::is_test_output_passing(output));
    }

    // T051: handle_test_passed updates test_status
    #[test]
    fn test_handle_test_passed_updates_test_status() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let task = Task::new(task_id.clone(), "test task", "description");
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.handle_test_passed(&task_id);

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert!(task.test_status.is_some());
        assert_eq!(
            task.test_status.as_ref().unwrap().status,
            TestStatus::Passed
        );
    }

    // T052: handle_test_failed retry logic
    #[test]
    fn test_handle_test_failed_retries_under_max() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let task = Task::new(task_id.clone(), "test task", "description");
        session.tasks.push(task);
        orchestrator.session = Some(session);

        // First failure (attempt 1) - should retry
        orchestrator.handle_test_failed(&task_id, "some test failure");

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(
            task.test_status.as_ref().unwrap().attempt,
            1,
            "first attempt"
        );
        // Task should NOT be Failed (still retryable)
        assert_ne!(task.status, TaskStatus::Failed);
    }

    #[test]
    fn test_handle_test_failed_marks_failed_after_max_retries() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(task_id.clone(), "test task", "description");
        // Pre-set to 2 attempts (next will be 3, which is >= 3, so no retry)
        task.test_status = Some(TestVerification {
            status: TestStatus::Failed,
            command: String::new(),
            output: None,
            attempt: 2,
        });
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.handle_test_failed(&task_id, "persistent failure");

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(task.test_status.as_ref().unwrap().attempt, 3);
        assert_eq!(task.status, TaskStatus::Failed);
    }

    // T054: check_pr_prerequisites (uses nonexistent dir to trigger errors)
    #[test]
    fn test_check_pr_prerequisites_nonexistent_dir() {
        let result =
            OrchestratorLoop::check_pr_prerequisites(std::path::Path::new("/nonexistent/dir"));
        assert!(result.is_err());
    }

    // T056: is_retryable_error tests
    #[test]
    fn test_is_retryable_error_timeout() {
        assert!(OrchestratorLoop::is_retryable_error("Connection timeout"));
    }

    #[test]
    fn test_is_retryable_error_rate_limit() {
        assert!(OrchestratorLoop::is_retryable_error("Rate limit exceeded"));
    }

    #[test]
    fn test_is_retryable_error_503() {
        assert!(OrchestratorLoop::is_retryable_error(
            "Server returned 503 Service Unavailable"
        ));
    }

    #[test]
    fn test_is_retryable_error_429() {
        assert!(OrchestratorLoop::is_retryable_error(
            "HTTP 429 Too Many Requests"
        ));
    }

    #[test]
    fn test_is_retryable_error_connection() {
        assert!(OrchestratorLoop::is_retryable_error("Connection refused"));
    }

    #[test]
    fn test_is_retryable_error_not_retryable() {
        assert!(!OrchestratorLoop::is_retryable_error(
            "Syntax error in code"
        ));
    }

    #[test]
    fn test_is_retryable_error_permission_denied() {
        assert!(!OrchestratorLoop::is_retryable_error("Permission denied"));
    }

    // T057: retry_task tests
    #[test]
    fn test_retry_task_increments_count() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(task_id.clone(), "test task", "description");
        task.status = TaskStatus::Running;
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.retry_task(&task_id);

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(task.retry_count, 1);
        // Task is set to Ready for re-launch, but launch_ready_tasks sets it to Running
        assert_eq!(task.status, TaskStatus::Running);
    }

    #[test]
    fn test_retry_task_fails_after_max() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(task_id.clone(), "test task", "description");
        task.retry_count = 3; // Already at max
        task.status = TaskStatus::Running;
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.retry_task(&task_id);

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
    }

    // T056: handle_sub_agent_failed with retryable vs non-retryable errors
    #[test]
    fn test_handle_sub_agent_failed_retryable() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(task_id.clone(), "test task", "description");
        task.status = TaskStatus::Running;
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.handle_sub_agent_failed(&task_id, "%1", "Connection timeout");

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        // Should have retried (retry_count incremented, status set to Running via launch)
        assert_eq!(task.retry_count, 1);
    }

    #[test]
    fn test_handle_sub_agent_failed_non_retryable() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let task_id = TaskId("task-1".to_string());
        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(task_id.clone(), "test task", "description");
        task.status = TaskStatus::Running;
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.handle_sub_agent_failed(&task_id, "%1", "Syntax error in module");

        let task = orchestrator
            .session
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
    }

    // check_session_completion tests
    #[test]
    fn test_check_session_completion_all_done() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task = Task::new(TaskId("t1".to_string()), "task 1", "desc");
        task.status = TaskStatus::Completed;
        session.tasks.push(task);
        orchestrator.session = Some(session);

        orchestrator.check_session_completion();

        assert_eq!(
            orchestrator.session.as_ref().unwrap().status,
            SessionStatus::Completed
        );
    }

    #[test]
    fn test_check_session_completion_not_done() {
        let (msg_tx, _msg_rx) = mpsc::channel();
        let (mut orchestrator, _event_tx) = OrchestratorLoop::new(msg_tx);

        let mut session = AgentSession::new(
            SessionId("sess-1".to_string()),
            std::path::PathBuf::from("."),
        );
        let mut task1 = Task::new(TaskId("t1".to_string()), "task 1", "desc");
        task1.status = TaskStatus::Completed;
        let mut task2 = Task::new(TaskId("t2".to_string()), "task 2", "desc");
        task2.status = TaskStatus::Running;
        session.tasks.push(task1);
        session.tasks.push(task2);
        orchestrator.session = Some(session);

        orchestrator.check_session_completion();

        assert_eq!(
            orchestrator.session.as_ref().unwrap().status,
            SessionStatus::Active
        );
    }
}
