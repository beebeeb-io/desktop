//! Starts the Office helper process confined.
//!
//! Any helper of ours starts through this crate in the same way: the Office helper now, the media helper next. Each
//! kind is an executable of its own; the confinement and the four promises below are the same for every one.
//!
//! The helper parses files nobody trusts. This crate starts it so that it
//!
//! 1. cannot open a network connection,
//! 2. cannot write a file anywhere and cannot read the user's files,
//! 3. cannot start another process,
//! 4. is gone when its parent is gone, and can be killed by the parent at any time.
//!
//! At its start the helper is handed two unnamed pipes (its standard input and output) and nothing else: no
//! arguments, no inherited environment, no other handle. Bulk bytes (a document, a rendered page) travel over memory
//! the parent shares with it afterwards: an unnamed [`Section`] the parent makes and hands over with
//! [`Child::grant`], one handle at a time, read-only or read-write as the parent decides. Nothing is ever written
//! to a file for it.
//!
//! **Profiles.** How much a helper may do beyond the four promises is a [`Profile`], a closed list chosen in code by
//! the caller: [`Profile::NoCapabilities`] (every helper, the default) or [`Profile::SystemDecoder`] (the media
//! helper only, for a file that Windows' own decoders play; it may also read the registry). There is no capability
//! by name, and every profile keeps the four promises above. The Office helper never names a profile.
//!
//! **Fail closed.** There is no unconfined way to start the helper through this crate. On an operating system
//! without a sandbox module here (everything but Windows today) [`Sandbox::spawn`] returns
//! [`SandboxError::Unsupported`] and starts nothing. On Windows the helper is created suspended, the confinement is
//! read back from the new process, and only then does it run; any failure on the way ends the process.
//!
//! Errors carry a step name and an operating-system code, never a path, a name or file content.

#![deny(unsafe_code)]

use std::fmt;
use std::fs::File;
use std::path::Path;
use std::time::Duration;

mod block;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as platform;

/// The viewer contract's cap on the helper's committed memory: 1 GiB.
pub const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;

/// The exit code a helper has after [`Child::kill`] (or after its `Child` was dropped).
pub const KILLED_EXIT_CODE: u32 = 0xBEEB_DEAD;

/// The largest [`Section`]: 4 GiB.
pub const MAX_SECTION_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Resource limits of one helper.
///
/// There is no time limit here on purpose: a job's CPU limit would be cumulative over the helper's whole life, so the
/// time one request may take is the caller's own timer (the viewer's 15 s wall clock, media's 5 s), followed by
/// [`Child::kill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Committed memory the helper may hold, in bytes. An allocation beyond it fails inside the helper.
    pub memory_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
        }
    }
}

/// What a helper may do beyond the four promises of the crate documentation. A closed list: a caller picks one of
/// these values, and no capability can be asked for by name.
///
/// Every profile is a less-privileged AppContainer (opted out of the "all application packages" group) in the same
/// job, from the same restricted token, with the same handle list. They differ only in the capabilities the helper's
/// token holds, and the crate reads that list back from the new process before it runs (Windows): the token must
/// hold exactly [`Profile::capabilities`], no more and no fewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Profile {
    /// No capability at all. Every helper of ours unless its caller names another profile; the Office helper
    /// never does.
    #[default]
    NoCapabilities,
    /// The media helper, only for a file whose decoder is Windows' own (its route is `System`; decision 2541).
    ///
    /// Windows' decoders (Media Foundation) do not start without `registryRead`. With `hevc`, also the four
    /// capabilities the HEVC decoder from Microsoft's Store extension needs: `lpacCom`, `lpacAppExperience`,
    /// `lpacIdentityServices` and `lpacMedia` (each one measured necessary, the five together enough).
    ///
    /// What it adds, measured: the helper can READ the machine's and the user's registry. What stays denied, as in
    /// every profile: reading a file of the user, writing anywhere, the network (loopback too) and starting a
    /// process.
    SystemDecoder {
        /// The file is HEVC: add the four capabilities of the Store decoder.
        hevc: bool,
    },
}

