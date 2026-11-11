//! One test per rule of `policy::check`: an honest release passes, and each single lie is refused by name.

use crate::manifest::Channel;
use crate::policy::{Checked, HighWater, MAX_ARTIFACT_BYTES, Refusal, Verdict, check, trusted_file_name};
use crate::testkit::{HOST, Publish, TestKey, artifact_bytes, artifact_name, artifact_url, trust, version};

fn run(key: &TestKey, installed: &str, seen: Option<u64>, publish: &Publish) -> Result<Checked, Refusal> {
    let (manifest, signature) = publish.manifest(key);
    check(
        &trust(key),
        publish.channel,
        version(installed),
        seen.map(|manifest_time| HighWater { manifest_time }),
        &manifest,
        &signature,
    )
}

fn honest(version: &str) -> Publish {
    Publish::new(Channel::Stable, version, artifact_bytes(5_000))
}

#[test]
fn offers_a_signed_newer_release() {
    let key = TestKey::generate();
    let checked = run(&key, "0.9.0", Some(1_799_999_999), &honest("0.9.1")).expect("an honest release passes");
    assert_eq!(checked.manifest_time, 1_800_000_000);
    let Verdict::Upgrade(offer) = checked.verdict else {
        panic!("expected an upgrade, got {:?}", checked.verdict)
    };
    assert_eq!(
        (offer.version, offer.channel, offer.size, offer.downgrade),
        (version("0.9.1"), Channel::Stable, 5_000, false)
    );
    assert_eq!(
        (offer.url.as_str(), offer.file_name.as_str()),
        (artifact_url("0.9.1").as_str(), "Beebeeb_0.9.1_x64-setup.exe")
    );
    assert_eq!(offer.notes, "What changed.");
}

#[test]
fn the_installed_version_is_up_to_date() {
    let key = TestKey::generate();
    assert_eq!(
        run(&key, "0.9.1", None, &honest("0.9.1")).map(|checked| checked.verdict),
        Ok(Verdict::UpToDate)
    );
}

#[test]
fn refuses_a_manifest_that_is_not_signed_by_our_key() {
    let (ours, theirs) = (TestKey::generate(), TestKey::generate());
    let publish = honest("0.9.1");
    let (manifest, signature) = publish.manifest(&ours);
    let check_with = |manifest: &[u8], signature: &str| {
        check(
            &trust(&ours),
            Channel::Stable,
            version("0.9.0"),
            None,
            manifest,
            signature,
        )
        .map(|c| c.verdict)
    };

    assert!(
        matches!(check_with(&manifest, &signature), Ok(Verdict::Upgrade(_))),
        "the control passes"
    );
    assert_eq!(
        check_with(&manifest, ""),
        Err(Refusal::ManifestSignature),
        "no signature"
    );
    assert_eq!(
        check_with(&manifest, "PGh0bWw+NDA0PC9odG1sPg=="),
        Err(Refusal::ManifestSignature),
        "an error page"
    );
    assert_eq!(
        check_with(&manifest, &publish.manifest(&theirs).1),
        Err(Refusal::ManifestSignature),
        "another key"
    );
    let mut edited = manifest.clone();
    let at = edited
        .windows(5)
        .position(|window| window == b"0.9.1")
        .expect("the version is in the manifest");
    edited[at + 4] = b'9';
    assert_eq!(
        check_with(&edited, &signature),
        Err(Refusal::ManifestSignature),
        "bytes changed after signing"
    );
}

#[test]
fn refuses_another_channels_manifest() {
    let key = TestKey::generate();
    // The beta manifest with its own valid signature, served where stable is asked for.
    let beta = Publish::new(Channel::Beta, "0.9.5", artifact_bytes(5_000));
    let (manifest, signature) = beta.manifest(&key);
    let as_stable = check(
        &trust(&key),
        Channel::Stable,
        version("0.9.0"),
        None,
        &manifest,
        &signature,
    );
    assert_eq!(as_stable.map(|c| c.verdict), Err(Refusal::WrongChannel));
    assert!(
        check(
            &trust(&key),
            Channel::Beta,
            version("0.9.0"),
            None,
            &manifest,
            &signature
        )
        .is_ok(),
        "the control: it is a good beta manifest"
    );

    // A manifest that says stable inside, signed under the beta manifest's file name: the signed name decides.
    let mut publish = honest("0.9.5");
    publish.signed_manifest_name = Some("beta.json");
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::WrongChannel)
    );

    // Signed under the stable file name, but the manifest itself says beta, another product, or neither (0.8.x format).
    let mut publish = honest("0.9.5");
    publish.claimed_channel = Some("beta");
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::WrongChannel)
    );
    let mut publish = honest("0.9.5");
    publish.product = Some("pass");
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::WrongChannel)
    );
    let mut publish = honest("0.9.5");
    (publish.product, publish.claimed_channel) = (None, None);
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::WrongChannel)
    );
}

