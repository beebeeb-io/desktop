//! Against what is really published: the stable manifest of 0.8.22 as served on 2026-11-11 from
//! `https://releases.beebeeb.io/desktop/latest.json` (`tests/fixtures/desktop-latest-0.8.22.json`, unchanged), and,
//! when its path is given, the real installer that manifest points at.

use crate::manifest::{Channel, Manifest};
use crate::policy::{Refusal, check};
use crate::signature::{PublicKey, Signature};
use crate::version::Version;

const PUBLISHED: &[u8] = include_bytes!("../tests/fixtures/desktop-latest-0.8.22.json");

#[test]
fn reads_the_published_0_8_manifest() {
    let manifest = Manifest::parse(PUBLISHED).expect("the published manifest parses");
    assert_eq!(manifest.version, Version::new(0, 8, 22));
    assert_eq!(manifest.pub_date.as_deref(), Some("2026-11-06T10:57:51Z"));
    assert!(
        manifest.notes.starts_with("# Beebeeb Desktop 0.8.22"),
        "the authored notes are the manifest's notes"
    );
    let targets: Vec<&str> = manifest.platforms.keys().map(String::as_str).collect();
    assert_eq!(
        targets,
        [
            "linux-x86_64",
            "windows-x86_64",
            "windows-x86_64-msi",
            "windows-x86_64-nsis"
        ]
    );
    let (target, platform) = manifest
        .platform(&["windows-x86_64-nsis", "windows-x86_64"])
        .expect("a Windows entry");
    assert_eq!(target, "windows-x86_64-nsis");
    assert_eq!(
        platform.url,
        "https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.8.22/Beebeeb_0.8.22_x64-setup.exe"
    );
}

/// Every published signature is in the form this crate reads, was made by the key this crate bakes in, and signs
/// the file name its address ends in, version included: the facts the native rules rely on already hold for 0.8.x.
#[test]
fn every_published_signature_is_by_the_baked_key_and_names_its_file() {
    let manifest = Manifest::parse(PUBLISHED).expect("the published manifest parses");
    let key = PublicKey::legacy_0_8();
    let mut checked = 0;
    for (target, platform) in &manifest.platforms {
        let signature =
            Signature::from_base64(&platform.signature).unwrap_or_else(|error| panic!("{target}: {error:?}"));
        assert_eq!(signature.made_by(&key), Ok(()), "{target}");
        assert_eq!(
            Some(signature.file.as_str()),
            platform.url.rsplit('/').next(),
            "{target}"
        );
        assert!(
            signature.file.split('_').any(|part| part == "0.8.22"),
            "{target}: {}",
            signature.file
        );
        assert!(signature.timestamp > 1_793_000_000, "{target}");
        checked += 1;
    }
    assert_eq!(checked, 4);
}

/// The 0.8.x manifest has no detached signature, so the native rules refuse it whatever is offered beside it.
#[test]
fn the_native_rules_refuse_the_unsigned_0_8_manifest() {
    // Even a build that trusted the 0.8.x key for every version (no native build does) would refuse it.
    let trust = crate::policy::Trust {
        keys: vec![crate::policy::TrustedKey {
            key: PublicKey::legacy_0_8(),
            valid_for: crate::policy::KeyScope::from(Version::new(0, 0, 0)),
        }],
        product: "desktop".to_string(),
        allowed_hosts: vec![crate::host::RELEASE_HOST.to_string()],
        targets: vec!["windows-x86_64-nsis".to_string()],
        floor: crate::policy::NATIVE_FLOOR,
        not_before: crate::policy::V2_KEY_CREATED,
    };
    let nsis = &Manifest::parse(PUBLISHED).expect("parses").platforms["windows-x86_64-nsis"].signature;
    for signature in ["", nsis.as_str()] {
        let verdict = check(
            &trust,
            Channel::Stable,
            Version::new(0, 8, 21),
            None,
            1_800_000_000,
            PUBLISHED,
            signature,
        );
        assert_eq!(verdict.map(|checked| checked.verdict), Err(Refusal::ManifestSignature));
    }
}

/// The real 8.4 MiB installer against its published signature and the baked key. Needs the file:
/// `curl -LO https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.8.22/Beebeeb_0.8.22_x64-setup.exe`,
/// then `BB_REAL_NSIS_0_8_22=<path> cargo test --locked -- --ignored real_installer`.
#[test]
#[ignore = "needs the published installer on disk (BB_REAL_NSIS_0_8_22)"]
fn real_installer_verifies_and_one_changed_byte_does_not() {
    let path = std::env::var("BB_REAL_NSIS_0_8_22").expect("BB_REAL_NSIS_0_8_22 names the downloaded installer");
    let bytes = std::fs::read(&path).expect("the installer is readable");
    assert_eq!(bytes.len(), 8_854_808, "the published installer's length");
    let manifest = Manifest::parse(PUBLISHED).expect("parses");
    let signature = Signature::from_base64(&manifest.platforms["windows-x86_64-nsis"].signature).expect("decodes");
    let key = PublicKey::legacy_0_8();
    assert_eq!(
        signature.verify_reader(&key, std::fs::File::open(&path).expect("opens")),
        Ok(())
    );

    let mut changed = bytes;
    changed[1_000_000] ^= 1;
    assert_eq!(
        signature.verify_bytes(&key, &changed),
        Err(crate::signature::SignatureError::Mismatch)
    );
}
