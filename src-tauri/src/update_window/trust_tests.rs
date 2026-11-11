//! Tests of `trust`: every rule with keys generated inside the test (no key, test or real, is committed), and the
//! committed public keys themselves.

use super::flow::strictly_newer;
use super::trust::*;
use base64::Engine as _;

/// Signing for tests, in the two forms Tauri stores: a key pair made here, and signatures with a chosen file name.
pub(crate) mod kit {
    use base64::Engine as _;

    pub(crate) struct TestKey {
        pair: minisign::KeyPair,
    }

    impl TestKey {
        pub(crate) fn generate() -> Self {
            Self {
                pair: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
            }
        }

        /// The public key as `tauri.conf.json` and a Tauri `.pub` file hold it.
        pub(crate) fn tauri_pubkey(&self) -> String {
            let text = self.pair.pk.to_box().unwrap().to_string();
            base64::engine::general_purpose::STANDARD.encode(text)
        }

        /// A Tauri-format signature (base64 of the minisign text) of `bytes` that signs `file` as its name.
        pub(crate) fn sign(&self, bytes: &[u8], file: &str) -> String {
            self.sign_at(bytes, file, &SIGNED_AT.to_string())
        }

        /// The same, with the signed time as the text the trusted comment carries.
        pub(crate) fn sign_at(&self, bytes: &[u8], file: &str, timestamp: &str) -> String {
            let comment = format!("timestamp:{timestamp}\tfile:{file}");
            let signed = minisign::sign(
                Some(&self.pair.pk),
                &self.pair.sk,
                std::io::Cursor::new(bytes),
                Some(&comment),
                Some("signature from tauri secret key"),
            )
            .unwrap();
            base64::engine::general_purpose::STANDARD.encode(signed.to_string())
        }
    }

    /// When the kit's signatures say they were made, and the clock the tests pass as `now`.
    pub(crate) const SIGNED_AT: u64 = 1_800_000_000;

    pub(crate) const URL_090: &str = "https://releases.beebeeb.io/desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe";

    /// A v2 manifest for `version` on `channel` with one Windows entry.
    pub(crate) fn manifest(
        version: &str,
        product: &str,
        channel: &str,
        url: &str,
        signature: &str,
        size: Option<u64>,
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": version, "notes": "", "pub_date": "2026-11-20T10:00:00Z", "product": product, "channel": channel,
            "platforms": { "windows-x86_64-nsis": { "url": url, "signature": signature, "size": size } }
        }))
        .unwrap()
    }
}

use kit::{SIGNED_AT, TestKey, URL_090, manifest};

/// `verify_v2_manifest` at the kit's clock, with no replay mark.
fn verify(keys: &TrustedKeys, channel: &str, bytes: &[u8], signature: &str) -> Result<SignedManifest, Refusal> {
    verify_v2_manifest(keys, channel, bytes, signature, SIGNED_AT, None)
}

fn baked_pubkey() -> String {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    conf["plugins"]["updater"]["pubkey"].as_str().unwrap().to_string()
}

fn two_keys() -> (TestKey, TestKey, TrustedKeys) {
    let (old, next) = (TestKey::generate(), TestKey::generate());
    let keys = TrustedKeys::new(&old.tauri_pubkey(), &next.tauri_pubkey()).unwrap();
    (old, next, keys)
}

#[test]
fn the_committed_next_key_is_a_real_key_distinct_from_the_baked_one() {
    assert!(
        !is_placeholder(NEXT_PUBKEY_FILE),
        "updater-v2.pub still holds the placeholder"
    );
    let keys = TrustedKeys::new(&baked_pubkey(), NEXT_PUBKEY_FILE).expect("both committed keys parse");
    let (legacy, native) = (
        keys.for_line(Line::Legacy).unwrap(),
        keys.for_line(Line::Native).unwrap(),
    );
    assert_eq!(
        legacy.tauri_pubkey,
        baked_pubkey().trim(),
        "the legacy line's key is the baked one"
    );
    assert_ne!(
        native.tauri_pubkey, legacy.tauri_pubkey,
        "the native line has its own key"
    );
    assert_eq!(native.tauri_pubkey, NEXT_PUBKEY_FILE.trim());
    let next = base64::engine::general_purpose::STANDARD
        .decode(NEXT_PUBKEY_FILE.trim())
        .unwrap();
    assert!(
        String::from_utf8(next)
            .unwrap()
            .starts_with("untrusted comment: minisign public key")
    );
}

