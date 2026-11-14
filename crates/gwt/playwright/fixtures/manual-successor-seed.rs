use std::path::Path;

use gwt::cli::execution_state as execution;

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
        })
    );
}
