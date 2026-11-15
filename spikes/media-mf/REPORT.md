# M10: can Media Foundation decode inside the media helper's sandbox? (spike result, 2026-11-15)

Throwaway spike, evidence only; nothing here merges into `v2-native`. The question comes from decision 2540 (D-MED-1, the gap
formats) and the owner's 17:30 ruling: HEVC plays on the system decoder.

**Test PC:** Windows 11 23H2 (10.0.22631), no GPU (Microsoft Basic Display Adapter). Microsoft's HEVC Video Extensions
2.4.149 and VP9 Video Extensions 1.2.20 are provisioned on the PC and registered for the test user.

**Where it ran:** all decoding runs are in the signed-in desktop session (why: "Instrument notes"). Four runs are not:
- the session-0 loader check;
- the session-0 control;
- the LocalSystem run;
- the plain process that made the Windows-encoded files.

**Fixtures:** made for this spike, with no third-party content (320x180, 25 fps, 2 s; 48 kHz stereo, 440/660 Hz plus
14 kHz).
- `make-fixtures.sh`: ffmpeg test sources; HE-AAC from fdk-aac.
- `src/make.rs`: WMV 9, VC-1 and WMA Pro from Windows' own encoders, since ffmpeg has no encoder for them.

**Reference:** each fixture has the SHA-256 of frame 0's visible luma as ffmpeg decodes it. The probe hashes the same
plane: 8-bit from NV12, 16-bit from P010.

## The answer

1. **The helper's sandbox as it is today cannot run Media Foundation at all.** The sandbox is a less-privileged AppContainer
   (LPAC: opted out of "all application packages") with zero capabilities. `CoInitializeEx` fails with `0x80070005`, then
   `MFStartup` with `0x80004001`. That holds for every format: 49 of 49 confined runs that start Media Foundation, over 5
   desktop-session runs.
2. **One capability, `registryRead`, is enough for every in-box decoder:**
   - H.264, including interlaced;
   - WMV 8, WMV 9 and VC-1;
   - MPEG-4 Part 2 (Xvid, DivX 3);
   - AAC-LC, full HE-AAC v1/v2, MP3, WMA 2 and WMA Pro.

   Every picture and every audio hash is the same as the unconfined decode. Adding `lpacCom` or `lpacMedia` changed nothing
   for these formats.
3. **Store-packaged decoders need more: HEVC (the required path) and VP9.** Two configurations decode them, bit-exact and
   50/50 frames in every HEVC file (Main and Main 10; MP4, MOV and `hev1`):
   - (a) the LPAC with **five** capabilities: `registryRead`, `lpacCom`, `lpacAppExperience`, `lpacIdentityServices` and
     `lpacMedia`;
   - (b) a **regular** AppContainer, not opted out of "all application packages", with zero capabilities.

   The five were found by leave-one-out from 13 candidates: each is necessary, and together they are enough. With any of
   them missing, Media Foundation either lists no HEVC decoder (`0xC00D5212`, exactly as on a PC without the extension) or
   lists it and fails to activate it (`0xC00D36B4`). **Then the helper would tell the person "without Microsoft's HEVC
   extension" while it is installed.**
4. **Media Foundation does not fill these gaps:**
   - 10-bit H.264 and 4:2:2 H.264: the decoder accepts the type, then fails the first read with `0x80004005`.
   - 4:2:2 HEVC: it decodes all 50 frames, but frame 0's luma differs from the reference. A conformant HEVC decode is
     bit-exact, as the Main and Main 10 rows show, so this row is not trusted.
5. **Not measured:**
   - Hardware decoding: no GPU.
   - WMA Lossless: no test file. The in-box encoder offers no output type without a VBR setup, and ffmpeg has no encoder for
     it. Its decoder is listed in every column where Media Foundation starts.
   - Real iPhone files: the HEVC files are x265 output with the `hvc1` tag, so no Dolby Vision metadata, no extra tracks
     and no rotation.

## Per format