#[test]
fn the_placeholder_is_no_key_and_a_damaged_or_repeated_key_file_is_an_error() {
    let old = TestKey::generate();
    let only_old = TrustedKeys::new(&old.tauri_pubkey(), "PLACEHOLDER: not set yet\n").unwrap();
    assert!(only_old.for_line(Line::Legacy).is_some());
    assert!(
        only_old.for_line(Line::Native).is_none(),
        "no v2 key: nothing from the native line"
    );
    assert!(
        TrustedKeys::new(&old.tauri_pubkey(), "not a key").is_err(),
        "never a silently missing key"
    );
    assert!(TrustedKeys::new("not a key", &old.tauri_pubkey()).is_err());
    assert!(
        TrustedKeys::new(&old.tauri_pubkey(), &old.tauri_pubkey()).is_err(),
        "one key for both lines is no scope at all"
    );
    // Without a v2 key a v2 manifest is refused, whoever signed it.
    let artifact = old.sign(b"i", "Beebeeb_0.9.0_x64-setup.exe");
    let bytes = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(5));
    assert_eq!(
        verify(&only_old, "stable", &bytes, &old.sign(&bytes, "stable.json")).err(),
        Some(Refusal::KeyScope)
    );
}

#[test]
fn a_signature_must_name_the_file_at_the_address_and_the_version() {
    let key = TestKey::generate();
    let name = "Beebeeb_0.8.26_x64-setup.exe";
    let url = format!("https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.8.26/{name}");
    let signature = key.sign(b"installer", name);
    assert_eq!(signed_file_name(&signature).as_deref(), Some(name));
    let rows: [(&str, &str, &str, bool); 6] = [
        ("0.8.26", &url, &signature, true),
        ("9.9.9", &url, &signature, false),
        ("0.8.2", &url, &signature, false),
        (
            "0.8.26",
            "https://github.com/x/Beebeeb_0.8.27_x64-setup.exe",
            &signature,
            false,
        ),
        ("0.8.26", &url, "PGh0bWw+NDA0PC9odG1sPg==", false),
        ("", &url, &signature, false),
    ];
    for (version, url, signature, want) in rows {
        assert_eq!(artifact_names_version(version, url, signature), want, "{version} {url}");
    }
}

#[test]
fn a_v2_manifest_is_taken_only_when_signed_by_the_v2_key_for_this_channel() {
    let (old, next, keys) = two_keys();
    let stranger = TestKey::generate();
    let artifact = next.sign(b"installer", "Beebeeb_0.9.0_x64-setup.exe");
    let good = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(9_000_001));
    let signed = verify(&keys, "stable", &good, &next.sign(&good, "stable.json")).unwrap();
    assert_eq!((signed.version.as_str(), signed.signed_at), ("0.9.0", SIGNED_AT));
    // Key scope: the legacy key is trusted, but not here.
    assert_eq!(
        verify(&keys, "stable", &good, &old.sign(&good, "stable.json")).err(),
        Some(Refusal::KeyScope),
        "a v2 manifest signed by the legacy key"
    );
    let refused = |bytes: &[u8], signature: &str, channel: &str| verify(&keys, channel, bytes, signature).err();
    assert_eq!(
        refused(&good, &stranger.sign(&good, "stable.json"), "stable"),
        Some(Refusal::ManifestSignature)
    );
    let mut changed = good.clone();
    *changed.last_mut().unwrap() = b' ';
    assert_eq!(
        refused(&changed, &next.sign(&good, "stable.json"), "stable"),
        Some(Refusal::ManifestSignature)
    );
    assert_eq!(refused(&good, "", "stable"), Some(Refusal::ManifestSignature));
    // Another channel's manifest file, validly signed.
    assert_eq!(
        refused(&good, &next.sign(&good, "beta.json"), "stable"),
        Some(Refusal::WrongChannel)
    );
    let table: [(Vec<u8>, Refusal); 6] = [
        (
            manifest("0.9.0", "pass", "stable", URL_090, &artifact, Some(1)),
            Refusal::WrongChannel,
        ),
        (
            manifest("0.9.0", "desktop", "beta", URL_090, &artifact, Some(1)),
            Refusal::WrongChannel,
        ),
        (
            manifest("0.8.30", "desktop", "stable", URL_090, &artifact, Some(1)),
            Refusal::BelowFloor,
        ),
        (
            manifest("0.8.26", "desktop", "stable", URL_090, &artifact, Some(1)),
            Refusal::BelowFloor,
        ),
        (
            manifest("0.9.0-beta.1", "desktop", "stable", URL_090, &artifact, Some(1)),
            Refusal::ManifestUnreadable,
        ),
        (b"<html>".to_vec(), Refusal::ManifestUnreadable),
    ];
    for (bytes, want) in table {
        assert_eq!(refused(&bytes, &next.sign(&bytes, "stable.json"), "stable"), Some(want));
    }
}

