//! Windows Hello user verification, raised by Rust for the window that asks (`UserConsentVerifier` through
//! `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`, so the prompt is owned by our window and not
//! hidden behind it). Used before a device approval leaves this PC (crypto review rule 4) and to open the app lock
//! (DW-0009). The prompt is Windows' own; its PIN fallback and its retry limit are Windows', never counted here.

/// How the prompt ended, as the web view's codes.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verification {
    Verified,
    Cancelled,
    /// Not recognised after Windows' own retries, or the device is busy.
    Failed,
    /// Windows Hello (or a PIN) is not set up on this PC.
    Unavailable,
}

impl Verification {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Cancelled => "hello_cancelled",
            Self::Failed => "hello_failed",
            Self::Unavailable => "hello_unavailable",
        }
    }
}

/// Ask Windows Hello for `window`, with `reason` as the prompt's line ("Approve Drive on Anna's iPhone").
#[cfg(target_os = "windows")]
pub(crate) async fn verify(window: &tauri::WebviewWindow, reason: String) -> Verification {
    let Ok(hwnd) = window.hwnd() else { return Verification::Failed };
    let raw = hwnd.0 as isize;
    tokio::task::spawn_blocking(move || imp::verify_blocking(raw, &reason)).await.unwrap_or(Verification::Failed)
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn verify(_window: &tauri::WebviewWindow, _reason: String) -> Verification {
    // macOS gets LocalAuthentication on the same commands later; until then nothing is verified off Windows.
    Verification::Unavailable
}

/// Is Windows Hello set up on this PC (the Continue-as card and the lock depend on it)?
#[tauri::command]
pub(crate) async fn auth_windows_hello_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        tokio::task::spawn_blocking(imp::available).await.unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// DW-0009: Windows Hello, then the existing keychain unlock. A cancel or a miss leaves the window locked.
#[tauri::command]
pub(crate) async fn auth_unlock_with_windows_hello(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    match verify(&window, "Unlock Beebeeb".into()).await {
        Verification::Verified => crate::vault_lock_commands::unlock_vault(app, state).await,
        other => Err(other.code().into()),
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows::Foundation::IAsyncOperation;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows::core::HSTRING;

    use super::Verification;

    pub(super) fn available() -> bool {
        UserConsentVerifier::CheckAvailabilityAsync()
            .and_then(|op| op.get())
            .map(|a| a == UserConsentVerifierAvailability::Available)
            .unwrap_or(false)
    }

    pub(super) fn verify_blocking(hwnd: isize, reason: &str) -> Verification {
        if !available() {
            return Verification::Unavailable;
        }
        let result = (|| -> windows::core::Result<UserConsentVerificationResult> {
            let interop = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
            // SAFETY: a COM call with a live top-level window handle Tauri handed us; the operation is awaited below.
            let op: IAsyncOperation<UserConsentVerificationResult> =
                unsafe { interop.RequestVerificationForWindowAsync(HWND(hwnd as *mut _), &HSTRING::from(reason))? };
            op.get()
        })();
        match result {
            Ok(UserConsentVerificationResult::Verified) => Verification::Verified,
            Ok(UserConsentVerificationResult::Canceled) => Verification::Cancelled,
            Ok(UserConsentVerificationResult::DeviceNotPresent)
            | Ok(UserConsentVerificationResult::NotConfiguredForUser)
            | Ok(UserConsentVerificationResult::DisabledByPolicy) => Verification::Unavailable,
            _ => Verification::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Verification;

    #[test]
    fn every_outcome_but_verified_is_a_refusal_code() {
        let refusals = [Verification::Cancelled, Verification::Failed, Verification::Unavailable];
        assert_eq!(refusals.iter().filter(|v| v.code().starts_with("hello_")).count(), 3);
        assert_eq!(Verification::Verified.code(), "verified");
    }
}
