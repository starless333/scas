use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BINARY: &str = env!("CARGO_BIN_EXE_scas");

fn run(source: &str) -> Output {
    let mut child = Command::new(BINARY)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start scas");

    child
        .stdin
        .take()
        .expect("child stdin was not piped")
        .write_all(source.as_bytes())
        .expect("failed to send source to scas");

    child.wait_with_output().expect("failed to wait for scas")
}

#[test]
fn help_and_version_are_conventional() {
    let help = Command::new(BINARY).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(text(&help.stdout).contains("USAGE:"));
    let version = Command::new(BINARY).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(text(&version.stdout), "scas 0.1.0\n");
}

#[test]
fn source_file_and_precision_flag_are_supported() {
    let path = std::env::temp_dir().join(format!("scas-cli-{}.scas", std::process::id()));
    fs::write(&path, "print(1.0 / 7);\n").unwrap();
    let output = Command::new(BINARY)
        .args(["--precision", "6"])
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(output.status.success());
    assert_eq!(text(&output.stdout), "0.142857\n");
    assert_eq!(text(&output.stderr), "");
}

fn fixtures(directory: &str) -> Vec<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(directory);
    let mut fixtures: Vec<_> = fs::read_dir(path)
        .expect("fixture directory should exist")
        .map(|entry| entry.expect("fixture should be readable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "scas")
        })
        .collect();
    fixtures.sort();
    fixtures
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("scas output should be UTF-8")
}

fn assert_exit_one(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "unexpected process status: {:?}",
        output.status
    );
}

#[test]
fn every_should_pass_fixture_succeeds_with_expected_output() {
    let fixtures = fixtures("should_pass");
    assert!(
        !fixtures.is_empty(),
        "expected at least one passing fixture"
    );

    for fixture in fixtures {
        let source = fs::read_to_string(&fixture).expect("passing fixture should be readable");
        let expected = fs::read_to_string(fixture.with_extension("stdout"))
            .expect("every passing fixture needs a matching .stdout file");
        let output = run(&source);

        assert!(
            output.status.success(),
            "{} failed with stderr:\n{}",
            fixture.display(),
            text(&output.stderr),
        );
        assert_eq!(
            text(&output.stdout),
            expected,
            "stdout differed for {}",
            fixture.display()
        );
        assert_eq!(
            text(&output.stderr),
            "",
            "stderr was not empty for {}",
            fixture.display()
        );
    }
}

#[test]
fn every_should_fail_fixture_fails_with_expected_diagnostic() {
    let fixtures = fixtures("should_fail");
    assert!(
        !fixtures.is_empty(),
        "expected at least one failing fixture"
    );

    for fixture in fixtures {
        let source = fs::read_to_string(&fixture).expect("failing fixture should be readable");
        let expected = fs::read_to_string(fixture.with_extension("stderr"))
            .expect("every failing fixture needs a matching .stderr file");
        let output = run(&source);

        assert_exit_one(&output);
        assert_eq!(
            text(&output.stdout),
            "",
            "stdout was not empty for {}",
            fixture.display()
        );
        assert_eq!(
            text(&output.stderr),
            expected,
            "stderr differed for {}",
            fixture.display()
        );
    }
}

#[test]
fn lexical_errors_fail_on_stderr() {
    let output = run("let value = @;\n");

    assert_exit_one(&output);
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        text(&output.stderr),
        "lexical error: unexpected character `@`\n  --> <stdin>:1:13\n   |\n 1 | let value = @;\n   |             ^\n",
    );
}

#[test]
fn runtime_errors_fail_on_stderr() {
    let output = run("print(1 / 0);\n");

    assert_exit_one(&output);
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        text(&output.stderr),
        "runtime error: division by zero in '/'\n  --> <stdin>:1:1\n   |\n 1 | print(1 / 0);\n   | ^^^^^\n"
    );
}

#[cfg(unix)]
#[test]
fn stdin_read_errors_fail_on_stderr() {
    let directory = fs::File::open(env!("CARGO_MANIFEST_DIR"))
        .expect("the project directory should be openable");
    let output = Command::new(BINARY)
        .stdin(Stdio::from(directory))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to start scas");

    assert_exit_one(&output);
    assert_eq!(text(&output.stdout), "");
    assert!(
        text(&output.stderr).starts_with("error: failed to read standard input:"),
        "unexpected stderr: {}",
        text(&output.stderr),
    );
}