#[test]
fn an_artifact_of_a_signed_manifest_is_on_our_host_sized_and_signed_for_its_own_name() {
    let (old, next, keys) = two_keys();
    let stranger = TestKey::generate();
    let check = |version: &str, url: &str, signature: &str, size: Option<u64>| {
        let bytes = manifest(version, "desktop", "stable", url, signature, size);
        let signed = verify(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
        signed
            .artifact(&keys, version, url, signature)
            .map(|key| key.tauri_pubkey.clone())
    };
    let name = "Beebeeb_0.9.0_x64-setup.exe";
    // The key returned is the v2 key. An artefact signed by the legacy key is not of the manifest's line.
    assert_eq!(
        check("0.9.0", URL_090, &next.sign(b"i", name), Some(5)),
        Ok(next.tauri_pubkey())
    );
    assert_eq!(
        check("0.9.0", URL_090, &old.sign(b"i", name), Some(5)),
        Err(Refusal::KeyScope),
        "an artefact signed by the other line's key"
    );
    assert_eq!(
        check("0.9.0", URL_090, &stranger.sign(b"i", name), Some(5)),
        Err(Refusal::ArtifactSignature)
    );
    assert_eq!(
        check("0.9.0", URL_090, &next.sign(b"i", name), None),
        Err(Refusal::BadSize)
    );
    assert_eq!(
        check("0.9.0", URL_090, &next.sign(b"i", name), Some(0)),
        Err(Refusal::BadSize)
    );
    for url in [
        "https://evil.example/desktop/Beebeeb_0.9.0_x64-setup.exe",
        "http://releases.beebeeb.io/desktop/Beebeeb_0.9.0_x64-setup.exe",
        "https://releases.beebeeb.io:444/desktop/Beebeeb_0.9.0_x64-setup.exe",
        "https://releases.beebeeb.io/desktop/../Beebeeb_0.9.0_x64-setup.exe",
        "https://releases.beebeeb.io/desktop/Beebeeb_0.9.0_x64-setup.exe?x=1",
    ] {
        assert_eq!(
            check("0.9.0", url, &next.sign(b"i", name), Some(5)),
            Err(Refusal::UntrustedAddress),
            "{url}"
        );
    }
    // An older installer with its honest signature, offered as 0.9.0.
    let old_url = "https://releases.beebeeb.io/desktop/artifacts/0.8.4/Beebeeb_0.8.4_x64-setup.exe";
    let old_sig = old.sign(b"i", "Beebeeb_0.8.4_x64-setup.exe");
    assert_eq!(check("0.9.0", old_url, &old_sig, Some(5)), Err(Refusal::KeyScope));
    // The same trick with a file the v2 key signed for another version.
    let other_url = "https://releases.beebeeb.io/desktop/artifacts/0.9.1/Beebeeb_0.9.1_x64-setup.exe";
    assert_eq!(
        check(
            "0.9.0",
            other_url,
            &next.sign(b"i", "Beebeeb_0.9.1_x64-setup.exe"),
            Some(5)
        ),
        Err(Refusal::SignatureForOtherFile)
    );
    // Not the entry the manifest holds.
    let bytes = manifest("0.9.0", "desktop", "stable", URL_090, &next.sign(b"i", name), Some(5));
    let signed = verify(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
    let other = next.sign(b"other", name);
    assert_eq!(
        signed.artifact(&keys, "0.9.0", URL_090, &other).err(),
        Some(Refusal::NotInManifest)
    );
    assert_eq!(
        signed.artifact(&keys, "0.9.1", URL_090, &next.sign(b"i", name)).err(),
        Some(Refusal::NotInManifest)
    );
}

/// Windows and Linux follow the same rules on the legacy feed.
const WINDOWS: Host = Host { macos: false };
const LINUX: Host = Host { macos: false };
const MAC: Host = Host { macos: true };

#[test]
fn the_legacy_feed_offers_only_a_newer_0_8_version_signed_by_the_legacy_key_for_its_own_file() {
    let (old, next, keys) = two_keys();
    let stranger = TestKey::generate();
    let url = |v: &str| {
        format!("https://github.com/beebeeb-io/desktop/releases/download/desktop-v{v}/Beebeeb_{v}_x64-setup.exe")
    };
    let sig = |key: &TestKey, v: &str| key.sign(b"i", &format!("Beebeeb_{v}_x64-setup.exe"));
    let run = |host: Host, direction: Direction, installed: &str, version: &str, url: &str, signature: &str| {
        accept(
            &keys,
            &Feed::Unsigned,
            host,
            direction,
            installed,
            version,
            url,
            signature,
            strictly_newer,
        )
        .map(|key| key.tauri_pubkey.clone())
    };
    use Direction::{AnyVersion, NewerOnly};
    assert_eq!(
        run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            "0.8.26",
            &url("0.8.26"),
            &sig(&old, "0.8.26")
        ),
        Ok(old.tauri_pubkey())
    );
    // Key scope: the v2 key signs nothing on the legacy feed, on any OS.
    for host in [WINDOWS, LINUX, MAC] {
        assert_eq!(
            run(
                host,
                NewerOnly,
                "0.8.25",
                "0.8.26",
                &url("0.8.26"),
                &sig(&next, "0.8.26")
            ),
            Err(Refusal::KeyScope)
        );
    }
    // Task 2449: the manifest claims 9.9.9 (or 0.8.99) and points at the 0.8.4 installer with its real signature.
    assert_eq!(
        run(LINUX, NewerOnly, "0.8.25", "9.9.9", &url("0.8.4"), &sig(&old, "0.8.4")),
        Err(Refusal::NativeFromUnsignedFeed)
    );
    assert_eq!(
        run(LINUX, NewerOnly, "0.8.25", "0.8.99", &url("0.8.4"), &sig(&old, "0.8.4")),
        Err(Refusal::SignatureForOtherFile)
    );
    assert_eq!(
        run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            "0.8.99",
            &url("0.8.4"),
            &sig(&old, "0.8.4")
        ),
        Err(Refusal::SignatureForOtherFile)
    );
    for not_newer in ["0.8.25", "0.8.24", "0.8.4", "garbage", ""] {
        let refused = run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            not_newer,
            &url(not_newer),
            &sig(&old, not_newer),
        );
        assert_eq!(refused, Err(Refusal::NotNewer), "{not_newer:?}");
    }
    // A native release never comes from the legacy feed, on any OS and whichever key signed it; a version nobody
    // can read counts as one.
    for host in [WINDOWS, LINUX, MAC] {
        for signer in [&old, &next] {
            for native in ["0.9.0", "0.10.2", "9.9.9"] {
                assert_eq!(
                    run(host, NewerOnly, "0.8.25", native, &url(native), &sig(signer, native)),
                    Err(Refusal::NativeFromUnsignedFeed),
                    "{native} on the legacy feed"
                );
            }
        }
        assert_eq!(
            run(host, AnyVersion, "0.8.25", "x", &url("x"), &sig(&old, "x")),
            Err(Refusal::NativeFromUnsignedFeed)
        );
        assert_eq!(
            run(
                host,
                AnyVersion,
                "0.8.25",
                "0.9.0-beta.1",
                &url("0.9.0-beta.1"),
                &sig(&old, "0.9.0-beta.1")
            ),
            Err(Refusal::NativeFromUnsignedFeed)
        );
    }
    assert_eq!(
        run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            "0.8.26",
            &url("0.8.26"),
            &sig(&stranger, "0.8.26")
        ),
        Err(Refusal::ArtifactSignature)
    );
    // The manual check and the named downgrade see an older version, bound to its own file all the same.
    assert!(
        run(
            WINDOWS,
            AnyVersion,
            "0.8.25",
            "0.8.22",
            &url("0.8.22"),
            &sig(&old, "0.8.22")
        )
        .is_ok()
    );
    assert_eq!(
        run(
            WINDOWS,
            AnyVersion,
            "0.8.25",
            "0.8.22",
            &url("0.8.4"),
            &sig(&old, "0.8.4")
        ),
        Err(Refusal::SignatureForOtherFile)
    );
    // A Mac's bundle carries no version in its name and is never installed in place.
    let mac_url = "https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.8.26/Beebeeb.app.tar.gz";
    assert!(
        run(
            MAC,
            NewerOnly,
            "0.8.25",
            "0.8.26",
            mac_url,
            &old.sign(b"i", "Beebeeb.app.tar.gz")
        )
        .is_ok()
    );
    assert_eq!(
        run(
            LINUX,
            NewerOnly,
            "0.8.25",
            "0.8.26",
            mac_url,
            &old.sign(b"i", "Beebeeb.app.tar.gz")
        ),
        Err(Refusal::SignatureForOtherFile)
    );
}

