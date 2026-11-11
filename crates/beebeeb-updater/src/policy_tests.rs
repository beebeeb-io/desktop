//! One test per rule of `policy::check`: an honest release passes, and each single lie is refused by name.

use crate::manifest::Channel;
use crate::policy::{
    Checked, HighWater, KeyScope, MAX_ARTIFACT_BYTES, MAX_CLOCK_SKEW_SECS, Refusal, Trust, TrustedKey, Verdict, check,
    trusted_file_name,
};
use crate::testkit::{HOST, NOW, Publish, TestKey, artifact_bytes, artifact_name, artifact_url, trust, version};

fn run(key: &TestKey, installed: &str, seen: Option<u64>, publish: &Publish) -> Result<Checked, Refusal> {
    let (manifest, signature) = publish.manifest(key);
    check(
        &trust(key),
        publish.channel,
        version(installed),
        seen.map(|manifest_time| HighWater { manifest_time }),
        NOW,
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
            NOW,
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
        NOW,
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
            NOW,
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
    // The 0.8.x shape (neither field) is not a signed manifest at all: the strict reader refuses it.
    (publish.product, publish.claimed_channel) = (None, None);
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::ManifestUnreadable)
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
}

fn run_with(
    trust: &Trust,
    installed: &str,
    seen: Option<u64>,
    now: u64,
    manifest: &[u8],
    signature: &str,
) -> Result<Verdict, Refusal> {
    let seen = seen.map(|manifest_time| HighWater { manifest_time });
    check(
        trust,
        Channel::Stable,
        version(installed),
        seen,
        now,
        manifest,
        signature,
    )
    .map(|checked| checked.verdict)
}

/// Review test 1. A fresh install has no mark, so the mark cannot stop a replay; the build's own time does. A
/// manifest signed before this build was made never installs anything.
#[test]
fn a_manifest_older_than_this_build_never_installs_anything_even_with_no_mark() {
    let key = TestKey::generate();
    let mut built_later = trust(&key);
    built_later.not_before = 1_800_000_500;
    // Signed at 1_800_000_000, before the build: a withdrawn 0.9.1 served again to a fresh 0.9.0.
    let (manifest, signature) = honest("0.9.1").manifest(&key);
    assert_eq!(
        run_with(&built_later, "0.9.0", None, NOW, &manifest, &signature),
        Err(Refusal::OlderThanThisBuild)
    );
    // The control: the same manifest and no mark, for a build made before it was signed.
    let mut built_earlier = trust(&key);
    built_earlier.not_before = 1_800_000_000;
    assert!(matches!(
        run_with(&built_earlier, "0.9.0", None, NOW, &manifest, &signature),
        Ok(Verdict::Upgrade(_))
    ));
    // An older version in such a manifest is a channel with nothing for this build yet: never an offer.
    assert_eq!(
        run_with(&built_later, "0.9.4", None, NOW, &manifest, &signature),
        Ok(Verdict::UpToDate)
    );
    assert_eq!(
        run_with(&built_later, "0.9.1", None, NOW, &manifest, &signature),
        Ok(Verdict::UpToDate)
    );
}

