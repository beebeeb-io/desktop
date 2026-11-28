//! What a Mac shows by itself (dm-70, dm-74): whether AVFoundation plays a file, whether Quick Look previews a type,
//! and the two ways the system puts one file on screen. The system decodes out of this process: AVFoundation in the
//! system's media services, Quick Look's generators in their own XPC service, which is the isolation the Windows
//! sandbox helper gives. No child process, no `open`, no AppleScript: Objective-C messages only.
//!
//! - **Inside the window** (dm-74, [`embed`], [`place`]): an `AVPlayerView` (AVKit, the system's playback controls)
//!   for a recording or a video, a `QLPreviewView` (Quartz) for a document, added as a child view of the window's
//!   content view, over the toolkit's Metal surface. The toolkit says where the viewer's body was laid out
//!   (`El::on_layout`, window points from the top left) and `place` puts the view there, so it follows a resize and
//!   the details panel. A child view covers whatever the toolkit draws under it, so the viewer hides it
//!   (`place(.., None)`) while one of its menus, toasts or dialogs is open. One view at a time.
//! - **The panel** ([`show`]): the standard Quick Look panel (`QLPreviewPanel`), a window of its own. Only the
//!   fallback now, for when no view could be hosted (no window, no class).
//!
//! Threads: the type questions ([`plays_type`], [`playable`], [`quicklook_shows_type`]) may be asked anywhere. The
//! views and the panel are AppKit: [`embed`], [`place`], [`show`] and [`showing`] do nothing (`false`) off the main
//! thread, [`close`] hops to it.
//!
//! Must not: log a path or a name.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSResponder, NSView, NSWindow};
use objc2_foundation::{NSArray, NSInteger, NSPoint, NSRect, NSSize, NSString, NSUInteger, NSURL};

// AVURLAsset and AVPlayer, AVPlayerView (AVKit), QLPreviewPanel and QLPreviewView (QuickLookUI is part of Quartz)
// and UTType: system frameworks, no crate of their own.
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "AVKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
}

/// The types Quick Look draws a real preview of (a type outside them gets a plain icon, which is not a preview):
/// pictures (PSD, HEIC, RAW, TIFF), text, documents (Word, Pages, RTF), sheets, presentations and 3D models.
const QUICK_LOOK_SHOWS: [&str; 6] = [
    "public.image",
    "public.text",
    "public.composite-content",
    "public.spreadsheet",
    "public.presentation",
    "public.3d-content",
];
/// PostScript and EPS: macOS has no converter for them since macOS 14, and Quick Look shows nothing of one (measured
/// on macOS 26.6: `qlmanage` makes no picture of an EPS). They go to the page with the other apps.
const QUICK_LOOK_CANNOT: [&str; 1] = ["com.adobe.postscript"];

/// The declared type of a file name's extension (`UTType`). `None`: no extension, or one the system does not know.
fn type_of(name: &str) -> Option<Retained<AnyObject>> {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty())?;
    let class = AnyClass::get(c"UTType")?;
    let ext = NSString::from_str(ext);
    // SAFETY: `+[UTType typeWithFilenameExtension:]` takes an NSString and returns a UTType or nil.
    let found: Option<Retained<AnyObject>> = unsafe { msg_send![class, typeWithFilenameExtension: &*ext] };
    let found = found?;
    // SAFETY: `-[UTType isDeclared]` returns a BOOL. A dynamic type (an unknown extension) is not declared.
    let declared: bool = unsafe { msg_send![&*found, isDeclared] };
    declared.then_some(found)
}

/// Whether `kind` is, or is a kind of, the type named `identifier`.
fn conforms(kind: &AnyObject, identifier: &str) -> bool {
    let Some(class) = AnyClass::get(c"UTType") else {
        return false;
    };
    let identifier = NSString::from_str(identifier);
    // SAFETY: `+[UTType typeWithIdentifier:]` takes an NSString and returns a UTType or nil.
    let parent: Option<Retained<AnyObject>> = unsafe { msg_send![class, typeWithIdentifier: &*identifier] };
    // SAFETY: `-[UTType conformsToType:]` takes a UTType and returns a BOOL.
    parent.is_some_and(|parent| unsafe { msg_send![kind, conformsToType: &*parent] })
}

