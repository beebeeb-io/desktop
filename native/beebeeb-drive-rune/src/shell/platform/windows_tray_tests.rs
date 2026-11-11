//! A harness for the tray's window procedure: a real hidden window on the test's own thread,
//! a fake shell that re-enters the procedure the way Windows does, and no notification area
//! (the build VM has a desktop only at its console).

use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsWindow, PM_REMOVE, PeekMessageW, SendMessageW, WM_QUERYENDSESSION,
};

use super::*;
use crate::shell::tray::{EngineState, TrayInputs, tray_model};

#[derive(Default)]
struct Fake {
    hwnd: Cell<isize>,
    /// Every `Shell_NotifyIconW` message, in order.
    calls: RefCell<Vec<u32>>,
    add_fails: Cell<bool>,
    modify_fails: Cell<bool>,
    /// Deliver a timer tick and a settings change INSIDE each notify call, as a
    /// cross-process send does. (The nested messages' own notify calls do not nest again.)
    reenter_on_notify: Cell<bool>,
    inside_notify: Cell<bool>,
    /// How deep `Shell_NotifyIconW` calls are nested right now, and the deepest it got: a
    /// refresh that ran INSIDE another refresh's notify call shows as 2.
    notify_depth: Cell<u32>,
    max_notify_depth: Cell<u32>,
    /// How many times a message was delivered inside a notify call.
    reentries: Cell<u32>,
    /// What the "menu" returns, after pumping this thread's queue like its modal loop.
    menu_pick: Cell<u32>,
    menus_shown: Cell<u32>,
    /// `Shell_NotifyIconW` calls made while the menu was open.
    calls_during_menu: Cell<usize>,
}

struct FakeShell(Rc<Fake>);

struct Leave<'a>(&'a Cell<u32>);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

fn pump() {
    // SAFETY: drains this thread's own queue; MSG is plain data filled by PeekMessageW.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn send(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: a same-thread send to the harness's own live window.
    unsafe { SendMessageW(hwnd, message, w, l) }
}

impl ShellCalls for FakeShell {
    fn notify(&self, message: u32, _data: &NOTIFYICONDATAW) -> bool {
        self.0.calls.borrow_mut().push(message);
        let depth = self.0.notify_depth.get() + 1;
        self.0.notify_depth.set(depth);
        self.0.max_notify_depth.set(self.0.max_notify_depth.get().max(depth));
        let _leave = Leave(&self.0.notify_depth);
        if self.0.reenter_on_notify.get() && !self.0.inside_notify.replace(true) {
            let hwnd = self.0.hwnd.get() as HWND;
            send(hwnd, WM_TIMER, TIMER_ANIMATE, 0);
            send(hwnd, WM_SETTINGCHANGE, 0, 0);
            self.0.reentries.set(self.0.reentries.get() + 2);
            self.0.inside_notify.set(false);
        }
        match message {
            NIM_ADD => !self.0.add_fails.get(),
            NIM_MODIFY => !self.0.modify_fails.get(),
            _ => true,
        }
    }

    fn track_menu(&self, hwnd: HWND, _popup: HMENU, _at: POINT) -> u32 {
        self.0.menus_shown.set(self.0.menus_shown.get() + 1);
        let before = self.0.calls.borrow().len();
        // What the menu's modal loop dispatches while files sync: animation ticks and a
        // model update, both to this window, both while `pick` is on the stack.
        // SAFETY: posts to the harness's own live window.
        unsafe {
            PostMessageW(hwnd, WM_TIMER, TIMER_ANIMATE, 0);
            PostMessageW(hwnd, UPDATE, 0, 0);
            PostMessageW(hwnd, WM_TIMER, TIMER_ANIMATE, 0);
        }
        pump();
        self.0.calls_during_menu.set(self.0.calls.borrow().len() - before);
        self.0.menu_pick.get()
    }
}

struct Harness {
    hwnd: HWND,
    context: Rc<Context>,
    fake: Rc<Fake>,
    events: Arc<Mutex<Vec<TrayEvent>>>,
    present: Arc<AtomicBool>,
    faults: Arc<AtomicU32>,
    model: Arc<Mutex<TrayModel>>,
}

