//! macOS menu-bar b (DM-1201…): an `NSStatusItem` with the template image (macOS tints it), the model's tooltip, a
//! left click that reports [`TrayEvent::Activate`] with where the item is, and a right click (or ctrl-click) that opens
//! the model's menu. The popover is the flyout window; its placement is `shell/flyout.rs`'s.
//!
//! Everything AppKit runs on the main thread (`on_main`); the click and menu callbacks arrive there too and only hand
//! the event on. The Objective-C runtime is called directly, like `macos_panels.rs`: no new crate.
//!
//! Pure and tested (`macos_tray_tests.rs`): the frame → [`Anchor`] conversion (AppKit's origin is bottom-left of the
//! primary display, the flyout's is top-left), the appearance → theme pick, the menu spec from the model.

use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::shell::flyout::{Anchor, Rect};
use crate::shell::platform::{PlatformError, TrayEvent, TrayHandler, TrayIcon};
use crate::shell::tray::{MenuAction, MenuEntry, TrayModel};
use crate::shell::tray_icon::TaskbarTheme;

static TEMPLATE_1X: &[u8] = include_bytes!("../../../../../packaging/macos/tray-template.png");
static TEMPLATE_2X: &[u8] = include_bytes!("../../../../../packaging/macos/tray-template@2x.png");
/// The template's size in points (the 1x PNG is 22 px).
const ICON_POINTS: f64 = 22.0;

// ---- pure ---------------------------------------------------------------------------------------------------------

/// An AppKit rectangle: points, origin bottom-left of the primary display.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub(super) struct NsRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Top-left-origin rectangle of `r`, given the primary display's height (the flip's axis).
fn flip(r: NsRect, primary_height: f64) -> Rect {
    Rect {
        x: r.x.round() as i32,
        y: (primary_height - (r.y + r.h)).round() as i32,
        w: r.w.round() as i32,
        h: r.h.round() as i32,
    }
}

/// The anchor for a click: the item's frame, the display it is on and that display's visible frame, all flipped to
/// top-left origin, logical points.
pub(super) fn anchor_from_frames(
    item: NsRect,
    screen: NsRect,
    visible: NsRect,
    primary_height: f64,
    scale: f64,
) -> Anchor {
    Anchor {
        monitor: flip(screen, primary_height),
        work: flip(visible, primary_height),
        scale,
        icon: Some(flip(item, primary_height)),
    }
}

/// The menu bar's mode from the appearance name (`NSAppearanceNameDarkAqua`, `…VibrantDark`, `…Aqua`).
pub(super) fn theme_from_appearance(name: &str) -> TaskbarTheme {
    if name.contains("Dark") {
        TaskbarTheme::Dark
    } else {
        TaskbarTheme::Light
    }
}

/// One line of the b's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MenuSpec {
    Item {
        action: MenuAction,
        title: String,
        enabled: bool,
    },
    Separator,
}

pub(super) fn menu_spec(model: &TrayModel) -> Vec<MenuSpec> {
    model
        .menu
        .iter()
        .map(|entry| match entry {
            MenuEntry::Separator => MenuSpec::Separator,
            MenuEntry::Item { action, label, enabled } => MenuSpec::Item {
                action: *action,
                title: label.clone(),
                enabled: *enabled,
            },
        })
        .collect()
}

/// Ctrl-click is a right click on a Mac.
pub(super) fn is_menu_click(event_type: u64, modifier_flags: u64) -> bool {
    const RIGHT_MOUSE_UP: u64 = 4;
    const CONTROL: u64 = 1 << 18;
    event_type == RIGHT_MOUSE_UP || modifier_flags & CONTROL != 0
}

// ---- Objective-C runtime ------------------------------------------------------------------------------------------

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(class: Id);
    fn class_addMethod(class: Id, sel: Sel, imp: *const c_void, types: *const c_char) -> bool;
}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_sync_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> c_int;
}

fn sel(name: &str) -> Sel {
    let c = CString::new(name).expect("selector");
    unsafe { sel_registerName(c.as_ptr()) }
}
fn class(name: &str) -> Id {
    let c = CString::new(name).expect("class");
    unsafe { objc_getClass(c.as_ptr()) }
}

macro_rules! send {
    ($ret:ty; $obj:expr, $name:expr $(, $arg:expr => $t:ty)*) => {{
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f($obj, sel($name) $(, $arg)*) }
    }};
}

