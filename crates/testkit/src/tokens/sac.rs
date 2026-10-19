use std::cell::Cell;

use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env, IntoVal};

use crate::core::{TestEnv, TestkitError};

/// The decimal precision every Stellar Asset Contract uses. This is fixed
/// by the protocol (the classic Stellar asset precision), not something a
/// deployer can choose — verified against soroban-sdk 27.0.6:
/// `register_stellar_asset_contract_v2` takes no decimals parameter, and a
/// freshly deployed SAC's own `decimals()` always returns 7.
const SAC_DECIMALS: u32 = 7;

/// A deployed Stellar Asset Contract test double, with one-line mint/balance
/// helpers so tests don't hand-roll SAC setup boilerplate.
///
/// # Administrative authorization
///
/// The double deploys the SAC with a fresh test address as its admin and
/// mocks that admin's authorization for [`TestToken::mint`] /
/// [`TestToken::mint_raw`] only — minting therefore works with no global auth
/// mocking enabled. Every other operation (transfers, burns, approvals) still
/// needs the test to authorize the caller with `mock_all_auths` or a per-call
/// `mock_auths`; see [`crate::tokens`] for the same rule stated module-wide.
pub struct TestToken {
    env: Env,
    address: Address,
    admin: Address,
    total_tracked: Cell<i128>,
}

impl TestToken {
    fn new(env: &Env, admin: Address) -> Self {
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        Self {
            env: env.clone(),
            address: sac.address(),
            admin,
            total_tracked: Cell::new(0),
        }
    }

    /// The token contract's address.
    pub fn address(&self) -> Address {
        self.address.clone()
    }

    /// The token's decimal precision. Always `7`, fixed by the protocol.
    pub fn decimals(&self) -> u32 {
        SAC_DECIMALS
    }

    /// Mint `amount` **whole units** to `to` (converted internally to base
    /// units, i.e. `amount * 10^decimals`).
    ///
    /// The Stellar Asset Contract admin's authorization is mocked for this
    /// call, so it succeeds with no global auth mocking enabled.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::Misuse`] if the conversion to base
    /// units would overflow `i128`; use [`TestToken::mint_raw`] for amounts
    /// already in base units.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    ///
    /// token.mint(&alice, 100);
    /// token.assert_balance(&alice, 1_000_000_000); // 100 * 10^7
    /// ```
    /// Convert `amount` whole units into base units (stroops) at this token's decimal precision.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// assert_eq!(token.to_base_units(5).unwrap(), 50_000_000);
    /// ```
    pub fn to_base_units(&self, whole: i128) -> Result<i128, TestkitError> {
        whole_to_base_units(whole, self.decimals())
    }

    /// Convert `base` units into whole units at this token's decimal precision.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// assert_eq!(token.to_whole_units(50_000_000).unwrap(), 5);
    /// ```
    pub fn to_whole_units(&self, base: i128) -> Result<i128, TestkitError> {
        base_to_whole_units(base, self.decimals())
    }

    /// Mint `amount` **whole units** to `to` (converted internally to base
    /// units, i.e. `amount * 10^decimals`).
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::Misuse`] if the conversion to base
    /// units would overflow `i128`, or if the contract invocation fails or traps.
    /// Use [`TestToken::mint_raw`] for amounts already in base units, or
    /// [`TestToken::try_mint`] for fallible invocations.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    ///
    /// token.mint(&alice, 100);
    /// token.assert_balance(&alice, 1_000_000_000); // 100 * 10^7
    /// ```
    pub fn mint(&self, to: &Address, amount: i128) {
        let raw = self.to_base_units(amount).unwrap_or_else(|err| {
            panic!(
                "{}",
                TestkitError::Misuse(format!(
                    "mint({amount}) overflows i128 once converted to base units at {} \
                     decimals; use mint_raw for amounts already in base units: {err}",
                    self.decimals()
                ))
            )
        });
        self.mint_raw(to, raw);
    }