The columns, left to right:
- **unconfined**: a plain child process. Run twice: with the parent's environment, and with exactly the sandbox's four
  variables and working directory. Both runs printed identical lines.
- **product sandbox**: `beebeeb-office-sandbox`, unchanged.
- **LPAC+registryRead**: the spike's copy of the sandbox, with that one capability added.
- **LPAC+5**: the five capabilities above.
- **AppContainer**: the spike's copy, with the opt-out removed and zero capabilities.

`ok` means: no error; the frame count shown; the luma hash equal to ffmpeg's where marked "bit-exact"; and the same picture
and audio hash in every column that decoded. "same" means the same lines as the column to its left that decoded.

| Format (fixture) | unconfined | product sandbox | LPAC+registryRead | LPAC+5 | AppContainer | MF route in the helper |
|---|---|---|---|---|---|---|
| H.264 High (`h264.mp4`) | ok, 50/50, bit-exact | `MFStartup 0x80004001` | ok, same | ok, same | ok, same | yes, `registryRead` |
| H.264 interlaced (`h264-interlaced.ts`) | ok, 49 of 50, frame 0 bit-exact | same failure | ok, same | ok, same | ok, same | yes, `registryRead` |
| H.264 10-bit (`h264-10bit.mp4`) | `v-read 0x80004005`, 0 frames | same failure | `0x80004005` | not run | `0x80004005` | **no** (no decoder) |
| H.264 4:2:2 (`h264-422.mp4`) | `v-read 0x80004005`, 0 frames | same failure | `0x80004005` | not run | `0x80004005` | **no** |
| **HEVC Main, MP4 / MOV / hev1** | ok, 50/50, bit-exact | same failure | **`0xC00D5212`, decoder not listed** | **ok, bit-exact** | ok, bit-exact | **yes, LPAC+5 or a regular AppContainer** |
| **HEVC Main 10 (HLG tags), MP4 / MOV** | ok, 50/50, P010, bit-exact | same failure | **`0xC00D5212`** | **ok, bit-exact** | ok, bit-exact | **yes, LPAC+5 or a regular AppContainer** |
| HEVC 4:2:2 10-bit (`hevc-422-10.mp4`) | 50 frames, luma differs from the reference | same failure | `0xC00D5212` | not run | as unconfined | no (not trusted) |
| WMV 8 + WMA 2, ASF (`wmv2.wmv`) | ok, 50/50, bit-exact; audio ok | same failure | ok, same | ok, same | ok, same | yes, `registryRead` |
| WMV 9 Main + WMA Pro, ASF (`wmv3-wmapro.wmv`) | ok, 50/50; audio ok | same failure | ok, same | not run | ok, same | yes, `registryRead` |
| VC-1 Advanced, ASF (`wvc1.wmv`) | ok, 50/50 | same failure | ok, same | not run | ok, same | yes, `registryRead` |
| WMA 2 / WMA Pro (`wmav2.wma`, `wmapro.wma`) | ok | same failure | ok, same | not run | ok, same | yes, `registryRead` |
| WMA Lossless | decoder listed, no file | MF does not start | decoder listed | not run | decoder listed | not measured |
| MPEG-4 Part 2, Xvid ASP with B-frames, AVI (`mpeg4p2.avi`) | ok, 49 frames; luma ≠ ffmpeg (expected, see "Instrument notes") | same failure | ok, same | ok, same | ok, same | yes, `registryRead` |
| MPEG-4 Part 2, DivX 3 (MP43), AVI (`msmpeg4v3.avi`) | ok, 50/50; luma ≠ ffmpeg (expected, see "Instrument notes") | same failure | ok, same | not run | ok, same | yes, `registryRead` |
| HE-AAC v1, SBR (`he-aac.m4a`) | ok, full (below) | same failure | ok, same | not run | ok, same | yes, `registryRead` |
| HE-AAC v2, SBR+PS (`he-aac-v2.m4a`) | ok, full (below) | same failure | ok, same | ok, same | ok, same | yes, `registryRead` |
| AAC-LC, MP3 (controls) | ok | same failure | ok, same | AAC-LC ok, same | ok, same | yes |
| VP9, Store extension (`vp9.webm`, control) | ok, 50/50, bit-exact | same failure | `0xC00D5212`, not listed | ok, bit-exact | ok, bit-exact | LPAC+5 or a regular AppContainer |
| MPEG-2, program stream (`mpeg2.mpg`, control) | `0xC00D36B4` at the output type | same failure | same | not run | same | no (not a gap row: the plan decodes MPEG-2 in Rust) |

