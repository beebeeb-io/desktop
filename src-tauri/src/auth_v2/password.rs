//! The password legs of `desktop_login` / `desktop_login_2fa` (B10b, crypto rule 0): core's `auth_flow::sign_in` runs
//! OPAQUE (with the server's `ksf_version`) and reads every answer; this module only sends each request and maps core's
//! codes to the words the sign-in screens already show. The session install (credential store, `account_binding`)
//! stays in `login_commands`.
use std::future::Future;

use beebeeb_core::auth_flow::sign_in::{PendingSecondFactor, SignIn, SignInStep};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, HttpResponse};
use zeroize::Zeroizing;

/// What the password legs ended in.
pub(crate) enum PasswordLegs {
    SignedIn(Zeroizing<String>),
    /// Two-factor is on: core holds the partial token until the code is sent.
    SecondFactor(PendingSecondFactor),
}

/// The words the sign-in form shows for each of core's codes. A server body is never shown.
pub(crate) fn sign_in_error(e: &AuthFlowError) -> String {
    match e {
        AuthFlowError::InvalidCredentials => "Invalid email or password".to_string(),
        AuthFlowError::InvalidInput(_) => "Email and password are required.".to_string(),
        AuthFlowError::WrongCode => "Invalid authentication code".to_string(),
        other => super::flow_code(other),
    }
}

/// Run the login-start and login-finish legs over `send`.
pub(crate) async fn password_legs<F, Fut>(mut send: F, email: &str, password: &str) -> Result<PasswordLegs, String>
where
    F: FnMut(HttpRequest) -> Fut,
    Fut: Future<Output = Result<HttpResponse, String>>,
{
    let (mut flow, mut req) = SignIn::start(email, password.as_bytes()).map_err(|e| sign_in_error(&e))?;
    loop {
        let resp = send(req).await?;
        match flow.on_response(resp).map_err(|e| sign_in_error(&e))? {
            SignInStep::Send(next) => req = next,
            SignInStep::SignedIn { session_token } => return Ok(PasswordLegs::SignedIn(session_token)),
            SignInStep::SecondFactor(pending) => return Ok(PasswordLegs::SecondFactor(pending)),
        }
    }
}

/// The verify request for a typed code (6 digits, or an 8-digit backup code).
pub(crate) fn second_factor_request(pending: &PendingSecondFactor, code: &str) -> Result<HttpRequest, String> {
    if code.trim().is_empty() {
        return Err("Enter your authentication code.".to_string());
    }
    pending.verify(code).map_err(|e| sign_in_error(&e))
}

