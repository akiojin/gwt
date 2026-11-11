use super::*;

#[test]
fn unsupported_codex_ultrafast_warning_reaches_terminal_without_process_exit() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "codex-1");
    // Codex can omit an unavailable tier and continue running. Preserve its
    // actual diagnostic rather than depending on a nonzero process exit.
    let warning = "Configured service tier `ultrafast` is not advertised as supported for model `gpt-6-astra` and will be omitted from requests.\r\n";
    let events = runtime.handle_owner_runtime_output(
        id.clone(),
        warning.as_bytes().to_vec(),
        1,
        warning.into(),
    );
    assert!(events.iter().any(|event| {
        matches!(&event.event, BackendEvent::TerminalOutput { id: actual, data_base64 }
            if actual == &id
                && base64::engine::general_purpose::STANDARD.decode(data_base64).unwrap()
                    == warning.as_bytes())
    }));
}
