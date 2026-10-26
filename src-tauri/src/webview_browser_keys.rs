//! A sync app has no use for a web browser's own shortcuts (0.8.15).
//!
//! On Windows every window is a WebView2, and WebView2 acts on browser shortcuts before the page or the app's menu sees
//! them: Ctrl+P and Ctrl+Shift+P print the page (Guus pressed the menu's Pause shortcut in 0.8.14 and got a print
//! dialog), F5 and Ctrl+R reload the window, Ctrl+F opens a find bar, F12 opens DevTools. This turns them off in every
//! webview the app creates, through a plugin whose `on_webview_ready` runs for each one (the windows from
//! `tauri.conf.json` and the ones built at runtime).
//!
//! Two switches, because one alone has a gap:
//! - `ICoreWebView2Settings3::AreBrowserAcceleratorKeysEnabled = false` turns them all off, but Microsoft applies a
//!   settings change made after navigation started only from the next navigation, and this runs after the first one has
//!   started. It still covers every reload.
//! - An `AcceleratorKeyPressed` handler sets `IsBrowserAcceleratorKeyEnabled = false` on each key as it is pressed
//!   (WebView2 runtime 1.0.2210.55 and later). That takes effect at once. It does not mark the key handled, so the page
//!   and the app's menu accelerators still receive it; only WebView2's own browser action is skipped.
//!
//! Text editing and movement keys (Ctrl+C/V/X/A/Z, Home, End, ...) are not browser shortcuts and stay as they are. The
//! zoom keys are browser shortcuts: the View menu's Ctrl+= / Ctrl+- / Ctrl+0 keep zooming, through the menu.
//!
//! Tauri 2.11 does not expose wry's `with_browser_accelerator_keys`, hence the direct WebView2 calls. On macOS and Linux
//! the plugin does nothing: WKWebView and WebKitGTK have no such browser shortcuts.

/// The plugin `run()` registers. Its name only has to be unique among the app's plugins.
pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("browser-keys-off")
        .on_webview_ready(|webview| turn_off_browser_keys(&webview))
        .build()
}

#[cfg(target_os = "windows")]
fn turn_off_browser_keys(webview: &tauri::Webview<tauri::Wry>) {
    let label = webview.label().to_string();
    let label_in_closure = label.clone();
    // `with_webview` runs the closure on the main thread, where WebView2's COM objects live.
    let dispatched = webview.with_webview(move |platform| {
        if let Err(error) = windows_impl::turn_off(&platform.controller()) {
            tracing::warn!(%error, label = %label_in_closure, "could not turn off the web view's browser shortcuts");
        }
    });
    if let Err(error) = dispatched {
        tracing::warn!(%error, %label, "could not reach the web view to turn off its browser shortcuts");
    }
}

#[cfg(not(target_os = "windows"))]
fn turn_off_browser_keys(_webview: &tauri::Webview<tauri::Wry>) {}

#[cfg(target_os = "windows")]
mod windows_impl {
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2AcceleratorKeyPressedEventArgs2, ICoreWebView2Controller, ICoreWebView2Settings3,
    };
    use windows_core_webview2::Interface;

    pub(super) fn turn_off(controller: &ICoreWebView2Controller) -> windows_core_webview2::Result<()> {
        // Per key, at once: WebView2 skips its own browser action and still passes the key on (see the module docs).
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(|_controller, args| {
            if let Some(args) = args
                && let Ok(args2) = args.cast::<ICoreWebView2AcceleratorKeyPressedEventArgs2>()
            {
                // SAFETY: a COM call on the event args WebView2 passes for the duration of this call.
                unsafe { args2.SetIsBrowserAcceleratorKeyEnabled(false)? };
            }
            Ok(())
        }));
        // The handler lives as long as the webview; it is never removed, so the token is not kept.
        let mut token = 0i64;
        // SAFETY: plain COM calls on a live controller Tauri handed us, made on the thread that owns it (see the caller).
        unsafe {
            let settings = controller.CoreWebView2()?.Settings()?;
            // A runtime too old for ICoreWebView2Settings3 has no setting to change: the handler still applies.
            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
            }
            controller.add_AcceleratorKeyPressed(&handler, &mut token)?;
        }
        Ok(())
    }
}
