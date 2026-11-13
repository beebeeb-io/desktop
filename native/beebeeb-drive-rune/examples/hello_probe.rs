//! Native rung for `shell/platform/windows_hello.rs`: what does Windows Hello answer on THIS machine?
//!
//! ```sh
//! cargo run --locked --example hello_probe                               # prints the availability
//! cargo run --locked --example hello_probe -- --prompt                   # and shows the prompt, prints how it ended
//! cargo run --locked --example hello_probe -- --prompt --cancel-after 5  # takes the prompt away after 5 s
//! ```
//!
//! With `--prompt` it opens a small window of its own ("Beebeeb Hello probe") and asks for the prompt on that
//! window, as the app will on its own: the prompt must come up in front of it. Nobody answering is not a hang:
//! after `--cancel-after` seconds (default 60; 0 waits for ever) the probe takes the prompt away itself, says so,
//! and stays 3 s more so that it can be seen whether Windows closed the prompt.
//! It signs in to nothing, reads no account and prints only the names of the answers. It needs a desktop: over
//! plain SSH there is none (`scripts/rungs/hello-probe.ps1` is the wrapper for the build VM's desktop session).

#[allow(dead_code)]
#[path = "../src/shell/platform/windows_hello.rs"]
mod windows_hello;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_hello::{HelloRequest, HelloResult, HelloWindow};

const REASON: &str = "Beebeeb Hello probe";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut prompt = false;
    let mut cancel_after = Some(Duration::from_secs(60));
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--prompt" => prompt = true,
            "--cancel-after" => match rest.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(0) => cancel_after = None,
                Some(seconds) => cancel_after = Some(Duration::from_secs(seconds)),
                None => usage(),
            },
            _ => usage(),
        }
    }

    println!("availability: {:?}", windows_hello::availability());
    if !prompt {
        return;
    }
    let Some(window) = probe_window() else {
        println!("result: none (no window to own the prompt)");
        std::process::exit(1);
    };
    let (tx, rx) = mpsc::channel();
    let Some(request) = windows_hello::start(window, REASON, move |result| {
        let _ = tx.send(result);
    }) else {
        println!("result: none (no thread for the prompt)");
        std::process::exit(1);
    };
    println!("result: {:?}", wait(&rx, &request, cancel_after));
}

fn usage() -> ! {
    eprintln!("usage: hello_probe [--prompt [--cancel-after <seconds>]]");
    std::process::exit(2);
}

/// Keeps the probe's window alive (its messages handled) until the answer is in.
fn wait(rx: &mpsc::Receiver<HelloResult>, request: &HelloRequest, cancel_after: Option<Duration>) -> HelloResult {
    let started = Instant::now();
    let mut cancelled = false;
    loop {
        pump_messages();
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(result) => {
                if cancelled {
                    linger(Duration::from_secs(3));
                }
                return result;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // `start` answers exactly once; a sender that went without an answer is a failure of the module.
            Err(mpsc::RecvTimeoutError::Disconnected) => return HelloResult::Failed,
        }
        if !cancelled && cancel_after.is_some_and(|after| started.elapsed() >= after) {
            cancelled = true;
            println!(
                "no answer after {} s: the probe takes the prompt away",
                started.elapsed().as_secs()
            );
            request.cancel();
        }
    }
}

/// Keeps the probe's window (and so a prompt Windows has not closed yet) on screen a little longer.
fn linger(time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        pump_messages();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn probe_window() -> Option<HelloWindow> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, SetForegroundWindow, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (class, title) = (wide("STATIC"), wide(REASON));
    // SAFETY: both strings are NUL-terminated and outlive the call; "STATIC" is a class Windows registers for
    // every process; no parent, menu, module or creation data. The window lives until the process ends.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            200,
            200,
            520,
            240,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return None;
    }
    // SAFETY: `hwnd` is the window made above, on this thread. A refusal (another app has the foreground) is fine.
    unsafe { SetForegroundWindow(hwnd) };
    HelloWindow::from_hwnd(hwnd as isize)
}

#[cfg(windows)]
fn pump_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    // SAFETY: the plain message loop of the thread that made the probe's window; `message` is written by
    // PeekMessageW before it is read.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

// Off Windows there is no window to make and no prompt to show: the module answers `Unavailable` by itself.
#[cfg(not(windows))]
fn probe_window() -> Option<HelloWindow> {
    HelloWindow::from_hwnd(1)
}

#[cfg(not(windows))]
fn pump_messages() {}