    /// Attempt to mint `amount` **whole units** to `to`, returning
    /// `Err(TestkitError)` if conversion overflows or if the mint traps.
    ///
    /// If the mint traps, `total_tracked` remains unchanged and consistent.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    /// assert!(token.try_mint(&alice, 10).is_ok());
    /// ```
    pub fn try_mint(&self, to: &Address, amount: i128) -> Result<(), TestkitError> {
        let raw = self.to_base_units(amount)?;
        self.try_mint_raw(to, raw)
    }

    /// Mint `amount` **base units** (stroops) to `to`, with no unit
    /// conversion.
    ///
    /// The Stellar Asset Contract admin's authorization is mocked for this
    /// call, so it succeeds with no global auth mocking enabled.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError`] if the mint traps or fails.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    ///
    /// token.mint_raw(&alice, 42);
    /// token.assert_balance(&alice, 42);
    /// ```
    pub fn mint_raw(&self, to: &Address, amount: i128) {
        if let Err(err) = self.try_mint_raw(to, amount) {
            panic!("{err}");
        }
    }

    /// Attempt to mint `amount` **base units** (stroops) to `to`.
    ///
    /// If the contract invocation traps or rejects the mint, `total_tracked`
    /// remains consistent (unchanged) and a [`TestkitError`] is returned with
    /// actionable context.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    /// assert!(token.try_mint_raw(&alice, 42).is_ok());
    /// ```
    pub fn try_mint_raw(&self, to: &Address, amount: i128) -> Result<(), TestkitError> {
        if amount < 0 {
            return Err(TestkitError::Misuse(format!(
                "cannot mint negative amount {amount} to {to:?}"
            )));
        }

        let new_total = self
            .total_tracked
            .get()
            .checked_add(amount)
            .ok_or_else(|| {
                TestkitError::Misuse(format!(
                    "minting {amount} base units would overflow tracked supply (current total: {})",
                    self.total_tracked.get()
                ))
            })?;

        let invoke_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            StellarAssetClient::new(&self.env, &self.address)
                .mock_auths(&[MockAuth {
                    address: &self.admin,
                    invoke: &MockAuthInvoke {
                        contract: &self.address,
                        fn_name: "mint",
                        args: (to.clone(), amount).into_val(&self.env),
                        sub_invokes: &[],
                    },
                }])
                .try_mint(to, &amount)
        }));

        match invoke_res {
            Ok(Ok(Ok(()))) => {
                self.total_tracked.set(new_total);
                Ok(())
            }
            Ok(Err(err)) => Err(TestkitError::AssertionFailed(format!(
                "mint of {amount} base units to {to:?} failed or trapped: {err:?}"
            ))),
            Ok(Ok(Err(err))) => Err(TestkitError::DecodeFailed(format!(
                "mint response decoding failed: {err:?}"
            ))),
            Err(_) => Err(TestkitError::AssertionFailed(format!(
                "mint of {amount} base units to {to:?} trapped with panic"
            ))),
        }
    }

    /// The balance of `of`, in **base units**.
    pub fn balance(&self, of: &Address) -> i128 {
        TokenClient::new(&self.env, &self.address).balance(of)
    }

    /// Assert that `of`'s balance equals `expected`, in **base units**.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::AssertionFailed`] showing the expected
    /// and actual balances if they differ.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    ///
    /// token.mint_raw(&alice, 10);
    /// token.assert_balance(&alice, 10);
    /// ```
    pub fn assert_balance(&self, of: &Address, expected: i128) {
        let actual = self.balance(of);
        if actual != expected {
            panic!(
                "{}",
                TestkitError::AssertionFailed(format!(
                    "expected balance of {of:?} to be {expected}, found {actual}"
                ))
            );
        }
    }

    /// The whole-unit balance of `of` (truncated).
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    /// token.mint(&alice, 10);
    /// assert_eq!(token.balance_whole(&alice), 10);
    /// ```
    pub fn balance_whole(&self, of: &Address) -> i128 {
        let raw = self.balance(of);
        self.to_whole_units(raw).unwrap_or_else(|err| {
            panic!("{err}");
        })
    }

    /// Assert that `of`'s whole-unit balance equals `expected_whole`.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::Misuse`] if scaling `expected_whole`
    /// to base units overflows `i128`.
    /// Panics with a [`TestkitError::AssertionFailed`] if balances differ.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// let alice = env.address();
    /// token.mint(&alice, 10);
    /// token.assert_balance_whole(&alice, 10);
    /// ```
    pub fn assert_balance_whole(&self, of: &Address, expected_whole: i128) {
        let raw_expected = self.to_base_units(expected_whole).unwrap_or_else(|err| {
            panic!("{err}");
        });
        self.assert_balance(of, raw_expected);
    }

    /// Total **base units** minted through this `TestToken` (via
    /// [`TestToken::mint`] or [`TestToken::mint_raw`]) across every
    /// recipient — for supply-conservation assertions.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// token.mint_raw(&env.address(), 10);
    /// token.mint_raw(&env.address(), 5);
    /// assert_eq!(token.total_tracked(), 15);
    /// ```
    pub fn total_tracked(&self) -> i128 {
        self.total_tracked.get()
    }
}

