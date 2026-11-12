//! The password legs of `desktop_login` / `desktop_login_2fa` live in the engine (`beebeeb_engine::signin::password`);
//! this is their old path, and the pin that the commands call them.
pub(crate) use beebeeb_engine::signin::password::*;

#[cfg(test)]
mod tests {
    /// The commands call this module, not OPAQUE directly (Guus: every auth flow lives in core).
    #[test]
    fn desktop_login_commands_call_cores_flow() {
        let source = include_str!("../login_commands.rs").replace("\r\n", "\n");
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
