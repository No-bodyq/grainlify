#[cfg(test)]
mod test {
    use crate::{DataKey, GrainlifyContract, GrainlifyContractClient, LIVENESS_SCHEMA_VERSION, STORAGE_SCHEMA_VERSION};
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup_test(env: &Env) -> (GrainlifyContractClient, Address) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, GrainlifyContract);
        let client = GrainlifyContractClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.init_admin(&admin);
        (client, admin)
    }

    #[test]
    fn test_storage_schema_version_constant() {
        assert_eq!(STORAGE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn test_verify_storage_layout_after_init() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);
        assert!(client.verify_storage_layout());
    }

    #[test]
    fn test_all_instance_keys_readable_after_init() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        env.as_contract(&client.address, || {
            assert!(env.storage().instance().has(&DataKey::Admin));
            assert!(env.storage().instance().has(&DataKey::Version));
            assert!(env.storage().instance().has(&DataKey::ReadOnlyMode));
        });
    }

    // =========================================================================
    // Liveness Watchdog Tests (LW-01 … LW-08)
    // =========================================================================

    /// LW-01: liveness_watchdog returns operational=true after init.
    #[test]
    fn test_liveness_watchdog_operational_after_init() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        let status = client.liveness_watchdog();
        assert!(!status.is_paused, "should not be paused after init");
        assert!(!status.is_read_only, "should not be read-only after init");
        assert!(status.is_operational, "should be operational after init");
        assert!(status.admin_set, "admin must be set after init");
        assert!(status.version > 0, "version must be set after init");
    }

    /// LW-02: is_operational is false when read-only mode is enabled.
    #[test]
    fn test_liveness_watchdog_not_operational_when_read_only() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        client.set_read_only_mode(&true);
        let status = client.liveness_watchdog();

        assert!(status.is_read_only, "is_read_only must be true");
        assert!(!status.is_operational, "is_operational must be false when read-only");
    }

    /// LW-03: is_operational is restored when read-only mode is disabled.
    #[test]
    fn test_liveness_watchdog_operational_restored_after_read_only_cleared() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        client.set_read_only_mode(&true);
        client.set_read_only_mode(&false);
        let status = client.liveness_watchdog();

        assert!(!status.is_read_only);
        assert!(status.is_operational);
    }

    /// LW-04: schema_version equals LIVENESS_SCHEMA_VERSION constant after init.
    #[test]
    fn test_liveness_watchdog_schema_version_matches_constant() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        let status = client.liveness_watchdog();
        assert_eq!(status.schema_version, LIVENESS_SCHEMA_VERSION);
        assert_eq!(status.schema_version, 1);
    }

    /// LW-05: get_liveness_schema_version returns 1 after init.
    #[test]
    fn test_get_liveness_schema_version_after_init() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        assert_eq!(client.get_liveness_schema_version(), LIVENESS_SCHEMA_VERSION);
    }

    /// LW-06: LivenessSchemaVersion key is present in instance storage after init.
    #[test]
    fn test_liveness_schema_version_key_written_at_init() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        env.as_contract(&client.address, || {
            assert!(
                env.storage().instance().has(&DataKey::LivenessSchemaVersion),
                "LivenessSchemaVersion must be written at init"
            );
        });
    }

    /// LW-07: timestamp in LivenessStatus matches ledger timestamp.
    #[test]
    fn test_liveness_watchdog_timestamp_matches_ledger() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        let ledger_ts = env.ledger().timestamp();
        let status = client.liveness_watchdog();
        assert_eq!(status.timestamp, ledger_ts);
    }

    /// LW-08: is_operational is the logical AND of !is_paused && !is_read_only.
    #[test]
    fn test_liveness_watchdog_is_operational_invariant() {
        let env = Env::default();
        let (client, _admin) = setup_test(&env);

        // Operational baseline
        let s = client.liveness_watchdog();
        assert_eq!(s.is_operational, !s.is_paused && !s.is_read_only);

        // Read-only mode
        client.set_read_only_mode(&true);
        let s = client.liveness_watchdog();
        assert_eq!(s.is_operational, !s.is_paused && !s.is_read_only);

        // Restore
        client.set_read_only_mode(&false);
        let s = client.liveness_watchdog();
        assert_eq!(s.is_operational, !s.is_paused && !s.is_read_only);
    }
}
