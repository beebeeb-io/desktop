//! How a start that stops for a reason the person must read says so (decision 2462, the lead's ruling for #335 delta
//! 2): a refused `--api-base`, the test build's data not for this run's server, stored credentials without local data.
//! Each line goes to stderr and into the start-refusal box ([`super::refuse_start`]), except where a start shows no
//! box (at login, the installer's call, a tooling run), and ends the start with 2. The box is the fake platform's
//! recorder: `show_start_refusal` keeps the title and the text it was given.

use super::platform::fake::FakePlatform;
use super::*;
use crate::engine_host::open::{CredentialsWithoutData, OtherServer, Refused, Server};

fn launch(args: &[&str]) -> instance::Launch {
    instance::parse_args(args.iter().map(|arg| (*arg).to_owned()), false, true)
}

/// Every refusal kind, as the line `run` hands over.
fn refusals() -> Vec<(&'static str, OpenErrorOrFlag)> {
    vec![
        ("a refused flag", OpenErrorOrFlag::Flag(Refused::UserInfo)),
        ("a glued flag", OpenErrorOrFlag::Flag(Refused::Glued)),
        (
            "data made for another server",
            OpenErrorOrFlag::Open(OpenError::OtherServer(OtherServer {
                made_for: Some(Server::Default),
                asked: Server::Custom("http://127.0.0.1:3001".into()),
            })),
        ),
        (
            "credentials without local data",
            OpenErrorOrFlag::Open(OpenError::CredentialsWithoutData(CredentialsWithoutData)),
        ),
    ]
}

enum OpenErrorOrFlag {
    Flag(Refused),
    Open(OpenError),
}

/// What `run` does with it: the flag's refusal goes to `refuse_start`, an open's to `engine_not_opened`.
fn stop(platform: &FakePlatform, launch: &instance::Launch, why: &OpenErrorOrFlag) -> (i32, String) {
    let identity = AppIdentity::CURRENT;
    match why {
        OpenErrorOrFlag::Flag(refused) => (
            refuse_start(platform, launch, identity, &refused.to_string()),
            refused.to_string(),
        ),
        OpenErrorOrFlag::Open(error) => {
            let line = match error {
                OpenError::OtherServer(other) => other.to_string(),
                OpenError::CredentialsWithoutData(alone) => alone.to_string(),
                OpenError::Failed(_) => unreachable!("not a refusal"),
            };
            (engine_not_opened(platform, launch, identity, error), line)
        }
    }
}

#[test]
fn every_refusal_reaches_the_start_refusal_box_with_its_line_and_ends_with_2() {
    for (kind, why) in refusals() {
        let platform = FakePlatform::default();
        let (code, line) = stop(&platform, &launch(&[]), &why);
        assert_eq!(code, 2, "{kind}");
        let shown = platform.refusals.lock().unwrap().clone();
        assert_eq!(
            shown,
            [(AppIdentity::CURRENT.product_name().to_owned(), line.clone())],
            "{kind}"
        );
        assert!(
            line.ends_with("The app did not start.") && !line.contains("127.0.0.1"),
            "{kind}: {line}"
        );
    }
}

#[test]
fn no_box_at_login_for_the_installer_or_in_a_tooling_run_and_still_2() {
    let mut tooling = launch(&[]);
    tooling.tooling = true;
    for (start, launch) in [
        ("at login", launch(&["--autostart"])),
        ("the installer's call", launch(&["--quit-for-update"])),
        ("a tooling run", tooling),
    ] {
        for (kind, why) in refusals() {
            let platform = FakePlatform::default();
            let (code, _) = stop(&platform, &launch, &why);
            assert_eq!(code, 2, "{start}, {kind}");
            assert!(platform.refusals.lock().unwrap().is_empty(), "{start}, {kind}: no box");
        }
    }
}

/// A handle that could not be opened for any other reason is said in the box as before, even at login, and ends the
/// start with 1.
#[test]
fn a_failed_open_is_said_in_the_box_as_before_and_ends_with_1() {
    for args in [&[][..], &["--autostart"][..]] {
        let platform = FakePlatform::default();
        let error = OpenError::Failed("The folder for app data could not be found.".into());
        assert_eq!(
            engine_not_opened(&platform, &launch(args), AppIdentity::CURRENT, &error),
            1
        );
        let shown = platform.refusals.lock().unwrap().clone();
        let name = AppIdentity::CURRENT.product_name();
        assert_eq!(
            shown,
            [(
                name.to_owned(),
                format!("{name} could not open its data on this computer. The folder for app data could not be found.")
            )]
        );
    }
}

/// `run` hands every refusal to these two and says none of them itself: the flag's after the platform object is made,
/// the open's for any error of `open_engine`.
#[test]
fn run_hands_every_refusal_to_the_box_path() {
    let squeezed = |text: &str| text.split_whitespace().collect::<String>();
    let shell = squeezed(include_str!("mod.rs"));
    let run =
        &shell[shell.find(&squeezed("pub fn run() {")).unwrap()..shell.find(&squeezed("fn refuse_start(")).unwrap()];
    for call in [
        "Err(refused) => std::process::exit(refuse_start(&*platform, &launch, identity, &refused.to_string())),",
        "Err(error) => std::process::exit(engine_not_opened(&*platform, &launch, identity, &error)),",
    ] {
        assert_eq!(run.matches(&squeezed(call)).count(), 1, "{call}");
    }
    let made = run
        .find(&squeezed("let platform = platform::current();"))
        .expect("run makes the platform object");
    assert!(made < run.find(&squeezed("open::api_base(")).expect("run checks the flag"));
    for said_here in [
        "eprintln!(\"beebeeb-drive: {refused}\")",
        "eprintln!(\"beebeeb-drive: {other}\")",
        "OpenError::",
    ] {
        assert!(!run.contains(&squeezed(said_here)), "run says {said_here} itself");
    }
    // `open` is the only caller that can hand back the binding's refusals, and `engine_not_opened` covers each kind.
    let not_opened = &shell[shell.find(&squeezed("fn engine_not_opened(")).unwrap()..];
    for arm in [
        "OpenError::OtherServer(other) => refuse_start(platform, launch, identity, &other.to_string()),",
        "OpenError::CredentialsWithoutData(alone) => refuse_start(platform, launch, identity, &alone.to_string()),",
    ] {
        assert!(not_opened.contains(&squeezed(arm)), "{arm}");
    }
}