/// The capability Media Foundation needs to start in a less-privileged AppContainer.
const REGISTRY_READ: &str = "registryRead";

impl Profile {
    /// The capabilities this profile's helper holds, by their Windows names, in the order they are asked for.
    pub const fn capabilities(self) -> &'static [&'static str] {
        match self {
            Profile::NoCapabilities => &[],
            Profile::SystemDecoder { hevc: false } => &[REGISTRY_READ],
            Profile::SystemDecoder { hevc: true } => &[
                REGISTRY_READ,
                "lpacCom",
                "lpacAppExperience",
                "lpacIdentityServices",
                "lpacMedia",
            ],
        }
    }
}

/// Why a helper was not started, or why an operation on it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxError {
    /// This operating system has no sandbox module in this crate. Nothing was started.
    Unsupported,
    /// The helper path was refused before anything was started (the reason is a fixed text, never the path).
    HelperPath(&'static str),
    /// A limit was refused before anything was started (also: a section or grant length out of range).
    Limits(&'static str),
    /// An operating-system call failed. `step` is a fixed name of ours, `code` the system's error code.
    Os {
        /// Which step failed.
        step: &'static str,
        /// The operating system's error code.
        code: u32,
    },
    /// The new process did not have the confinement that was asked for. It was ended before it ran.
    NotConfined(&'static str),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "no sandbox on this operating system; the helper was not started"),
            Self::HelperPath(why) => write!(f, "helper path refused: {why}"),
            Self::Limits(why) => write!(f, "limits refused: {why}"),
            Self::Os { step, code } => write!(f, "sandbox step '{step}' failed with system code {code}"),
            Self::NotConfined(what) => write!(f, "the helper was not confined ({what}); it was ended before it ran"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// How a helper ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: u32,
}

impl ExitStatus {
    /// The process exit code.
    pub fn code(&self) -> u32 {
        self.code
    }

    /// True when the helper was ended by [`Child::kill`] or by dropping its [`Child`].
    pub fn was_killed(&self) -> bool {
        self.code == KILLED_EXIT_CODE
    }
}

/// The two unnamed pipes between the parent and the helper.
#[derive(Debug)]
pub struct Pipes {
    /// Bytes written here arrive on the helper's standard input.
    pub to_helper: File,
    /// The helper's standard output. A read returns end-of-file once the helper is gone.
    pub from_helper: File,
}

/// Starts confined helpers.
#[derive(Debug, Clone, Copy)]
pub struct Sandbox {
    limits: Limits,
    profile: Profile,
}

impl Sandbox {
    /// A sandbox with these limits and [`Profile::NoCapabilities`]. Refuses a memory limit of zero or one the
    /// platform cannot express.
    ///
    /// On Windows this also deletes container profiles of ours that a killed parent left behind (only names of
    /// our own shape, only ones older than a minute; see the Windows module).
    pub fn new(limits: Limits) -> Result<Self, SandboxError> {
        Self::with_profile(limits, Profile::NoCapabilities)
    }

    /// A sandbox with these limits whose helpers get `profile`. The same refusals and the same clean-up as
    /// [`Sandbox::new`].
    pub fn with_profile(limits: Limits, profile: Profile) -> Result<Self, SandboxError> {
        if limits.memory_bytes == 0 {
            return Err(SandboxError::Limits("the memory limit is zero"));
        }
        if usize::try_from(limits.memory_bytes).is_err() {
            return Err(SandboxError::Limits("the memory limit does not fit this platform"));
        }
        // Not part of the confinement: tidies up after a parent that was killed while starting a helper.
        platform::remove_left_over_profiles();
        Ok(Self { limits, profile })
    }

    /// The limits helpers of this sandbox get.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The profile helpers of this sandbox get.
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Starts `helper_exe` confined and returns it with its pipes.
    ///
    /// `helper_exe` must be an absolute path to a file; nothing is searched and no shell is involved. The helper
    /// gets no arguments. Its environment is exactly [`helper_environment_names`]. Its standard error is not
    /// connected.
    ///
    /// Never starts an unconfined process: see the crate documentation. On Windows the helper's token is read back
    /// before it runs, and a token whose capabilities are not exactly the profile's ends it.
    pub fn spawn(&self, helper_exe: &Path) -> Result<Child, SandboxError> {
        let (inner, pipes) = platform::spawn(helper_exe, self.limits, self.profile)?;
        Ok(Child {
            inner,
            pipes: Some(pipes),
        })
    }
}

/// The names of the environment variables a helper gets (the values come from the system, not from the parent's
/// environment). Nothing else is passed on.
pub fn helper_environment_names() -> &'static [&'static str] {
    block::ENVIRONMENT_NAMES
}

/// A running (or ended) confined helper. Dropping it kills the helper.
#[derive(Debug)]
pub struct Child {
    inner: platform::ChildInner,
    pipes: Option<Pipes>,
}

impl Child {
    /// The helper's process id.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// The pipes, once. Later calls return `None`.
    pub fn take_pipes(&mut self) -> Option<Pipes> {
        self.pipes.take()
    }

    /// `Some` when the helper has ended, without waiting.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, SandboxError> {
        self.inner.wait(Duration::ZERO)
    }

    /// Waits at most `bound` for the helper to end. `None` means it is still running: the caller decides whether
    /// that is a hang, and calls [`Child::kill`].
    pub fn wait_timeout(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        self.inner.wait(bound)
    }

    /// Ends the helper now. It is not asked. Fine to call on a helper that already ended.
    pub fn kill(&self) -> Result<(), SandboxError> {
        self.inner.kill()
    }

    /// Hands `section` to the helper with exactly `access`: the section's handle is duplicated into the helper's
    /// process with those rights only, and not inheritable. Send the returned [`Granted`] to the helper over the
    /// pipe; it maps the section with [`helper::map`], which closes that handle again when the mapping ends.
    ///
    /// A grant cannot be taken back: the helper can use it until it drops the mapping or ends. Each call places a
    /// new handle in the helper. The two pipe ends of [`Sandbox::spawn`] stay the only handles a helper inherits.
    pub fn grant(&self, section: &Section, access: Access) -> Result<Granted, SandboxError> {
        self.inner.grant(&section.inner, access)
    }
}

/// Memory the app shares with a helper: an unnamed section backed by the paging file, mapped read-write in the app.
///
/// Nothing can open it by name, its handle is not inheritable, and a helper reaches it only through
/// [`Child::grant`]. Dropping it overwrites every byte with zero before the memory is released, so a helper that
/// still holds a grant then sees zeros, never the old bytes.
///
/// While a helper holds a read-write grant it can change these bytes at any moment: read them once the helper has
/// said it is done.
pub struct Section {
    inner: platform::SectionInner,
}

impl Section {
    /// A section of `len` bytes, all zero. Refuses 0 bytes and more than [`MAX_SECTION_BYTES`].
    ///
    /// Off Windows: [`SandboxError::Unsupported`].
    pub fn new(len: usize) -> Result<Self, SandboxError> {
        let len = section_length(len as u64)?;
        Ok(Self {
            inner: platform::SectionInner::new(len)?,
        })
    }

    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        self.inner.as_slice()
    }

    /// The bytes, to write.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.inner.as_mut_slice()
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Always false: a section holds at least one byte.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }
}

