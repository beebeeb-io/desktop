//! Task 2422: no web view of this app offers to save or fill a password, a recovery phrase or a code.
//!
//! WebView2's general autofill and password autosave would offer to store what is typed into the sign-in, recovery
//! phrase, TOTP and backup-code fields in the Edge profile. This plugin turns both off in every webview the app creates
//! (`on_webview_ready` runs for the windows in `tauri.conf.json` and the ones built at runtime), through
//! `ICoreWebView2Settings4`. A settings change made after the first navigation started applies from the next
//! navigation (the same gap `webview_browser_keys` documents), so every field also carries `autocomplete="off"`,
//! `spellcheck={false}` and `autoCorrect="off"` in the page. On macOS and Linux the plugin does nothing.

/// What every web view gets. Pure, so it is tested on every platform.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutofillPolicy {
    pub(crate) general_autofill: bool,
    pub(crate) password_autosave: bool,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) const POLICY: AutofillPolicy = AutofillPolicy { general_autofill: false, password_autosave: false };

pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("autofill-off").on_webview_ready(|webview| turn_off_autofill(&webview)).build()
}

#[cfg(target_os = "windows")]
fn turn_off_autofill(webview: &tauri::Webview<tauri::Wry>) {
    let label = webview.label().to_string();
    let label_in_closure = label.clone();
    // `with_webview` runs the closure on the main thread, where WebView2's COM objects live.
    let dispatched = webview.with_webview(move |platform| {
        if let Err(error) = imp::apply(&platform.controller(), POLICY) {
            tracing::warn!(%error, label = %label_in_closure, "could not turn off the web view's autofill");
        }
    });
    if let Err(error) = dispatched {
        tracing::warn!(%error, %label, "could not reach the web view to turn off its autofill");
    }
}

#[cfg(not(target_os = "windows"))]
fn turn_off_autofill(_webview: &tauri::Webview<tauri::Wry>) {}

#[cfg(target_os = "windows")]
mod imp {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Controller, ICoreWebView2Settings4};
    use windows_core_webview2::Interface;

    use super::AutofillPolicy;

    pub(super) fn apply(controller: &ICoreWebView2Controller, policy: AutofillPolicy) -> windows_core_webview2::Result<()> {
        // SAFETY: plain COM calls on a live controller Tauri handed us, on the thread that owns it (see the caller).
        unsafe {
            let settings = controller.CoreWebView2()?.Settings()?;
            // A runtime too old for ICoreWebView2Settings4 has neither feature to turn off.
            if let Ok(settings4) = settings.cast::<ICoreWebView2Settings4>() {
                settings4.SetIsGeneralAutofillEnabled(policy.general_autofill)?;
                settings4.SetIsPasswordAutosaveEnabled(policy.password_autosave)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_autofill_switches_are_off() {
        assert!(!super::POLICY.general_autofill);
        assert!(!super::POLICY.password_autosave);
    }

    #[test]
    fn the_plugin_is_registered_for_every_window() {
        let run = include_str!("../app_run.rs");
        assert_eq!(run.matches(".plugin(crate::auth_v2::webview_autofill::plugin())").count(), 1);
        let windows_impl = include_str!("webview_autofill.rs");
        assert!(windows_impl.contains("SetIsGeneralAutofillEnabled(policy.general_autofill)"));
        assert!(windows_impl.contains("SetIsPasswordAutosaveEnabled(policy.password_autosave)"));
    }
}
