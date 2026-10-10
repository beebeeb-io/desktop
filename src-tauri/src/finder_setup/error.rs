//! One NSError as the ObjC bridge saw it (spec §6.1), plus the errors we synthesize ourselves.

use crate::surfaces::phase::FinderFailureReason;

pub const FILE_PROVIDER_DOMAIN: &str = "NSFileProviderErrorDomain";
pub const COCOA_DOMAIN: &str = "NSCocoaErrorDomain";
pub const POSIX_DOMAIN: &str = "NSPOSIXErrorDomain";
/// Errors `FileProviderBridge.m` synthesizes when no NSError exists. Fixed codes below,
/// mirrored by the `BeebeebBridge*` enum in the bridge.
pub const BRIDGE_DOMAIN: &str = "io.beebeeb.bridge";
/// Errors the Rust side synthesizes (engine start, saving the sync folder, a bridge
/// return code the bridge does not document, a refused launch location).
pub const APP_DOMAIN: &str = "io.beebeeb.app";

// The codes the bridge synthesizes. Only `STABILIZATION_TIMEOUT` is classified; the rest mirror the
// ObjC enum so a test pins the two sets against each other (`the_bridge_error_codes_are_the_rust_constants`).
#[allow(dead_code)]
pub mod bridge_code {
    pub const MANAGER_UNAVAILABLE: i64 = 1;
    pub const STABILIZATION_TIMEOUT: i64 = 2;
    pub const RESOLVE_URL_TIMEOUT: i64 = 3;
    pub const SIGNAL_TIMEOUT: i64 = 4;
    pub const NO_IDENTIFIER: i64 = 5;
}

pub mod app_code {
    pub const ENGINE_START: i64 = 1;
    pub const FINISH_READY: i64 = 2;
    pub const UNEXPECTED_BRIDGE_RETURN: i64 = 3;
    pub const LAUNCH_LOCATION: i64 = 4;
    /// The reconciler task panicked and stopped (`driver::start`'s supervisor).
    pub const RECONCILER_PANIC: i64 = 5;
    /// A port call never returned and `driver::within` cut it off at its limit (lead ruling T7-1).
    /// Classified as `timeout`: macOS, or this app's own call, did not finish.
    pub const OP_TIMEOUT: i64 = 6;
    /// A stop took the engine out of its slot and `abort()` could not confirm its task ended
    /// (`engine_stop_unconfirmed` is set; sign-out refuses until a restart).
    pub const ENGINE_STOP_UNCONFIRMED: i64 = 7;
}

/// The domain and code of an `NSUnderlyingErrorKey` error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FpErrorCode {
    pub domain: String,
    pub code: i64,
}

/// Classification reads only `domain`, `code` and `underlying` (spec §6.1). `message` goes
/// to the lifecycle log, redacted, and nowhere else: it is never serialized (so no payload to the
/// frontend and no saved file can carry it), and a payload without it deserializes with an empty one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FpError {
    pub domain: String,
    pub code: i64,
    #[serde(skip_serializing, default)]
    pub message: String,
    pub underlying: Option<FpErrorCode>,
}

impl FpError {
    pub fn new(domain: &str, code: i64, message: impl Into<String>) -> Self {
        Self {
            domain: domain.to_string(),
            code,
            message: message.into(),
            underlying: None,
        }
    }

    pub fn with_underlying(mut self, domain: &str, code: i64) -> Self {
        self.underlying = Some(FpErrorCode {
            domain: domain.to_string(),
            code,
        });
        self
    }

    pub fn app(code: i64, message: impl Into<String>) -> Self {
        Self::new(APP_DOMAIN, code, message)
    }
}

impl FpError {
    /// Domain and code, plus the underlying pair when there is one, and NEVER the message
    /// (spec §6.1, lead ruling T1-4). This is the only text of an `FpError` that may reach a Tauri
    /// command's return value, a saved config, a toast or "Copy details": the message is the
    /// OS's own localized prose and can name a file or a folder. `Display` keeps the message for
    /// stdout tracing only.
    pub fn redacted(&self) -> String {
        match &self.underlying {
            Some(underlying) => {
                format!(
                    "{} {} (underlying {} {})",
                    self.domain, self.code, underlying.domain, underlying.code
                )
            }
            None => format!("{} {}", self.domain, self.code),
        }
    }
}

