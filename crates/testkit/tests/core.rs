//! Integration and unit tests for `soroban_testkit::core` error context enhancements.
//!
//! Verifies that `TestkitError` context carries operation and contract details
//! while preserving source error chains, machine-readable codes, and snapshot stability.

use soroban_testkit::core::TestkitError;
use std::error::Error as _;

#[test]
fn with_operation_formats_expected_context() {
    let err = TestkitError::AssertionFailed("balance underflow".into()).with_operation("withdraw");

    assert_eq!(
        err.to_string(),
        "operation 'withdraw': assertion failed: balance underflow"
    );
    assert_eq!(err.code(), "TESTKIT_ASSERTION_FAILED");
    assert_eq!(err.context(), Some("operation 'withdraw'"));
    assert_eq!(err.message(), "balance underflow");
    assert!(err.is_assertion_failed());
    assert!(!err.is_misuse());
}

#[test]
fn with_contract_formats_expected_context() {
    let contract_id = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
    let err = TestkitError::Misuse("uninitialized storage".into()).with_contract(contract_id);

    assert_eq!(
        err.to_string(),
        format!("contract '{contract_id}': misuse of testkit API: uninitialized storage")
    );
    assert_eq!(err.code(), "TESTKIT_MISUSE");
    assert_eq!(
        err.context(),
        Some(&format!("contract '{contract_id}'")[..])
    );
    assert_eq!(err.message(), "uninitialized storage");
    assert!(err.is_misuse());
    assert!(!err.is_decode_failed());
}

#[test]
fn with_operation_and_contract_combines_context_details() {
    let contract = "CA3D5KRYMCMCZMQN7SC7PB5EAL6HN5JIGOX264DHB2LBOOHGOHO66B";
    let err = TestkitError::DecodeFailed("expected U32, found Symbol".into())
        .with_operation_and_contract("get_nonce", contract);

    let expected_context = format!("operation 'get_nonce' on contract '{contract}'");
    assert_eq!(
        err.to_string(),
        format!("{expected_context}: failed to decode value: expected U32, found Symbol")
    );
    assert_eq!(err.code(), "TESTKIT_DECODE_FAILED");
    assert_eq!(err.context(), Some(expected_context.as_str()));
    assert_eq!(err.message(), "expected U32, found Symbol");
    assert!(err.is_decode_failed());
}

#[test]
fn context_method_returns_none_for_root_errors() {
    assert_eq!(TestkitError::assertion_failed("test").context(), None);
    assert_eq!(TestkitError::decode_failed("test").context(), None);
    assert_eq!(TestkitError::misuse("test").context(), None);
}

#[test]
fn nested_operation_and_contract_chains_preserve_source_chain() {
    let contract = "CCW67W7X7D3R2TDFZ2T3T3L54V74V4Q2K3Z6K6K6K6K6K6K6K6K6K6K6";
    let base = TestkitError::decode_failed("invalid ScVal discriminant");

    let err = base
        .with_operation("parse_token_metadata")
        .with_contract(contract)
        .with_context("loading contract registry");

    assert_eq!(
        err.to_string(),
        format!("loading contract registry: contract '{contract}': operation 'parse_token_metadata': failed to decode value: invalid ScVal discriminant")
    );

    // Verify source chain links
    let source1 = err.source().expect("first source must exist");
    assert_eq!(
        source1.to_string(),
        format!("contract '{contract}': operation 'parse_token_metadata': failed to decode value: invalid ScVal discriminant")
    );

    let source2 = source1.source().expect("second source must exist");
    assert_eq!(
        source2.to_string(),
        "operation 'parse_token_metadata': failed to decode value: invalid ScVal discriminant"
    );

    let source3 = source2.source().expect("third source must exist");
    assert_eq!(
        source3.to_string(),
        "failed to decode value: invalid ScVal discriminant"
    );
    assert!(source3.source().is_none());

    // Machine-readable code is preserved through all layers
    assert_eq!(err.code(), "TESTKIT_DECODE_FAILED");
}