/// A message that returns an `NSRect` (a struct returned in memory on x86_64).
fn send_rect(obj: Id, name: &str) -> NsRect {
    #[cfg(target_arch = "x86_64")]
    {
        let f: unsafe extern "C" fn(*mut NsRect, Id, Sel) =
            unsafe { std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn()) };
        let mut out = NsRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        unsafe { f(&mut out, obj, sel(name)) };
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> NsRect =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f(obj, sel(name)) }
    }
}

fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).expect("nul removed");
    let p = c.as_ptr();
    send!(Id; class("NSString"), "stringWithUTF8String:", p => *const c_char)
}

fn string_of(ns: Id) -> String {
    if ns.is_null() {
        return String::new();
    }
    let c = send!(*const c_char; ns, "UTF8String");
    if c.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(c) }.to_string_lossy().into_owned()
}

/// Runs `f` on the main thread and returns its result.
fn on_main<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if unsafe { pthread_main_np() } == 1 {
        return f();
    }
    extern "C" fn run(ctx: *mut c_void) {
        let job = unsafe { &mut *(ctx as *mut Option<Box<dyn FnOnce() + '_>>) };
        if let Some(f) = job.take() {
            f();
        }
    }
    let mut out: Option<T> = None;
    {
        let slot = &mut out;
        let mut job: Option<Box<dyn FnOnce() + '_>> = Some(Box::new(move || *slot = Some(f())));
        // dispatch_sync_f returns only after `run` ended, so the borrows above outlive the call.
        unsafe { dispatch_sync_f(&raw const _dispatch_main_q, &mut job as *mut _ as *mut c_void, run) };
    }
    out.expect("main queue ran the job")
}

// ---- the status item ----------------------------------------------------------------------------------------------

/// What the callbacks (which carry no Rust state) reach: the one b of the process.
struct Shared {
    on_event: TrayHandler,
    /// The menu's actions by item tag.
    actions: Mutex<Vec<MenuAction>>,
    menu: Mutex<Vec<MenuSpec>>,
    /// The status item (`NSStatusItem`), null once removed.
    item: Mutex<usize>,
    present: AtomicBool,
}

static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);
static TARGET: OnceLock<usize> = OnceLock::new();

fn shared() -> Option<Arc<Shared>> {
    SHARED.lock().ok().and_then(|g| g.clone())
}

/// The object that receives the button's action and the menu's: a class made once.
#[allow(unused_unsafe)] // a send! nested in another expands inside its unsafe block
fn target() -> Id {
    *TARGET.get_or_init(|| {
        let name = CString::new("BBMenuBarTarget").expect("name");
        let cls = unsafe { objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0) };
        let types = CString::new("v@:@").expect("types");
        unsafe {
            class_addMethod(cls, sel("clicked:"), clicked as *const c_void, types.as_ptr());
            class_addMethod(cls, sel("picked:"), picked as *const c_void, types.as_ptr());
            objc_registerClassPair(cls);
        }
        let obj = send!(Id; send!(Id; cls, "alloc"), "init");
        obj as usize
    }) as Id
}

fn primary_height() -> f64 {
    let screens = send!(Id; class("NSScreen"), "screens");
    if send!(c_ulong; screens, "count") == 0 {
        return 0.0;
    }
    let primary = send!(Id; screens, "objectAtIndex:", 0 as c_ulong => c_ulong);
    send_rect(primary, "frame").h
}

/// The anchor of the menu-bar screen with no icon: the popover then opens at its top right, never centred.
fn fallback_anchor() -> Option<Anchor> {
    let screens = send!(Id; class("NSScreen"), "screens");
    if send!(c_ulong; screens, "count") == 0 {
        return None;
    }
    let primary = send!(Id; screens, "objectAtIndex:", 0 as c_ulong => c_ulong);
    let scale = send!(f64; primary, "backingScaleFactor");
    Some(anchor_from_frames(
        NsRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        },
        send_rect(primary, "frame"),
        send_rect(primary, "visibleFrame"),
        primary_height(),
        scale,
    ))
    .map(|a| Anchor { icon: None, ..a })
}

/// The anchor is always there: the item's frame when AppKit gives it, else the menu-bar screen's top right.
fn current_anchor(button: Id) -> Option<Anchor> {
    let window = send!(Id; button, "window");
    if window.is_null() {
        return fallback_anchor();
    }
    let screen = send!(Id; window, "screen");
    if screen.is_null() {
        return fallback_anchor();
    }
    let item = send_rect(window, "frame");
    let scale = send!(f64; screen, "backingScaleFactor");
    Some(anchor_from_frames(
        item,
        send_rect(screen, "frame"),
        send_rect(screen, "visibleFrame"),
        primary_height(),
        scale,
    ))
}