#[test]
fn a_signed_manifest_only_moves_an_install_forward_to_what_it_holds() {
    let (_old, next, keys) = two_keys();
    let name = "Beebeeb_0.9.0_x64-setup.exe";
    let artifact = next.sign(b"i", name);
    let bytes = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(5));
    let signed = verify(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
    let run = |direction: Direction, installed: &str, signature: &str| {
        accept(
            &keys,
            &Feed::Signed(&signed),
            WINDOWS,
            direction,
            installed,
            "0.9.0",
            URL_090,
            signature,
            strictly_newer,
        )
        .map(|key| key.tauri_pubkey.clone())
    };
    assert_eq!(run(Direction::NewerOnly, "0.8.25", &artifact), Ok(next.tauri_pubkey()));
    assert_eq!(run(Direction::AnyVersion, "0.8.25", &artifact), Ok(next.tauri_pubkey()));
    for direction in [Direction::NewerOnly, Direction::AnyVersion] {
        assert_eq!(run(direction, "0.9.0", &artifact), Err(Refusal::NotNewer));
        assert_eq!(run(direction, "0.9.1", &artifact), Err(Refusal::NotNewer));
    }
    assert_eq!(
        run(Direction::NewerOnly, "0.8.25", &next.sign(b"other", name)),
        Err(Refusal::NotInManifest)
    );
}

