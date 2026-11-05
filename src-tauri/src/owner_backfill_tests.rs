    use super::*;
    use crate::account_binding::Identity;

    fn profile(user_id: &str, email: &str) -> account_dto::AccountProfile {
        serde_json::from_str(&format!(
            r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
        ))
        .unwrap()
    }

    /// The owner after a backfill from `profile`, starting from `owner` (`None`: no owner recorded), and what it returned.
    fn backfilled(
        owner: Option<Identity>,
        profile: &account_dto::AccountProfile,
    ) -> (Result<bool, String>, Option<Identity>) {
        let dir = tempfile::tempdir().unwrap();
        let db = state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        if let Some(owner) = &owner {
            db.set_owner(owner).unwrap();
        }
        let result = backfill_local_data_owner(&SessionWrite::for_test(), dir.path(), profile);
        (result, db.owner().unwrap())
    }

    #[test]
    fn a_legacy_email_only_owner_gains_the_user_id_and_the_servers_spelling() {
        let sam = profile("u-1", "sam@beebeeb.io");
        let complete = Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
        assert_eq!(
            backfilled(Some(Identity::new(None, Some("sam@beebeeb.io"))), &sam),
            (Ok(true), Some(complete.clone())),
            "the same spelling"
        );
        assert_eq!(
            backfilled(Some(Identity::new(None, Some("Sam@Beebeeb.IO"))), &sam),
            (Ok(true), Some(complete.clone())),
            "another letter case (ASCII)"
        );
        assert_eq!(
            backfilled(Some(Identity::new(Some("u-1"), Some("Sam@Beebeeb.IO"))), &sam),
            (Ok(true), Some(complete.clone())),
            "its own id: the server's spelling"
        );
        assert_eq!(
            backfilled(Some(complete.clone()), &sam),
            (Ok(false), Some(complete)),
            "already complete: nothing written"
        );
    }

    #[test]
    fn an_owner_that_is_not_this_account_is_left_to_the_binding() {
        let sam = profile("u-1", "sam@beebeeb.io");
        for owner in [
            Identity::new(Some("u-2"), Some("sam@beebeeb.io")),
            Identity::new(None, Some("kim@beebeeb.io")),
            Identity::new(None, Some("sám@beebeeb.io")),
            Identity::new(Some("u-2"), None),
        ] {
            assert_eq!(
                backfilled(Some(owner.clone()), &sam),
                (Ok(false), Some(owner.clone())),
                "{owner:?}"
            );
        }
        assert_eq!(backfilled(None, &sam), (Ok(false), None), "no owner: nothing invented");
        assert_eq!(
            backfilled(
                Some(Identity::new(None, Some("sam@beebeeb.io"))),
                &profile(" ", "sam@beebeeb.io")
            )
            .1,
            Some(Identity::new(None, Some("sam@beebeeb.io"))),
            "a profile without an id completes nothing"
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            backfill_local_data_owner(&SessionWrite::for_test(), empty.path(), &sam),
            Ok(false),
            "no database yet"
        );
        assert!(!empty.path().join("state.db").exists(), "and none is created");
    }
