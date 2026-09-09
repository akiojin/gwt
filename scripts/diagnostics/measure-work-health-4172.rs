//! Local diagnostic only; link against the checkout's normal (not test-support) gwt rlib.
//! Usage: measure-work-health-4172 <fixture-worktree> <1,32,254> <repeats> [expected-bin]
//! Parent must set HOME and USERPROFILE to the same isolated fixture home.
//! Run separately with an empty ledger and a copied ledger; this program never writes.
use gwt::cli::hook::health::{read_managed_hook_health, ManagedHookHealthInput};
use std::{fs, hint::black_box, path::PathBuf, time::Instant};

fn run() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(3..=4).contains(&args.len()) {
        return Err("invalid argument count");
    }
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("missing isolated HOME")?)
        .canonicalize()
        .map_err(|_| "invalid HOME")?;
    let profile =
        PathBuf::from(std::env::var_os("USERPROFILE").ok_or("missing isolated USERPROFILE")?)
            .canonicalize()
            .map_err(|_| "invalid USERPROFILE")?;
    let worktree = PathBuf::from(&args[0])
        .canonicalize()
        .map_err(|_| "invalid fixture")?;
    if home != profile || !worktree.starts_with(&home) || worktree == home {
        return Err("fixture must be under matching isolated HOME and USERPROFILE");
    }
    // A marker supplied by the parent avoids accidentally accepting the real home.
    if !home.join(".issue-4172-measurement-fixture").is_file() {
        return Err("missing isolation marker");
    }
    let counts = args[1]
        .to_str()
        .ok_or("invalid counts")?
        .split(',')
        .map(|s| {
            s.parse::<usize>()
                .ok()
                .filter(|n| (1..=10000).contains(n))
                .ok_or("invalid count")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let repeats = args[2]
        .to_str()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| (1..=100).contains(n))
        .ok_or("invalid repeats")?;
    let mut input = ManagedHookHealthInput::new(&worktree);
    input.runtime_state_path = None;
    input.profile_path = None;
    input.expected_hook_bin = args
        .get(3)
        .map(|s| s.to_str().ok_or("invalid expected bin"))
        .transpose()?
        .map(str::to_owned);

    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut lines = 0usize;
    let ledger = home.join(".gwt/logs/errors");
    if ledger.exists() {
        for entry in fs::read_dir(ledger).map_err(|_| "cannot enumerate fixture ledger")? {
            let path = entry.map_err(|_| "cannot read fixture entry")?.path();
            let eligible = path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("errors.") && s.ends_with(".jsonl"));
            if !eligible {
                continue;
            }
            let data = fs::read(path).map_err(|_| "cannot read fixture ledger")?;
            files += 1;
            bytes += data.len() as u64;
            lines += data
                .split(|b| *b == b'\n')
                .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
                .count();
        }
    }
    println!(
        "{{\"ledger_files\":{files},\"ledger_bytes\":{bytes},\"ledger_nonempty_lines\":{lines}}}"
    );
    // These are repeated evaluations of ONE fixture, not distinct Work projections.
    black_box(read_managed_hook_health(&input));
    for count in counts {
        for repeat in 0..repeats {
            let start = Instant::now();
            let mut issues = 0usize;
            for _ in 0..count {
                let health = black_box(read_managed_hook_health(black_box(&input)));
                issues += health.issues.len();
            }
            let elapsed_us = start.elapsed().as_micros();
            println!("{{\"evaluations\":{count},\"repeat\":{repeat},\"elapsed_us\":{elapsed_us},\"issue_count_sum\":{issues}}}");
        }
    }
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        eprintln!("diagnostic error: {message}");
        std::process::exit(1);
    }
}