#[test]
fn every_refusal_has_its_own_code() {
    use Refusal::*;
    let all = [
        ManifestSignature,
        WrongChannel,
        ManifestUnreadable,
        BelowFloor,
        NotInManifest,
        UntrustedAddress,
        BadSize,
        ArtifactSignature,
        SignatureForOtherFile,
        NotNewer,
        NativeFromUnsignedFeed,
        KeyScope,
        OlderThanKey,
        SignedInTheFuture,
        StaleManifest,
    ];
    let codes: std::collections::BTreeSet<_> = all.iter().map(|refusal| refusal.code()).collect();
    assert_eq!(codes.len(), all.len());
    assert!(ManifestSignature.message().starts_with("update_manifest_not_signed: "));
    assert!(!is_native_line("0.8.99") && is_native_line("0.9.0") && is_native_line("1.0.0") && is_native_line("?"));
}

/// The alpha feed as published on 2026-11-11 (notes emptied): what every 0.8.x release looks like. The rules must
/// keep taking it, or 0.8.25 would refuse its own successors.
#[test]
fn the_published_0_8_24_feed_passes_the_rules_and_cannot_pose_as_another_version() {
    let feed: serde_json::Value = serde_json::from_str(include_str!("fixtures/desktop-alpha-0.8.24.json")).unwrap();
    let keys = TrustedKeys::new(&baked_pubkey(), NEXT_PUBKEY_FILE).unwrap();
    let platforms = feed["platforms"].as_object().unwrap();
    assert_eq!(platforms.len(), 4);
    for (target, platform) in platforms {
        let (url, signature) = (
            platform["url"].as_str().unwrap(),
            platform["signature"].as_str().unwrap(),
        );
        let host = Host { macos: false };
        let run = |installed: &str, version: &str| {
            accept(
                &keys,
                &Feed::Unsigned,
                host,
                Direction::NewerOnly,
                installed,
                version,
                url,
                signature,
                strictly_newer,
            )
            .map(|key| key.tauri_pubkey.clone())
        };
        assert_eq!(
            run("0.8.23", "0.8.24"),
            Ok(keys.for_line(Line::Legacy).unwrap().tauri_pubkey.clone()),
            "{target}: signed with the baked key"
        );
        assert_eq!(
            run("0.8.25", "0.8.26"),
            Err(Refusal::SignatureForOtherFile),
            "{target}: 0.8.24 offered as 0.8.26"
        );
        assert_eq!(run("0.8.25", "0.8.24"), Err(Refusal::NotNewer), "{target}: a rollback");
    }
}