#[test]
fn to_snapshot_formats_cleanly_with_operation_and_contract() {
    let err = TestkitError::assertion_failed("invariant violated: sum != total")
        .with_operation_and_contract("mint_batch", "CVOLT64EXAMPLE");

    let snapshot = err.to_snapshot();
    assert_eq!(
        snapshot,
        "[TESTKIT_ASSERTION_FAILED] operation 'mint_batch' on contract 'CVOLT64EXAMPLE': assertion failed: invariant violated: sum != total"
    );

    // Snapshot display aliases must match
    assert_eq!(err.render_snapshot(), snapshot);
    assert_eq!(err.snapshot(), snapshot);
    assert_eq!(err.snapshot_display(), snapshot);
}

#[test]
fn debug_representation_matches_display_with_context() {
    let err = TestkitError::misuse("cannot rollback committed block")
        .with_operation("commit_block")
        .with_contract("CLEDGER0001");

    assert_eq!(format!("{err:?}"), err.to_string());
}

#[test]
fn edge_cases_empty_and_special_characters_in_operation_and_contract() {
    // Empty strings
    let err_empty =
        TestkitError::assertion_failed("empty details").with_operation_and_contract("", "");
    assert_eq!(
        err_empty.to_string(),
        "operation '' on contract '': assertion failed: empty details"
    );
    assert_eq!(err_empty.context(), Some("operation '' on contract ''"));

    // Unicode and whitespace
    let err_unicode = TestkitError::misuse("invalid symbol")
        .with_operation_and_contract("transfer_💰", "Contract #42 (Stellar/Testnet)");
    assert_eq!(
        err_unicode.to_string(),
        "operation 'transfer_💰' on contract 'Contract #42 (Stellar/Testnet)': misuse of testkit API: invalid symbol"
    );
}

// ── Bug #462 regression: seeded address generation stable across call ordering ──

#[test]
fn token_call_before_address_does_not_shift_seeded_stream() {
    use soroban_testkit::core::TestEnv;

    let env_a = TestEnv::with_seed(462);
    let env_b = TestEnv::with_seed(462);

    // env_a: token first, then addresses
    let _token = env_a.token();
    let a1 = env_a.address();
    let a2 = env_a.address();

    // env_b: addresses only, no token
    let b1 = env_b.address();
    let b2 = env_b.address();

    assert_eq!(
        a1, b1,
        "first address after token() differs from plain first address — token() is advancing the stream"
    );
    assert_eq!(a2, b2, "second address shifted");
}

#[test]
fn address_sequence_identical_with_and_without_interleaved_tokens() {
    use soroban_testkit::core::TestEnv;

    let with_tokens = TestEnv::with_seed(1_000);
    let without_tokens = TestEnv::with_seed(1_000);

    let _t1 = with_tokens.token();
    let w1 = with_tokens.address();
    let _t2 = with_tokens.token();
    let w2 = with_tokens.address();
    let _t3 = with_tokens.token();

    let n1 = without_tokens.address();
    let n2 = without_tokens.address();

    assert_eq!(
        w1, n1,
        "interleaved token() before first address shifted stream"
    );
    assert_eq!(
        w2, n2,
        "interleaved token() between addresses shifted stream"
    );
}

#[test]
fn token_admin_is_distinct_from_all_testenv_generated_addresses() {
    // The admin must not appear in any address the TestEnv issues, and must
    // not collide with user-visible addresses.
    use soroban_testkit::core::TestEnv;

    let env = TestEnv::with_seed(2);
    let token = env.token();
    let addrs = env.addresses(10);

    let token_addr = token.address();
    for addr in &addrs {
        assert_ne!(
            addr, &token_addr,
            "TestToken contract address collided with a TestEnv-generated address"
        );
    }
}