impl fmt::Debug for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The length only, never the bytes.
        f.debug_struct("Section")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsRawHandle for Section {
    /// The app's own handle, for checks. It is not inheritable and must stay so: a helper gets the section through
    /// [`Child::grant`], never by inheritance.
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.inner.raw_handle()
    }
}

/// A length the crate accepts for a section or a grant: 1 byte to [`MAX_SECTION_BYTES`].
fn section_length(len: u64) -> Result<usize, SandboxError> {
    if len == 0 {
        return Err(SandboxError::Limits("a section of zero bytes"));
    }
    if len > MAX_SECTION_BYTES {
        return Err(SandboxError::Limits("a section over 4 GiB"));
    }
    usize::try_from(len).map_err(|_| SandboxError::Limits("a section that does not fit this platform"))
}

/// What a helper may do with a granted [`Section`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Read only (a document's bytes).
    Read,
    /// Read and write (a page the helper renders into).
    ReadWrite,
}

/// A section handed to a helper: what the caller sends to the helper over the pipe, for [`helper::map`] there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Granted {
    /// The handle's value as the helper sees it. It means nothing in the app.
    pub wire: u64,
    /// The section's length in bytes.
    pub len: u64,
}

/// The helper's side of a grant. Compiled into helpers too; it starts nothing.
pub mod helper {
    use std::fmt;

