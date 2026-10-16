# Android port plan

Azeroth Survivors runs today as a wasm/WebGPU page (`crates/wenilla`) and as a native desktop
binary (`crates/warcraft-survivors`). This is the plan for a third launcher: an Android build of
the survivors mode. It is written for the people and agents who will do the work, in the same
spirit as [UPSTREAM.md](UPSTREAM.md): what to touch, what not to touch, how to know a phase is
done. Line numbers are as of `dd5e330`; re-check them after a sync.

## Goal, scope, non-goals

**Goal.** A sideloaded APK that boots the survivors mode (`benilla_app::run_survivors`,
`crates/benilla-app/src/lib.rs:230`) on a mid-range Android phone, playable with touch, offline,
reading a WoW 1.12.1 `Data` folder the user supplies.

| in scope | out of scope (non-goals) |
|---|---|
| survivors mode only (`survivors::ACTIVE`, `lib.rs:231`; `connect: false`, `lib.rs:436`) | the full client: login, realm, FrameXML/Lua UI, chat, addons |
| offline, single player | the realm relay, WebBridge, any network use |
| sideloaded APK (`adb install`, or a file the user copies over) | Google Play, AAB, Play Asset Delivery, in-app updates |
| data-less app: the user imports their own `Data` folder | shipping, downloading or mirroring game data from the app |
| landscape, phones and tablets, Vulkan GPUs | portrait, GLES-only devices, Chromebooks (untested, not blocked) |
| touch controls that leave keyboard/mouse working everywhere | gamepad (gilrs is compiled but disabled, `benilla-world/src/boot.rs:83`; could come later) |

**Why data-less and why no store.** The MPQ contents are Blizzard's copyright (README, "Legal").
This repository ships engine code only and reads a client the user already owns. An APK carrying
the data would distribute that content; a store listing would do so to the public under our name.
`scripts/fetch-data.sh` pulls from archive.org on the user's own machine and stays a desktop-side
step. The only defensible shape on a phone is the same one as on the desktop: our code, their
data, their device. The app must boot to a clear "import your Data folder" screen when there is
none, never to a download button.

## Route: native, with a web spike first

### Recommendation: native Android (cargo-ndk + Gradle + GameActivity)

A new launcher crate `crates/warcraft-survivors-android` (cdylib, `#[bevy_main]`), built with
`cargo-ndk` into `jniLibs/`, wrapped by a minimal Gradle project using `GameActivity`. This is
Bevy 0.18's own flow and nearly everything it needs is already in `Cargo.lock`: `android-activity
0.6.1`, `bevy_android 0.18.1`, `ndk`, `jni`, `oboe 0.6.1` (through cpal). `default_platform`
(`Cargo.toml:21-33`) already enables `android-game-activity` and `android_shared_stdcxx`; Bevy
0.19 drops the former from the defaults, so the next bump adds it by name.

Why native over web:

- **Rendering.** The world needs storage buffers in every WGSL shader (WebGL2 is glue-only,
  `benilla-world/Cargo.toml:21-25`), so the browser route needs WebGPU on Android Chrome, which is
  Android 12+ and a GPU allow-list we do not control. Native wgpu on Vulkan is ours to configure.
- **Data.** The wasm path fetches every file by synchronous XHR from a host that unpacks the chain
  (`benilla-formats/src/web.rs:74-79`). On a phone that means either a local HTTP server in the
  app (then it is a native app anyway) or hosting 5 GB per user. Native reads MPQs straight off
  the filesystem (`benilla-mpq/src/lib.rs:228`), which is what the user's imported folder is.
- **Memory.** The wasm build is a 32-bit address space; the texture fallback below can push past
  it. Native aarch64 has the full heap.
- **Lifecycle, audio, back button, storage** all exist as Android APIs on the native side and as
  workarounds on the web side.

### Alternative considered: mobile web / TWA

Serve `web/dist` plus `/data` from a wenilla-host-like service, open it in Chrome or wrap it as a
Trusted Web Activity. Zero new build infrastructure, and all touch work is shared Rust. Rejected
as the shipping route (hosted data is the legal problem again, WebGPU availability, 32-bit heap,
no lifecycle control), but kept as **P0**: the touch input and UI-scaling work is `#[cfg]`-free
Rust in `survivors/`, and a phone with Chrome 121+ and `--enable-unsafe-webgpu` can exercise it
weeks before the APK builds. The spike costs a `<meta viewport>` line on `web/index.html` (dev
page only; `play.html` is not in this checkout).