/// Convert a whole-unit amount into base units at the given decimal precision without overflowing.
///
/// Returns `Err(TestkitError::Misuse)` with actionable context if `10^decimals` or the
/// multiplication overflows `i128`.
///
/// # Example
///
/// ```
/// use soroban_testkit::tokens::whole_to_base_units;
///
/// let raw = whole_to_base_units(10, 7).unwrap();
/// assert_eq!(raw, 100_000_000);
/// ```
pub fn whole_to_base_units(whole: i128, decimals: u32) -> Result<i128, TestkitError> {
    let scale = 10i128.checked_pow(decimals).ok_or_else(|| {
        TestkitError::Misuse(format!(
            "decimal precision {decimals} is too large: 10^{decimals} overflows i128"
        ))
    })?;
    whole.checked_mul(scale).ok_or_else(|| {
        TestkitError::Misuse(format!(
            "converting {whole} whole units at {decimals} decimals overflows i128 (scale factor: {scale})"
        ))
    })
}

/// Convert a base-unit amount into whole units at the given decimal precision.
///
/// Returns `Err(TestkitError::Misuse)` with actionable context if `10^decimals` overflows `i128`.
///
/// # Example
///
/// ```
/// use soroban_testkit::tokens::base_to_whole_units;
///
/// let whole = base_to_whole_units(100_000_000, 7).unwrap();
/// assert_eq!(whole, 10);
/// ```
pub fn base_to_whole_units(base: i128, decimals: u32) -> Result<i128, TestkitError> {
    let scale = 10i128.checked_pow(decimals).ok_or_else(|| {
        TestkitError::Misuse(format!(
            "decimal precision {decimals} is too large: 10^{decimals} overflows i128"
        ))
    })?;
    Ok(base / scale)
}

impl TestEnv {
    /// Deploy a Stellar Asset Contract with the standard 7 decimals, using a
    /// fresh test address as its admin.
    ///
    /// The admin's authorization is mocked for minting, so [`TestToken::mint`]
    /// / [`TestToken::mint_raw`] need no global auth mocking; other operations
    /// still do — see [`crate::tokens`].
    ///
    /// The token admin is an implementation detail of the double and does **not**
    /// advance the `TestEnv` address stream. Calling `token()` before, after, or
    /// between [`TestEnv::address`] / [`TestEnv::addresses`] calls therefore has
    /// no effect on subsequent address generation: the seeded sequence remains
    /// stable regardless of call order.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token();
    /// assert_eq!(token.decimals(), 7);
    /// ```
    pub fn token(&self) -> TestToken {
        // Use the raw SDK generator for the admin address so that calling
        // token() does not advance the TestEnv's own address counter.  The
        // admin is an internal implementation detail of the double; tests
        // should never need to refer to it.  Keeping it off the TestEnv
        // stream means the seeded address sequence stays stable regardless
        // of how many tokens are created and in what order.
        use soroban_sdk::testutils::Address as _;
        let admin = Address::generate(self.env());
        TestToken::new(self.env(), admin)
    }

