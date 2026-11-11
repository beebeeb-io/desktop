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
            let comment = format!("timestamp:1800000000\tfile:{file}");
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

use kit::{TestKey, URL_090, manifest};

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
    assert_eq!(keys.len(), 2, "the next key is a second key, not the baked one again");
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
fn the_placeholder_is_no_key_and_a_damaged_key_file_is_an_error() {
    let old = TestKey::generate();
    let only_old = TrustedKeys::new(&old.tauri_pubkey(), "PLACEHOLDER: not set yet\n").unwrap();
    assert_eq!(only_old.len(), 1);
    assert!(
        TrustedKeys::new(&old.tauri_pubkey(), "not a key").is_err(),
        "never a silently shorter list"
    );
    assert!(TrustedKeys::new("not a key", &old.tauri_pubkey()).is_err());
    assert_eq!(
        TrustedKeys::new(&old.tauri_pubkey(), &old.tauri_pubkey())
            .unwrap()
            .len(),
        1,
        "the same key once"
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
fn a_v2_manifest_is_taken_only_when_signed_by_a_trusted_key_for_this_channel() {
    let (old, next, keys) = two_keys();
    let stranger = TestKey::generate();
    let artifact = next.sign(b"installer", "Beebeeb_0.9.0_x64-setup.exe");
    let good = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(9_000_001));
    for signer in [&old, &next] {
        let signed = verify_v2_manifest(&keys, "stable", &good, &signer.sign(&good, "stable.json"));
        assert_eq!(
            signed.map(|m| m.version).ok().as_deref(),
            Some("0.9.0"),
            "either trusted key signs a manifest"
        );
    }
    let refused =
        |bytes: &[u8], signature: &str, channel: &str| verify_v2_manifest(&keys, channel, bytes, signature).err();
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
    let table: [(Vec<u8>, Refusal); 5] = [
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
        let signed = verify_v2_manifest(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
        signed
            .artifact(&keys, version, url, signature)
            .map(|key| key.tauri_pubkey.clone())
    };
    let name = "Beebeeb_0.9.0_x64-setup.exe";
    // The key returned is the one that signed the artefact: the next key for a native build, the old one too.
    assert_eq!(
        check("0.9.0", URL_090, &next.sign(b"i", name), Some(5)),
        Ok(next.tauri_pubkey())
    );
    assert_eq!(
        check("0.9.0", URL_090, &old.sign(b"i", name), Some(5)),
        Ok(old.tauri_pubkey())
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
    assert_eq!(
        check("0.9.0", old_url, &old_sig, Some(5)),
        Err(Refusal::SignatureForOtherFile)
    );
    // Not the entry the manifest holds.
    let bytes = manifest("0.9.0", "desktop", "stable", URL_090, &next.sign(b"i", name), Some(5));
    let signed = verify_v2_manifest(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
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

const WINDOWS: Host = Host {
    macos: false,
    windows: true,
};
const LINUX: Host = Host {
    macos: false,
    windows: false,
};
const MAC: Host = Host {
    macos: true,
    windows: false,
};

#[test]
fn the_unsigned_feed_offers_only_a_newer_version_whose_signature_names_it() {
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
    // A later 0.8.x signed with the next key is verified with the next key.
    assert_eq!(
        run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            "0.8.26",
            &url("0.8.26"),
            &sig(&next, "0.8.26")
        ),
        Ok(next.tauri_pubkey())
    );
    // Task 2449: the manifest claims 9.9.9 (or 0.8.99) and points at the 0.8.4 installer with its real signature.
    assert_eq!(
        run(LINUX, NewerOnly, "0.8.25", "9.9.9", &url("0.8.4"), &sig(&old, "0.8.4")),
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
    // A native release never comes from the unsigned feed on Windows; a version nobody can read counts as one.
    assert_eq!(
        run(
            WINDOWS,
            NewerOnly,
            "0.8.25",
            "0.9.0",
            &url("0.9.0"),
            &sig(&next, "0.9.0")
        ),
        Err(Refusal::NativeFromUnsignedFeed)
    );
    assert_eq!(
        run(WINDOWS, AnyVersion, "0.8.25", "x", &url("x"), &sig(&old, "x")),
        Err(Refusal::NativeFromUnsignedFeed)
    );
    assert!(run(LINUX, NewerOnly, "0.8.25", "0.9.0", &url("0.9.0"), &sig(&old, "0.9.0")).is_ok());
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
    let signed = verify_v2_manifest(&keys, "stable", &bytes, &next.sign(&bytes, "stable.json")).unwrap();
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
        let host = Host {
            macos: false,
            windows: target.starts_with("windows"),
        };
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
            Ok(keys.baked().tauri_pubkey.clone()),
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
