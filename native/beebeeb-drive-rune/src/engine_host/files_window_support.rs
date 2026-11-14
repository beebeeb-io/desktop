//! The Files window's harness for the adapter tests that look at the element tree: the engine behind the shell, a
//! window on it, and what a test needs to wait for and press. Test support only; every name a test uses is invented.

use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::app::Engine;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use rune::{Env, Runtime};

use crate::State;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::shell::actions::{ActionError as ShellError, EngineActions, HostActions};
use crate::shell::app::Session;
use crate::shell::host::{self, Shell, Wiring};
use crate::shell::identity::engine_identity;
use crate::shell::instance::ProfileHolder;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::TrayInputs;
use crate::shell::{registry, state};

/// The engine behind the shell, with a sync start that is not carried out (the process's one state folder is
/// shared by every test of this binary).
struct NoStart {
    live: Arc<LiveEngine<MemoryCredentials>>,
}

impl EngineActions for NoStart {
    fn session(&self) -> Session {
        self.live.session()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.live.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }
    fn start_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn stop_for_quit(&self) {}
    fn resume_after_cancelled_quit(&self) {}
    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        self.live.sync_refusal()
    }
}

pub(super) const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// The Files window on the engine of `world`, as the app builds it (signed out until a test signs in).
pub(super) fn window(engine: &Arc<Engine<MemoryCredentials>>) -> Runtime<State> {
    let (_host, messages) = ChannelHost::new(|| {});
    let opened = Opened {
        engine: engine.clone(),
        identity: engine_identity(),
        gate: engine.gate(),
    };
    let live = Arc::new(LiveEngine::new(&opened, messages));
    let platform: Arc<FakePlatform> = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(Arc::new(NoStart { live }), platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut rt = Runtime::new(
        state::init_on_engine(shell, engine.clone()),
        registry::view,
        (1280.0, 800.0),
        ENV,
    );
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt
}

pub(super) fn settle(rt: &mut Runtime<State>) {
    for _ in 0..2 {
        rt.frame(0.016);
        rt.settle_all();
        rt.frame(0.016);
    }
}

/// Everything the window holds as text: each element's text and its accessibility label.
pub(super) fn drawn(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn until_drawn(rt: &mut Runtime<State>, words: &str) {
    let began = Instant::now();
    loop {
        settle(rt);
        if drawn(rt).contains(words) {
            return;
        }
        assert!(
            began.elapsed() < Duration::from_secs(15),
            "{words:?} was never drawn; the window draws:\n{}",
            drawn(rt)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}; the window draws:\n{}", drawn(rt)))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
