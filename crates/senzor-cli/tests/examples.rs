//! Every project file the documentation shows lives in `examples/` and must stay valid.

use std::path::PathBuf;

use assert_cmd::Command;

fn example_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    files
}

/// The variable names written as `password_env = "<NAME>"`, found by a plain text scan.
fn password_env_names(text: &str) -> Vec<String> {
    const KEY: &str = "password_env";
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        rest = &rest[at + KEY.len()..];
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let Some(value) = value.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = value.find('"') {
            names.push(value[..end].to_string());
        }
    }
    names
}

#[test]
fn password_env_names_are_read_from_the_text() {
    let text = "password_env = \"A\"\nbroker = { password_env=\"B\" }\npassword_env = 3\nhost = \"password_env\"\n";
    assert_eq!(password_env_names(text), ["A", "B"]);
}

#[test]
fn every_example_project_is_valid() {
    let files = example_files();
    assert!(!files.is_empty(), "examples/ has no .toml files");
    let mut failures = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let mut command = Command::cargo_bin("senzor-cli").unwrap();
        command.arg("validate").arg(&file);
        // An example that reads its password from the environment gets a dummy one.
        for name in password_env_names(&text) {
            command.env(name, "example");
        }
        let output = command.output().unwrap();
        if output.status.code() != Some(0) {
            failures.push(format!(
                "{} ({}): {}",
                file.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} example(s) are invalid:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
