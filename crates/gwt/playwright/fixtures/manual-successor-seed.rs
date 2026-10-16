use std::path::Path;

use gwt::cli::execution_state as execution;
use gwt_core::workspace_projection::{
    canonical_work_id, load_workspace_projection, load_workspace_work_items,
    record_workspace_work_event, save_workspace_projection, successor_work_id,
    transact_workspace_close_state_for_work_event_root, WorkEvent, WorkEventKind,
    WorkspaceExecutionContainerRef, WorkspaceProjection,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let worktree = Path::new(&args[2]);
    let owner = execution::ExecutionOwnerKey {
        kind: execution::ExecutionOwnerKind::Issue,
        number: args[3].parse().unwrap(),
    };
    match args[1].as_str() {
        "seed" => {
            let predecessor = format!("fixture-completed-predecessor-{}", owner.number);
            let entrypoint = format!("$gwt-execute #{}", owner.number);
            execution::materialize_at_launch(
                worktree,
                owner.kind,
                owner.number,
                &predecessor,
                &entrypoint,
                false,
            )
            .unwrap();
            assert!(matches!(
                execution::settle(
                    worktree,
                    &predecessor,
                    execution::ExecutionSettlement::Completed
                )
                .unwrap(),
                execution::SettleResult::Settled(_)
            ));
            execution::ensure_generation_ledger(
                worktree,
                owner,
                execution::LegacyActiveDisposition::Unknown,
            )
            .unwrap();
            let record = execution::load(worktree).unwrap().unwrap();
            let branch = format!("work/issue-{}", owner.number);
            let mut work_id = canonical_work_id(worktree, Some(&branch), Some(worktree)).unwrap();
            let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, record.launched_at);
            start.title = Some("Manual successor retry fixture".into());
            // Issue #5332: real terminal history can lack its owner even
            // though fresh launch proves the exact linked execution owner.
            start.owner = None;
            start.agent_session_id = Some(predecessor.clone());
            start.agent_id = Some("codex".into());
            start.execution_container = Some(WorkspaceExecutionContainerRef {
                branch: Some(branch),
                worktree_path: Some(format!(r"E:\gwt\work\issue-{}", owner.number).into()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            });
            record_workspace_work_event(worktree, start).unwrap();
            let works = load_workspace_work_items(worktree).unwrap().unwrap();
            let predecessor = works
                .work_items
                .iter()
                .find(|work| work.id == work_id)
                .unwrap();
            let mut workspace = WorkspaceProjection::default_for_project(worktree);
            workspace.apply_work_item(predecessor, predecessor.updated_at);
            save_workspace_projection(worktree, &workspace).unwrap();
            // FR-384: a local close must survive the GUI's initial shared
            // event intake, which intentionally omits shared close events.
            transact_workspace_close_state_for_work_event_root(worktree, worktree, |_, _, _| {
                Ok((
                    (),
                    vec![WorkEvent::new(
                        WorkEventKind::Done,
                        &work_id,
                        record.settled_at.unwrap(),
                    )],
                ))
            })
            .unwrap();
            // Issue #5288 AC-R7: three legitimate same-branch Resume stamps
            // must survive ordinary startup intake and owner-bleed repair.
            for index in 0..3 {
                let parent = work_id;
                work_id = successor_work_id(&parent);
                let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, record.launched_at);
                start.title = Some("Manual successor retry fixture".into());
                start.owner = Some(format!("Issue #{}", owner.number));
                start.related_work_item_id = Some(parent);
                start.execution_container = predecessor.execution_containers.first().cloned();
                record_workspace_work_event(worktree, start.clone()).unwrap();
                start.id = format!("fixture-resume-{index}");
                start.kind = WorkEventKind::Resume;
                start.next_action = Some("Check Board for latest updates".into());
                record_workspace_work_event(worktree, start).unwrap();
                let works = load_workspace_work_items(worktree).unwrap().unwrap();
                let item = works.work_items.iter().find(|work| work.id == work_id).unwrap();
                workspace.apply_work_item(item, item.updated_at);
                save_workspace_projection(worktree, &workspace).unwrap();
                transact_workspace_close_state_for_work_event_root(worktree, worktree, |_, _, _| {
                    Ok(((), vec![WorkEvent::new(WorkEventKind::Done, &work_id, record.settled_at.unwrap())]))
                }).unwrap();
            }
            // Issue #5288: a stale Active successor can retain its owner and
            // container while losing its explicit predecessor link.
            let mut invalid = WorkEvent::new(
                WorkEventKind::Start,
                successor_work_id(&work_id),
                record.settled_at.unwrap(),
            );
            invalid.owner = Some(format!("Issue #{}", owner.number));
            invalid.execution_container = Some(WorkspaceExecutionContainerRef {
                branch: Some(format!("work/issue-{}", owner.number)),
                worktree_path: Some(std::fs::canonicalize(worktree).unwrap()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            });
            record_workspace_work_event(worktree, invalid.clone()).unwrap();
            // Issue #5288: local detach receipts also remove the container
            // carried by a later supersession audit. Preserve that real shape.
            let works_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(worktree);
            std::fs::write(
                works_path.with_extension("detachments.json"),
                serde_json::to_vec(&std::collections::BTreeMap::from([(
                    invalid.work_item_id,
                    vec![invalid.execution_container.unwrap()],
                )]))
                .unwrap(),
            )
            .unwrap();
        }
        "inspect" => {}
        _ => panic!("unknown fixture action"),
    }
    println!(
        "{}",
        serde_json::json!({
            "ledger": execution::load_generation_ledger(worktree, owner).unwrap().unwrap(),
            "record": execution::load(worktree).unwrap(),
            "binding": execution::current_execution_binding(worktree, owner).unwrap(),
            "works": load_workspace_work_items(worktree).unwrap(),
            "workspace": load_workspace_projection(worktree).unwrap(),
            "verification_plan": gwt::cli::verification_record::load_plan(worktree).unwrap(),
            "verification_run": gwt::cli::verification_record::load(worktree).unwrap(),
        })
    );
}