    /// Deploy a Stellar Asset Contract, asserting it uses `decimals`.
    ///
    /// # Panics
    ///
    /// Panics with a [`TestkitError::Misuse`] if `decimals != 7`. Stellar
    /// Asset Contracts fix their decimal precision at 7 (the classic
    /// Stellar asset precision) — verified against soroban-sdk 27.0.6,
    /// where `register_stellar_asset_contract_v2` takes no decimals
    /// parameter. This method exists so a test that expects a specific
    /// precision fails loudly if that ever changes, rather than silently
    /// assuming it.
    ///
    /// # Example
    ///
    /// ```
    /// use soroban_testkit::core::TestEnv;
    ///
    /// let env = TestEnv::new();
    /// let token = env.token_with_decimals(7);
    /// assert_eq!(token.decimals(), 7);
    /// ```
    pub fn token_with_decimals(&self, decimals: u32) -> TestToken {
        if decimals != SAC_DECIMALS {
            panic!(
                "{}",
                TestkitError::Misuse(format!(
                    "token_with_decimals({decimals}): Stellar Asset Contracts fix decimals at \
                     {SAC_DECIMALS} and cannot be deployed with a different precision; use \
                     token() for the standard {SAC_DECIMALS}-decimal token"
                ))
            );
        }
        self.token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl};

    #[test]
    fn token_is_immediately_usable_for_transfers() {
        let env = TestEnv::new();
        let token = env.token();
        let alice = env.address();
        let bob = env.address();
        token.mint_raw(&alice, 100);

        TokenClient::new(env.env(), &token.address())
            .mock_auths(&[MockAuth {
                address: &alice,
                invoke: &MockAuthInvoke {
                    contract: &token.address(),
                    fn_name: "transfer",
                    args: (alice.clone(), bob.clone(), 30_i128).into_val(env.env()),
                    sub_invokes: &[],
                },
            }])
            .transfer(&alice, soroban_sdk::MuxedAddress::from(bob.clone()), &30);

        token.assert_balance(&alice, 70);
        token.assert_balance(&bob, 30);
    }

    #[test]
    fn mint_works_with_no_global_auth_mocking() {
        let env = TestEnv::new();
        let token = env.token();
        let alice = env.address();

        // No mock_all_auths/mock_auths anywhere: the double mocks the SAC
        // admin's authorization for mint itself.
        token.mint(&alice, 2);
        token.mint_raw(&alice, 42);

        token.assert_balance(&alice, 2 * 10i128.pow(7) + 42);
    }

    #[test]
    fn transfers_still_require_authorization() {
        let env = TestEnv::new();
        let token = env.token();
        let alice = env.address();
        let bob = env.address();
        token.mint_raw(&alice, 100);

        // Only minting is mocked; an un-authorized transfer must still fail.
        env.assert_requires_auth(|| {
            TokenClient::new(env.env(), &token.address()).transfer(
                &alice,
                soroban_sdk::MuxedAddress::from(bob.clone()),
                &10,
            );
        });
    }

    #[test]
    fn mint_converts_whole_units_at_seven_decimals() {
        let env = TestEnv::new();
        let token = env.token();
        let alice = env.address();
        token.mint(&alice, 100);
        token.assert_balance(&alice, 1_000_000_000);
    }

    #[test]
    fn mint_and_mint_raw_disagree_by_exactly_ten_pow_decimals() {
        let env = TestEnv::new();
        let token = env.token();
        let a = env.address();
        let b = env.address();

        token.mint(&a, 1);
        token.mint_raw(&b, 1);

        assert_eq!(token.balance(&a), token.balance(&b) * 10i128.pow(7));
    }