/// The popover's native look, once its window exists and is still hidden: transparent window with a system
/// shadow, content rounded like a macOS popover, above other windows, off Mission Control and the window list.
#[allow(unused_unsafe)] // a send! nested in another expands inside its unsafe block
pub fn adopt_popover(ns_view: *mut c_void) {
    if ns_view.is_null() {
        return;
    }
    let view = ns_view as usize;
    on_main(move || {
        let view = view as Id;
        let window = send!(Id; view, "window");
        if window.is_null() {
            return;
        }
        const POPOVER_CORNER_RADIUS: f64 = 10.0;
        const STATUS_WINDOW_LEVEL: isize = 25;
        // canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
        const BEHAVIOR: c_ulong = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
        send!((); window, "setOpaque:", false => bool);
        send!((); window, "setBackgroundColor:", send!(Id; class("NSColor"), "clearColor") => Id);
        send!((); window, "setHasShadow:", true => bool);
        send!((); window, "setLevel:", STATUS_WINDOW_LEVEL => isize);
        send!((); window, "setCollectionBehavior:", BEHAVIOR => c_ulong);
        send!((); view, "setWantsLayer:", true => bool);
        let layer = send!(Id; view, "layer");
        if !layer.is_null() {
            send!((); layer, "setCornerRadius:", POPOVER_CORNER_RADIUS => f64);
            send!((); layer, "setMasksToBounds:", true => bool);
        }
        send!((); window, "invalidateShadow");
    });
}

extern "C" fn clicked(_this: Id, _cmd: Sel, sender: Id) {
    let Some(shared) = shared() else { return };
    let app = send!(Id; class("NSApplication"), "sharedApplication");
    let event = send!(Id; app, "currentEvent");
    let (ty, flags) = if event.is_null() {
        (2, 0)
    } else {
        (
            send!(c_ulong; event, "type") as u64,
            send!(c_ulong; event, "modifierFlags") as u64,
        )
    };
    if is_menu_click(ty, flags) {
        show_menu(&shared, sender);
    } else {
        (shared.on_event)(TrayEvent::Activate {
            anchor: current_anchor(sender),
        });
    }
}

extern "C" fn picked(_this: Id, _cmd: Sel, sender: Id) {
    let Some(shared) = shared() else { return };
    let tag = send!(isize; sender, "tag");
    let action = shared
        .actions
        .lock()
        .ok()
        .and_then(|a| a.get(usize::try_from(tag).ok()?).copied());
    if let Some(action) = action {
        (shared.on_event)(TrayEvent::Menu(action));
    }
}

/// Opens the menu under the item: assigned for one click, then taken off again so a left click still reaches the action.
#[allow(unused_unsafe)] // a send! nested in another expands inside its unsafe block
fn show_menu(shared: &Shared, button: Id) {
    let spec = shared.menu.lock().map(|m| m.clone()).unwrap_or_default();
    let menu = send!(Id; send!(Id; class("NSMenu"), "alloc"), "init");
    send!((); menu, "setAutoenablesItems:", false => bool);
    let mut actions = Vec::new();
    for line in &spec {
        match line {
            MenuSpec::Separator => {
                let sep = send!(Id; class("NSMenuItem"), "separatorItem");
                send!((); menu, "addItem:", sep => Id);
            }
            MenuSpec::Item { action, title, enabled } => {
                let item = send!(Id; class("NSMenuItem"), "alloc");
                let item = send!(Id; item, "initWithTitle:action:keyEquivalent:",
                    nsstring(title) => Id, sel("picked:") => Sel, nsstring("") => Id);
                send!((); item, "setTarget:", target() => Id);
                send!((); item, "setTag:", actions.len() as isize => isize);
                send!((); item, "setEnabled:", *enabled => bool);
                actions.push(*action);
                send!((); menu, "addItem:", item => Id);
            }
        }
    }
    if let Ok(mut a) = shared.actions.lock() {
        *a = actions;
    }
    let item = *shared.item.lock().expect("item");
    if item == 0 {
        return;
    }
    let item = item as Id;
    send!((); item, "setMenu:", menu => Id);
    send!((); button, "performClick:", std::ptr::null_mut::<c_void>() => Id);
    send!((); item, "setMenu:", std::ptr::null_mut::<c_void>() => Id);
}

