//! O-CP2's window proof: page 1 of an Office document, rendered by the confined helper, shown in a Rune window.
//!
//! ```text
//! office_page <file.docx> [--zoom <percent>] [--dpi <n>] [--helper <exe>]
//! ```
//!
//! It opens the file through the app's own host (`engine_host/office_host.rs`, included here as it is), asks for page 1
//! at `check::render_scale(zoom / 100, dpi)` (defaults: 100 %, 96 DPI), prints ONE line and then shows the page:
//!
//! ```text
//! office_page: state=<variant> pages=<n> first_page_ms=<t> w=<w> h=<h> missing_content=<bool>
//! ```
//!
//! No file name and no text of the document is printed. A state other than `Pages` (or no page within 60 s) prints
//! that state and exits 1. It is a development tool for test fixtures only; the app never reads a file this way.
//!
//! How the lead runs it on VM 401 (lanes cannot drive PowerShell there), under `with-lock`, from the desktop session:
//!
//! ```text
//! cargo build --release --locked -p beebeeb-office-render          # in native/beebeeb-office-render
//! cargo run --release --locked --example office_page -- <fixtures>/system-font.docx --zoom 100 --helper <release>/beebeeb-office-render.exe
//! ```
//!
//! Without `--helper` the helper is looked for beside this example's executable (`office_host::helper_exe`). The
//! helper starts confined exactly as in the app: off Windows this prints `state=NoPreview(NoSandbox)` and exits 1.

#[path = "../src/engine_host/office_host.rs"]
mod office_host;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check;
use office_host::{CloseWhy, DocKind, Event, OfficeHost, OfficeState};
use zeroize::Zeroizing;

const USAGE: &str = "usage: office_page <file.docx> [--zoom <percent>] [--dpi <n>] [--helper <exe>]";

struct Args {
    file: PathBuf,
    zoom: f32,
    dpi: f32,
    helper: Option<PathBuf>,
}

fn parse() -> Option<Args> {
    let mut file = None;
    let mut zoom = 100.0;
    let mut dpi = 96.0;
    let mut helper = None;
    let mut rest = std::env::args().skip(1);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--zoom" => zoom = rest.next()?.parse().ok()?,
            "--dpi" => dpi = rest.next()?.parse().ok()?,
            "--helper" => helper = Some(PathBuf::from(rest.next()?)),
            _ if file.is_none() && !arg.starts_with("--") => file = Some(PathBuf::from(&arg)),
            _ => return None,
        }
    }
    Some(Args {
        file: file?,
        zoom,
        dpi,
        helper,
    })
}

/// The document kind by the file's extension (a .docx when there is none it knows).
fn kind_of(file: &Path) -> DocKind {
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("doc") => DocKind::Doc,
        Some("pptx") => DocKind::Pptx,
        Some("xlsx") => DocKind::Xlsx,
        Some("xls") => DocKind::Xls,
        _ => DocKind::Docx,
    }
}

/// The state's variant, with its reason: never any content.
fn variant(state: &OfficeState) -> String {
    match state {
        OfficeState::Opening => "Opening".to_owned(),
        OfficeState::Pages { .. } => "Pages".to_owned(),
        OfficeState::Sheets { .. } => "Sheets".to_owned(),
        OfficeState::NoPreview(why) => format!("NoPreview({why:?})"),
        OfficeState::CannotOpen(why) => format!("CannotOpen({why:?})"),
        OfficeState::Stopped { why, .. } => format!("Stopped({why:?})"),
        OfficeState::Closed(why) => format!("Closed({why:?})"),
    }
}

struct Page {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    missing_content: bool,
    after: Duration,
}

fn main() -> ExitCode {
    let Some(args) = parse() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    // No helper beside the exe and none named: the host's own answer for a missing helper (NotInstalled).
    let helper = args.helper.clone().or_else(office_host::helper_exe).unwrap_or_else(|| {
        std::env::current_exe()
            .unwrap_or_default()
            .with_file_name(office_host::HELPER_EXE)
    });
    let bytes = match std::fs::read(&args.file) {
        Ok(bytes) => Zeroizing::new(bytes),
        Err(error) => {
            eprintln!("office_page: the file could not be read ({})", error.kind());
            return ExitCode::from(2);
        }
    };
    let scale = check::render_scale(args.zoom / 100.0, args.dpi);

    let started = Instant::now();
    let mut host = OfficeHost::open(&helper, kind_of(&args.file), bytes, Arc::new(|| {}));
    let until = started + Duration::from_secs(60);
    let mut asked = false;
    let mut page = None;
    while page.is_none() && Instant::now() < until {
        for event in host.poll() {
            if let Event::Page {
                w,
                h,
                rgba,
                missing_content,
                ..
            } = event
            {
                page = Some(Page {
                    w,
                    h,
                    rgba,
                    missing_content,
                    after: started.elapsed(),
                });
            }
        }
        let open = matches!(host.state(), OfficeState::Pages { .. });
        if !open && !matches!(host.state(), OfficeState::Opening) {
            break;
        }
        if open && !asked {
            asked = true;
            if host.request_page(0, scale, None).is_none() {
                eprintln!("office_page: page 1 cannot be asked at this zoom and DPI");
                break;
            }
        }
        thread::sleep(Duration::from_millis(2));
    }

    let pages = match host.state() {
        OfficeState::Pages { pages, .. } => *pages,
        _ => 0,
    };
    let state = variant(host.state());
    let Some(page) = page else {
        println!("office_page: state={state} pages={pages} first_page_ms=0 w=0 h=0 missing_content=false");
        return ExitCode::from(1);
    };
    println!(
        "office_page: state={state} pages={pages} first_page_ms={} w={} h={} missing_content={}",
        page.after.as_millis(),
        page.w,
        page.h,
        page.missing_content
    );
    // The page is copied out: the helper can go before the window opens.
    host.close(CloseWhy::Closed);

    let pixels = rune::Pixels::new();
    pixels.set(page.w, page.h, page.rgba);
    // Shown at one device pixel per pixel: points are pixels / (dpi / 96).
    let factor = args.dpi / 96.0;
    let shown = Shown {
        pixels,
        w: page.w as f32 / factor,
        h: page.h as f32 / factor,
    };
    let size = (shown.w + 48.0, shown.h + 48.0);
    let app = rune::App::new(rune::AppSpec::new("Beebeeb Office page", size, nothing, view)).init(move || shown);
    // No arguments for Rune: this example's own flags are not Rune's.
    rune::run_windows_with_args(rune::RunOptions::default(), Vec::<String>::new(), move |_displays| {
        vec![rune::WindowSpec::from_app("office-page", app)]
    });
    ExitCode::SUCCESS
}

struct Shown {
    pixels: rune::Pixels,
    w: f32,
    h: f32,
}

fn nothing() -> Shown {
    Shown {
        pixels: rune::Pixels::new(),
        w: 0.0,
        h: 0.0,
    }
}

fn view(shown: &Shown, _cx: &mut rune::Cx<Shown>) -> rune::El {
    rune::el().fill(rune::Color::BLACK.alpha(0.08)).child(
        rune::el()
            .key("page")
            .w(shown.w)
            .h(shown.h)
            .fill(rune::Color::WHITE)
            .pixels_fit(&shown.pixels, rune::SurfaceFit::Fill),
    )
}
