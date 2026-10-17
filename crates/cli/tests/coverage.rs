//! Integration tests for `soroban-testkit coverage`.
//!
//! Exercises the CLI binary for coverage collection options, platform open behavior,
//! output file creation, package filtering, and validation diagnostics.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soroban-testkit"))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("failed to run soroban-testkit")
}

#[test]
fn coverage_help_documents_options_and_platform_notes() {
    let output = run(&["coverage", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("--format"), "{stdout}");
    assert!(stdout.contains("--fail-under"), "{stdout}");
    assert!(stdout.contains("--open"), "{stdout}");
    assert!(stdout.contains("--include"), "{stdout}");
    assert!(stdout.contains("--exclude"), "{stdout}");
    assert!(stdout.contains("--output-dir"), "{stdout}");
    assert!(stdout.contains("--output"), "{stdout}");
}

#[test]
fn combining_include_and_exclude_is_rejected_with_actionable_error() {
    let output = run(&["coverage", "--include", "core", "--exclude", "cli"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--include and --exclude cannot be combined"),
        "{stderr}"
    );
}

#[test]
fn output_option_rejected_for_non_json_formats() {
    let output = run(&["coverage", "--format", "text", "--output", "coverage.json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--output is only valid with --format json"),
        "{stderr}"
    );

    let output_lcov = run(&["coverage", "--format", "lcov", "--output", "coverage.json"]);
    assert!(!output_lcov.status.success());
    let stderr_lcov = String::from_utf8_lossy(&output_lcov.stderr);
    assert!(
        stderr_lcov.contains("--output is only valid with --format json"),
        "{stderr_lcov}"
    );
}

#[test]
fn open_option_rejected_for_incompatible_formats() {
    // Rejected for JSON
    let output_json = run(&["coverage", "--format", "json", "--open"]);
    assert!(!output_json.status.success());
    let stderr_json = String::from_utf8_lossy(&output_json.stderr);
    assert!(
        stderr_json.contains("--open is only valid with --format html"),
        "{stderr_json}"
    );

    // Rejected for LCOV
    let output_lcov = run(&["coverage", "--format", "lcov", "--open"]);
    assert!(!output_lcov.status.success());
    let stderr_lcov = String::from_utf8_lossy(&output_lcov.stderr);
    assert!(
        stderr_lcov.contains("--open is only valid with --format html"),
        "{stderr_lcov}"
    );
}

#[test]
fn fail_under_percentage_bounds_are_enforced() {
    let output_negative = run(&["coverage", "--fail-under=-5"]);
    assert!(!output_negative.status.success());
    let stderr_neg = String::from_utf8_lossy(&output_negative.stderr);
    assert!(
        stderr_neg.contains("must be between 0 and 100"),
        "{stderr_neg}"
    );

    let output_over = run(&["coverage", "--fail-under=105"]);
    assert!(!output_over.status.success());
    let stderr_over = String::from_utf8_lossy(&output_over.stderr);
    assert!(
        stderr_over.contains("must be between 0 and 100"),
        "{stderr_over}"
    );
}

#[test]
fn format_names_are_consistent_in_errors_and_help() {
    let output = run(&["coverage", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("text"), "{stdout}");
    assert!(stdout.contains("lcov"), "{stdout}");
    assert!(stdout.contains("html"), "{stdout}");
    assert!(stdout.contains("json"), "{stdout}");

    // Format names in error messages:
    let output_text_out = run(&["coverage", "--format", "text", "--output", "coverage.json"]);
    assert!(!output_text_out.status.success());
    let stderr_text = String::from_utf8_lossy(&output_text_out.stderr);
    assert!(
        stderr_text.contains("--output is only valid with --format json (got --format text)"),
        "{stderr_text}"
    );

    let output_lcov_open = run(&["coverage", "--format", "lcov", "--open"]);
    assert!(!output_lcov_open.status.success());
    let stderr_lcov = String::from_utf8_lossy(&output_lcov_open.stderr);
    assert!(
        stderr_lcov.contains("--open is only valid with --format html (got --format lcov)"),
        "{stderr_lcov}"
    );
}

// ── Bug 1 regression: no browser opened when HTML generation fails ────────────

#[test]
fn open_rejected_for_non_html_non_text_formats_is_actionable() {
    // Already covered by open_option_rejected_for_incompatible_formats, but
    // explicitly verifies the error is actionable and doesn't mention browser
    // launch when the real problem is the format mismatch.
    let output = run(&["coverage", "--format", "json", "--open"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--open is only valid with --format html"),
        "{stderr}"
    );
    // The rejection happens before any subprocess or browser launch attempt
    assert!(
        !stderr.contains("browser opener"),
        "early validation should not mention browser opener: {stderr}"
    );
}

// ── Bug 2 regression: --output-dir rejected with incompatible formats ─────────

#[test]
fn output_dir_with_text_format_is_rejected_with_actionable_error() {
    let output = run(&["coverage", "--format", "text", "--output-dir", "/tmp/cov"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--output-dir is not valid with --format text"),
        "{stderr}"
    );
    assert!(
        stderr.contains("text output is written to the terminal"),
        "error should explain why: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn output_dir_with_text_format_and_open_is_accepted() {
    use std::os::unix::fs::PermissionsExt;

    // Stub Cargo so this integration test checks argument validation without
    // recursively running cargo-llvm-cov while the outer coverage job is
    // instrumenting this very test binary. The previous test could hang CI.
    let root = std::env::temp_dir().join(format!("stk-coverage-accept-{}", std::process::id()));
    let bin = root.join("bin");
    let report = root.join("report");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\nexit 17\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var_os("PATH")
            .unwrap_or_default()
            .to_string_lossy()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-testkit"))
        .args([
            "coverage",
            "--format",
            "text",
            "--open",
            "--output-dir",
            report.to_str().unwrap(),
        ])
        .env("PATH", path)
        .current_dir(workspace_root())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "stub cargo should exit nonzero");
    assert!(
        stderr.contains("cargo llvm-cov exited with exit status: 17"),
        "valid flags must reach the stubbed cargo command: {stderr}"
    );
    assert!(
        !stderr.contains("--output-dir is not valid with --format text"),
        "valid combination was wrongly rejected: {stderr}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
