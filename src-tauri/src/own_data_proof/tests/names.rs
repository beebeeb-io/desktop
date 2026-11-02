//! Names that prove nothing, split out of `own_data_proof.rs`.
//!
//! A sealed name opens only under the master key the tree was sealed with (AES-GCM
//! authenticates it), so a node sealed under another key, a plaintext name, or a missing
//! `name_encrypted` field proves nothing about whose data this is. These tests walk those
//! cases and the tally they produce.

use super::*;

    // ---- names that prove nothing ----

    /// A name that proves nothing either way is set aside: a plaintext legacy name, no name, a file received through a
    /// file request, and (review P2-3) a file a collaborator uploaded into this account's folder (`uploaded_by`, or a
    /// file key wrapped for the owner).
    #[test]
    fn a_name_that_proves_nothing_is_set_aside_not_a_veto() {
        let id = IDS[0];
        let other = sealed_name(THEIRS, id);
        let cases = [
            (with(mine(id), "name_encrypted", serde_json::json!("Photos")), "plaintext"),
            (with(mine(id), "name_encrypted", serde_json::Value::Null), "no name"),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "file_request_id", serde_json::json!("0f000000-0000-4000-8000-000000000001")),
                "a file request upload",
            ),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "uploaded_by", serde_json::json!("user-colleague")),
                "uploaded by a collaborator",
            ),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "owner_wrapped_file_key", serde_json::json!("wrapped")),
                "a file key wrapped for the owner",
            ),
        ];
        for (node, why) in &cases {
            assert_eq!(probe(&MINE, node), Probe::Neutral, "{why}");
        }

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![cases[0].0.clone(), mine(IDS[1]), mine(IDS[2])]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::Adopted(t) if t.own == 2 && t.neutral == 1), "{outcome:?}");
    }

    /// Lead review of round 2, P3-6 (replaces self-review F1): a node that does not say who sealed its name (no
    /// `file_request_id`, `uploaded_by` or `owner_wrapped_file_key` key at all) and whose name does not open under this
    /// account's key is unproven, as one that says nobody else sealed it: a missing key is no evidence of a file
    /// request. An own name still proves it.
    #[test]
    fn a_name_that_does_not_open_is_unproven_when_the_api_does_not_say_who_sealed_it() {
        let without = |mut node: serde_json::Value, fields: &[&str]| {
            for field in fields {
                node.as_object_mut().unwrap().remove(*field);
            }
            node
        };
        let attribution = ["file_request_id", "uploaded_by", "owner_wrapped_file_key"];
        let received = |id: &str| with(mine(id), "name_encrypted", serde_json::json!(sealed_name(THEIRS, id)));
        assert_eq!(probe(&MINE, &without(received(IDS[0]), &attribution)), Probe::Unproven, "the API does not say");
        assert_eq!(
            probe(&MINE, &without(received(IDS[0]), &["uploaded_by", "owner_wrapped_file_key"])),
            Probe::Unproven,
            "the API says only that no file request sealed it"
        );
        assert_eq!(probe(&MINE, &received(IDS[0])), Probe::Unproven, "the API says nobody else sealed it");
        assert_eq!(probe(&MINE, &without(mine(IDS[0]), &attribution)), Probe::Own, "an own name still proves it");

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![
            without(received(IDS[0]), &attribution),
            without(mine(IDS[1]), &attribution),
            without(mine(IDS[2]), &attribution),
        ]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::NotProven(t) if t.own == 2 && t.unproven == 1), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// Self-review F2: a row the person put in the trash on the server while sync was refused is not in the snapshot,
    /// which lists only what is not in the trash. The proof reads the trash too (`listing_of`), so a top-level folder
    /// this account trashed (with a file in it that the trash listing does not name) is its own, and the data is
    /// adopted. Read from the snapshot alone, the same PC is refused. Either listing unread: no verdict.
    #[test]
    fn a_folder_this_account_trashed_on_the_server_is_its_own() {
        let trashed = "0b000000-0000-4000-8000-000000000009";
        let inside = "0b000000-0000-4000-8000-00000000000a";
        let snapshot = || crate::api_client::SyncSnapshot { seq_id: 7, nodes: vec![mine(IDS[0]), mine(IDS[1]), mine(IDS[2])] };
        let pc = || {
            let pc = pc_with(&IDS);
            add_folder(&pc, trashed, None, Shape::Snapshot);
            add_row(&pc, inside, Some(trashed), Shape::Snapshot);
            adopt_unbound(&pc.db, None).unwrap();
            pc
        };

        let outcome = prove(&pc(), &session(), MINE, &Server::new(snapshot().nodes));
        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing == 2), "precondition, the snapshot alone: {outcome:?}");

        let in_trash = vec![with(mine(trashed), "is_trashed", serde_json::json!(true))];
        let Listing::Nodes(nodes) = listing_of(Ok(snapshot()), Ok(in_trash)) else {
            panic!("both listings read: the nodes of both");
        };
        let outcome = prove(&pc(), &session(), MINE, &Server::new(nodes));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.own == 4 && t.missing == 0), "{outcome:?}");

        assert!(matches!(listing_of(Ok(snapshot()), Err(anyhow::anyhow!("offline"))), Listing::Unavailable));
        assert!(matches!(listing_of(Err(anyhow::anyhow!("offline")), Ok(Vec::new())), Listing::Unavailable));
    }

    /// Review P3-1: more rows that prove nothing than rows that prove ownership is not enough.
    #[test]
    fn more_names_that_prove_nothing_than_own_names_is_not_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let plain = |id: &str| with(mine(id), "name_encrypted", serde_json::json!("Photos"));
        let server = Server::new(vec![mine(IDS[0]), plain(IDS[1]), plain(IDS[2])]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::NotProven(t) if t.own == 1 && t.neutral == 2), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// A name this account sealed under the legacy key derivation (the old CLI's binary UUID) is this account's own,
    /// as the engine reads it.
    #[test]
    fn a_name_sealed_under_the_legacy_key_derivation_proves_ownership() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, &sealed_binary_node(MINE, id)), Probe::Own);
        assert_eq!(probe(&MINE, &sealed_binary_node(THEIRS, id)), Probe::Unproven, "another account's, legacy");

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![sealed_binary_node(MINE, IDS[0]), mine(IDS[1]), mine(IDS[2])]);
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3));
    }

    /// Review P3-3 (main): a name sealed the way core seals names for v2 content, under the name subkey
    /// (`beebeeb/name/v1`, task 1908), is this account's own; a plaintext name never proves anything.
    #[test]
    fn a_name_sealed_under_the_name_subkey_proves_ownership_and_a_plaintext_name_never_does() {
        let id = IDS[0];
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(MINE);
        let sealed = beebeeb_core::encrypt::encrypt_name_with_chunk_format(
            &mk,
            id,
            "notes.txt",
            Some("text/plain"),
            beebeeb_core::chunk_stream::ExpectedChunkFormat::V2,
        )
        .unwrap();
        let raw_key = beebeeb_core::kdf::derive_file_key(&mk, id.as_bytes());
        let blob: beebeeb_types::EncryptedBlob = serde_json::from_str(&sealed).unwrap();
        assert!(
            beebeeb_core::encrypt::decrypt_metadata(&raw_key, &blob).is_err(),
            "precondition: sealed under the name subkey, not the raw file key"
        );
        assert!(name_opens(&MINE, id, &sealed), "this account's own");
        assert!(!name_opens(&THEIRS, id, &sealed), "another account's key");
        assert!(!name_opens(&MINE, IDS[1], &sealed), "another file's key");
        assert!(!name_opens(&MINE, id, "notes.txt"), "a plaintext name");
    }
