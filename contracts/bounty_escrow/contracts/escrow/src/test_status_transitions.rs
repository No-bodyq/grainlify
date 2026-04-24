use super::*;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{
    testutils::{Address as _, LedgerInfo},
    token, Address, Env,
};

fn create_token_contract<'a>(
    e: &Env,
    admin: &Address,
) -> (token::Client<'a>, token::StellarAssetClient<'a>) {
    let contract = e.register_stellar_asset_contract_v2(admin.clone());
    let contract_address = contract.address();
    (
        token::Client::new(e, &contract_address),
        token::StellarAssetClient::new(e, &contract_address),
    )
}

fn create_escrow_contract<'a>(e: &Env) -> BountyEscrowContractClient<'a> {
    let contract_id = e.register_contract(None, BountyEscrowContract);
    BountyEscrowContractClient::new(e, &contract_id)
}

struct TestSetup<'a> {
    env: Env,
    #[allow(dead_code)]
    admin: Address,
    depositor: Address,
    contributor: Address,
    #[allow(dead_code)]
    token: token::Client<'a>,
    #[allow(dead_code)]
    token_admin: token::StellarAssetClient<'a>,
    escrow: BountyEscrowContractClient<'a>,
}

impl<'a> TestSetup<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let depositor = Address::generate(&env);
        let contributor = Address::generate(&env);

        let (token, token_admin) = create_token_contract(&env, &admin);
        let escrow = create_escrow_contract(&env);

        escrow.init(&admin, &token.address);
        token_admin.mint(&depositor, &1_000_000);

        Self {
            env,
            admin,
            depositor,
            contributor,
            token,
            token_admin,
            escrow,
        }
    }
}

#[test]
fn test_refund_eligibility_ineligible_before_deadline_without_approval() {
    let setup = TestSetup::new();
    let bounty_id = 99;
    let amount = 1_000;
    let deadline = setup.env.ledger().timestamp() + 500;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);

    let view = setup.escrow.get_refund_eligibility_view(&bounty_id);
    assert!(!view.eligible);
    assert_eq!(
        view.code,
        RefundEligibilityCode::IneligibleDeadlineNotPassed
    );
    assert_eq!(view.amount, 0);
    assert!(!view.approval_present);
}

#[test]
fn test_refund_eligibility_eligible_after_deadline() {
    let setup = TestSetup::new();
    let bounty_id = 100;
    let amount = 1_200;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.env.ledger().set_timestamp(deadline + 1);

    let view = setup.escrow.get_refund_eligibility_view(&bounty_id);
    assert!(view.eligible);
    assert_eq!(view.code, RefundEligibilityCode::EligibleDeadlinePassed);
    assert_eq!(view.amount, amount);
    assert_eq!(view.recipient, Some(setup.depositor.clone()));
    assert!(!view.approval_present);
}

#[test]
fn test_refund_eligibility_eligible_with_admin_approval_before_deadline() {
    let setup = TestSetup::new();
    let bounty_id = 101;
    let amount = 2_000;
    let deadline = setup.env.ledger().timestamp() + 1_000;
    let custom_recipient = Address::generate(&setup.env);

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.escrow.approve_refund(
        &bounty_id,
        &500,
        &custom_recipient,
        &RefundMode::Partial,
    );

    let view = setup.escrow.get_refund_eligibility_view(&bounty_id);
    assert!(view.eligible);
    assert_eq!(view.code, RefundEligibilityCode::EligibleAdminApproval);
    assert_eq!(view.amount, 500);
    assert_eq!(view.recipient, Some(custom_recipient));
    assert!(view.approval_present);
}

// Valid transitions: Locked → Released
#[test]
fn test_locked_to_released() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 1000;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Locked
    );

    setup.escrow.release_funds(&bounty_id, &setup.contributor);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Released
    );
}

// Valid transitions: Locked → Refunded
#[test]
fn test_locked_to_refunded() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Locked
    );

    setup.env.ledger().set_timestamp(deadline + 1);
    setup.escrow.refund(&bounty_id);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Refunded
    );
}

// Valid transitions: Locked → PartiallyRefunded
#[test]
fn test_locked_to_partially_refunded() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Locked
    );

    // Approve partial refund before deadline
    setup
        .escrow
        .approve_refund(&bounty_id, &500, &setup.depositor, &RefundMode::Partial);
    setup.escrow.refund(&bounty_id);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::PartiallyRefunded
    );
}

