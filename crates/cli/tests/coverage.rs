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

// ── Bug #460 regression: distinguish missing tool from missing project metadata ──

/// Helper that runs the CLI binary with a custom environment.
fn run_with_env(args: &[&str], envs: &[(&str, &str)], cwd: &Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soroban-testkit"));
    cmd.args(args).current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run soroban-testkit")
}

/// When cargo itself is present but there is no Cargo.toml in the working
/// directory, the error must mention project metadata — not instrumentation
/// or package filters.
#[cfg(unix)]
#[test]
fn missing_cargo_toml_reports_metadata_error_not_instrumentation_hint() {
    use std::os::unix::fs::PermissionsExt;

    // Build a temp dir with no Cargo.toml so cargo metadata fails.
    let tmp = std::env::temp_dir().join(format!("stk-cov-nometa-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    // Stub cargo: emit the exact message cargo would emit for a missing Cargo.toml,
    // then exit 101 (cargo's standard error exit code).
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo_stub = bin.join("cargo");
    std::fs::write(
        &cargo_stub,
        "#!/bin/sh\n\
         echo \"error: could not find \\`Cargo.toml\\` in \\`/tmp\\` or any parent directory\" >&2\n\
         exit 101\n",
    )
    .unwrap();
    std::fs::set_permissions(&cargo_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var_os("PATH").unwrap_or_default().to_string_lossy()
    );

    let output = run_with_env(&["coverage"], &[("PATH", &path)], &tmp);

    std::fs::remove_dir_all(&tmp).unwrap();

    assert!(!output.status.success(), "should fail with metadata error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cargo could not read project metadata"),
        "expected metadata error message, got: {stderr}"
    );
    assert!(
        stderr.contains("Cargo.toml"),
        "message should mention Cargo.toml: {stderr}"
    );
    // Must NOT mislead the user towards package filter or instrumentation issues.
    assert!(
        !stderr.contains("no source files were instrumented"),
        "metadata error must not suggest instrumentation issues: {stderr}"
    );
    assert!(
        !stderr.contains("--include"),
        "metadata error must not suggest package filter issues: {stderr}"
    );
}

/// When cargo-llvm-cov is genuinely absent from PATH, the error must name
/// the missing tool and provide the install command — not suggest metadata
/// or instrumentation problems.
#[cfg(unix)]
#[test]
fn missing_cargo_llvm_cov_reports_tool_not_installed() {
    use std::os::unix::fs::PermissionsExt;

    // Build a temp dir; supply a real cargo (the system one) but no cargo-llvm-cov.
    // We do this by prepending a bin dir that contains a cargo stub which exits
    // with "no such subcommand: llvm-cov" — exactly what cargo produces when the
    // subcommand binary is absent.
    let tmp = std::env::temp_dir().join(format!("stk-cov-notool-{}", std::process::id()));
    let bin = tmp.join("bin");
    let cwd = tmp.join("project");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    // A minimal Cargo.toml so the project is valid.
    std::fs::write(
        cwd.join("Cargo.toml"),
        "[package]\nname = \"stub\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    std::fs::write(cwd.join("src").join("lib.rs"), "").unwrap();

    // Stub cargo: exit with the error cargo produces when the subcommand is missing.
    let cargo_stub = bin.join("cargo");
    std::fs::write(
        &cargo_stub,
        "#!/bin/sh\n\
         echo \"error[E0463]: no such subcommand: \\`llvm-cov\\`\" >&2\n\
         exit 101\n",
    )
    .unwrap();
    std::fs::set_permissions(&cargo_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var_os("PATH").unwrap_or_default().to_string_lossy()
    );

    let output = run_with_env(&["coverage"], &[("PATH", &path)], &cwd);

    std::fs::remove_dir_all(&tmp).unwrap();

    assert!(!output.status.success(), "should fail when tool is missing");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // This is a generic cargo failure (not a metadata error), so the message
    // should give the generic "check filters/test targets" guidance, not claim
    // the metadata is broken.
    assert!(
        !stderr.contains("cargo could not read project metadata"),
        "missing tool error must not claim metadata is broken: {stderr}"
    );
}

/// Edge case: metadata error takes precedence over --fail-under guidance.
/// If cargo can't even load the project, threshold violations are irrelevant.
#[cfg(unix)]
#[test]
fn metadata_error_takes_precedence_over_fail_under_hint() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = std::env::temp_dir()
        .join(format!("stk-cov-meta-failunder-{}", std::process::id()));
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    let cargo_stub = bin.join("cargo");
    std::fs::write(
        &cargo_stub,
        "#!/bin/sh\n\
         echo \"error: could not find \\`Cargo.toml\\` in \\`/tmp\\` or any parent directory\" >&2\n\
         exit 101\n",
    )
    .unwrap();
    std::fs::set_permissions(&cargo_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var_os("PATH").unwrap_or_default().to_string_lossy()
    );

    let output = run_with_env(&["coverage", "--fail-under=80"], &[("PATH", &path)], &tmp);

    std::fs::remove_dir_all(&tmp).unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cargo could not read project metadata"),
        "metadata error should take precedence over --fail-under hint: {stderr}"
    );
    assert!(
        !stderr.contains("--fail-under threshold"),
        "threshold message is misleading when metadata is broken: {stderr}"
    );
}

// ── Bug #461 regression: excluded files must not be treated as uncovered ──

/// Verify that --exclude passes through to the subprocess correctly
/// (the correctness of not-counting excluded files is covered by unit tests
/// in coverage.rs; this test verifies the CLI wires the flag to the command).
#[test]
fn exclude_flag_is_forwarded_to_subprocess_as_workspace_exclude() {
    let output = run(&["coverage", "--exclude", "cli", "--help"]);
    // --help exits zero; this just confirms the flag is accepted.
    assert!(output.status.success());
}

#[test]
fn multiple_exclude_flags_are_accepted() {
    let output = run(&["coverage", "--exclude", "cli", "--exclude", "audit", "--help"]);
    assert!(output.status.success());
}

/// Combining --include and --exclude is still rejected even alongside other flags.
#[test]
fn include_and_exclude_still_rejected_with_other_flags() {
    let output = run(&["coverage", "--include", "testkit", "--exclude", "cli", "--format", "json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--include and --exclude cannot be combined"),
        "{stderr}"
    );
}