/// Review test 2. A manifest's signature signs `<channel>.json` and an artefact's signs `Beebeeb_<V>_….exe`: one
/// never passes as the other, even over the same bytes and by the same key.
#[test]
fn a_manifest_signature_and_an_artefact_signature_are_never_interchangeable() {
    let key = TestKey::generate();
    let trust = trust(&key);
    let publish = honest("0.9.1");
    let (manifest, detached) = publish.manifest(&key);
    assert!(matches!(
        run_with(&trust, "0.9.0", None, NOW, &manifest, &detached),
        Ok(Verdict::Upgrade(_))
    ));

    // An artefact signature offered as the manifest's: the installer's own, then one made over the manifest bytes.
    let of_installer = key.sign(&publish.artifact, 1_800_000_000, &artifact_name("0.9.1"));
    assert_eq!(
        run_with(&trust, "0.9.0", None, NOW, &manifest, &of_installer),
        Err(Refusal::ManifestSignature)
    );
    let named_as_installer = key.sign(&manifest, 1_800_000_000, &artifact_name("0.9.1"));
    assert_eq!(
        run_with(&trust, "0.9.0", None, NOW, &manifest, &named_as_installer),
        Err(Refusal::WrongChannel)
    );

    // The manifest's signature offered as the artefact's: by our key, but it signs `stable.json`.
    let mut reversed = honest("0.9.1");
    reversed.artifact_signature = Some(detached);
    let (manifest, detached) = reversed.manifest(&key);
    assert_eq!(
        run_with(&trust, "0.9.0", None, NOW, &manifest, &detached),
        Err(Refusal::SignatureForOtherFile)
    );
    // Even an artefact published under a manifest's name is not an installer.
    let mut as_json = honest("0.9.1");
    as_json.url = format!("https://{HOST}/desktop/v2/stable.json");
    as_json.artifact_signature = Some(key.sign(&as_json.artifact, 1_800_000_000, "stable.json"));
    let (manifest, detached) = as_json.manifest(&key);
    assert_eq!(
        run_with(&trust, "0.9.0", None, NOW, &manifest, &detached),
        Err(Refusal::SignatureForOtherFile)
    );
}

#[test]
fn the_artefact_name_is_exactly_beebeeb_version_rest_exe() {
    let key = TestKey::generate();
    let trust = trust(&key);
    let named = |name: &str| {
        let mut publish = honest("0.9.1");
        publish.url = format!("https://{HOST}/desktop/artifacts/0.9.1/{name}");
        publish.artifact_signature = Some(key.sign(&publish.artifact, 1_800_000_000, name));
        let (manifest, detached) = publish.manifest(&key);
        run_with(&trust, "0.9.0", None, NOW, &manifest, &detached)
    };
    assert!(matches!(named("Beebeeb_0.9.1_x64-setup.exe"), Ok(Verdict::Upgrade(_))));
    for name in [
        "Other_0.9.1_x64-setup.exe",
        "Beebeeb_0.9.1_x64-setup.msi",
        "Beebeeb_0.9.1_x64-setup.exe.txt",
        "Beebeeb_x64_0.9.1_setup.exe",
        "Beebeeb_0.9.1_.exe",
        "Beebeeb_0.9.1.exe",
        "Beebeeb_0.9.10_x64-setup.exe",
        "xBeebeeb_0.9.1_x64-setup.exe",
    ] {
        assert_eq!(named(name), Err(Refusal::SignatureForOtherFile), "{name}");
    }
}

/// A signature made with a wrong clock must not block a channel: it is refused, so it never becomes the mark, and
/// a mark that is already beyond belief is not held against honest manifests.
#[test]
fn a_signed_time_far_in_the_future_is_refused_and_cannot_block_the_channel() {
    let key = TestKey::generate();
    let trust = trust(&key);
    let at = |manifest_time: u64, seen: Option<u64>| {
        let mut publish = honest("0.9.1");
        publish.manifest_time = manifest_time;
        let (manifest, detached) = publish.manifest(&key);
        run_with(&trust, "0.9.0", seen, NOW, &manifest, &detached)
    };
    assert!(at(NOW + MAX_CLOCK_SKEW_SECS, None).is_ok(), "within the tolerance");
    assert_eq!(at(NOW + MAX_CLOCK_SKEW_SECS + 1, None), Err(Refusal::SignedInTheFuture));
    assert_eq!(at(4_000_000_000, None), Err(Refusal::SignedInTheFuture));
    // A mark from the year 2096 (written under a wrong clock) does not make today's manifest stale.
    assert!(at(1_800_000_000, Some(4_000_000_000)).is_ok());
    // A believable mark still does.
    assert_eq!(
        at(1_800_000_000, Some(NOW + MAX_CLOCK_SKEW_SECS)),
        Err(Refusal::StaleManifest)
    );
}