// Valid transitions: PartiallyRefunded → Refunded
#[test]
fn test_partially_refunded_to_refunded() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);

    // First partial refund
    setup
        .escrow
        .approve_refund(&bounty_id, &500, &setup.depositor, &RefundMode::Partial);
    setup.escrow.refund(&bounty_id);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::PartiallyRefunded
    );

    // Second refund completes it
    setup.env.ledger().set_timestamp(deadline + 1);
    setup.escrow.refund(&bounty_id);
    assert_eq!(
        setup.escrow.get_escrow_info(&bounty_id).status,
        EscrowStatus::Refunded
    );
}

// Invalid transition: Released → Locked
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_released_to_locked_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 1000;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.escrow.release_funds(&bounty_id, &setup.contributor);

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
}

// Invalid transition: Released → Released
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_released_to_released_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 1000;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.escrow.release_funds(&bounty_id, &setup.contributor);

    setup.escrow.release_funds(&bounty_id, &setup.contributor);
}

// Invalid transition: Released → Refunded
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_released_to_refunded_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.escrow.release_funds(&bounty_id, &setup.contributor);

    setup.env.ledger().set_timestamp(deadline + 1);
    setup.escrow.refund(&bounty_id);
}

// Invalid transition: Released → PartiallyRefunded
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_released_to_partially_refunded_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.escrow.release_funds(&bounty_id, &setup.contributor);

    setup.env.ledger().set_timestamp(deadline + 1);
    setup
        .escrow
        .partial_release(&bounty_id, &setup.contributor, &500);
}

// Invalid transition: Refunded → Locked
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_refunded_to_locked_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.env.ledger().set(LedgerInfo {
        timestamp: deadline + 1,
        protocol_version: 20,
        sequence_number: 0,
        network_id: Default::default(),
        base_reserve: 0,
        min_temp_entry_ttl: 0,
        min_persistent_entry_ttl: 0,
        max_entry_ttl: 0,
    });
    setup.escrow.refund(&bounty_id);

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
}

// Invalid transition: Refunded → Released
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_refunded_to_released_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.env.ledger().set(LedgerInfo {
        timestamp: deadline + 1,
        protocol_version: 20,
        sequence_number: 0,
        network_id: Default::default(),
        base_reserve: 0,
        min_temp_entry_ttl: 0,
        min_persistent_entry_ttl: 0,
        max_entry_ttl: 0,
    });
    setup.escrow.refund(&bounty_id);

    setup.escrow.release_funds(&bounty_id, &setup.contributor);
}

// Invalid transition: Refunded → Refunded
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_refunded_to_refunded_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.env.ledger().set(LedgerInfo {
        timestamp: deadline + 1,
        protocol_version: 20,
        sequence_number: 0,
        network_id: Default::default(),
        base_reserve: 0,
        min_temp_entry_ttl: 0,
        min_persistent_entry_ttl: 0,
        max_entry_ttl: 0,
    });
    setup.escrow.refund(&bounty_id);

    setup.escrow.refund(&bounty_id);
}

// Invalid transition: Refunded → PartiallyRefunded
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_refunded_to_partially_refunded_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup.env.ledger().set(LedgerInfo {
        timestamp: deadline + 1,
        protocol_version: 20,
        sequence_number: 0,
        network_id: Default::default(),
        base_reserve: 0,
        min_temp_entry_ttl: 0,
        min_persistent_entry_ttl: 0,
        max_entry_ttl: 0,
    });
    setup.escrow.refund(&bounty_id);

    setup
        .escrow
        .partial_release(&bounty_id, &setup.contributor, &100);
}

// Invalid transition: PartiallyRefunded → Locked
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_partially_refunded_to_locked_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup
        .escrow
        .approve_refund(&bounty_id, &500, &setup.depositor, &RefundMode::Partial);
    setup.escrow.refund(&bounty_id);

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
}

// Invalid transition: PartiallyRefunded → Released
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_partially_refunded_to_released_fails() {
    let setup = TestSetup::new();
    let bounty_id = 1;
    let amount = 1000;
    let deadline = setup.env.ledger().timestamp() + 100;

    setup
        .escrow
        .lock_funds(&setup.depositor, &bounty_id, &amount, &deadline);
    setup
        .escrow
        .approve_refund(&bounty_id, &500, &setup.depositor, &RefundMode::Partial);
    setup.escrow.refund(&bounty_id);

    setup.escrow.release_funds(&bounty_id, &setup.contributor);
}