- **"Same failure"** means `CoInitializeEx 0x80070005`, then `MFStartup 0x80004001`.
- **Full HE-AAC.** The probe measures the first second of the left channel: `hf` = RMS of the first difference over the RMS,
  and `lr` = RMS(left − right) over RMS(left). A core-only decode loses the 14 kHz tone (`hf` near 0.06), and for v2
  without PS the left and right channels are equal (`lr` = 0). Media Foundation matches ffmpeg's full decode:

  | File | Media Foundation | ffmpeg |
  |---|---|---|
  | HE-AAC v1 | hf 0.3029, lr 1.4475, output 48 kHz | hf 0.3015, lr 1.4476 |
  | HE-AAC v2 | hf 0.7283, lr 1.2759 | hf 0.7284, lr 1.2764 |

  **This removes the "Reduced quality on this PC" case (D-MED-7) wherever the MF route runs.**

## Which capabilities: how they were found

All on `hevc-main.mov`; "listed" refers to `MFTEnumEx` for an HEVC input type.

| LPAC with | HEVC listed | HEVC decodes |
|---|---|---|
| nothing (the product) | Media Foundation does not start | no |
| `registryRead` | 0 | no, `0xC00D5212` |
| `registryRead` + any one of `packageQuery`, `packageContents`, `lpacPackageManagerOperation`, `lpacAppExperience`, `lpacServicesManagement`, `lpacCom` | 0 | no, `0xC00D5212` |
| `registryRead`, `lpacCom`, `packageContents`, `packageQuery`, `lpacPackageManagerOperation`, `lpacAppExperience`, `lpacServicesManagement` | **1** | no: `0xC00D36B4` on every output type; the package DLL is not loaded |
| all 13 (the 7 above + `lpacSessionManagement`, `lpacPnPNotifications`, `lpacInstrumentation`, `lpacIdentityServices`, `lpacEnterprisePolicyChangeNotifications`, `lpacMedia`) | 1 | **yes**, bit-exact |
| all 13 minus one, for each of the 13 | — | fails without `registryRead` (MF does not start), `lpacCom`, `lpacAppExperience`, `lpacIdentityServices` or `lpacMedia`; still decodes without each of the other 8 |
| **the five** | 1 | **yes**, bit-exact, in all 5 HEVC Main/Main 10 files and VP9 |

**How "five is minimal" was established:** each of the five is necessary when the other twelve are present, and the five
alone are enough. Other combinations were not tried.

**The package files are not the blocker.** The ACL of the package folder and of `HEVCDECODER_STORE.dll` grants read and
execute to ALL RESTRICTED APP PACKAGES, which includes the LPAC. What the capabilities open is package discovery and
activation. With the 13 or the five, the probe loads `windows.staterepositoryclient`/`core`/`ps`, and then the package
DLL.

**Instrument limit:** Windows does not validate capability names. A misspelled name is accepted and counted
(`caps=2`). So "made no difference" is all this spike can say about a capability that changed nothing.

## HEVC, the required path

**With the extension (LPAC+5, or a regular AppContainer):**
- Main and Main 10 decode 50 of 50 frames, in MP4 and in MOV (`hvc1`, as an iPhone writes it) and with `hev1`.
- Frame 0's luma is bit-exact against ffmpeg: NV12 for Main, P010 for Main 10.
- The decoder is `HEVCDECODER_STORE.dll`, loaded from the package `Microsoft.HEVCVideoExtension_2.4.149.0_x64__8wekyb3d8bbwe`
  (seen with `EnumProcessModules`).