/// Same shape the bridge's old flattened string had, so `%error` in existing `tracing` call
/// sites reads the same. Includes the message: for stdout tracing only, never for anything a
/// person sees or a file keeps (use [`FpError::redacted`] there).
impl std::fmt::Display for FpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} {})", self.message, self.domain, self.code)
    }
}

/// The persisted last failure (spec §9: `finder_last_failure = { reason, domain, code, at }`).
/// `at` is Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FailureRecord {
    pub reason: FinderFailureReason,
    pub domain: String,
    pub code: i64,
    pub at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH_MESSAGE: &str = "The file \u{201c}/Users/sam/Secret Folder/tax.pdf\u{201d} could not be added.";

    #[test]
    fn redacted_is_domain_and_code_and_never_the_message() {
        let error = FpError::new(FILE_PROVIDER_DOMAIN, -2011, PATH_MESSAGE);
        assert_eq!(error.redacted(), "NSFileProviderErrorDomain -2011");
        for leaked in ["/Users/sam", "Secret Folder", "tax.pdf", "could not be added"] {
            assert!(
                !error.redacted().contains(leaked),
                "redacted() leaked {leaked:?}: {}",
                error.redacted()
            );
        }
    }

    #[test]
    fn redacted_names_the_underlying_pair_and_still_not_the_message() {
        let error = FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE).with_underlying(POSIX_DOMAIN, 17);
        assert_eq!(
            error.redacted(),
            "NSCocoaErrorDomain 516 (underlying NSPOSIXErrorDomain 17)"
        );
        assert!(!error.redacted().contains("Secret Folder"));
    }

    #[test]
    fn serializing_an_fp_error_never_carries_the_message() {
        // Whatever Task 7/8 later emit to the frontend or save to disk, the OS's prose is not in it.
        let error = FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE).with_underlying(POSIX_DOMAIN, 17);
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "domain": "NSCocoaErrorDomain",
                "code": 516,
                "underlying": { "domain": "NSPOSIXErrorDomain", "code": 17 },
            })
        );
        let text = json.to_string();
        for leaked in ["message", "/Users/sam", "Secret Folder", "tax.pdf"] {
            assert!(!text.contains(leaked), "serialized FpError leaked {leaked:?}: {text}");
        }
        let without_underlying =
            serde_json::to_string(&FpError::new(FILE_PROVIDER_DOMAIN, -2011, PATH_MESSAGE)).unwrap();
        assert_eq!(
            without_underlying,
            r#"{"domain":"NSFileProviderErrorDomain","code":-2011,"underlying":null}"#
        );
    }

    #[test]
    fn a_payload_without_a_message_still_deserializes_and_round_trips_without_one() {
        let sent =
            serde_json::to_string(&FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE).with_underlying(POSIX_DOMAIN, 17))
                .unwrap();
        let received: FpError = serde_json::from_str(&sent).expect("no message key is fine");
        assert_eq!(received.message, "", "the message does not travel");
        assert_eq!((received.domain.as_str(), received.code), (COCOA_DOMAIN, 516));
        assert_eq!(
            received.underlying,
            Some(FpErrorCode {
                domain: POSIX_DOMAIN.to_string(),
                code: 17
            })
        );
        assert_eq!(serde_json::to_string(&received).unwrap(), sent);
    }

    #[test]
    fn display_keeps_the_message_for_stdout_tracing() {
        // The contrast that makes the two tests above mean something: Display DOES carry the
        // message, so any site that turns an FpError into a String with it is the leak.
        let error = FpError::new(FILE_PROVIDER_DOMAIN, -2011, PATH_MESSAGE);
        assert!(error.to_string().contains("/Users/sam/Secret Folder"), "{error}");
        assert_ne!(error.to_string(), error.redacted());
    }
}