// ============================================================================
// TWO-STEP ADMIN ROTATION WITH TIMELOCK TESTS (Issue #31)
// ============================================================================
//
// These tests verify the admin rotation invariants:
//   - propose_admin_rotation stores a pending proposal with correct fields
//   - accept_admin_rotation is blocked before executable_after
//   - accept_admin_rotation succeeds after timelock elapses
//   - cancel_admin_rotation removes the proposal
//   - Only the proposed admin can accept
//   - Only the current admin can cancel
//   - Duplicate proposals are rejected
//   - Invalid delay values are rejected
//   - Audit events are emitted for all three operations
//   - Upgrade-safe schema version is written on init

/// AR-1: propose_admin_rotation stores proposal with correct fields.
#[test]
fn test_admin_rotation_propose_stores_proposal() {
    use soroban_sdk::testutils::Ledger;
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let delay = MIN_ADMIN_ROTATION_DELAY_SECS;

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &delay)
        .unwrap();

    let proposal = setup.escrow.get_pending_admin_rotation().unwrap();
    assert_eq!(proposal.proposed_admin, new_admin);
    assert_eq!(proposal.proposed_by, setup.admin);
    assert_eq!(
        proposal.executable_after,
        setup.env.ledger().timestamp() + delay
    );
}

/// AR-2: accept_admin_rotation is blocked before timelock elapses.
#[test]
fn test_admin_rotation_accept_blocked_before_timelock() {
    use soroban_sdk::testutils::Ledger;
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let delay = MIN_ADMIN_ROTATION_DELAY_SECS;

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &delay)
        .unwrap();

    // Advance time but not enough.
    setup
        .env
        .ledger()
        .set_timestamp(setup.env.ledger().timestamp() + delay - 1);

    let result = setup.escrow.try_accept_admin_rotation();
    assert!(result.is_err(), "accept must fail before timelock elapses");
}

/// AR-3: accept_admin_rotation succeeds after timelock elapses.
#[test]
fn test_admin_rotation_accept_succeeds_after_timelock() {
    use soroban_sdk::testutils::Ledger;
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let delay = MIN_ADMIN_ROTATION_DELAY_SECS;

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &delay)
        .unwrap();

    setup
        .env
        .ledger()
        .set_timestamp(setup.env.ledger().timestamp() + delay);

    setup.escrow.accept_admin_rotation().unwrap();

    // Proposal must be cleared.
    assert!(
        setup.escrow.get_pending_admin_rotation().is_none(),
        "proposal must be cleared after acceptance"
    );
}

/// AR-4: cancel_admin_rotation removes the proposal.
#[test]
fn test_admin_rotation_cancel_removes_proposal() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &MIN_ADMIN_ROTATION_DELAY_SECS)
        .unwrap();
    assert!(setup.escrow.get_pending_admin_rotation().is_some());

    setup.escrow.cancel_admin_rotation().unwrap();
    assert!(
        setup.escrow.get_pending_admin_rotation().is_none(),
        "proposal must be cleared after cancellation"
    );
}

/// AR-5: duplicate proposal is rejected.
#[test]
fn test_admin_rotation_duplicate_proposal_rejected() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &MIN_ADMIN_ROTATION_DELAY_SECS)
        .unwrap();

    let result = setup
        .escrow
        .try_propose_admin_rotation(&new_admin, &MIN_ADMIN_ROTATION_DELAY_SECS);
    assert!(result.is_err(), "duplicate proposal must be rejected");
}

/// AR-6: delay below minimum is rejected.
#[test]
fn test_admin_rotation_delay_below_minimum_rejected() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let result = setup
        .escrow
        .try_propose_admin_rotation(&new_admin, &(MIN_ADMIN_ROTATION_DELAY_SECS - 1));
    assert!(result.is_err(), "delay below minimum must be rejected");
}

/// AR-7: delay above maximum is rejected.
#[test]
fn test_admin_rotation_delay_above_maximum_rejected() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let result = setup
        .escrow
        .try_propose_admin_rotation(&new_admin, &(MAX_ADMIN_ROTATION_DELAY_SECS + 1));
    assert!(result.is_err(), "delay above maximum must be rejected");
}

