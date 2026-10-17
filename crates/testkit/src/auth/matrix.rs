use soroban_sdk::Address;

use crate::core::{TestEnv, TestkitError};

type InvokeFn<'a> = Box<dyn Fn(&Address) -> Result<(), soroban_sdk::Error> + 'a>;

struct EntryPointSpec<'a> {
    name: String,
    allowed: Vec<Address>,
    invoke: InvokeFn<'a>,
}

/// A declarative table of who may call what, so that a contract's full
/// caller/entry-point grid is checked by default instead of the four cases
/// someone thought to write by hand.
///
/// Register each privileged entry point with [`AuthMatrix::entry_point`],
/// then call [`AuthMatrix::assert_enforced`] to check that every allowed
/// address succeeds and every other **known actor** is rejected, for every
/// entry point.
///
/// # What counts as a "known actor"
///
/// A *known actor* is any address named in at least one entry point's
/// `allowed` list — the de-duplicated union across every registered entry
/// point, returned by [`AuthMatrix::known_actors`]. The matrix cross-checks
/// each known actor against *every* entry point, not only the one that named
/// it: that is how an actor permitted elsewhere gets caught on a privileged
/// entry point that should have rejected it.
///
/// An address that is never listed on any entry point is **not** a known
/// actor and is never placed in the grid, so a caller that should be rejected
/// must still be named on at least one entry point to be covered.
pub struct AuthMatrix<'a> {
    // Ties the matrix's lifetime to its TestEnv; entry point closures
    // capture their own references into it independently.
    #[allow(dead_code)]
    env: &'a TestEnv,
    entry_points: Vec<EntryPointSpec<'a>>,
}

impl<'a> AuthMatrix<'a> {
    /// Start an empty matrix against `env`.
    pub fn new(env: &'a TestEnv) -> Self {
        Self {
            env,
            entry_points: Vec::new(),
        }
    }

    /// Register a privileged entry point: its name (for failure messages),
    /// the addresses permitted to call it, and a closure that attempts the
    /// call as a given caller, returning `Ok(())` if it succeeded and
    /// `Err` if the contract rejected it.
    ///
    /// `invoke` is typically built from a generated client's `try_*`
    /// method with the candidate address mocked as the caller — see
    /// [`AuthMatrix::assert_enforced`]'s example.
    ///
    /// # Duplicates
    ///
    /// Repeating an address in `allowed` is harmless: the matrix de-duplicates
    /// addresses before building its grid, so a duplicated caller is still
    /// checked exactly once per entry point rather than being counted (or
    /// reported) twice.
    ///
    /// Entry-point *names* are not required to be unique. Registering the same
    /// name twice produces two independently-checked sections in the report —
    /// useful when one contract function is expected to behave differently
    /// under two different setups — but the failure message will name both
    /// with the same string, so give them distinct names unless the
    /// duplication is deliberate.
    pub fn entry_point(
        mut self,
        name: &str,
        allowed: &[Address],
        invoke: impl Fn(&Address) -> Result<(), soroban_sdk::Error> + 'a,
    ) -> Self {
        self.entry_points.push(EntryPointSpec {
            name: name.to_string(),
            allowed: allowed.to_vec(),
            invoke: Box::new(invoke),
        });
        self
    }

    /// The matrix's **known actors**: every address named in any entry
    /// point's `allowed` list, de-duplicated and kept in first-seen order.
    ///
    /// These are exactly the addresses [`AuthMatrix::assert_enforced`] places
    /// in its grid; an address that never appears as `allowed` on any entry
    /// point is not a known actor and is not checked.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::auth::AuthMatrix;
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let alice = env.address();
    /// let bob = env.address();
    ///
    /// let matrix = AuthMatrix::new(&env)
    ///     .entry_point("a", &[alice.clone()], |_| Ok(()))
    ///     .entry_point("b", &[alice.clone(), bob.clone()], |_| Ok(()));
    ///
    /// // `alice` is named on both entry points but appears once.
    /// assert_eq!(matrix.known_actors(), vec![alice, bob]);
    /// ```
    pub fn known_actors(&self) -> Vec<Address> {
        let mut known: Vec<Address> = Vec::new();
        for ep in &self.entry_points {
            for addr in &ep.allowed {
                if !known.contains(addr) {
                    known.push(addr.clone());
                }
            }
        }
        known
    }

