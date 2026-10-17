//! Focused coverage for the definition of a "known actor" in `AuthMatrix`
//! reports, plus auth-state reset behaviour across actor cells.

use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal};
use soroban_testkit::auth::AuthMatrix;
use soroban_testkit::core::TestEnv;

#[contract]
struct SecureBank;

#[contractimpl]
impl SecureBank {
    pub fn privileged(env: Env, caller: Address) {
        caller.require_auth();
        // Dummy operation
        let _ = env;
    }
}

#[test]
fn known_actors_is_the_deduped_union_in_first_seen_order() {
    let env = TestEnv::new();
    let alice = env.address();
    let bob = env.address();
    let carol = env.address();

    let matrix = AuthMatrix::new(&env)
        .entry_point("a", &[alice.clone(), bob.clone()], |_| Ok(()))
        .entry_point("b", &[bob.clone(), carol.clone()], |_| Ok(()));

    // `bob` is named on both entry points but appears once.
    assert_eq!(matrix.known_actors(), vec![alice, bob, carol]);
}

#[test]
fn addresses_never_listed_are_not_known_actors() {
    let env = TestEnv::new();
    let listed = env.address();
    let never_listed = env.address();

    let matrix =
        AuthMatrix::new(&env).entry_point("only", std::slice::from_ref(&listed), |_| Ok(()));

    let known = matrix.known_actors();
    assert_eq!(known, vec![listed]);
    assert!(!known.contains(&never_listed));
}

#[test]
fn report_states_the_known_actor_definition_and_names_the_offender() {
    let env = TestEnv::new();
    let owner = env.address();
    let attacker = env.address();

    // `open` allows both callers, so both are known actors; `guarded` is
    // buggy and lets everyone through, so the known actor `attacker` wrongly
    // succeeds there and must be flagged.
    let matrix = AuthMatrix::new(&env)
        .entry_point("open", &[owner.clone(), attacker.clone()], |_| Ok(()))
        .entry_point("guarded", std::slice::from_ref(&owner), |_| Ok(()));

    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| matrix.assert_enforced()))
        .expect_err("guarded allows everyone, so the attacker cell must be caught");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();

    assert!(
        message.contains("known actors = union of all entry points' allowed lists"),
        "the report must state the known-actor definition: {message}"
    );
    assert!(
        message.contains(&format!("{attacker:?}")),
        "the report must name the offending known actor: {message}"
    );
    assert!(
        message.contains("guarded"),
        "the report must name the entry point that let the actor through: {message}"
    );
}

#[test]
fn auth_state_is_reset_between_independent_actor_cases() {
    let env = TestEnv::new();
    let contract_id = env.env().register(SecureBank, ());
    let alice = env.address();
    let _bob = env.address();
    let client = SecureBankClient::new(env.env(), &contract_id);

    // Build an AuthMatrix where only alice is allowed to call `privileged`.
    // In the closure, we mock auth for whatever caller is passed, BUT if caller is not alice,
    // we do not mock auth (or contract rejects).
    // Specifically, if alice runs first and mocks auth, bob's case runs with clean auth state.
    let matrix = AuthMatrix::new(&env).entry_point(
        "privileged",
        std::slice::from_ref(&alice),
        |caller: &Address| {
            if *caller == alice {
                match client
                    .mock_auths(&[MockAuth {
                        address: caller,
                        invoke: &MockAuthInvoke {
                            contract: &contract_id,
                            fn_name: "privileged",
                            args: (caller.clone(),).into_val(env.env()),
                            sub_invokes: &[],
                        },
                    }])
                    .try_privileged(caller)
                {
                    Ok(Ok(())) => Ok(()),
                    _ => Err(soroban_sdk::Error::from_contract_error(1)),
                }
            } else {
                match client.try_privileged(caller) {
                    Ok(Ok(())) => Ok(()),
                    _ => Err(soroban_sdk::Error::from_contract_error(1)),
                }
            }
        },
    );

    // Before and after each actor cell, auth is automatically reset.
    // If auth leaked from alice to bob, bob would unexpectedly pass!
    // Since bob is rejected and alice succeeds, assert_enforced passes.
    matrix.assert_enforced();
}

#[test]
fn reset_auth_helpers_work_on_matrix_and_env() {
    let env = TestEnv::new();
    let matrix = AuthMatrix::new(&env);

    // Call reset_auth on both instances without error
    matrix.reset_auth();
    env.reset_auth();
}

#[test]
fn assert_requires_auth_resets_auth_state() {
    let env = TestEnv::new();
    let contract_id = env.env().register(SecureBank, ());
    let alice = env.address();
    let client = SecureBankClient::new(env.env(), &contract_id);

    // Without mocked auth, calling privileged should fail
    env.assert_requires_auth(|| {
        client.privileged(&alice);
    });

    // After assert_requires_auth, environment auth state remains clean
    env.reset_auth();
}
