//! The pure parts of creating an account: which addresses the engine keeps, the password's verdict, the word list,
//! and an attempt that was left. The steps against a server are tested through the native source
//! (`engine_host/signup_tests.rs`), on the engine's fake server.

use serde_json::json;

use super::{SignupAttempt, SignupHint, own_https_address, word_suggestions};

fn address(text: &str) -> Option<String> {
    own_https_address(Some(&json!(text)))
}

#[test]
fn only_an_https_page_on_beebeebs_own_domain_is_kept() {
    assert_eq!(
        address("https://app.beebeeb.io/signup").as_deref(),
        Some("https://app.beebeeb.io/signup")
    );
    assert_eq!(address("https://beebeeb.io").as_deref(), Some("https://beebeeb.io"));
    for refused in [
        "http://app.beebeeb.io/signup",
        "https://evil.example/signup",
        "https://beebeeb.io.evil.example/signup",
        "https://evilbeebeeb.io/signup",
        "https://user@app.beebeeb.io/signup",
        "https://app.beebeeb.io:8443/signup",
        "https://app.beebeeb.io\\@evil.example/",
        "https://app.beebeeb.io/sign up",
        "javascript:alert(1)",
        "",
    ] {
        assert_eq!(address(refused), None, "{refused}");
    }
    assert_eq!(own_https_address(None), None);
    assert_eq!(own_https_address(Some(&json!(7))), None);
}

#[test]
fn the_password_is_judged_by_cores_rules_with_no_server_numbers_yet() {
    let attempt = SignupAttempt::new();
    let short = attempt.verdict("short");
    assert_eq!(
        (short.level, short.meets_minimum, short.min_length, short.missing),
        (1, false, 12, 7)
    );
    assert_eq!(short.hint, SignupHint::MoreCharacters);
    let fair = attempt.verdict("alllowercaseletters");
    assert_eq!(
        (fair.level, fair.meets_minimum, fair.hint),
        (2, true, SignupHint::MixCaseAndNumberOrSymbol)
    );
    let strong = attempt.verdict("Mixed-Case-and-1-number");
    assert_eq!((strong.level, strong.hint), (4, SignupHint::None));
}

#[test]
fn words_complete_what_is_typed_up_to_the_limit_and_nothing_for_nothing() {
    assert!(word_suggestions("", 4).is_empty());
    let some = word_suggestions("th", 3);
    assert_eq!(some.len(), 3);
    assert!(some.iter().all(|word| word.starts_with("th")));
    assert!(word_suggestions("qqqq", 4).is_empty());
}

#[test]
fn a_left_attempt_holds_nothing_and_calls_that_started_before_see_it() {
    let attempt = SignupAttempt::new();
    let epoch = attempt.epoch();
    *super::locked(&attempt.positions) = Some([1, 2, 3]);
    *super::locked(&attempt.web) = Some("https://beebeeb.io/x".to_string());
    assert_eq!(attempt.confirm_positions(), Some([1, 2, 3]));
    attempt.leave();
    assert!(!attempt.current(epoch), "a call that began before sees it was left");
    attempt.cancel();
    assert_eq!(attempt.confirm_positions(), None);
    assert_eq!(attempt.web_address(), None);
    let mut lent = false;
    attempt.with_phrase(&mut |_| lent = true);
    assert!(!lent, "no phrase is lent by an attempt that holds none");
}