    /// Reset all authorization entries and mock authorizations in the test environment.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    /// use soroban_testkit::auth::AuthMatrix;
    ///
    /// let env = TestEnv::new();
    /// let matrix = AuthMatrix::new(&env);
    /// matrix.reset_auth();
    /// ```
    pub fn reset_auth(&self) {
        self.env.env().set_auths(&[]);
    }

    /// For every registered entry point, assert every allowed address
    /// succeeds and every other **known actor** (see [`AuthMatrix`] and
    /// [`AuthMatrix::known_actors`]) fails.
    ///
    /// Known addresses are de-duplicated, so an address listed more than once
    /// — on one entry point or across several — is still checked (and counted)
    /// exactly once per entry point.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::AssertionFailed`] naming every
    /// violating (entry point, caller) cell, followed by the full grid
    /// (every cell's expected vs. actual outcome) so a passing cell isn't
    /// mistaken for a gap in coverage.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::auth::AuthMatrix;
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let owner = env.address();
    ///
    /// // A single-entry-point matrix where the only allowed caller is
    /// // itself: trivially enforced.
    /// AuthMatrix::new(&env)
    ///     .entry_point("noop", &[owner.clone()], move |caller| {
    ///         if *caller == owner {
    ///             Ok(())
    ///         } else {
    ///             Err(soroban_sdk::Error::from_contract_error(1))
    ///         }
    ///     })
    ///     .assert_enforced();
    /// ```
    pub fn assert_enforced(self) {
        let known = self.known_actors();

        let mut violations = Vec::new();
        let mut grid = format!(
            "known actors = union of all entry points' allowed lists ({} address(es)); \
             each is checked against every entry point\n",
            known.len()
        );

        for ep in &self.entry_points {
            grid.push_str(&format!("  [{}]\n", ep.name));
            for addr in &known {
                let expected = ep.allowed.contains(addr);
                // Reset auth state before invoking this actor case so that
                // previously mocked or recorded auths do not leak into it.
                self.reset_auth();
                let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (ep.invoke)(addr).is_ok()
                }))
                .unwrap_or(false);
                // Reset auth state immediately after invocation to keep the
                // slate clean for the next actor case.
                self.reset_auth();

                grid.push_str(&format!(
                    "    {addr:?}: expected={} actual={}{}\n",
                    if expected { "pass" } else { "reject" },
                    if actual { "pass" } else { "reject" },
                    if expected == actual {
                        ""
                    } else {
                        "  <-- MISMATCH"
                    },
                ));
                if expected != actual {
                    violations.push(format!(
                        "entry point {:?}: caller {addr:?} expected to {} but {}",
                        ep.name,
                        if expected { "succeed" } else { "be rejected" },
                        if actual { "succeeded" } else { "was rejected" },
                    ));
                }
            }
        }

        if !violations.is_empty() {
            panic!(
                "{}",
                TestkitError::AssertionFailed(format!(
                    "auth matrix violated ({} of {} cells):\n{}\nfull grid:\n{grid}",
                    violations.len(),
                    self.entry_points.len() * known.len(),
                    violations.join("\n"),
                ))
            );
        }
    }
}

impl TestEnv {
    /// Reset any active authorizations or mocked auths in this environment.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// env.reset_auth();
    /// ```
    pub fn reset_auth(&self) {
        self.env().set_auths(&[]);
    }