/// The file types AVFoundation reads (`+[AVURLAsset audiovisualTypes]`), asked once.
fn av_types() -> &'static [String] {
    static TYPES: OnceLock<Vec<String>> = OnceLock::new();
    TYPES.get_or_init(|| {
        let Some(class) = AnyClass::get(c"AVURLAsset") else {
            return Vec::new();
        };
        // SAFETY: `+[AVURLAsset audiovisualTypes]` returns an NSArray of NSString type identifiers.
        let types: Option<Retained<NSArray<NSString>>> = unsafe { msg_send![class, audiovisualTypes] };
        types.map_or_else(Vec::new, |types| types.iter().map(|id| id.to_string()).collect())
    })
}

/// Whether AVFoundation reads this kind of file, by its name (a .mov, an .mp3; not an .mkv). The file itself is asked
/// with [`playable`] once its copy is there.
pub(crate) fn plays_type(name: &str) -> bool {
    type_of(name).is_some_and(|kind| av_types().iter().any(|id| conforms(&kind, id)))
}

/// Whether AVFoundation plays this file (`AVURLAsset.isPlayable`). Reads the file's header; a worker may ask.
pub(crate) fn playable(file: &Path) -> bool {
    let Some(class) = AnyClass::get(c"AVURLAsset") else {
        return false;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    // SAFETY: `+[AVURLAsset URLAssetWithURL:options:]` takes an NSURL and a nullable NSDictionary.
    let asset: Option<Retained<AnyObject>> =
        unsafe { msg_send![class, URLAssetWithURL: &*url, options: Option::<&AnyObject>::None] };
    // SAFETY: `-[AVAsset isPlayable]` returns a BOOL.
    asset.is_some_and(|asset| unsafe { msg_send![&*asset, isPlayable] })
}

/// Whether Quick Look draws a preview of this kind of file, by its name.
pub(crate) fn quicklook_shows_type(name: &str) -> bool {
    type_of(name).is_some_and(|kind| {
        QUICK_LOOK_SHOWS.iter().any(|id| conforms(&kind, id)) && !QUICK_LOOK_CANNOT.iter().any(|id| conforms(&kind, id))
    })
}

/// What hosts a file inside the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hosted {
    /// `AVPlayerView`: a recording or a video, with the system's playback controls.
    Player,
    /// `QLPreviewView`: a document.
    QuickLook,
}

/// The view inside the window now, and the file it shows.
struct Embedded {
    file: PathBuf,
    view: Retained<NSView>,
    /// A player view's `AVPlayer`: paused and emptied when the view goes.
    player: Option<Retained<AnyObject>>,
}

thread_local! {
    /// The one hosted view (the main thread's).
    static EMBEDDED: RefCell<Option<Embedded>> = const { RefCell::new(None) };
}

/// The app's window: the main one, else the key one, else the first one on screen that can be main (the app was
/// not in front when the file was ready).
fn app_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    app.mainWindow().or_else(|| app.keyWindow()).or_else(|| {
        app.windows()
            .iter()
            .find(|window| window.isVisible() && window.canBecomeMainWindow())
    })
}

/// Where a rect the toolkit laid out (`[x, y, w, h]` in window points from the top left) is in a parent view
/// `height` points tall: as it is in a flipped parent, measured from the bottom in any other. `[x, y, w, h]`.
pub(crate) fn frame_in(height: f64, flipped: bool, rect: [f32; 4]) -> [f64; 4] {
    let [x, y, w, h] = rect.map(f64::from);
    [x, if flipped { y } else { height - y - h }, w.max(0.0), h.max(0.0)]
}

/// An `AVPlayerView` on `url` with its player, already playing.
fn player_view(url: &NSURL, frame: NSRect) -> Option<(Retained<NSView>, Option<Retained<AnyObject>>)> {
    let (players, views) = (AnyClass::get(c"AVPlayer")?, AnyClass::get(c"AVPlayerView")?);
    // SAFETY: `+[AVPlayer playerWithURL:]` takes an NSURL and returns a player.
    let player: Option<Retained<AnyObject>> = unsafe { msg_send![players, playerWithURL: url] };
    let player = player?;
    // SAFETY: AVPlayerView is an NSView; `-initWithFrame:` is its designated initialiser. On the main thread (the
    // caller holds its marker).
    let view: Option<Retained<NSView>> = unsafe {
        let view: Allocated<NSView> = msg_send![views, alloc];
        msg_send![view, initWithFrame: frame]
    };
    let view = view?;
    // SAFETY: `-[AVPlayerView setPlayer:]` takes an AVPlayer (the view retains it); `-[AVPlayer play]` takes nothing.
    unsafe {
        let _: () = msg_send![&*view, setPlayer: &*player];
        let _: () = msg_send![&*player, play];
    }
    Some((view, Some(player)))
}