/// Each trusted key signs for its own versions only, manifests and artefacts alike.
#[test]
fn a_trusted_key_signs_only_the_versions_it_is_valid_for() {
    let (old, new) = (TestKey::generate(), TestKey::generate());
    let scoped = |key: &TestKey, from: &str, before: Option<&str>| TrustedKey {
        key: key.public(),
        valid_for: KeyScope {
            from: version(from),
            before: before.map(version),
        },
    };
    let mut both = trust(&new);
    both.keys = vec![scoped(&old, "0.9.0", Some("0.9.5")), scoped(&new, "0.9.5", None)];
    let by = |key: &TestKey, artefact_key: &TestKey, offered: &str| {
        let mut publish = honest(offered);
        publish.artifact_signature = Some(artefact_key.sign(&publish.artifact, 1_800_000_000, &artifact_name(offered)));
        let (manifest, detached) = publish.manifest(key);
        run_with(&both, "0.9.0", None, NOW, &manifest, &detached)
    };
    assert!(matches!(by(&old, &old, "0.9.4"), Ok(Verdict::Upgrade(_))));
    assert!(matches!(by(&new, &new, "0.9.5"), Ok(Verdict::Upgrade(_))));
    assert_eq!(by(&old, &old, "0.9.5"), Err(Refusal::KeyNotValidForVersion));
    assert_eq!(by(&new, &new, "0.9.4"), Err(Refusal::KeyNotValidForVersion));
    assert_eq!(
        by(&new, &old, "0.9.5"),
        Err(Refusal::ArtifactSignature),
        "the artefact's key too"
    );
    let stranger = TestKey::generate();
    assert_eq!(by(&stranger, &new, "0.9.5"), Err(Refusal::ManifestSignature));
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
    for size in [Some(0), Some(MAX_ARTIFACT_BYTES + 1)] {
        let mut publish = honest("0.9.1");
        publish.size = size;
        assert_eq!(
            run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
            Err(Refusal::BadSize),
            "{size:?}"
        );
    }
    // No length at all is not a signed manifest's shape (the strict reader): refused before the size rule.
    let mut publish = honest("0.9.1");
    publish.size = None;
    assert_eq!(
        run(&key, "0.9.0", None, &publish).map(|c| c.verdict),
        Err(Refusal::ManifestUnreadable)
    );
}

#[test]
fn refuses_a_release_without_a_build_for_this_pc() {
    let key = TestKey::generate();
    let mut narrow = trust(&key);
    narrow.targets = vec!["windows-aarch64-nsis".to_string()];
    let (manifest, signature) = honest("0.9.1").manifest(&key);
    assert_eq!(
        check(
            &narrow,
            Channel::Stable,
            version("0.9.0"),
            None,
            NOW,
            &manifest,
            &signature
        )
        .map(|c| c.verdict),
        Err(Refusal::NoBuildForThisPc)
    );
}

/// Signed is not enough: a manifest with a field this build does not know is refused, however honest its signature.
#[test]
fn policy_strict_a_signed_manifest_with_an_unknown_field_is_refused() {
    let key = TestKey::generate();
    let (manifest, _) = honest("0.9.1").manifest(&key);
    let resigned = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut value: serde_json::Value = serde_json::from_slice(&manifest).expect("json");
        edit(&mut value);
        let bytes = serde_json::to_vec(&value).expect("json");
        let detached = key.sign(&bytes, 1_800_000_000, "stable.json");
        run_with(&trust(&key), "0.9.0", None, NOW, &bytes, &detached)
    };
    assert!(matches!(resigned(&|_| {}), Ok(Verdict::Upgrade(_))), "the control");
    assert_eq!(
        resigned(&|manifest| manifest["install_args"] = "/S".into()),
        Err(Refusal::ManifestUnreadable)
    );
}