#[allow(unused_unsafe)] // a send! nested in another expands inside its unsafe block
fn template_image() -> Id {
    let data = |bytes: &[u8]| send!(Id; class("NSData"), "dataWithBytes:length:", bytes.as_ptr() as *const c_void => *const c_void, bytes.len() as c_ulong => c_ulong);
    let image = send!(Id; send!(Id; class("NSImage"), "alloc"), "initWithData:", data(TEMPLATE_1X) => Id);
    let rep2x = send!(Id; class("NSBitmapImageRep"), "imageRepWithData:", data(TEMPLATE_2X) => Id);
    if !image.is_null() {
        if !rep2x.is_null() {
            // The 44 px bitmap is the same 22 pt image at 2x.
            send!((); rep2x, "setSize:", NsSize { w: ICON_POINTS, h: ICON_POINTS } => NsSize);
            send!((); image, "addRepresentation:", rep2x => Id);
        }
        send!((); image, "setSize:", NsSize { w: ICON_POINTS, h: ICON_POINTS } => NsSize);
        send!((); image, "setTemplate:", true => bool);
    }
    image
}

#[derive(Clone, Copy)]
#[repr(C)]
struct NsSize {
    w: f64,
    h: f64,
}

fn set_tooltip(item: Id, tooltip: &str) {
    let button = send!(Id; item, "button");
    if !button.is_null() {
        send!((); button, "setToolTip:", nsstring(tooltip) => Id);
    }
}

struct MacTray {
    shared: Arc<Shared>,
}

impl MacTray {
    fn remove(&self) {
        let item = std::mem::replace(&mut *self.shared.item.lock().expect("item"), 0);
        self.shared.present.store(false, Ordering::SeqCst);
        if item != 0 {
            on_main(|| {
                let bar = send!(Id; class("NSStatusBar"), "systemStatusBar");
                send!((); bar, "removeStatusItem:", item as Id => Id);
                send!((); item as Id, "release");
            });
        }
    }
}

impl TrayIcon for MacTray {
    fn apply(&self, model: &TrayModel) {
        if let Ok(mut m) = self.shared.menu.lock() {
            *m = menu_spec(model);
        }
        let item = *self.shared.item.lock().expect("item");
        if item != 0 {
            let tip = model.tooltip.clone();
            on_main(move || set_tooltip(item as Id, &tip));
        }
    }
    fn present(&self) -> bool {
        self.shared.present.load(Ordering::SeqCst)
    }
    fn remove_for_quit(&self) {
        self.remove();
    }
}

impl Drop for MacTray {
    fn drop(&mut self) {
        self.remove();
        if let Ok(mut g) = SHARED.lock() {
            *g = None;
        }
    }
}

pub fn create_tray(model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
    let shared = Arc::new(Shared {
        on_event,
        actions: Mutex::new(Vec::new()),
        menu: Mutex::new(menu_spec(model)),
        item: Mutex::new(0),
        present: AtomicBool::new(false),
    });
    *SHARED
        .lock()
        .map_err(|_| PlatformError::Os("tray state poisoned".into()))? = Some(shared.clone());
    let tip = model.tooltip.clone();
    let item = on_main(move || {
        const VARIABLE_LENGTH: f64 = -1.0;
        let bar = send!(Id; class("NSStatusBar"), "systemStatusBar");
        let item = send!(Id; bar, "statusItemWithLength:", VARIABLE_LENGTH => f64);
        if item.is_null() {
            return 0usize;
        }
        send!(Id; item, "retain");
        let button = send!(Id; item, "button");
        if button.is_null() {
            return 0usize;
        }
        send!((); button, "setImage:", template_image() => Id);
        send!((); button, "setTarget:", target() => Id);
        send!((); button, "setAction:", sel("clicked:") => Sel);
        // Left and right mouse up (NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp).
        send!(c_ulong; button, "sendActionOn:", ((1u64 << 2) | (1u64 << 4)) as c_ulong => c_ulong);
        set_tooltip(item, &tip);
        item as usize
    });
    if item == 0 {
        if let Ok(mut g) = SHARED.lock() {
            *g = None;
        }
        return Err(PlatformError::Os("the menu bar gave no status item".into()));
    }
    *shared.item.lock().expect("item") = item;
    shared.present.store(true, Ordering::SeqCst);
    Ok(Box::new(MacTray { shared }))
}

/// The menu bar's mode: the app's effective appearance follows the system's (light or dark menu bar).
pub fn taskbar_theme() -> TaskbarTheme {
    on_main(|| {
        let app = send!(Id; class("NSApplication"), "sharedApplication");
        let appearance = if app.is_null() {
            std::ptr::null_mut()
        } else {
            send!(Id; app, "effectiveAppearance")
        };
        if appearance.is_null() {
            return TaskbarTheme::Dark;
        }
        theme_from_appearance(&string_of(send!(Id; appearance, "name")))
    })
}

#[cfg(test)]
#[path = "macos_tray_tests.rs"]
mod tests;