/// A `QLPreviewView` on `url`.
fn quick_look_view(url: &NSURL, frame: NSRect) -> Option<Retained<NSView>> {
    let views = AnyClass::get(c"QLPreviewView")?;
    /// `QLPreviewViewStyleNormal`.
    const NORMAL: NSUInteger = 0;
    // SAFETY: QLPreviewView is an NSView; `-initWithFrame:style:` is its designated initialiser (nil when the style
    // is not known). On the main thread.
    let view: Option<Retained<NSView>> = unsafe {
        let view: Allocated<NSView> = msg_send![views, alloc];
        msg_send![view, initWithFrame: frame, style: NORMAL]
    };
    let view = view?;
    // SAFETY: `-[QLPreviewView setPreviewItem:]` takes a preview item; an NSURL is one (QuickLookUI's category).
    let _: () = unsafe { msg_send![&*view, setPreviewItem: url] };
    Some(view)
}

/// Takes the hosted view out of the window, when it shows `file` (`None`: whatever it shows). A player stops and
/// lets go of the file; the keyboard goes back to the window's own view.
fn unembed(file: Option<&Path>) {
    let taken = EMBEDDED.with(|slot| {
        let mut slot = slot.borrow_mut();
        let ours = slot.as_ref().is_some_and(|on| file.is_none_or(|file| on.file == file));
        if ours { slot.take() } else { None }
    });
    let Some(on) = taken else {
        return;
    };
    // SAFETY: AVPlayer's and QLPreviewView's own messages, on the main thread (the slot is the main thread's).
    unsafe {
        if let Some(player) = &on.player {
            let _: () = msg_send![&**player, pause];
            let _: () = msg_send![&**player, replaceCurrentItemWithPlayerItem: Option::<&AnyObject>::None];
        } else {
            let _: () = msg_send![&*on.view, close];
        }
    }
    let window = on.view.window();
    on.view.removeFromSuperview();
    if let Some(window) = window {
        let content = window.contentView();
        let responder: Option<&NSResponder> = content.as_deref().map(|view| &**view);
        window.makeFirstResponder(responder);
    }
}

/// Hosts `file` inside the app's window, in place of what was hosted: hidden until [`place`] says where. `false`: not
/// on the main thread, no window, or the system has no such view (the caller falls back to the panel).
pub(crate) fn embed(kind: Hosted, file: &Path) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    unembed(None);
    let Some(content) = app_window(mtm).and_then(|window| window.contentView()) else {
        return false;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    let frame = content.bounds();
    let made = match kind {
        Hosted::Player => player_view(&url, frame),
        Hosted::QuickLook => quick_look_view(&url, frame).map(|view| (view, None)),
    };
    let Some((view, player)) = made else {
        return false;
    };
    view.setHidden(true);
    content.addSubview(&view);
    EMBEDDED.with(|slot| {
        *slot.borrow_mut() = Some(Embedded {
            file: file.to_path_buf(),
            view,
            player,
        });
    });
    true
}

/// Puts the view that shows `file` at `rect` (`[x, y, w, h]` in window points from the top left, as the toolkit
/// lays the viewer's body out). `None`, or a rect with no room: the view is hidden (something the toolkit draws is
/// over that place; a player keeps playing). Nothing when `file` is not the one hosted.
pub(crate) fn place(file: &Path, rect: Option<[f32; 4]>) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    EMBEDDED.with(|slot| {
        let slot = slot.borrow();
        let Some(on) = slot.as_ref().filter(|on| on.file == file) else {
            return;
        };
        // SAFETY: `-[NSView superview]` on the main thread; the parent is alive while the view is in it.
        let parent = unsafe { on.view.superview() };
        let (Some(rect), Some(parent)) = (rect.filter(|rect| rect[2] >= 1.0 && rect[3] >= 1.0), parent) else {
            on.view.setHidden(true);
            return;
        };
        let [x, y, w, h] = frame_in(parent.bounds().size.height, parent.isFlipped(), rect);
        on.view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
        on.view.setHidden(false);
    });
}

/// Whether the view hosted inside the window shows `file` (and is still in a window).
fn embedded(file: &Path) -> bool {
    EMBEDDED.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|on| on.file == file && on.view.window().is_some())
    })
}

/// The file the panel shows now.
struct Shown {
    url: RefCell<Option<Retained<NSURL>>>,
}

