# M10: can Media Foundation decode inside the media helper's sandbox? (spike result, 2026-11-15)

Throwaway spike, evidence only; nothing here merges into `v2-native`. The question comes from decision 2540 (D-MED-1, the gap
formats) and the owner's 17:30 ruling: HEVC plays on the system decoder.

**Test PC:** Windows 11 23H2 (10.0.22631), no GPU (Microsoft Basic Display Adapter). Microsoft's HEVC Video Extensions
2.4.149 and VP9 Video Extensions 1.2.20 are installed for the test user. Every run is in the signed-in desktop session (why:
"Instrument notes").

**Fixtures:** made for this spike, with no third-party content (320x180, 25 fps, 2 s; 48 kHz stereo, 440/660 Hz plus
14 kHz).
- `make-fixtures.sh`: ffmpeg test sources; HE-AAC from fdk-aac.
- `src/make.rs`: WMV 9, VC-1 and WMA Pro from Windows' own encoders, since ffmpeg has no encoder for them.

**Reference:** each fixture has the SHA-256 of frame 0's visible luma as ffmpeg decodes it. The probe hashes the same
plane: 8-bit from NV12, 16-bit from P010.

## The answer

1. **The helper's sandbox as it is today cannot run Media Foundation at all.** The sandbox is an AppContainer that opted out of
   "all application packages", with zero capabilities. `CoInitializeEx` fails with `0x80070005`, then `MFStartup` with
   `0x80004001`. That holds for every format: 49 of 49 confined runs that start Media Foundation, over 5 desktop-session runs.
2. **One capability, `registryRead`, is enough for every in-box decoder:**
   - H.264, including interlaced;
   - WMV 8, WMV 9 and VC-1;
   - MPEG-4 Part 2 (Xvid, DivX 3);
   - AAC-LC, full HE-AAC v1/v2, MP3, WMA 2 and WMA Pro.

   Every picture and every audio hash is the same as the unconfined decode. `lpacCom` and `lpacMedia` are not needed.
3. **Store-packaged decoders need more: HEVC (the required path) and VP9.** They decode only in a *regular* AppContainer: one
   that has not opted out of "all application packages", still with zero capabilities.
   - Under any less-privileged variant (`registryRead`, `+lpacCom`, `+lpacMedia`), Media Foundation lists no HEVC decoder,
     even with the extension installed.
   - It fails exactly as on a PC without the extension (section "HEVC").
   - **The helper would tell the person "without Microsoft's HEVC extension" while it is installed.**
4. **Media Foundation does not fill these gaps:**
   - 10-bit H.264 and 4:2:2 H.264: the decoder accepts the type, then fails the first read with `0x80004005`.
   - 4:2:2 HEVC: it decodes all 50 frames, but frame 0's luma differs from the reference. A conformant HEVC decode is
     bit-exact, as the Main and Main 10 rows show, so this row is not trusted.
5. **Not measured:**
   - Hardware decoding: the test PC has no GPU.
   - WMA Lossless: there is no test file. The in-box encoder offers no output type without a VBR setup, and ffmpeg has no
     encoder for it. Its decoder is listed in every column where Media Foundation starts.

## Per format

The columns, left to right:
- **unconfined**: a plain child process. Run twice: with the parent's environment, and with exactly the sandbox's four
  variables and working directory. Both runs printed identical lines.
- **product sandbox**: `beebeeb-office-sandbox`, unchanged.
- **LPAC+registryRead**: the spike's copy of the sandbox, with that one capability added.
- **AppContainer**: the spike's copy, with the opt-out removed and zero capabilities.

`ok` means: no error; the frame count shown; the luma hash equal to ffmpeg's where marked "bit-exact"; and the same picture
and audio hash in every column that decoded.