/// The answer to [`second_factor_request`]: the session token. A wrong code keeps `pending` usable.
pub(crate) fn second_factor_answer(
    pending: &PendingSecondFactor,
    resp: HttpResponse,
) -> Result<Zeroizing<String>, String> {
    pending.on_verify_response(resp).map_err(|e| sign_in_error(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use beebeeb_core::opaque_protocol as op;
    use serde_json::json;
    use std::cell::RefCell;

    const EMAIL: &str = "anna@example.eu";
    const PASSWORD: &str = "correct horse battery";

    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn unb64(v: &serde_json::Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap()
    }

    /// A mock server running the real server half of OPAQUE; it records every path it was sent.
    struct Mock {
        setup: Vec<u8>,
        file: Vec<u8>,
        state: Option<Vec<u8>>,
        two_factor: bool,
        log: Vec<String>,
    }

    impl Mock {
        fn new(two_factor: bool) -> Self {
            let setup = op::create_server_setup();
            let start = op::client_registration_start(PASSWORD.as_bytes()).unwrap();
            let resp = op::server_registration_start(&setup, &start.message, EMAIL.as_bytes()).unwrap();
            let upload = op::client_registration_finish(&start.state, PASSWORD.as_bytes(), &resp).unwrap();
            let file = op::server_registration_finish(&upload).unwrap();
            Self {
                setup,
                file,
                state: None,
                two_factor,
                log: vec![],
            }
        }

        fn answer(&mut self, req: HttpRequest) -> HttpResponse {
            self.log.push(req.path.clone());
            let body = req.body.unwrap();
            match req.path.as_str() {
                "/api/v1/opaque/login-start" => {
                    let s = op::server_login_start(
                        &self.setup,
                        &self.file,
                        &unb64(&body["client_message"]),
                        EMAIL.as_bytes(),
                    )
                    .unwrap();
                    self.state = Some(s.state);
                    HttpResponse::new(
                        200,
                        json!({ "server_message": b64(&s.message), "server_state": "st", "ksf_version": 1 }),
                    )
                }
                "/api/v1/opaque/login-finish" => {
                    match op::server_login_finish(&self.state.take().unwrap(), &unb64(&body["client_message"])) {
                        Ok(_) if self.two_factor => {
                            HttpResponse::new(200, json!({ "requires_2fa": true, "partial_token": "partial" }))
                        }
                        Ok(_) => HttpResponse::new(200, json!({ "session_token": "session-1" })),
                        Err(_) => HttpResponse::new(401, json!({})),
                    }
                }
                "/api/v1/auth/2fa/verify" if body["code"] == "123456" && body["partial_token"] == "partial" => {
                    HttpResponse::new(200, json!({ "session_token": "session-2" }))
                }
                "/api/v1/auth/2fa/verify" => HttpResponse::new(401, json!({})),
                other => panic!("unexpected {other}"),
            }
        }
    }

    fn run(mock: &RefCell<Mock>, email: &str, password: &str) -> Result<PasswordLegs, String> {
        tauri::async_runtime::block_on(password_legs(
            |r| std::future::ready(Ok(mock.borrow_mut().answer(r))),
            email,
            password,
        ))
    }

    #[test]
    fn the_password_legs_run_cores_sign_in() {
        let mock = RefCell::new(Mock::new(false));
        let Ok(PasswordLegs::SignedIn(token)) = run(&mock, " Anna@Example.eu ", PASSWORD) else {
            panic!("signed in")
        };
        assert_eq!(token.as_str(), "session-1");
        assert_eq!(
            mock.borrow().log,
            ["/api/v1/opaque/login-start", "/api/v1/opaque/login-finish"]
        );
        let mock = RefCell::new(Mock::new(false));
        assert_eq!(
            run(&mock, EMAIL, "not the password").err().as_deref(),
            Some("Invalid email or password")
        );
        assert_eq!(
            run(&mock, EMAIL, "").err().as_deref(),
            Some("Email and password are required.")
        );
    }

    #[test]
    fn the_second_factor_leg_keeps_the_challenge_after_a_wrong_code() {
        let mock = RefCell::new(Mock::new(true));
        let Ok(PasswordLegs::SecondFactor(pending)) = run(&mock, EMAIL, PASSWORD) else {
            panic!("two-factor")
        };
        let wrong = second_factor_request(&pending, "000000").unwrap();
        let resp = mock.borrow_mut().answer(wrong);
        assert_eq!(
            second_factor_answer(&pending, resp).err().as_deref(),
            Some("Invalid authentication code")
        );
        let right = second_factor_request(&pending, "123 456").unwrap();
        let resp = mock.borrow_mut().answer(right);
        assert_eq!(second_factor_answer(&pending, resp).unwrap().as_str(), "session-2");
        assert_eq!(
            second_factor_request(&pending, "  ").err().as_deref(),
            Some("Enter your authentication code.")
        );
    }

    /// The commands call this module, not OPAQUE directly (Guus: every auth flow lives in core).
    #[test]
    fn desktop_login_commands_call_cores_flow() {
        let source = include_str!("../login_commands.rs");
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let start = source.find(signature).expect(signature);
            let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
            assert!(!body.contains("opaque_protocol"), "{signature} runs OPAQUE itself");
            assert!(!body.contains("/api/v1/"), "{signature} names a route itself");
        }
        assert!(source.contains("auth_v2::password::password_legs("));
        assert!(source.contains("auth_v2::password::second_factor_request("));
    }
}