define_class!(
    // SAFETY: NSResponder has no subclassing requirements, and this class has no `Drop`.
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "BeebeebQuickLookControl"]
    #[ivars = Shown]
    struct Control;

    /// The panel's controller (it asks the responder chain for one) and its data source: one file, or none.
    impl Control {
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts(&self, _panel: &AnyObject) -> bool {
            self.ivars().url.borrow().is_some()
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin(&self, panel: &AnyObject) {
            // SAFETY: `-[QLPreviewPanel setDataSource:]` takes an object; this one lives as long as the thread.
            let _: () = unsafe { msg_send![panel, setDataSource: self] };
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end(&self, _panel: &AnyObject) {}

        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: &AnyObject) -> NSInteger {
            NSInteger::from(self.ivars().url.borrow().is_some())
        }

        // An NSURL is a preview item (QuickLookUI's category on it).
        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: &AnyObject, _index: NSInteger) -> Option<Retained<NSURL>> {
            self.ivars().url.borrow().clone()
        }
    }
);

impl Control {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Shown {
            url: RefCell::new(None),
        });
        // SAFETY: NSResponder's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// The one controller, made at the first preview and kept: the panel holds it without retaining it.
    static CONTROL: RefCell<Option<Retained<Control>>> = const { RefCell::new(None) };
    /// The window whose responder chain holds the controller.
    static HOME: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

fn control(mtm: MainThreadMarker) -> Retained<Control> {
    CONTROL.with(|slot| slot.borrow_mut().get_or_insert_with(|| Control::new(mtm)).clone())
}

fn same(responder: Option<&NSResponder>, control: &Control) -> bool {
    responder.is_some_and(|responder| std::ptr::eq(responder, &raw const **control))
}

/// Puts the controller after the app's main window in the responder chain, where the panel looks for it. A window it
/// sat behind before gets its own next responder back.
fn settle(control: &Control, mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.mainWindow().or_else(|| app.keyWindow()) else {
        return;
    };
    // SAFETY: plain responder-chain reads and writes on the main thread; every object is alive here.
    unsafe {
        if same(window.nextResponder().as_deref(), control) {
            return;
        }
        if let Some(before) = HOME.with(|home| home.borrow_mut().take())
            && same(before.nextResponder().as_deref(), control)
        {
            before.setNextResponder(control.nextResponder().as_deref());
        }
        control.setNextResponder(window.nextResponder().as_deref());
        window.setNextResponder(Some(control));
    }
    HOME.with(|home| *home.borrow_mut() = Some(window));
}

fn panel() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"QLPreviewPanel")?;
    // SAFETY: `+[QLPreviewPanel sharedPreviewPanel]` returns the panel.
    unsafe { msg_send![class, sharedPreviewPanel] }
}

/// The panel, only when it was made already (asking for it makes it).
fn panel_if_made() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"QLPreviewPanel")?;
    // SAFETY: `+[QLPreviewPanel sharedPreviewPanelExists]` returns a BOOL.
    let made: bool = unsafe { msg_send![class, sharedPreviewPanelExists] };
    made.then(panel).flatten()
}

/// Opens the Quick Look panel on `file`, in place of what it showed. `false`: not on the main thread, or no panel.
pub(crate) fn show(file: &Path) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(panel) = panel() else {
        return false;
    };
    let control = control(mtm);
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    *control.ivars().url.borrow_mut() = Some(url);
    settle(&control, mtm);
    // SAFETY: QLPreviewPanel's own messages, on the main thread. The data source is not retained by the panel; the
    // controller is kept for the thread's life (`CONTROL`).
    unsafe {
        let _: () = msg_send![&*panel, makeKeyAndOrderFront: Option::<&AnyObject>::None];
        let _: () = msg_send![&*panel, updateController];
        // No window of ours is main (the app sits in the menu bar): the panel found no controller, so it is handed
        // its data source directly.
        let found: Option<Retained<AnyObject>> = msg_send![&*panel, currentController];
        if found.is_none() {
            let _: () = msg_send![&*panel, setDataSource: &*control];
        }
        let _: () = msg_send![&*panel, reloadData];
    }
    true
}

/// Whether `file` is on screen: in the view inside the window, or in the panel.
pub(crate) fn showing(file: &Path) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    if embedded(file) {
        return true;
    }
    let ours = CONTROL.with(|slot| {
        slot.borrow().as_ref().is_some_and(|control| {
            let held = control.ivars().url.borrow();
            held.as_ref()
                .and_then(|url| url.path())
                .is_some_and(|path| Path::new(&path.to_string()) == file)
        })
    });
    // SAFETY: `-[NSWindow isVisible]` returns a BOOL.
    ours && panel_if_made().is_some_and(|panel| unsafe { msg_send![&*panel, isVisible] })
}