| Format (fixture) | unconfined | product sandbox | LPAC+registryRead | AppContainer | MF route in the helper |
|---|---|---|---|---|---|
| H.264 High (`h264.mp4`) | ok, 50/50, bit-exact | `MFStartup 0x80004001` | ok, same | ok, same | yes, with `registryRead` |
| H.264 interlaced (`h264-interlaced.ts`) | ok, 49 of 50, frame 0 bit-exact | same failure | ok, same | ok, same | yes, with `registryRead` |
| H.264 10-bit (`h264-10bit.mp4`) | `v-read 0x80004005`, 0 frames | same failure | `0x80004005` | `0x80004005` | **no** (no decoder) |
| H.264 4:2:2 (`h264-422.mp4`) | `v-read 0x80004005`, 0 frames | same failure | `0x80004005` | `0x80004005` | **no** |
| **HEVC Main, MP4 / MOV / hev1** | ok, 50/50, bit-exact | same failure | **`0xC00D5212`, decoder not listed** | ok, bit-exact | **only in a regular AppContainer** |
| **HEVC Main 10 (HLG tags), MP4 / MOV** | ok, 50/50, P010, bit-exact | same failure | **`0xC00D5212`** | ok, bit-exact | **only in a regular AppContainer** |
| HEVC 4:2:2 10-bit (`hevc-422-10.mp4`) | 50 frames, luma differs from the reference | same failure | `0xC00D5212` | as unconfined | no (not trusted) |
| WMV 8 + WMA 2, ASF (`wmv2.wmv`) | ok, 50/50, bit-exact; audio ok | same failure | ok, same | ok, same | yes, with `registryRead` |
| WMV 9 Main + WMA Pro, ASF (`wmv3-wmapro.wmv`) | ok, 50/50; audio ok | same failure | ok, same | ok, same | yes, with `registryRead` |
| VC-1 Advanced, ASF (`wvc1.wmv`) | ok, 50/50 | same failure | ok, same | ok, same | yes, with `registryRead` |
| WMA 2 / WMA Pro (`wmav2.wma`, `wmapro.wma`) | ok | same failure | ok, same | ok, same | yes, with `registryRead` |
| WMA Lossless | decoder listed, no file | MF does not start | decoder listed | decoder listed | not measured |
| MPEG-4 Part 2, Xvid ASP with B-frames, AVI (`mpeg4p2.avi`) | ok, 49 frames; luma ≠ ffmpeg (expected, see "Instrument notes") | same failure | ok, same | ok, same | yes, with `registryRead` |
| MPEG-4 Part 2, DivX 3 (MP43), AVI (`msmpeg4v3.avi`) | ok, 50/50; luma ≠ ffmpeg (expected, see "Instrument notes") | same failure | ok, same | ok, same | yes, with `registryRead` |
| HE-AAC v1, SBR (`he-aac.m4a`) | ok, full (below) | same failure | ok, same | ok, same | yes, with `registryRead` |
| HE-AAC v2, SBR+PS (`he-aac-v2.m4a`) | ok, full (below) | same failure | ok, same | ok, same | yes, with `registryRead` |
| AAC-LC, MP3 (controls) | ok | same failure | ok, same | ok, same | yes |
| VP9, Store extension (`vp9.webm`, control) | ok, 50/50, bit-exact | same failure | `0xC00D5212`, not listed | ok, bit-exact | only in a regular AppContainer |
| MPEG-2, program stream (`mpeg2.mpg`, control) | `0xC00D36B4` at the output type | same failure | same | same | no (not a gap row: the plan decodes MPEG-2 in Rust) |

- **"Same failure"** means `CoInitializeEx 0x80070005`, then `MFStartup 0x80004001`.
- **Full HE-AAC.** The probe measures the first second of the left channel: `hf` = RMS of the first difference over the RMS,
  and `lr` = RMS(left − right) over RMS(left). A core-only decode loses the 14 kHz tone (`hf` near 0.06), and for v2
  without PS the left and right channels are equal (`lr` = 0). Media Foundation matches ffmpeg's full decode:

  | File | Media Foundation | ffmpeg |
  |---|---|---|
  | HE-AAC v1 | hf 0.3029, lr 1.4475, output 48 kHz | hf 0.3015, lr 1.4476 |
  | HE-AAC v2 | hf 0.7283, lr 1.2759 | hf 0.7284, lr 1.2764 |

  **This removes the "Reduced quality on this PC" case (D-MED-7) wherever the MF route runs.**

## HEVC, the required path

**With the extension, in a regular AppContainer:**
- Main and Main 10 decode 50 of 50 frames, in MP4 and in MOV (`hvc1`, as an iPhone writes it) and with `hev1`.
- Frame 0's luma is bit-exact against ffmpeg: NV12 for Main, P010 for Main 10.
- The decoder is `HEVCDECODER_STORE.dll`, loaded from the package `Microsoft.HEVCVideoExtension_2.4.149.0_x64__8wekyb3d8bbwe`
  (seen with `EnumProcessModules`).

**Without the extension, measured without removing anything.** The extension is registered per user. A process under an
account with no registration (LocalSystem, unconfined) shows:
- `MFTEnumEx(video decoder, input {Video, HEVC})` → **0 decoders**, with all three flag sets tried (`ALL`,
  `ALL|UNTRUSTED_STOREMFT`, the default sorted set). H.264 still lists 1 decoder.
- The source still opens, and its native subtype is **HEVC**. So the helper knows it is HEVC (and the audio still decodes).
- `SetCurrentMediaType` for NV12, P010, YUY2 and RGB32 → **`0xC00D5212` (MF_E_TOPO_CODEC_NOT_FOUND)**, every one.
- No module from a Store package loads.

The less-privileged sandbox variants print exactly these lines while the extension is installed.

**For `native/beebeeb-media-helper/src/system.rs::available(Hevc)`:**
- Ask `MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, …, input {MFMediaType_Video, MFVideoFormat_HEVC})` inside the helper.
  0 decoders means `Error(NoSystemDecoder(Hevc))`.
- A source that reports HEVC, followed by `0xC00D5212` on the output type, is the same answer, caught later.
- Both are only true answers in a sandbox that can see packages. In a less-privileged one they always say "missing".

**Two more measurements:**
- **Cost of a failed lookup:** in the less-privileged variants, an HEVC open took 1.9–3.6 s to fail. In the regular
  AppContainer it took 0.3 s to decode.