- An open and decode of a 2 s file takes 0.31–0.33 s per process in both configurations.

**Without the extension, measured without removing anything.** The extension is registered per user. A process under an
account with no registration (LocalSystem, unconfined, in session 0) shows:
- `MFTEnumEx(video decoder, input {Video, HEVC})` → **0 decoders**, with all three flag sets tried (`ALL`,
  `ALL|UNTRUSTED_STOREMFT`, the default sorted set). H.264 still lists 1 decoder.
- The source still opens, and its native subtype is **HEVC**. So the helper knows it is HEVC (and the audio still decodes).
- `SetCurrentMediaType` for NV12, P010, YUY2 and RGB32 → **`0xC00D5212` (MF_E_TOPO_CODEC_NOT_FOUND)**, every one.
- No module from a Store package loads.

**The control.** The test user, also unconfined and also in session 0, lists 1 HEVC decoder and decodes Main and Main 10
(50/50). So the difference is the account's registration, not the session. The LPAC with `registryRead` alone (or with
`lpacCom`/`lpacMedia` added) prints the same lines as LocalSystem while the extension is installed.

**For `native/beebeeb-media-helper/src/system.rs::available(Hevc)`:**
- Ask `MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, …, input {MFMediaType_Video, MFVideoFormat_HEVC})` inside the helper.
  0 decoders means `Error(NoSystemDecoder(Hevc))`.
- A source that reports HEVC, followed by `0xC00D5212` on the output type, is the same answer, caught later.
- `0xC00D36B4` with the decoder listed means a sandbox that can find the decoder but not start it. That is not "missing",
  and must not be shown as such.
- These answers are only true in a sandbox that can see and start the package (LPAC+5, or a regular AppContainer).

**Cost of a failed lookup:** in the variants that cannot see packages, an HEVC open took 1.9–3.6 s to fail.

**D3D11 inside the sandbox:** a D3D11 device with video support opens in both sandbox variants tried (`AppContainer`,
`LPAC+registryRead`). On this PC it is the software adapter, so the decoders report `hardware=0`. With that device
manager, HEVC Main 10 fails its first read with `0xC00D36B4`. This row says nothing about real GPUs.

## What the sandbox would have to give up (measured with the sandbox crate's own attack probe)

| Attempt | unconfined | product sandbox | AppContainer | LPAC+lpacCom+registryRead | LPAC+5 |
|---|---|---|---|---|---|
| read a file in the user's Documents | ok | denied (5) | denied (5) | denied (5) | denied (5) |
| read a test file outside the profile | ok | denied (5) | denied (5) | denied (5) | denied (5) |
| read system files (hosts, win.ini) and a file under Program Files | ok | ok | ok | ok | ok |
| write to the user's TEMP, or to Public | ok | denied (5) | denied (5) | denied (5) | denied (5) |
| loopback connection to a listener of the runner (accepted) | 1 | 0 (network start refused, 10107) | 0 (timed out) | 0 (10013) | 0 (10013) |
| start a process | ok | refused by the job (1816) | refused (1816) | refused (1816) | refused (1816) |
| open registry keys: HKLM `CurrentVersion`, `Windows Media Foundation`, `Uninstall`; HKCR `CLSID`; HKCU `Explorer`, `Environment` | 6/6 | **0/6** (5) | **6/6** | **6/6** (also with `registryRead` alone) | **6/6** |
| environment, job limits, memory cap | (the parent's) | 4 variables, 1 process, 1 GiB, UI limits | same | same | same |

**What was checked.** Per relaxed column: 1 user file, 1 file outside the profile, 2 write targets, 1 loopback connection
and 1 process start. Every relaxed column denies what the product sandbox denies.

**Not checked under the relaxations:**
- parent death;
- the container's own folders;
- other network targets;
- named objects;
- the sandbox crate's own test suite (`tests/sandbox.rs`).

**The measured cost: registry reads.** All three relaxations open the same registry reads. A helper that a hostile file
takes over can read the machine's and the user's registry, including `HKCU\…\Explorer` (where Windows keeps recently used
names) and the user's environment variables. Its only way out is the pipe to the app, which accepts frames and PCM of the
asked sizes, short tags and fixed codes. It could still encode what it read into the pixels it returns, which are shown on
screen and sent nowhere.

**The unmeasured cost: the system services each capability opens.**
- Each of `lpacCom`, `lpacAppExperience`, `lpacIdentityServices` and `lpacMedia` grants what Windows allows that
  capability. Judging by the names: COM, the app-experience services, identity services, and media services. This spike did
  not enumerate them.
- A regular AppContainer joins "all application packages" and reaches everything Windows grants that group. That is a
  larger set; the less-privileged mode exists to cut it.
- Of the two, LPAC+5 is the narrower grant on paper. Neither surface was measured here.

**Product change.** The sandbox crate has no switch today, on purpose ("no feature, no `cfg`, no hidden function"). Any of
these is a change to that contract (D-MED-3, D-OV-5): it needs its own reviewed mode and its own tests, and it must not reach
the Office helper.

## What a route would cost

These are estimates, not measured.
- **The MF system module in the helper:**
  - an `IMFByteStream` over the protocol's `ReadAt` (the spike's in-memory one is the shape);
  - a source reader to NV12/P010, converted to RGBA at the asked size;
  - float PCM out, which the spike already gets from Media Foundation;
  - seek;
  - the `available()` lookup above;
  - tests driven by these fixtures.

  About 3–4 days on Opus.