#[test]
fn the_v2_key_not_before_is_the_day_the_key_was_made() {
    // 2026-11-11T00:00:00Z. 20_768 days after 1970-01-01 (56 years with 14 leap days, then 314 days into 2026).
    assert_eq!(V2_KEY_NOT_BEFORE, 1_794_355_200);
    assert_eq!(V2_KEY_NOT_BEFORE, (56 * 365 + 14 + 314) * 86_400);
    assert_eq!(MAX_CLOCK_SKEW_SECS, 86_400);
}

#[test]
fn a_signed_time_is_read_from_the_trusted_comment_or_not_at_all() {
    let key = TestKey::generate();
    let rows: [(&str, Option<u64>); 9] = [
        ("1800000000", Some(1_800_000_000)),
        ("0", Some(0)),
        ("999999999999", Some(999_999_999_999)),
        ("1000000000000", None),
        ("", None),
        ("-5", None),
        ("+5", None),
        ("18e8", None),
        (" 1800000000", None),
    ];
    for (text, want) in rows {
        assert_eq!(signed_time(&key.sign_at(b"m", "stable.json", text)), want, "{text:?}");
    }
    assert_eq!(signed_time("PGh0bWw+NDA0PC9odG1sPg=="), None);
}

#[test]
fn a_manifest_signed_before_the_key_or_a_day_ahead_of_the_clock_or_before_the_mark_is_refused() {
    const K: u64 = V2_KEY_NOT_BEFORE;
    const DAY: u64 = MAX_CLOCK_SKEW_SECS;
    let now = K + 10 * DAY;
    use Refusal::{OlderThanKey, SignedInTheFuture, StaleManifest};
    // (signed at, now, mark, verdict)
    let rows: [(u64, u64, Option<u64>, Result<(), Refusal>); 14] = [
        (K, now, None, Ok(())),
        (K - 1, now, None, Err(OlderThanKey)),
        (0, now, None, Err(OlderThanKey)),
        // A clock set far back does not let an older manifest in.
        (K - 1, 0, None, Err(OlderThanKey)),
        (now, now, None, Ok(())),
        (now + DAY, now, None, Ok(())),
        (now + DAY + 1, now, None, Err(SignedInTheFuture)),
        (u64::MAX, u64::MAX, None, Ok(())),
        (u64::MAX, now, None, Err(SignedInTheFuture)),
        // The replay mark.
        (now, now, Some(now), Ok(())),
        (now - 1, now, Some(now), Err(StaleManifest)),
        (now, now, Some(now + DAY), Err(StaleManifest)),
        // A mark beyond what the future bound lets in is not believed.
        (now, now, Some(now + DAY + 1), Ok(())),
        (K - 1, now, Some(K - 2), Err(OlderThanKey)),
    ];
    for (signed_at, now, seen, want) in rows {
        assert_eq!(
            check_signed_time(signed_at, now, seen),
            want,
            "{signed_at} {now} {seen:?}"
        );
    }
    // Through the manifest check, with the time inside the signature.
    let (_old, next, keys) = two_keys();
    let artifact = next.sign(b"i", "Beebeeb_0.9.0_x64-setup.exe");
    let bytes = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(5));
    let at = |signed_at: &str, now: u64, seen: Option<u64>| {
        let signature = next.sign_at(&bytes, "stable.json", signed_at);
        verify_v2_manifest(&keys, "stable", &bytes, &signature, now, seen).map(|signed| signed.signed_at)
    };
    assert_eq!(at(&K.to_string(), now, None), Ok(K));
    assert_eq!(at(&(K - 1).to_string(), now, None), Err(OlderThanKey));
    assert_eq!(at(&(now + DAY + 1).to_string(), now, None), Err(SignedInTheFuture));
    assert_eq!(at(&(now - 1).to_string(), now, Some(now)), Err(StaleManifest));
    assert_eq!(at("soon", now, None), Err(Refusal::ManifestUnreadable));
    assert_eq!(at("", now, None), Err(Refusal::ManifestUnreadable));
}

#[test]
fn a_replay_mark_only_ever_grows() {
    assert_eq!(next_replay_mark(None, 7), 7);
    assert_eq!(next_replay_mark(Some(5), 7), 7);
    assert_eq!(
        next_replay_mark(Some(9), 7),
        9,
        "a clock set back does not lower the mark"
    );
    assert_eq!(next_replay_mark(Some(7), 7), 7);
}