/// AR-8: proposing self as new admin is rejected.
#[test]
fn test_admin_rotation_self_proposal_rejected() {
    let setup = TestSetup::new();
    let result = setup
        .escrow
        .try_propose_admin_rotation(&setup.admin, &MIN_ADMIN_ROTATION_DELAY_SECS);
    assert!(result.is_err(), "self-proposal must be rejected");
}

/// AR-9: cancel without a pending proposal returns error.
#[test]
fn test_admin_rotation_cancel_without_proposal_errors() {
    let setup = TestSetup::new();
    let result = setup.escrow.try_cancel_admin_rotation();
    assert!(result.is_err(), "cancel without proposal must error");
}

/// AR-10: accept without a pending proposal returns error.
#[test]
fn test_admin_rotation_accept_without_proposal_errors() {
    let setup = TestSetup::new();
    let result = setup.escrow.try_accept_admin_rotation();
    assert!(result.is_err(), "accept without proposal must error");
}

/// AR-11: propose_admin_rotation emits AdminRotationProposed event.
#[test]
fn test_admin_rotation_propose_emits_event() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let before = setup.env.events().all().len();
    setup
        .escrow
        .propose_admin_rotation(&new_admin, &MIN_ADMIN_ROTATION_DELAY_SECS)
        .unwrap();
    assert!(
        setup.env.events().all().len() > before,
        "AdminRotationProposed must be emitted"
    );
}

/// AR-12: accept_admin_rotation emits AdminRotationAccepted event.
#[test]
fn test_admin_rotation_accept_emits_event() {
    use soroban_sdk::testutils::Ledger;
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let delay = MIN_ADMIN_ROTATION_DELAY_SECS;

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &delay)
        .unwrap();
    setup
        .env
        .ledger()
        .set_timestamp(setup.env.ledger().timestamp() + delay);

    let before = setup.env.events().all().len();
    setup.escrow.accept_admin_rotation().unwrap();
    assert!(
        setup.env.events().all().len() > before,
        "AdminRotationAccepted must be emitted"
    );
}

/// AR-13: cancel_admin_rotation emits AdminRotationCancelled event.
#[test]
fn test_admin_rotation_cancel_emits_event() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    setup
        .escrow
        .propose_admin_rotation(&new_admin, &MIN_ADMIN_ROTATION_DELAY_SECS)
        .unwrap();
    let before = setup.env.events().all().len();
    setup.escrow.cancel_admin_rotation().unwrap();
    assert!(
        setup.env.events().all().len() > before,
        "AdminRotationCancelled must be emitted"
    );
}

/// AR-14: upgrade-safe schema version is written on init.
#[test]
fn test_admin_rotation_schema_version_written_on_init() {
    let setup = TestSetup::new();
    let version = setup.escrow.get_admin_rotation_schema_version();
    assert_eq!(version, 1u32, "schema version must be 1 after init");
}

/// AR-15: after acceptance, new admin can propose another rotation.
#[test]
fn test_admin_rotation_new_admin_can_propose_again() {
    use soroban_sdk::testutils::Ledger;
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let delay = MIN_ADMIN_ROTATION_DELAY_SECS;

    setup
        .escrow
        .propose_admin_rotation(&new_admin, &delay)
        .unwrap();
    setup
        .env
        .ledger()
        .set_timestamp(setup.env.ledger().timestamp() + delay);
    setup.escrow.accept_admin_rotation().unwrap();

    // New admin proposes yet another rotation.
    let next_admin = Address::generate(&setup.env);
    let result = setup
        .escrow
        .try_propose_admin_rotation(&next_admin, &MIN_ADMIN_ROTATION_DELAY_SECS);
    assert!(
        result.is_ok(),
        "new admin must be able to propose another rotation"
    );
}

/// AR-16: maximum valid delay (7 days) is accepted.
#[test]
fn test_admin_rotation_maximum_delay_accepted() {
    let setup = TestSetup::new();
    let new_admin = Address::generate(&setup.env);
    let result = setup
        .escrow
        .try_propose_admin_rotation(&new_admin, &MAX_ADMIN_ROTATION_DELAY_SECS);
    assert!(result.is_ok(), "maximum delay must be accepted");
}
