//! Test fixtures for the profile and subscription DTOs.
//!
//! Covers full and minimal `AccountProfile` shapes plus the paid and free
//! `Subscription` branches, including the absent provider-configuration flag.

use super::*;

    #[test]
    fn account_profile_parses_full_shape() {
        let json = r#"{
            "user_id": "11111111-1111-1111-1111-111111111111",
            "email": "guus@example.com",
            "email_verified": true,
            "created_at": "2026-01-02T03:04:05Z",
            "frozen_at": null,
            "role": "user",
            "totp_enabled": false,
            "is_impersonation": false,
            "admin_user_id": null
        }"#;
        let p: AccountProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.email, "guus@example.com");
        assert!(p.email_verified);
        assert_eq!(p.created_at, "2026-01-02T03:04:05Z");
        assert_eq!(p.frozen_at, None);
        assert_eq!(p.role.as_deref(), Some("user"));
        // Round-trip back out to the WebView.
        let back = serde_json::to_string(&p).unwrap();
        assert!(back.contains("\"email_verified\":true"));
    }

    #[test]
    fn account_profile_parses_minimal_shape() {
        // An older server build that only emits the four core fields.
        let json = r#"{
            "user_id": "u",
            "email": "a@b.c",
            "email_verified": false,
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let p: AccountProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.role, None);
        assert_eq!(p.totp_enabled, None);
    }

    #[test]
    fn subscription_paid_branch_parses() {
        let json = r#"{
            "plan": "pro",
            "billing_cycle": "yearly",
            "status": "active",
            "seats": 1,
            "region": "falkenstein",
            "is_mock": false,
            "created_at": "2026-01-01T00:00:00Z",
            "current_period_end": "2027-01-01T00:00:00Z",
            "quota_bytes": 2199023255552,
            "used_bytes": 12345,
            "extra_storage_tb": 1,
            "extra_users": 0,
            "billing_state": "past_due",
            "past_due_since": "2026-06-10T00:00:00Z",
            "pending_downgrade_plan": null,
            "downgrade_cooldown_until": null,
            "storage_grace_deadline": null,
            "pause_until": null
        }"#;
        let s: Subscription = serde_json::from_str(json).unwrap();
        assert_eq!(s.plan, "pro");
        assert_eq!(s.current_period_end.as_deref(), Some("2027-01-01T00:00:00Z"));
        assert_eq!(s.quota_bytes, 2199023255552);
        assert_eq!(s.extra_storage_tb, 1);
        // past_due_since is now captured (was silently dropped before).
        assert_eq!(s.past_due_since.as_deref(), Some("2026-06-10T00:00:00Z"));
    }

    #[test]
    fn subscription_free_branch_has_null_dates() {
        let json = r#"{
            "plan": "free",
            "billing_cycle": "monthly",
            "seats": 1,
            "region": "falkenstein",
            "status": "active",
            "is_mock": true,
            "created_at": null,
            "current_period_end": null,
            "quota_bytes": 10737418240,
            "used_bytes": 0,
            "extra_storage_tb": 0,
            "extra_users": 0
        }"#;
        let s: Subscription = serde_json::from_str(json).unwrap();
        assert_eq!(s.plan, "free");
        assert_eq!(s.created_at, None);
        assert_eq!(s.current_period_end, None);
        assert_eq!(s.quota_bytes, 10737418240);
        // Absent from the free-plan branch → defaults to None (no parse error).
        assert_eq!(s.past_due_since, None);
    }

    #[test]
    fn subscription_carries_no_provider_configuration_flag() {
        // Task 2285: the server stopped sending the billing-provider
        // configuration flag on /billing/subscription. The DTO must not keep a
        // slot for it, and must not resurrect one when it serialises back out.
        let json = r#"{
            "plan": "pro",
            "billing_cycle": "yearly",
            "status": "active",
            "seats": 1,
            "region": "falkenstein",
            "is_mock": false,
            "created_at": "2026-01-01T00:00:00Z",
            "current_period_end": "2027-01-01T00:00:00Z",
            "quota_bytes": 2199023255552,
            "used_bytes": 12345
        }"#;
        let s: Subscription = serde_json::from_str(json).unwrap();
        let out = serde_json::to_value(&s).unwrap();
        let obj = out.as_object().expect("subscription object");
        assert!(
            !obj.keys().any(|k| k.ends_with("_configured")),
            "the subscription payload may carry no provider-configuration flag: {out}"
        );
    }

    /// The two impersonation optionals are one value: every wire combination
    /// lands in exactly one variant, and the match has no wildcard so a new
    /// variant must be decided here.
    #[test]
    fn account_access_unions_the_impersonation_fields() {
        fn profile(is_impersonation: &str, admin_user_id: &str) -> AccountProfile {
            let json = format!(
                r#"{{"user_id":"u","email":"e@example.com","email_verified":true,
                    "created_at":"2026-01-02T03:04:05Z",
                    "is_impersonation":{is_impersonation},"admin_user_id":{admin_user_id}}}"#
            );
            serde_json::from_str(&json).unwrap()
        }

        let cases = [
            ("false", "null"),
            ("true", "\"admin-1\""),
            ("null", "\"admin-1\""),
            ("true", "null"),
        ];
        for (flag, admin) in cases {
            let p = profile(flag, admin);
            match p.access() {
                AccountAccess::Own => assert_ne!(flag, "true", "flag {flag} / admin {admin}"),
                AccountAccess::Impersonated { admin_user_id } => {
                    assert_eq!(flag, "true");
                    assert_eq!(admin_user_id, p.admin_user_id);
                }
            }
        }
        assert_eq!(profile("false", "null").access(), AccountAccess::Own);
        assert_eq!(profile("null", "null").access(), AccountAccess::Own);
        assert_eq!(
            profile("true", "\"admin-1\"").access(),
            AccountAccess::Impersonated {
                admin_user_id: Some("admin-1".to_string())
            }
        );
    }