    /// Assert that `f` fails when it runs without the authorization it
    /// requires — the single-call counterpart to [`AuthMatrix`], for a
    /// caller that just needs "this must reject an unauthorized attempt"
    /// without a full grid.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::AssertionFailed`] if `f` completes
    /// (returns) instead of failing.
    ///
    /// # Example
    ///
    /// ```should_panic
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// // A closure that succeeds unconditionally can never satisfy this.
    /// env.assert_requires_auth(|| {});
    /// ```
    pub fn assert_requires_auth(&self, f: impl FnOnce()) {
        self.reset_auth();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        self.reset_auth();
        if result.is_ok() {
            panic!(
                "{}",
                TestkitError::AssertionFailed(
                    "expected the call to fail without authorization, but it succeeded".into()
                )
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    use vault::{VaultClient, VaultError};

    fn flatten<T>(
        result: Result<
            Result<T, soroban_sdk::ConversionError>,
            Result<VaultError, soroban_sdk::InvokeError>,
        >,
    ) -> Result<(), soroban_sdk::Error> {
        match result {
            Ok(Ok(_)) => Ok(()),
            _ => Err(soroban_sdk::Error::from_contract_error(0)),
        }
    }

    fn panic_message(err: Box<dyn std::any::Any + Send>) -> String {
        err.downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
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

        // `bob` is named on both entry points but appears once; order follows
        // first appearance.
        assert_eq!(matrix.known_actors(), vec![alice, bob, carol]);
    }

    #[test]
    fn known_actors_ignores_addresses_never_listed() {
        let env = TestEnv::new();
        let listed = env.address();
        let never_listed = env.address();

        let matrix =
            AuthMatrix::new(&env).entry_point("only", std::slice::from_ref(&listed), |_| Ok(()));

        let known = matrix.known_actors();
        assert_eq!(known, vec![listed]);
        assert!(
            !known.contains(&never_listed),
            "an address never listed as allowed is not a known actor"
        );
    }

    #[test]
    fn assert_enforced_report_states_the_known_actor_definition() {
        let env = TestEnv::new();
        let owner = env.address();
        let attacker = env.address();

        // `open` allows both callers (making both "known"), while `guarded`
        // is buggy: it allows everyone. `attacker` is known from `open` and so
        // is cross-checked against `guarded`, where it wrongly succeeds.
        let matrix = AuthMatrix::new(&env)
            .entry_point("open", &[owner.clone(), attacker.clone()], |_| Ok(()))
            .entry_point("guarded", std::slice::from_ref(&owner), |_| Ok(()));

        let err =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| matrix.assert_enforced()))
                .expect_err("guarded allows everyone, so the attacker cell must be caught");
        let message = panic_message(err);

        assert!(
            message.contains("known actors = union of all entry points' allowed lists"),
            "the report must state the definition of a known actor: {message}"
        );
        assert!(
            message.contains(&format!("{attacker:?}")),
            "the report must name the offending known actor: {message}"
        );
    }

    #[test]
    fn assert_enforced_catches_a_real_missing_auth_bug() {
        let env = TestEnv::new();
        let owner = env.address();
        let attacker = env.address();
        let vault_id = env.env().register(vault::Vault, ());
        VaultClient::new(env.env(), &vault_id).initialize(&owner);
        VaultClient::new(env.env(), &vault_id).deposit(&100);

        // `deposit` is legitimately open to anyone, which is what puts
        // `attacker` into the matrix's "known addresses" set. `withdraw`
        // (the buggy entry point) never calls owner.require_auth(), so
        // when we check that only `owner` may withdraw, `attacker` — a
        // known address the matrix now cross-checks against every entry
        // point — incorrectly succeeds too. That's exactly the bug.
        let matrix = AuthMatrix::new(&env)
            .entry_point(
                "deposit",
                &[owner.clone(), attacker.clone()],
                |caller: &Address| {
                    let result = VaultClient::new(env.env(), &vault_id)
                        .mock_auths(&[MockAuth {
                            address: caller,
                            invoke: &MockAuthInvoke {
                                contract: &vault_id,
                                fn_name: "deposit",
                                args: (1_i128,).into_val(env.env()),
                                sub_invokes: &[],
                            },
                        }])
                        .try_deposit(&1);
                    flatten(result)
                },
            )
            .entry_point(
                "withdraw",
                std::slice::from_ref(&owner),
                |caller: &Address| {
                    let result = VaultClient::new(env.env(), &vault_id)
                        .mock_auths(&[MockAuth {
                            address: caller,
                            invoke: &MockAuthInvoke {
                                contract: &vault_id,
                                fn_name: "withdraw",
                                args: (10_i128,).into_val(env.env()),
                                sub_invokes: &[],
                            },
                        }])
                        .try_withdraw(&10);
                    flatten(result)
                },
            );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            matrix.assert_enforced();
        }));

        let err = result.expect_err("expected assert_enforced to catch the missing auth check");
        let message = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            message.contains("withdraw"),
            "failure message should name the entry point: {message}"
        );
        assert!(
            message.contains(&format!("{attacker:?}")),
            "failure message should name the offending caller: {message}"
        );
    }

    #[test]
    fn assert_enforced_passes_once_the_bug_is_fixed() {
        let env = TestEnv::new();
        let owner = env.address();
        let other = env.address();
        let vault_id = env.env().register(vault::Vault, ());
        VaultClient::new(env.env(), &vault_id).initialize(&owner);
        VaultClient::new(env.env(), &vault_id).deposit(&100);

        // Same shape as the buggy-case test (two entry points, so `other`
        // is a known address cross-checked against withdraw_checked), but
        // against the fixed entry point: `other` is correctly rejected.
        AuthMatrix::new(&env)
            .entry_point(
                "deposit",
                &[owner.clone(), other.clone()],
                |caller: &Address| {
                    let result = VaultClient::new(env.env(), &vault_id)
                        .mock_auths(&[MockAuth {
                            address: caller,
                            invoke: &MockAuthInvoke {
                                contract: &vault_id,
                                fn_name: "deposit",
                                args: (1_i128,).into_val(env.env()),
                                sub_invokes: &[],
                            },
                        }])
                        .try_deposit(&1);
                    flatten(result)
                },
            )
            .entry_point(
                "withdraw_checked",
                std::slice::from_ref(&owner),
                |caller: &Address| {
                    let result = VaultClient::new(env.env(), &vault_id)
                        .mock_auths(&[MockAuth {
                            address: caller,
                            invoke: &MockAuthInvoke {
                                contract: &vault_id,
                                fn_name: "withdraw_checked",
                                args: (10_i128,).into_val(env.env()),
                                sub_invokes: &[],
                            },
                        }])
                        .try_withdraw_checked(&10);
                    flatten(result)
                },
            )
            .assert_enforced();
    }

    #[test]
    fn a_duplicated_allowed_address_is_checked_once_not_twice() {
        let env = TestEnv::new();
        let alice = env.address();

        // The same actor listed twice in one `allowed` list must still count
        // as a single known address: the report should cover one cell, not two.
        let matrix =
            AuthMatrix::new(&env).entry_point("dup", &[alice.clone(), alice.clone()], |_| {
                Err(soroban_sdk::Error::from_contract_error(1))
            });

        let err =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| matrix.assert_enforced()))
                .expect_err("alice is expected to pass but is rejected");
        let message = panic_message(err);

        assert!(
            message.contains("of 1 cells"),
            "a duplicated actor must be counted once: {message}"
        );
    }

    #[test]
    fn duplicate_entry_point_names_produce_independent_sections() {
        let env = TestEnv::new();
        let alice = env.address();

        // Deliberately reusing a name: both sections are checked separately,
        // one violating and one passing, and both appear in the report.
        let matrix = AuthMatrix::new(&env)
            .entry_point("deposit", std::slice::from_ref(&alice), |_| {
                Err(soroban_sdk::Error::from_contract_error(1))
            })
            .entry_point("deposit", std::slice::from_ref(&alice), |_| Ok(()));

        let err =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| matrix.assert_enforced()))
                .expect_err("the first `deposit` rejects alice and must be flagged");
        let message = panic_message(err);

        assert_eq!(
            message.matches("[deposit]").count(),
            2,
            "two entry points sharing a name must both be reported: {message}"
        );
        assert!(
            message.contains("of 2 cells"),
            "each same-named entry point still gets its own cells: {message}"
        );
    }

    #[test]
    fn assert_requires_auth_passes_when_call_panics() {
        let env = TestEnv::new();
        env.assert_requires_auth(|| panic!("simulated auth failure"));
    }

    #[test]
    #[should_panic(expected = "expected the call to fail without authorization")]
    fn assert_requires_auth_panics_when_call_succeeds() {
        let env = TestEnv::new();
        env.assert_requires_auth(|| {});
    }
}
