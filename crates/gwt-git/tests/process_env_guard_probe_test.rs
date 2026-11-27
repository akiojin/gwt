//! SPEC #4072 T-6 probe: a deliberate process-env write in a crate other than
//! gwt-core. The workspace guard in gwt-core's test_hygiene_test must fail.
#[test]
fn writes_the_process_env() {
    std::env::set_var("X", "1");
}