## Phases

Sizes: S ≤ 1 day, M ≤ 1 week, L > 1 week, for one implementer. Every phase lands as its own PR
against `main`, so `check.yml` gates it and the pin bot sees nothing it should not ship. Android
code that does not compile yet must not break the wasm or native targets, which is what the
`#[cfg(target_os = "android")]` discipline buys.

### P0 · Touch spike on the web page (S/M)

| task | where |
|---|---|
| Add `<meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no">` to `web/index.html`; confirm winit's canvas gets `touchstart`/`touchmove` and Bevy emits `TouchInput` | `web/index.html` (ours) |
| Implement the `SurvivorsInput` abstraction and virtual joystick from "Touch control design" below, no cfg at all | `crates/benilla-app/src/survivors/input.rs` (new) |
| Open the dev page on a phone over LAN (`wenilla-host` is loopback-only; use `adb reverse tcp:8090 tcp:8090`) | docs only |

Exit: the hero walks with a thumb on a phone's Chrome; WASD still works on the desktop page;
`cargo check --target wasm32…` green. This phase is optional if a device with WebGPU is not at
hand; its Rust half is P3's first task either way.

### P1 · It builds and boots (M)

| task | where | fork rule |
|---|---|---|
| New crate `crates/warcraft-survivors-android`: `[lib] crate-type = ["cdylib"]`, `#[bevy_main] fn main()` calling `run_survivors(BuildId{..})` exactly as `warcraft-survivors/src/main.rs:5-16`; `build-dependencies = benilla-buildstamp` (it shells out to the host's git, fine when cross-compiling) | new crate | ours |
| `android/` Gradle project: `GameActivity` subclass, `minSdk 29`, `targetSdk 34`, landscape, `android:hasFragileUserData`, `android:requestLegacyExternalStorage` off | `android/` (new) | ours |
| `scripts/android-setup.sh` (NDK r26+, `cargo-ndk`, `aarch64-linux-android` target) and `scripts/android-build.sh` (`cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release -p warcraft-survivors-android --no-default-features`, then `./gradlew assembleDebug`) | `scripts/` | ours |
| Make the launcher-time env levers no-ops: `tuned_default_plugins` reads `video::boot_window_mode`/`boot_windowed_size` (`lib.rs:355-420`); add `#[cfg(target_os = "android")]` returning `WindowMode::BorderlessFullscreen`/default size in `video.rs` beside the existing android arms at `video.rs:547/569` | `benilla-app/src/video.rs` | cfg arm |
| `crash::install` (`lib.rs:268`, `crash.rs:22`) writes `benilla-config/Diagnostics`; it needs the home from P2, so for P1 gate it to `not(any(wasm32, android))` and log panics to logcat via `android_log` | `lib.rs:267`, `crash.rs` | cfg arm, one line |
| `arboard` has no Android backend: extend the `cfg(not(target_arch = "wasm32"))` at `textinput/clipboard.rs:24` and `Cargo.toml:124` to exclude android; the `not(unix \| macos \| android)` arms at `:92/:108` already exist | `clipboard.rs`, `Cargo.toml` | cfg arm |
| Confirm `third_party/mlua-sys` builds through the generic `cc` path (special arms are wasm32 only, `src/lib.rs:67`); confirm `oboe-sys` links against `android_shared_stdcxx`; confirm `cpal.rs:425-441`'s `SCHED_FIFO` fails without panicking under bionic | build log | none expected |
| Keep `android-activity` at one version (0.6) in the lock; a second copy breaks winit 0.30's `AndroidApp` handoff | `Cargo.lock` | check in `android-setup.sh` |

Exit: `adb install` succeeds; logcat shows the build banner (`benilla_world::build_id::banner`)
and the "no install" message from `wow_data()` (`benilla-formats/src/install.rs:57`) without a
crash; `cargo check --target aarch64-linux-android -p warcraft-survivors-android` passes locally.

### P2 · Data and storage (M)

| task | where | fork rule |
|---|---|---|
| `wow_data()` ladder (`install.rs:57-99`): add an android arm whose single candidate is `<filesDir>/Data`; expose the path from the shim via a `OnceLock<PathBuf>` set from `AndroidApp::internal_data_path()` before `run_survivors` | `install.rs`, shim | cfg arm + new fn |
| `local_state::home()` (`local_state.rs:26-44`): android arm returning `<filesDir>/benilla-config`, same `OnceLock`; `current_exe()` is `/system/bin/app_process` there and unwritable | `local_state.rs` | cfg arm |
| `webenv::var` (`benilla-formats/src/webenv.rs:16`): third arm reading `<filesDir>/benilla-config/env.toml` (a `WOW_*` key/value file), so cvar levers and `WOW_NO_BC` work without a shell | `webenv.rs` | cfg arm |
| Import screen (survivors `Phase::Menu` variant or a pre-menu state): "No game data" panel with two paths: an **Import** button that fires the SAF picker (`ACTION_OPEN_DOCUMENT_TREE`) through a small JNI call in the shim, and a text block with the `adb push` recipe | `survivors/menu.rs`, shim (JNI), `android/` (Kotlin: picker + copy) | new files |
| Copy the needed MPQs from the picked tree into `<filesDir>/Data` (Kotlin, background thread, progress over JNI); needed set = `mount_order`'s names (`chain.rs:111`); skip others | `android/` | ours |
| Validation: after copy, `Chain::open` (`chain.rs:97-125`) on the folder; report the first error in the panel; disk-space check before copy (`StatFs`), with the total from the picked files' sizes | shim + Kotlin | ours |
| Document `adb push /path/to/Data /sdcard/Android/data/<pkg>/files/Data` as the developer route; the app reads `getExternalFilesDir` second if `filesDir` is empty | `docs/ANDROID.md` | ours |

Why copy rather than read through SAF: `benilla-mpq` opens a fresh `File` per read by path
(`lib.rs:228`); a content-URI fd layer would be a new `Archive` backend in an upstream crate.
Copying once is the cheaper carry, and app-private storage is removed on uninstall. A traced
subset (what the sixteen zones in `survivors/data.rs:1389+` touch, captured as `?boottrace=1`
does on web, `web.rs:49`) is a follow-up.

Exit: fresh install → import panel → pick folder → progress → menu with a remembered pick
(`survivors-pick.txt` round-trips through the new `home()`); `adb push` route documented and
tested; uninstall leaves nothing in shared storage.

### P3 · Touch input and UI scaling (L)

See "Touch control design". Files: `survivors/input.rs` (new), `hero.rs:187-311` (`drive_hero`
reads `SurvivorsInput` instead of `ButtonInput<KeyCode>`), `hero.rs:148-183` (`aim_cursor` takes a
screen point from the resource), `hero.rs:313-380` (`place_camera`: pinch → `rig.dist`, `C`
toggle → resource), `mobility.rs:13` (`KEY` stays, also fires on the touch button), `cards.rs:
509-533` (`choose_card` already takes `Interaction::Pressed`), `hud.rs:1054-1085` (Esc/Enter →
resource; Back button → Esc), `hud.rs:1495-1504` (`apply_ui_scale` floor), `minimap.rs:265-300`
and `menu.rs:34-38` (hit-target sizes). All of this is survivors code, which is ours, so no cfg is
needed except the Back-key mapping (winit reports it as `KeyCode::BrowserBack`/`GoBack`; map in
`input.rs`, no cfg).

Exit: a full run (menu pick → play → level up → pause → die → results → menu) is completable with
touch only, on a 6" phone, with no on-screen element under 48 dp; the desktop binary and the web
page play identically to before (`CONTROLS.md` unchanged).

### P4 · Rendering, memory, performance (L)

See "Rendering and memory". Exit: 30 fps sustained in Elwynn and Naxxramas on the reference
device for a 10-minute run; RSS under 1.5 GB; no OOM kill across three runs; `WOW_FPS_JOURNAL`
csv captured per device.

### P5 · Lifecycle and audio (M)

| task | where | fork rule |
|---|---|---|
| Pause on `AppLifecycle::Suspended`/`WillSuspend` (nothing handles it today): set `Phase::Paused`, stop the game clock, mute | `survivors/lifecycle.rs` (new) | new file |
| Surface loss: Bevy 0.18 recreates the surface on resume, but `MsaaSupportPlugin` (`benilla-world/src/view.rs:72-76`) and `BlpGpuSupportPlugin` (`gpu_blp.rs:36-52`) cache adapter facts at `finish`; verify they survive a resume (the adapter does not change, so they should) and that the swapchain format does | test on device | likely none |
| Audio: cpal → oboe → AAudio; stop the output stream on suspend and rebuild on resume in `sound/output/cpal.rs` (android arm), because AAudio streams disconnect on focus loss | `benilla-app/src/sound/output/cpal.rs` | cfg arm |
| Back button: `Paused` ↔ `Playing` like Esc; on the menu, a second press within 2 s exits (`AppExit`, as the Quit buttons at `hud.rs:1045`) | `input.rs` | ours |
| Immersive mode / cutout handling in `GameActivity`; keep the screen on while `Playing` | `android/` | ours |

Exit: home → return ten times mid-run without a crash, with sound back; a phone call during a run
resumes paused; rotation locked.

### P6 · Packaging, CI, docs (S/M)

| task | where |
|---|---|
| `check.yml`: an `android` job with `aarch64-linux-android` target and `cargo check -p warcraft-survivors-android --no-default-features` (needs the NDK: `android-actions/setup-android` or the ubuntu runner's preinstalled NDK, `cargo-ndk` for the linker env) | `.github/workflows/check.yml` |
| Release build: `cargo ndk … --release` with `strip`, `assembleRelease` signed with a local keystore the operator keeps; `WEB_DEBUG`-style `ANDROID_DEBUG=1` keeps symbols for `ndk-stack` | `scripts/android-build.sh` |
| `make android` target; README "Play it on Android" section; `CONTROLS.md` touch table; this file's "where to look when" table | `Makefile`, `README.md`, `CONTROLS.md`, here |
| Add the android crate to `docs/UPSTREAM.md`'s carry list and the AGENTS.md "what lives where" table | docs |

Exit: a PR with an Android-only break fails `check.yml`; `make android` produces an installable APK
from a clean clone with the NDK installed.

## Touch control design

The desktop controls stay the reference (`CONTROLS.md`). Touch adds a second way to produce the
same intents, and the intents are what the game reads.

### Input abstraction

A `survivors/input.rs` with one resource and one system that runs first in `Update`:

```rust
#[derive(Resource, Default)]
pub(super) struct SurvivorsInput {
    pub mov: Vec2,                 // -1..1 per axis, world-relative like WASD
    pub aim: Option<Vec2>,         // a screen point to raycast, else the cursor
    pub jump: bool, pub mobility: bool, pub pause: bool, pub confirm: bool,
    pub camera_toggle: bool, pub grade_toggle: bool,
    pub pick_card: Option<usize>,
    pub zoom: f32,                 // multiplicative notches, wheel or pinch
    pub orbit: Vec2,               // drag delta when the camera is unlocked
}
```

`collect_input` fills it from, in order: keyboard and mouse (the existing `KeyCode`/`MouseButton`
/`MouseWheel`/`AccumulatedMouseMotion` reads, moved verbatim out of `drive_hero`, `place_camera`,
`mobility`, `cards`, `hud`), then `Touches` (bevy's `Res<Touches>`; first-press wins per zone). Any
touch sets a `TouchSeen` flag that switches the HUD into touch layout; a key press switches it
back. The consumers lose their direct `ButtonInput` params and read the resource. This is the one
refactor that touches many survivors lines, but survivors is ours, so it is sync-free.

### Zones and widgets

| control | touch | desktop equivalent |
|---|---|---|
| move | virtual joystick: appears under the first touch in the left 45 % of the screen, floating, 120 dp radius, dead zone 15 %; `mov` is the normalized offset | WASD |
| aim | right 55 %: a drag or tap sets `aim` to its screen point (`aim_cursor` raycasts it the same way as the cursor at `hero.rs:157`); when no touch has aimed for 1.5 s, auto-aim the nearest enemy within 30 yd, else `None` (the hero then faces the move direction, `hero.rs:237-249`, graceful today) | cursor |
| mobility (F) | the existing mobility slot (`hud.rs:398-460`, `MOBILITY_SIZE` 40 → 56 units on touch) becomes a `Button`; fires toward `aim` or forward (`mobility.rs:204-206`) | F |
| jump | a 56-unit button above-right of the mobility slot | Space |
| pause / back | a 48-unit button top-left; Android Back; both → `pause` | Esc |
| cards | tap (`Interaction::Pressed`, `cards.rs:520`; already works); the tilt animation keys off `RelativeCursorPosition` and simply stays flat without a hover | 1/2/3, hover |
| zoom | pinch: `zoom = (d1/d0 - 1) * k` on two touches in the right zone; feeds `rig.dist` at `hero.rs:355` | wheel |
| orbit | two-finger drag when unlocked | mouse drag |
| camera lock, grade | two toggles in the pause panel (`hud.rs:2191` `show_panels`) | C, G |
| tooltips | long-press (500 ms) on an icon shows `icon_tooltip` (`hud.rs:2070`) until release; `xp_hover` (`hud.rs:1595`) the same on the XP bar | hover |
| menu rows | `ROW_H` 34 → 48 on touch (`menu.rs:37`), minimap buttons 32 → 48 (`minimap.rs:258`); the menu's `HoverLabel` becomes a tap-to-highlight | hover/click |

### UiScale floor

`apply_ui_scale` sets `UiScale = height/768` (`hud.rs:1495-1504`). On a 2400×1080 phone that is
0.56 and a 40-unit button lands at ~22 px ≈ 8 dp. On touch, `want = (height/768).max(dpr * 0.9)`
where `dpr = window.scale_factor()`: 1080 p at DPR 2.6 gives 2.3, so a 48-unit button is 110 px ≈
42 dp; the widgets above are sized to reach 48 dp at that scale. Layout is already width-elastic
(landscape widths fit, see findings), so the floor only grows things.

## Rendering and memory

### Textures

`BlpGpuSupportPlugin` (`gpu_blp.rs:36-52`) tests `CompressedImageFormats::BC` at `finish`; without
it `for_upload` (`:71-89`) decodes BC1/2/3 to RGBA8 on the CPU with `texpresso`. Adreno, Mali and
PowerVR report no BC. The cost is 4× (BC3) to 8× (BC1) the VRAM per texture plus decode time per
upload, and the texture pool is a `texture_2d_array` sized to `max_texture_array_layers`. This is
the biggest risk on the port.

Options, in the order to try:

1. **Measure first.** `WOW_NO_BC=1` on the desktop reproduces the exact memory shape; take a
   zone-by-zone RSS/VRAM number there before touching anything.
2. **Mip dropping** (S): on android, drop the top one or two mips of chains wider than 512 in
   `for_upload`'s decode arm. Quarters the fallback cost; the top-down camera never sees texel
   detail at 1:1 anyway. A `WOW_MIP_DROP=n` lever, android default 1.
3. **Quality presets via existing cvars** (S): `farclip`, clutter density, MSAA (`MsaaSetting`),
   `renderScale`, anisotropic, `spellEffectLevel`, `grade` all exist (`benilla-ui/src/script/
   cvars.rs`). Ship an android default `config.toml` under `benilla-config` with `farclip` low,
   MSAA 1, `renderScale` 0.75-0.85 and grade off; the pause panel exposes Low/Medium/High.
4. **ETC2/ASTC transcode at import** (L): decode BC → RGBA8 → encode ASTC 6×6 (every Vulkan 1.1
   Android GPU) once during the P2 copy, written as a sidecar cache under `<filesDir>/cache/
   tex/`, keyed by MPQ name + file hash. Needs an encoder crate (`astc-encoder` via FFI or
   `intel_tex_2`-class), an hour-class one-time cost on a phone, and a new `BlpTexels` arm. Only
   if 2 + 3 fall short of the memory exit criterion.

Bloom and grade are extra full-screen passes; grade off by default on android, bloom gated by
preset.

### Minimum spec

| | minimum | target |
|---|---|---|
| Android | 10 (API 29; scoped storage, Vulkan 1.1 required by CDD for 64-bit) | 13-14 (API 33-34, `targetSdk 34`) |
| ABI | arm64-v8a only | — |
| GPU | Vulkan 1.1, storage buffers in vertex and fragment stages (every WGSL shader needs them), ≥ 256 texture array layers | Adreno 640+ / Mali-G77+ / Snapdragon 7-series 2021+ |
| RAM | 6 GB (the RGBA8 fallback) | 8 GB |
| storage | 7 GB free at import (5.5 GB chain + headroom) | — |

GLES is not an option: `webgl2` is a glue-only feature. No `WgpuSettings` are customised today
(`lib.rs`), so wgpu picks Vulkan on its own; P1 pins `backends: VULKAN` explicitly so a GL
fallback never half-works.

### Surface loss

On pause Android destroys the window; Bevy 0.18's winit backend recreates the surface on
`Resumed`. What must be checked on device (P5): the swapchain format/MSAA clamp from
`MsaaFormats` (`view.rs:43-50`) is per-adapter and should hold; render-world textures are not on
the surface and should hold; anything that cached a `TextureView` of the surface must not. The
cheap guard is to force `Phase::Paused` and skip rendering (`Camera::is_active = false`) between
`WillSuspend` and the first frame after `Resumed`.

## Data import UX

1. First launch with no `Data`: the import panel replaces the pick screen. Text: what is needed
   (a WoW 1.12.1 `Data` folder, ~5.5 GB, from a client you own), two buttons: **Choose folder**
   and **How to adb push**.
2. **Choose folder** → `ACTION_OPEN_DOCUMENT_TREE`. On return the Kotlin side lists the tree,
   matches names case-insensitively against `mount_order`'s set, totals their sizes, checks
   `StatFs(filesDir).availableBytes` against total + 512 MB, and refuses with the shortfall in GB.
3. Copy with a progress bar (bytes, current file, ETA) that survives backgrounding (a foreground
   service with a notification, since 5 GB over a SAF stream takes minutes); partial copies land
   in `Data.partial/` and rename on completion, so a killed import restarts clean.
4. Validation: `Chain::open` on the folder; failure deletes nothing and shows the error with a
   **Re-import** button. Success writes `benilla-config/data-ok` and the pick screen appears.
5. The `adb push` route for developers and for users with a PC: the recipe in the panel and here;
   `getExternalFilesDir` is checked after `filesDir`, so a pushed folder needs no import step.
6. Later: a trimmed subset. A traced run over the sixteen zones on the desktop produces a file
   list; the import copies only those files into a single re-packed MPQ, cutting the import to the
   files the survivors mode reads. The trace is empirical (clutter, creature models and sounds
   differ by seed), so this needs a generous margin and a fallback to the full copy.

## Agents and roles

| workstream | agent / model | why | hand-off |
|---|---|---|---|
| plan review, route decision, P4 and P5 design, any change to an upstream file | Opus-class reviewer | architecture, risk, fork invariant judgment | reviews each phase PR before merge; owns the "does this touch few upstream lines" call |
| P1 build/Gradle/shim, P2 storage arms and JNI, P3 input refactor, P4 mip dropping and presets, P5 audio stream | Fable-class implementer | implementation-heavy Rust and Kotlin, long sessions on one PR | opens the PR with the exit criteria as a checklist; pings the reviewer |
| cfg-gating audits (every `cfg(not(wasm32))` that assumes a desktop fs or `current_exe`), doc sweeps (README, CONTROLS, AGENTS table), lockfile single-version check, line-number refresh in this file after a sync | Haiku/Sonnet-class or Explore agents | mechanical, broad, low judgment | report as a table; the implementer acts on it |
| texture transcode (P4 option 4), traced-subset manifest | Fable-class, with Opus review of the format decision | new on-disk formats are hard to walk back | design note in this file first, PR second |
| device testing: install, lifecycle, thermal, memory, a 10-minute run per zone per device | human with phones | nothing here can be done without hardware; emulators lack the GPU | fills the device matrix below; a red cell blocks the phase |
| CI job, release signing | implementer, operator holds the keystore | the keystore never enters the repo | operator step documented in P6 |

Review gates: every phase PR needs the reviewer's pass on fork rules and the human's device-matrix
row for the phase's exit criterion. P4 and P5 additionally need a before/after `WOW_FPS_JOURNAL`
csv attached to the PR.

## Risk register

| risk | likelihood | impact | mitigation |
|---|---|---|---|
| RGBA8 texture fallback blows the memory budget on 6 GB phones | high | high | measure with `WOW_NO_BC=1` on desktop first; mip drop; presets; ASTC transcode as the last resort |
| A device's Vulkan driver rejects a shader or a bind group layout (storage buffer in vertex stage, array layers) | medium | high | pin Vulkan; test the three GPU families early in P1 with a single zone; `naga` validation in CI catches nothing device-specific, so the matrix is the gate |
| Surface recreation on resume breaks a cached view or the MSAA clamp | medium | medium | P5 guard (pause and skip rendering across suspend); test ten home/return cycles |
| `oboe-sys` C++ link or AAudio disconnect on focus loss | medium | medium | `android_shared_stdcxx` is on; rebuild the stream on resume |
| SAF copy is slow or killed mid-way | medium | medium | foreground service, `.partial` staging, resumable |
| Upstream sync conflicts on the new cfg arms | medium (every sync) | low | arms are small and sit next to existing wasm32 arms; list them in UPSTREAM.md |
| Bevy 0.19 drops `android-game-activity` from defaults | certain at the bump | low | feature by name in the android crate |
| Thermal throttling makes a 10-minute run unplayable at 30 fps | medium | medium | presets; 30 fps cap (`present_mode` + a frame limiter); test the full run, not the first minute |
| Legal exposure from an APK that looks like it ships Blizzard content | low with a data-less APK | high | no data, no download, no store; the import panel's text says whose data it is |
| `benilla-buildstamp` fails when cross-compiling without git | low | low | it already runs on the host for every target; stamp falls back to `unknown` |

### Open questions for the owner

1. Which phone(s) are the reference devices? The matrix below needs at least one Adreno and one
   Mali device in hand before P1 ends.
2. Is a 5.5 GB import acceptable for v1, or is the traced subset a v1 requirement?
3. Auto-aim when no touch is aiming: on (recommended for the right-thumb-free case) or strictly
   manual, which the desktop never does either?
4. Should the android launcher take `dev` features off entirely (`--no-default-features`, as the
   web build does), losing the captures and probes, or keep them behind `env.toml`?
5. Target framerate: 30 fps locked (cooler, longer sessions) or 60 fps on the High preset?

## Testing strategy

| layer | what | where it runs |
|---|---|---|
| unit, host | `input.rs`: joystick normalization, dead zone, pinch → zoom notches, Back → pause transitions; `install.rs` android candidate via `candidates_from` (it already takes the env as arguments); `local_state` home arm with `BENILLA_HOME` pinned | `cargo test -p benilla-app --lib survivors::input`, no data needed; `check.yml` `client-carries` job |
| compile, CI | `cargo check --target aarch64-linux-android -p warcraft-survivors-android --no-default-features` as a new `check.yml` job; fails on any `std::env::current_exe` or desktop-only dep an android arm missed | GitHub Actions |
| compile, CI | the existing wasm and native checks must stay green: an android arm must not change the other targets' code paths | existing jobs |
| emulator | API 34 arm64 image for install, import UX, SAF, lifecycle and Back-button tests only. **Do not use it for rendering or memory numbers:** the emulator's SwiftShader/ANGLE Vulkan is slow, often lacks storage-buffer limits and array-layer counts of real GPUs, and reports BC support the real devices lack, so it hides the fallback cost | developer machine |
| device matrix | per phase exit criterion, filled by the device-test role | phones |

Device matrix (fill in as devices are available):

| device | SoC / GPU | Android | RAM | P1 boots | P2 import | P3 touch run | P4 fps/RSS | P5 resume ×10 |
|---|---|---|---|---|---|---|---|---|
| reference Adreno (e.g. Snapdragon 7/8 Gen 1-2) | Adreno 6xx/7xx | 13-14 | 8 GB | | | | | |
| reference Mali (e.g. Pixel 7/8, Tensor) | Mali-G710/G715 | 14 | 8-12 GB | | | | | |
| minimum-spec | Adreno 640-class | 10-11 | 6 GB | | | | | |
| tablet | any | 13+ | 8 GB | | | | | |

Performance captures use the existing `WOW_FPS_JOURNAL=<csv>` lever through `env.toml`
(`benilla-config/Diagnostics/fps-journal.csv`, pulled with `adb pull`), so the android numbers
are comparable with the desktop ones in [PERFORMANCE.md](PERFORMANCE.md).

## Where to look when

| symptom | look at |
|---|---|
| the APK exits at once | `adb logcat -s RustPanic:* wgpu:*`; a duplicated `android-activity` in `Cargo.lock` |
| "no install" after an import | the `wow_data()` android arm and the `OnceLock` set before `run_survivors`; `adb shell run-as <pkg> ls files/Data` |
| textures crawl in, then OOM | `blp: device reports no BC support` in the log: the RGBA8 fallback; P4 options in order |
| no sound after returning from home | the AAudio stream disconnected: the P5 rebuild arm in `sound/output/cpal.rs` |
| touch works on the web spike, not in the APK | the Java side: `GameActivity` input filtering; the Rust half is shared |