    #[test]
    #[should_panic(expected = "token_with_decimals(6)")]
    fn token_with_decimals_rejects_non_seven() {
        let env = TestEnv::new();
        let _ = env.token_with_decimals(6);
    }

    #[test]
    fn total_tracked_sums_every_mint() {
        let env = TestEnv::new();
        let token = env.token();
        token.mint_raw(&env.address(), 10);
        token.mint(&env.address(), 1);
        assert_eq!(token.total_tracked(), 10 + 10i128.pow(7));
    }

    #[contract]
    struct Wallet;

    #[contractimpl]
    impl Wallet {
        pub fn send(env: Env, token: Address, from: Address, to: Address, amount: i128) {
            TokenClient::new(&env, &token).transfer(
                &from,
                soroban_sdk::MuxedAddress::from(to),
                &amount,
            );
        }

        pub fn spend(env: Env, token: Address, from: Address, amount: i128) {
            TokenClient::new(&env, &token).burn(&from, &amount);
        }

        pub fn allow(
            env: Env,
            token: Address,
            from: Address,
            spender: Address,
            amount: i128,
            live_until: u32,
        ) {
            TokenClient::new(&env, &token).approve(&from, &spender, &amount, &live_until);
        }
    }

    #[test]
    fn works_with_a_contract_that_transfers_burns_and_approves() {
        let env = TestEnv::new();
        let token = env.token();
        let alice = env.address();
        let bob = env.address();
        let spender = env.address();
        token.mint_raw(&alice, 1_000);

        env.env().mock_all_auths_allowing_non_root_auth();
        let wallet_id = env.env().register(Wallet, ());
        let wallet = WalletClient::new(env.env(), &wallet_id);

        wallet.send(&token.address(), &alice, &bob, &300);
        token.assert_balance(&alice, 700);
        token.assert_balance(&bob, 300);

        wallet.spend(&token.address(), &alice, &200);
        token.assert_balance(&alice, 500);

        wallet.allow(&token.address(), &alice, &spender, &50, &1_000);
        let allowance = TokenClient::new(env.env(), &token.address()).allowance(&alice, &spender);
        assert_eq!(allowance, 50);
    }

    // ── Bug #462 regression: token() must not advance the TestEnv address stream ──

    #[test]
    fn token_does_not_consume_a_slot_in_the_address_stream() {
        // Without the fix, creating a token before generating addresses would
        // shift every subsequent address by one slot.
        let with_token_first = TestEnv::with_seed(42);
        let without_token = TestEnv::with_seed(42);

        let _token = with_token_first.token();
        let a1 = with_token_first.address();

        let a2 = without_token.address();

        assert_eq!(
            a1, a2,
            "token() shifted the address stream; the admin must not draw from TestEnv::address()"
        );
    }

    #[test]
    fn address_sequence_is_stable_regardless_of_token_call_position() {
        // token() called before, between, and after address() calls must
        // produce the same sequence a plain env with no token calls would.
        let env = TestEnv::with_seed(99);
        let reference = TestEnv::with_seed(99);

        let _t0 = env.token(); // token before first address
        let a1 = env.address();
        let _t1 = env.token(); // token between addresses
        let a2 = env.address();
        let _t2 = env.token(); // token after second address

        let r1 = reference.address();
        let r2 = reference.address();

        assert_eq!(a1, r1, "first address shifted by leading token() call");
        assert_eq!(a2, r2, "second address shifted by interleaved token() call");
    }

    #[test]
    fn multiple_tokens_do_not_shift_seeded_address_stream() {
        let env = TestEnv::with_seed(7);
        let reference = TestEnv::with_seed(7);

        // Create several tokens then generate a batch.
        let _t1 = env.token();
        let _t2 = env.token();
        let _t3 = env.token();
        let batch = env.addresses(4);

        let ref_batch = reference.addresses(4);

        assert_eq!(batch, ref_batch, "batch shifted by preceding token() calls");
    }
}