- **A media mode in the sandbox crate:** a fixed capability list, the crate's tests repeated for that mode, and a security
  review of the five capabilities. About 1–2 days plus the review.
- **Decisions for the lead** (an architectural fork, honesty trigger 1):
  - in-box gap formats: LPAC + `registryRead`, or stay "Open in another app…";
  - HEVC on the system decoder (needs a relaxation): LPAC + the five capabilities (the narrower grant, measured to work),
    or a regular AppContainer, or FilmCraft's HEVC (D-MED-2, counsel), or "Open in another app…";
  - one helper with the HEVC capabilities for every file, or a separate helper mode used only when the file is HEVC.

## Instrument notes

**Session 0 is not the product's session.** Over an SSH session (Windows session 0), the confined probe died in the loader
(`0xC0000142`). `gdi32`, `user32` and `ole32` fail to initialise there (`0x8007045A`; `media-mf-loadcheck`). In the
signed-in desktop session, the same confined process loads all 20 libraries checked. Every sandbox table above is from the
desktop session. Tests of a media helper that run on a service or CI session will see the loader failure, not the decoder
result.

**The copy of the sandbox is faithful.** With nothing changed, the copy (`src/variant/`, the crate's code with three
switches and a larger capability read-back) printed the same lines as the product crate (`v:lpac` = `confined`, 4 of 4
files). The job's UI limits change nothing (`v:lpac+noui` = `confined`).

**Confinement is read back by the probe itself,** before any Media Foundation call: the `T` line, `appcontainer=1` with
the capability count that was asked for. `TokenIsLessPrivilegedAppContainer` is not readable by the process (`0x80070057`),
so the less-privileged mode is known from how the process was started.

**MPEG-4 Part 2 differs from ffmpeg by design.** Its decoders are not required to be bit-exact (the IDCT), so the luma
differs from ffmpeg's. Every column shows the same picture, and the frame counts differ by one from `ffprobe`'s (49 vs 48;
DivX 3: 50/50).

**Interlaced H.264** gave 49 of `ffprobe`'s 50 frames, with frame 0 bit-exact.

**The Windows-encoded files** were checked with `ffprobe`: `wmv3` Main, `vc1` Advanced and `wmapro`, with 50 video frames
each.

**Not done:**
- no Process Monitor (`EnumProcessModules` answered the DLL question);
- no PSNR for MPEG-4 Part 2;
- no GPU;
- no WMA Lossless file;
- no real iPhone file;
- no PC that never had the extension;
- no capability combination other than those listed;
- no enumeration of what each capability grants;
- macOS (out of scope);
- the product crate is unchanged.
