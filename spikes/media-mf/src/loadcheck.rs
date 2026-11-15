//! media-mf-loadcheck: loads the system libraries Media Foundation and COM need, one at a time, and prints each result.
//!
//! The probe itself died inside the sandbox before its first line (0xC0000142, a library's start-up failed). This exe
//! imports only what Rust's standard library and the token query need, so it starts, says whether it is confined, and
//! then names the library whose start-up fails and the system's code for it.
#![cfg(windows)]

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenCapabilities, TokenIsAppContainer};
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

/// In load order: the window-system libraries first (other libraries import them), then COM, Media Foundation, D3D,
/// and the in-box decoders.
const LIBRARIES: &[&str] = &[
    "win32u.dll",
    "gdi32.dll",
    "user32.dll",
    "combase.dll",
    "ole32.dll",
    "oleaut32.dll",
    "propsys.dll",
    "mfplat.dll",
    "mfreadwrite.dll",
    "mf.dll",
    "mfcore.dll",
    "dxgi.dll",
    "d3d11.dll",
    "msmpeg2vdec.dll",
    "msauddecmft.dll",
    "wmvdecod.dll",
    "wmadmod.dll",
    "mp4sdecd.dll",
    "mfmp4srcsnk.dll",
    "mfasfsrcsnk.dll",
];

fn main() {
    unsafe {
        let mut token = HANDLE::default();
        let mut line = String::from("T");
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut returned = 0u32;
            let mut is_container = 0u32;
            let _ = GetTokenInformation(
                token,
                TokenIsAppContainer,
                Some((&raw mut is_container).cast()),
                4,
                &mut returned,
            );
            let mut capabilities = [0u64; 512];
            let _ = GetTokenInformation(
                token,
                TokenCapabilities,
                Some(capabilities.as_mut_ptr().cast()),
                size_of_val(&capabilities) as u32,
                &mut returned,
            );
            line.push_str(&format!(" appcontainer={is_container} caps={}", capabilities[0] as u32));
            let _ = CloseHandle(token);
        } else {
            line.push_str(" open-token-failed");
        }
        println!("{line}");
        for name in LIBRARIES {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            match LoadLibraryW(PCWSTR(wide.as_ptr())) {
                Ok(_) => println!("L {name} ok"),
                Err(e) => println!("L {name} fail 0x{:08X}", e.code().0 as u32),
            }
        }
        println!("END");
    }
}