#[test]
fn refuses_a_manifest_older_than_one_already_accepted() {
    let key = TestKey::generate();
    let mut replayed = honest("0.9.1");
    replayed.manifest_time = 1_700_000_000;
    assert_eq!(
        run(&key, "0.9.0", Some(1_800_000_000), &replayed).map(|c| c.verdict),
        Err(Refusal::StaleManifest)
    );
    assert!(
        run(&key, "0.9.0", Some(1_700_000_000), &replayed).is_ok(),
        "the same manifest again is not a replay"
    );
    assert!(
        run(&key, "0.9.0", None, &replayed).is_ok(),
        "a fresh install has no mark yet"
    );
}

#[test]
fn an_older_version_is_a_downgrade_and_never_below_the_first_native_release() {
    let key = TestKey::generate();
    let Ok(Checked {
        verdict: Verdict::Downgrade(offer),
        ..
    }) = run(&key, "0.9.4", None, &honest("0.9.2"))
    else {
        panic!("expected a downgrade")
    };
    assert!(offer.downgrade);
    assert_eq!(offer.version, version("0.9.2"));
    assert_eq!(
        run(&key, "0.9.4", None, &honest("0.8.25")).map(|c| c.verdict),
        Err(Refusal::BelowFloor)
    );
}

#[test]
fn refuses_an_artefact_outside_our_host() {
    let key = TestKey::generate();
    let name = artifact_name("0.9.1");
    for url in [
        format!("http://{HOST}/desktop/artifacts/0.9.1/{name}"),
        format!("https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.9.1/{name}"),
        format!("https://{HOST}.example.com/{name}"),
        format!("https://{HOST}:8443/{name}"),
        format!("https://user@{HOST}/{name}"),
        format!("https://{HOST}/a/../{name}"),
        format!("https://{HOST}/{name}?mirror=1"),
        format!("https://{HOST}/{name}#x"),
        format!("https://RELEASES.beebeeb.io/{name}"),
        format!("https://{HOST}"),
    ] {
        let mut publish = honest("0.9.1");
        publish.url = url.clone();
        assert_eq!(
            run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
            Err(Refusal::UntrustedAddress),
            "{url}"
        );
    }
    let hosts = [HOST.to_string()];
    assert_eq!(
        trusted_file_name(&format!("https://{HOST}/desktop/artifacts/0.9.1/{name}"), &hosts),
        Some(name.as_str())
    );
    assert_eq!(trusted_file_name(&format!("https://{HOST}/"), &hosts), None);
}

#[test]
fn refuses_a_signature_made_for_another_file() {
    let key = TestKey::generate();
    let old = artifact_bytes(4_000);
    let old_signature = key.sign(&old, 1_790_000_000, &artifact_name("0.9.0"));

    // An old release's artefact and its honest signature, offered as 0.9.5 under the new name.
    let mut renamed = Publish::new(Channel::Stable, "0.9.5", old.clone());
    renamed.artifact_signature = Some(old_signature.clone());
    assert_eq!(
        run(&key, "0.9.1", None, &renamed).map(|c| c.verdict),
        Err(Refusal::SignatureForOtherFile)
    );

    // The same, pointing at the old file's real address: the name matches its signature but not the version.
    let mut same_name = Publish::new(Channel::Stable, "0.9.5", old);
    same_name.artifact_signature = Some(old_signature);
    same_name.url = artifact_url("0.9.0");
    assert_eq!(
        run(&key, "0.9.1", None, &same_name).map(|c| c.verdict),
        Err(Refusal::SignatureForOtherFile)
    );
}

#[test]
fn refuses_an_artefact_signature_by_another_key_before_downloading() {
    let (ours, theirs) = (TestKey::generate(), TestKey::generate());
    let mut publish = honest("0.9.1");
    publish.artifact_signature = Some(theirs.sign(&publish.artifact, 1, &artifact_name("0.9.1")));
    assert_eq!(
        run(&ours, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::ArtifactSignature)
    );
    publish.artifact_signature = Some("bm90IGEgc2lnbmF0dXJl".to_string());
    assert_eq!(
        run(&ours, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::ArtifactSignature)
    );
}

#[test]
fn refuses_an_artefact_without_a_sane_length() {
    let key = TestKey::generate();
    for size in [None, Some(0), Some(MAX_ARTIFACT_BYTES + 1)] {
        let mut publish = honest("0.9.1");
        publish.size = size;
        assert_eq!(
            run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
            Err(Refusal::BadSize),
            "{size:?}"
        );
    }
}

#[test]
fn refuses_a_release_without_a_build_for_this_pc() {
    let key = TestKey::generate();
    let mut narrow = trust(&key);
    narrow.targets = vec!["windows-aarch64-nsis".to_string()];
    let (manifest, signature) = honest("0.9.1").manifest(&key);
    assert_eq!(
        check(&narrow, Channel::Stable, version("0.9.0"), None, &manifest, &signature).map(|c| c.verdict),
        Err(Refusal::NoBuildForThisPc)
    );
}
