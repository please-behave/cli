//! Tests start processes only through `support::command`, which isolates the harness environment.
use std::{fs, path::Path};

#[test]
fn tests_spawn_processes_only_through_the_isolating_helper() {
    // Assembled so that this file does not contain the forbidden call itself.
    let forbidden = format!("{}::new(", "Command");
    let offenders: Vec<_> = fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != "hygiene.rs" && name != "mod.rs")
        })
        .filter(|path| {
            // `check_command::Command` builds rule commands; only process spawning is forbidden.
            fs::read_to_string(path)
                .unwrap()
                .replace("check_command::Command::new(", "")
                .contains(&forbidden)
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "use support::command instead of spawning directly in {offenders:?}"
    );
}