- **D3D11 inside the sandbox:** a D3D11 device with video support opens in both sandbox variants. On this PC it is the
  software adapter, so the decoders report `hardware=0`. With that device manager, HEVC Main 10 fails its first read with
  `0xC00D36B4`. This row says nothing about real GPUs.

## What the sandbox would have to give up (measured with the sandbox crate's own attack probe)

| Attempt | unconfined | product sandbox | AppContainer | LPAC+lpacCom+registryRead |
|---|---|---|---|---|
| read a file in the user's Documents | ok | denied (5) | denied (5) | denied (5) |
| read a test file outside the profile | ok | denied (5) | denied (5) | denied (5) |
| read system files (hosts, win.ini) and a file under Program Files | ok | ok | ok | ok |
| write to the user's TEMP, or to Public | ok | denied (5) | denied (5) | denied (5) |
| loopback connection to a listener of the runner (accepted) | 1 | 0 (network start refused, 10107) | 0 (timed out) | 0 (10013) |
| start a process | ok | refused by the job (1816) | refused (1816) | refused (1816) |
| open registry keys: HKLM `CurrentVersion`, `Windows Media Foundation`, `Uninstall`; HKCR `CLSID`; HKCU `Explorer`, `Environment` | 6/6 | **0/6** (5) | **6/6** | **6/6** (also with `registryRead` alone) |
| environment, job limits, memory cap | (the parent's) | 4 variables, 1 process, 1 GiB, UI limits | same | same |

**What stays.** Both relaxations keep the four promises of the sandbox crate:
- no network;
- no write anywhere, and no read of the user's files;
- no child process;
- gone with its parent (the job is unchanged).

**The measured cost: registry reads.** A helper that a hostile file takes over can read the machine's and the user's
registry, including `HKCU\…\Explorer` (where Windows keeps recently used names) and the user's environment variables. Its
only way out is the pipe to the app, which accepts frames and PCM of the asked sizes, short tags and fixed codes. It could
still encode what it read into the pixels it returns, which are shown on screen and sent nowhere.

**The cost of the regular AppContainer, partly measured.** On top of the registry, the regular AppContainer joins "all
application packages":
- It can reach whatever Windows grants that group: system services and objects that accept AppContainer callers. The
  less-privileged mode exists to cut exactly that surface.
- This spike did not enumerate that surface.

**Product change.** The sandbox crate has no switch today, on purpose ("no feature, no `cfg`, no hidden function"). A media
helper with `registryRead`, or one without the opt-out, is a change to that contract (D-MED-3, D-OV-5): it needs its own
reviewed mode and its own tests, and it must not reach the Office helper.

**Not done here:** the sandbox crate's full test suite under either relaxation (`tests/sandbox.rs`, which also covers the
profile folder and parent-death).

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
- **A sandbox mode for media:** `registryRead`, or a regular AppContainer for HEVC; the crate's tests repeated per mode; a
  security review. About 1–2 days plus the review.
- **Decisions for the lead** (an architectural fork, honesty trigger 1):
  - in-box gap formats: LPAC + `registryRead`, or stay "Open in another app…";
  - HEVC: a regular AppContainer, which the HEVC requirement needs, or FilmCraft's HEVC (D-MED-2, counsel), or HEVC as
    "Open in another app…".

## Instrument notes

**Session 0 is not the product's session.** Over an SSH session (Windows session 0), the confined probe died in the loader
(`0xC0000142`). `gdi32`, `user32` and `ole32` fail to initialise there (`0x8007045A`; `media-mf-loadcheck`). In the
signed-in desktop session, the same confined process loads all 20 libraries checked. Every table above is from the desktop
session. Tests of a media helper that run on a service or CI session will see the loader failure, not the decoder result.

**The copy of the sandbox is faithful.** With nothing changed, the copy (`src/variant/`, the crate's code with three
switches) printed the same lines as the product crate (`v:lpac` = `confined`, 4 of 4 files). The job's UI limits change
nothing (`v:lpac+noui` = `confined`).

**Confinement is read back by the probe itself,** before any Media Foundation call: the `T` line, `appcontainer=1` with
the capability count that was asked for. `TokenIsLessPrivilegedAppContainer` is not readable by the process (`0x80070057`),
so the less-privileged mode is known from how the process was started.

**MPEG-4 Part 2 differs from ffmpeg by design.** Its decoders are not required to be bit-exact (the IDCT), so the luma
differs from ffmpeg's. Every column shows the same picture, and the frame counts differ by one from `ffprobe`'s (49 vs 48;
DivX 3: 50/50).

**Interlaced H.264** gave 49 of `ffprobe`'s 50 frames, with frame 0 bit-exact.

**Not done:**
- no Process Monitor (`EnumProcessModules` answered the DLL question);
- no PSNR for MPEG-4 Part 2;
- no GPU;
- no WMA Lossless file;
- no PC that never had the extension;
- macOS (out of scope);
- the product crate is unchanged.