impl Harness {
    fn new(name: &str, inputs: TrayInputs) -> Harness {
        let fake = Rc::new(Fake::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let handler: TrayHandler = Arc::new(move |event| sink.lock().unwrap().push(event));
        let (present, faults) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
        let model = Arc::new(Mutex::new(tray_model(&inputs)));
        let (hwnd, context) = build(
            model.clone(),
            handler,
            present.clone(),
            faults.clone(),
            Box::new(FakeShell(fake.clone())),
            &format!("BeebeebTrayHarness-{name}-{}", std::process::id()),
        )
        .expect("a hidden window");
        fake.hwnd.set(hwnd as isize);
        Harness {
            hwnd,
            context,
            fake,
            events,
            present,
            faults,
            model,
        }
    }

    fn events(&self) -> Vec<TrayEvent> {
        self.events.lock().unwrap().clone()
    }

    fn calls(&self) -> Vec<u32> {
        self.fake.calls.borrow().clone()
    }

    fn faults(&self) -> u32 {
        self.faults.load(Ordering::SeqCst)
    }

    fn is_present(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        send(self.hwnd, SHUTDOWN, 0, 0);
        pump();
        destroy_last_icon(&self.context);
    }
}

fn syncing() -> TrayInputs {
    TrayInputs {
        signed_in: true,
        explorer_ready: true,
        engine: EngineState::Syncing,
        ..TrayInputs::default()
    }
}

fn context_menu(x: i16, y: i16) -> (WPARAM, LPARAM) {
    let w = ((y as u16 as usize) << 16) | (x as u16 as usize);
    (w, ((ICON_ID as isize) << 16) | WM_CONTEXTMENU as isize)
}

/// P1-1. A right-click while files sync: timer ticks and an update are dispatched by the
/// menu's own loop, and each `Shell_NotifyIconW` re-enters with more. A `RefCell` borrow
/// (or a `&mut Context`) held across either call panics in the nested message and is
/// counted in `faults`.
#[test]
fn messages_delivered_inside_the_menu_and_inside_notify_find_no_live_borrow() {
    let h = Harness::new("reenter", syncing());
    h.fake.reenter_on_notify.set(true);
    send(h.hwnd, UPDATE, 0, 0);
    assert!(h.is_present());
    // Quit is the 7th entry of the menu.
    h.fake.menu_pick.set(tray_win::menu_command(6) as u32);
    let (w, l) = context_menu(-300, 900);
    send(h.hwnd, CALLBACK, w, l);
    assert_eq!(h.fake.menus_shown.get(), 1);
    assert!(
        h.fake.calls_during_menu.get() >= 2,
        "the nested ticks really ran a refresh inside the menu call: {}",
        h.fake.calls_during_menu.get()
    );
    assert!(h.events().contains(&TrayEvent::Menu(MenuAction::Quit)));
    assert!(
        h.fake.reentries.get() >= 4,
        "messages really arrived inside notify calls"
    );
    assert_eq!(h.faults(), 0, "a borrow was live across a re-entrant call");
    assert!(!h.context.refreshing.get());
    assert_eq!(
        h.fake.max_notify_depth.get(),
        1,
        "a refresh asked for inside a refresh waits for it: the passes never nest"
    );
}

/// The model changes while the menu is open: the pick is read against the menu that was
/// shown, and a greyed entry of it never fires.
#[test]
fn a_pick_is_read_against_the_menu_that_was_shown() {
    let h = Harness::new("pick", TrayInputs::default());
    send(h.hwnd, UPDATE, 0, 0);
    // Signed out: "Pause syncing" (4th entry) is greyed.
    h.fake.menu_pick.set(tray_win::menu_command(3) as u32);
    let (w, l) = context_menu(10, 10);
    send(h.hwnd, CALLBACK, w, l);
    assert_eq!(h.fake.menus_shown.get(), 1);
    assert!(!h.events().iter().any(|e| matches!(e, TrayEvent::Menu(_))));
    h.fake.menu_pick.set(0);
    send(h.hwnd, CALLBACK, w, l);
    assert!(!h.events().iter().any(|e| matches!(e, TrayEvent::Menu(_))));
    assert_eq!(h.faults(), 0);
}

/// P2-1. A failed `NIM_ADD` is reported (`present` false, the retry timer armed), and the
/// icon is added when the taskbar announces itself.
#[test]
fn a_failed_add_is_not_presence_and_taskbar_created_adds_it() {
    let h = Harness::new("absent", syncing());
    h.fake.add_fails.set(true);
    send(h.hwnd, UPDATE, 0, 0);
    assert!(!h.is_present());
    assert!(h.context.snapshot().retrying, "it keeps trying");
    assert!(h.calls().contains(&NIM_ADD));
    assert!(
        !h.calls().contains(&NIM_SETVERSION),
        "no version on an icon that is not there"
    );
    assert!(h.events().is_empty(), "absent from the start: no change to report");
    // Explorer starts.
    h.fake.add_fails.set(false);
    send(h.hwnd, h.context.taskbar_created, 0, 0);
    assert!(h.is_present());
    assert!(!h.context.snapshot().retrying);
    assert_eq!(h.events(), vec![TrayEvent::Presence(true)]);
    assert!(h.calls().contains(&NIM_SETVERSION));
    // Explorer dies and restarts: delete, add and version again; presence unchanged.
    let before = h.calls().len();
    send(h.hwnd, h.context.taskbar_created, 0, 0);
    assert_eq!(&h.calls()[before..], &[NIM_DELETE, NIM_ADD, NIM_SETVERSION]);
    assert_eq!(h.events(), vec![TrayEvent::Presence(true)]);
    assert_eq!(h.faults(), 0);
}

/// A modify the shell refuses means the icon is gone: add again; if that fails too, say so.
#[test]
fn a_refused_modify_adds_again_and_reports_the_icon_gone_when_that_fails() {
    let h = Harness::new("modify", syncing());
    send(h.hwnd, UPDATE, 0, 0);
    assert!(h.is_present());
    h.fake.modify_fails.set(true);
    let before = h.calls().len();
    send(h.hwnd, WM_TIMER, TIMER_ANIMATE, 0);
    assert_eq!(&h.calls()[before..], &[NIM_MODIFY, NIM_DELETE, NIM_ADD, NIM_SETVERSION]);
    assert!(h.is_present());
    h.fake.add_fails.set(true);
    send(h.hwnd, WM_TIMER, TIMER_ANIMATE, 0);
    assert!(!h.is_present());
    assert_eq!(h.events(), vec![TrayEvent::Presence(true), TrayEvent::Presence(false)]);
    assert_eq!(h.faults(), 0);
}

/// P2-2. `WM_QUERYENDSESSION` (the quit hook removes the icon), then `WM_ENDSESSION(FALSE)`:
/// the icon is back and the app is told. A session that really ends restores nothing.
#[test]
fn a_cancelled_shutdown_puts_the_icon_back() {
    let h = Harness::new("shutdown", syncing());
    send(h.hwnd, UPDATE, 0, 0);
    assert!(h.is_present());
    assert_ne!(
        send(h.hwnd, WM_QUERYENDSESSION, 0, 0),
        0,
        "the tray never refuses a shutdown"
    );
    // What `TrayIcon::remove_for_quit` posts from the quit hook.
    // SAFETY: a post to the harness's own live window.
    unsafe { PostMessageW(h.hwnd, REMOVE_FOR_QUIT, 0, 0) };
    // The cancel arrives BEFORE this thread has pumped the removal: the restore must
    // still come after it.
    send(h.hwnd, WM_ENDSESSION, 0, 0);
    pump();
    assert!(h.is_present(), "the icon is back");
    let calls = h.calls();
    let removed = calls.iter().rposition(|c| *c == NIM_DELETE).unwrap();
    assert_eq!(&calls[removed..], &[NIM_DELETE, NIM_ADD, NIM_SETVERSION]);
    assert!(h.events().contains(&TrayEvent::ShutdownCancelled));
    assert!(!h.context.snapshot().removed_for_quit);
    assert!(unsafe { IsWindow(h.hwnd) } != 0);

    // A quit that is not cancelled: removed, and nothing puts it back.
    send(h.hwnd, REMOVE_FOR_QUIT, 0, 0);
    assert!(!h.is_present());
    send(h.hwnd, UPDATE, 0, 0);
    send(h.hwnd, WM_TIMER, TIMER_RETRY, 0);
    send(h.hwnd, h.context.taskbar_created, 0, 0);
    send(h.hwnd, WM_ENDSESSION, 1, 0);
    pump();
    assert!(!h.is_present());
    assert_eq!(h.calls().last(), Some(&NIM_DELETE));
    assert_eq!(
        h.events()
            .iter()
            .filter(|e| **e == TrayEvent::ShutdownCancelled)
            .count(),
        1
    );
    // A cancel with no removal before it restores nothing and reports nothing.
    let quiet = Harness::new("shutdown-quiet", syncing());
    send(quiet.hwnd, UPDATE, 0, 0);
    send(quiet.hwnd, WM_ENDSESSION, 0, 0);
    pump();
    assert!(!quiet.events().contains(&TrayEvent::ShutdownCancelled));
    assert_eq!(h.faults() + quiet.faults(), 0);
}

#[test]
fn wm_close_asks_the_app_to_quit_and_the_window_stays() {
    let h = Harness::new("close", syncing());
    send(h.hwnd, UPDATE, 0, 0);
    send(h.hwnd, WM_CLOSE, 0, 0);
    assert_eq!(h.events().last(), Some(&TrayEvent::CloseRequested));
    // SAFETY: a query on the harness's own window handle.
    assert!(unsafe { IsWindow(h.hwnd) } != 0);
    assert!(h.is_present());
}

/// Enter on the focused b sends NIN_KEYSELECT twice: one toggle.
#[test]
fn a_key_select_and_its_repeat_are_one_activation() {
    let h = Harness::new("keys", syncing());
    let l = ((ICON_ID as isize) << 16) | NIN_KEYSELECT as isize;
    send(h.hwnd, CALLBACK, 0, l);
    send(h.hwnd, CALLBACK, 0, l);
    let activations = h
        .events()
        .iter()
        .filter(|e| matches!(e, TrayEvent::Activate { .. }))
        .count();
    assert_eq!(activations, 1);
    send(h.hwnd, CALLBACK, 0, ((ICON_ID as isize) << 16) | NIN_SELECT as isize);
    let activations = h
        .events()
        .iter()
        .filter(|e| matches!(e, TrayEvent::Activate { .. }))
        .count();
    assert_eq!(activations, 2);
}

/// A panic in the app's handler does not unwind through Windows: it is counted, and the
/// procedure keeps working.
#[test]
fn a_panic_in_a_handler_is_caught_at_the_boundary() {
    let fake = Rc::new(Fake::default());
    let handler: TrayHandler = Arc::new(|_| panic!("the app's handler"));
    let faults = Arc::new(AtomicU32::new(0));
    let present = Arc::new(AtomicBool::new(false));
    let model = Arc::new(Mutex::new(tray_model(&syncing())));
    let (hwnd, context) = build(
        model,
        handler,
        present.clone(),
        faults.clone(),
        Box::new(FakeShell(fake.clone())),
        &format!("BeebeebTrayHarness-panic-{}", std::process::id()),
    )
    .unwrap();
    // The first add reports Presence(true) to a handler that panics.
    send(hwnd, UPDATE, 0, 0);
    assert_eq!(faults.load(Ordering::SeqCst), 1);
    assert!(
        !context.refreshing.get(),
        "the refresh guard was released by the unwind"
    );
    send(hwnd, UPDATE, 0, 0);
    assert!(present.load(Ordering::SeqCst));
    send(hwnd, SHUTDOWN, 0, 0);
    pump();
    destroy_last_icon(&context);
    // SAFETY: a query on a window handle (now destroyed).
    assert!(unsafe { IsWindow(hwnd) } == 0);
    assert_eq!(Rc::strong_count(&context), 1, "the window gave its reference back");
}

/// The model the app applies is what the next refresh shows (no copy goes stale).
#[test]
fn an_applied_model_reaches_the_next_refresh() {
    let h = Harness::new("model", syncing());
    send(h.hwnd, UPDATE, 0, 0);
    assert_eq!(h.context.snapshot().shown.map(|s| s.0), Some(TrayIconState::Syncing));
    *h.model.lock().unwrap() = tray_model(&TrayInputs {
        signed_in: true,
        paused: true,
        ..TrayInputs::default()
    });
    send(h.hwnd, UPDATE, 0, 0);
    let state = h.context.snapshot();
    assert_eq!(state.shown.map(|s| s.0), Some(TrayIconState::Paused));
    assert!(!state.animating);
    assert_eq!(h.faults(), 0);
}