    use crate::{Access, Granted, SandboxError, platform, section_length};

    /// A granted section, mapped into the helper. Unmapped, and the grant's handle closed, when dropped.
    pub struct Mapped {
        inner: platform::MappedInner,
    }

    impl Mapped {
        /// The section's bytes.
        pub fn as_slice(&self) -> &[u8] {
            self.inner.as_slice()
        }

        /// The section's bytes to write, for a read-write mapping; `None` for a read-only one.
        pub fn as_mut_slice(&mut self) -> Option<&mut [u8]> {
            self.inner.as_mut_slice()
        }
    }

    impl fmt::Debug for Mapped {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The length only, never the bytes.
            f.debug_struct("Mapped")
                .field("len", &self.as_slice().len())
                .finish_non_exhaustive()
        }
    }

    /// Maps a section the app granted to this process. `access` is what this side wants; asking for more than the
    /// grant carries is refused by the system ([`SandboxError::Os`] with code 5). The grant's handle belongs to
    /// this call: it is closed when the returned value is dropped, or at once when mapping fails, so a grant maps
    /// once.
    ///
    /// Off Windows: [`SandboxError::Unsupported`].
    pub fn map(granted: Granted, access: Access) -> Result<Mapped, SandboxError> {
        let len = section_length(granted.len)?;
        Ok(Mapped {
            inner: platform::map(granted.wire, len, access)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_memory_limit_is_refused() {
        assert_eq!(
            Sandbox::new(Limits { memory_bytes: 0 }).unwrap_err(),
            SandboxError::Limits("the memory limit is zero")
        );
    }

    #[test]
    fn the_default_limit_is_the_contracts_one_gibibyte() {
        assert_eq!(Limits::default().memory_bytes, 1 << 30);
        assert_eq!(Sandbox::new(Limits::default()).unwrap().limits().memory_bytes, 1 << 30);
    }

    #[test]
    fn an_error_text_carries_a_step_and_a_code_and_nothing_else() {
        let text = SandboxError::Os {
            step: "create process",
            code: 5,
        }
        .to_string();
        assert_eq!(text, "sandbox step 'create process' failed with system code 5");
    }

    #[test]
    fn a_killed_helper_is_told_apart_from_one_that_exited() {
        assert!(ExitStatus { code: KILLED_EXIT_CODE }.was_killed());
        assert!(!ExitStatus { code: 0 }.was_killed());
        assert!(!ExitStatus { code: 1 }.was_killed());
    }

    /// No runtime input may weaken or skip a part of the confinement. The spawn path therefore reads nothing
    /// from the parent's environment, its arguments or a file: this pins that in the source of every module.
    /// (A [`Profile`] is chosen in code by the caller, from a closed list; it is not such an input.)
    #[test]
    fn the_spawn_path_reads_no_environment_variable_argument_or_switch() {
        let sources = [
            ("lib", include_str!("lib.rs")),
            ("block", include_str!("block.rs")),
            ("windows", include_str!("windows.rs")),
            ("unsupported", include_str!("unsupported.rs")),
        ];
        // Built from pieces so this test's own text does not match.
        let forbidden = [
            ["env", "::var"].concat(),
            ["env", "::args"].concat(),
            ["env!", "("].concat(),
            ["option_env", "!"].concat(),
            ["BB_", "DIAG"].concat(),
            ["BB_", "ENV"].concat(),
            ["cfg(", "feature"].concat(),
            ["GetEnvironment", "Variable"].concat(),
        ];
        let mut checked = 0;
        for (module, source) in sources {
            assert!(source.len() > 500, "{module} was read");
            for word in &forbidden {
                assert!(
                    !source.contains(word.as_str()),
                    "module {module} holds a runtime or build switch: {word}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 32);
    }

    /// Both lengths are refused before any system call, on every operating system.
    #[test]
    fn a_section_of_zero_bytes_or_over_four_gibibytes_is_refused() {
        assert_eq!(
            Section::new(0).unwrap_err(),
            SandboxError::Limits("a section of zero bytes")
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            Section::new(MAX_SECTION_BYTES as usize + 1).unwrap_err(),
            SandboxError::Limits("a section over 4 GiB")
        );
        let refused = |len| helper::map(Granted { wire: 0, len }, Access::Read).unwrap_err();
        assert_eq!(refused(0), SandboxError::Limits("a section of zero bytes"));
        assert_eq!(
            refused(MAX_SECTION_BYTES + 1),
            SandboxError::Limits("a section over 4 GiB")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_sandbox_module_nothing_is_started() {
        let sandbox = Sandbox::new(Limits::default()).unwrap();
        // An existing, runnable program: were there an unconfined fallback, this would start it.
        let exe = std::env::current_exe().unwrap();
        assert_eq!(sandbox.spawn(&exe).unwrap_err(), SandboxError::Unsupported);
    }

    /// The capability lists are pinned word for word, as measured by the media spike (decision 2541). A capability
    /// added to a list, or a name misspelled (Windows accepts any name, so nothing at run time would notice), turns
    /// this red.
    #[test]
    fn each_profile_asks_for_exactly_its_measured_capabilities() {
        assert_eq!(Profile::default(), Profile::NoCapabilities);
        assert_eq!(Profile::NoCapabilities.capabilities(), &[] as &[&str]);
        assert_eq!(Profile::SystemDecoder { hevc: false }.capabilities(), &["registryRead"]);
        assert_eq!(
            Profile::SystemDecoder { hevc: true }.capabilities(),
            &[
                "registryRead",
                "lpacCom",
                "lpacAppExperience",
                "lpacIdentityServices",
                "lpacMedia"
            ]
        );
        let every = [
            Profile::NoCapabilities,
            Profile::SystemDecoder { hevc: false },
            Profile::SystemDecoder { hevc: true },
        ];
        let mut counted = 0;
        for profile in every {
            let names = profile.capabilities();
            for (index, name) in names.iter().enumerate() {
                assert!(!names[index + 1..].contains(name), "{profile:?} names {name} twice");
                counted += 1;
            }
        }
        assert_eq!(counted, 6, "the instrument counted every name of every profile");
    }

    #[test]
    fn a_sandbox_keeps_the_profile_it_was_made_with() {
        assert_eq!(
            Sandbox::new(Limits::default()).unwrap().profile(),
            Profile::NoCapabilities
        );
        let hevc = Profile::SystemDecoder { hevc: true };
        assert_eq!(Sandbox::with_profile(Limits::default(), hevc).unwrap().profile(), hevc);
        assert_eq!(
            Sandbox::with_profile(Limits { memory_bytes: 0 }, hevc).unwrap_err(),
            SandboxError::Limits("the memory limit is zero"),
            "a profile changes no limit check"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_sandbox_module_no_profile_starts_anything() {
        let exe = std::env::current_exe().unwrap();
        for profile in [
            Profile::NoCapabilities,
            Profile::SystemDecoder { hevc: false },
            Profile::SystemDecoder { hevc: true },
        ] {
            let sandbox = Sandbox::with_profile(Limits::default(), profile).unwrap();
            assert_eq!(
                sandbox.spawn(&exe).unwrap_err(),
                SandboxError::Unsupported,
                "{profile:?}"
            );
        }
    }
}