fn close_now(file: &Path) {
    unembed(Some(file));
    let Some(control) = CONTROL.with(|slot| slot.borrow().clone()) else {
        return;
    };
    let ours = control
        .ivars()
        .url
        .borrow()
        .as_ref()
        .and_then(|url| url.path())
        .is_some_and(|path| Path::new(&path.to_string()) == file);
    // Another file took the panel since: it is that file's now.
    if !ours {
        return;
    }
    *control.ivars().url.borrow_mut() = None;
    if let Some(panel) = panel_if_made() {
        // SAFETY: QLPreviewPanel's own messages, on the main thread.
        unsafe {
            let _: () = msg_send![&*panel, reloadData];
            let _: () = msg_send![&*panel, close];
        }
    }
}

/// Takes `file` off the screen (the view inside the window, the panel) and lets go of it. From any thread (it hops
/// to the main one).
pub(crate) fn close(file: &Path) {
    if MainThreadMarker::new().is_some() {
        close_now(file);
        return;
    }
    extern "C" fn run(ctx: *mut c_void) {
        // SAFETY: `ctx` is the box leaked below, handed over once.
        let file = unsafe { Box::from_raw(ctx.cast::<std::path::PathBuf>()) };
        close_now(&file);
    }
    let ctx = Box::into_raw(Box::new(file.to_path_buf()));
    // SAFETY: the main queue outlives the process's threads; `run` takes the box back.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, ctx.cast(), run) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../beebeeb-media-helper/tests/fixtures")
            .join(name)
    }

    #[test]
    fn avfoundation_plays_a_mov_and_not_an_unknown_file() {
        assert!(plays_type("clip.mov") && plays_type("clip.MP4") && plays_type("song.m4a"));
        assert!(!plays_type("thing.xyz") && !plays_type("film.mkv") && !plays_type("no-extension"));
        assert!(playable(&fixture("hevc-main10.mov")), "the .mov fixture plays");
        let dir = tempfile::tempdir().unwrap();
        let unknown = dir.path().join("thing.xyz");
        std::fs::write(&unknown, b"invented bytes, no media in them").unwrap();
        assert!(!playable(&unknown));
        assert!(!playable(&dir.path().join("not-there.mov")));
    }

    #[test]
    fn quick_look_shows_documents_and_pictures_and_not_what_macos_cannot_draw() {
        for name in [
            "a.docx",
            "a.xlsx",
            "a.pptx",
            "a.doc",
            "a.pages",
            "a.numbers",
            "a.key",
            "a.rtf",
            "a.psd",
            "a.heic",
        ] {
            assert!(quicklook_shows_type(name), "{name}");
        }
        // No PostScript converter on macOS 14 and later; an archive and an unknown type get an icon, not a preview.
        for name in ["a.eps", "a.ps", "a.zip", "a.xyz", "a.mkv", "noext"] {
            assert!(!quicklook_shows_type(name), "{name}");
        }
    }

    #[test]
    fn the_panel_and_the_views_are_the_main_threads() {
        // A test runs on a thread of its own: nothing opens, and a close of a file never shown is harmless.
        let file = Path::new("/nonexistent/invented.docx");
        assert!(!show(file));
        assert!(!embed(Hosted::QuickLook, file) && !embed(Hosted::Player, file));
        place(file, Some([0.0, 0.0, 10.0, 10.0]));
        assert!(!showing(file));
    }

    #[test]
    fn the_systems_views_are_there() {
        // AVKit and Quartz are linked: the classes the views are made of answer by name.
        assert!(AnyClass::get(c"AVPlayerView").is_some() && AnyClass::get(c"AVPlayer").is_some());
        assert!(AnyClass::get(c"QLPreviewView").is_some());
    }

    #[test]
    fn a_laid_out_rect_is_measured_from_the_bottom_in_a_view_that_is_not_flipped() {
        // The viewer's body under a 96-point toolbar in a window 800 points tall.
        let rect = [240.0, 96.0, 900.0, 600.0];
        assert_eq!(frame_in(800.0, true, rect), [240.0, 96.0, 900.0, 600.0]);
        assert_eq!(frame_in(800.0, false, rect), [240.0, 104.0, 900.0, 600.0]);
        // The details panel takes 320 points: the same top, a narrower view.
        assert_eq!(
            frame_in(800.0, false, [240.0, 96.0, 580.0, 600.0]),
            [240.0, 104.0, 580.0, 600.0]
        );
    }
}
