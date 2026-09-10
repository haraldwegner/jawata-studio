mod backups;
mod client_dialect;
mod commands;
mod config;
mod field_view;
mod gateway;
mod lombok;
mod conductor;
mod manager_service;
mod release_manager;
mod resident;
// MEASURED, Sprint 28 Stage 4 (D-UNWIRED) — the previous note here claimed
// this module was "consumed by the seat stages (9-14) + UI wiring". It is not.
// Production reaches exactly the seat-DEFINITION half — load_seat_definitions,
// parse_seat_definition, SeatDefinition, GateClass — from manager_service and
// conductor. The seat EXECUTION engine (run_seat, run_phase, the CLI adapters,
// the gate executors, proposals, journal, scheduler, shadow-apply) has no
// production caller: 72 items reachable only from this module's own tests,
// because the seats now run by stance handoff in the client rather than
// through a hosted runner.
//
// The attribute stays so the build is not buried in 74 warnings, but it no
// longer hides anything: build/unwired-gate.sh audits with --force-warn
// dead_code, which sees through it, and every one of those items is in the
// committed baseline.
//
// Whether the hosted runner ships or goes is SPRINT 28b's call (Lane B hosted
// execution — the sprint whose subject is exactly "the runner drives seats",
// and which already declares a dependency on Sprint 28). Shipping Lane B is
// what would give these items a production caller; deferring it again makes
// deleting them the honest alternative. The measurement is staged in that
// sprint's doc so the decision starts from evidence.
#[allow(dead_code)]
mod runner;
mod runtime_manager;

use config::ConfigStore;
use manager_service::ManagerService;
use release_manager::ReleaseManager;
use runtime_manager::{RuntimeManager, RuntimePhase};
use serde::Serialize;
use tauri::{
    image::Image,
    menu::{Menu, MenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};

pub struct AppState {
    pub manager_service: ManagerService,
}

const TRAY_ICON_SIZE: u32 = 32;

/// Stable id of the singleton tray icon. The tray is built once in setup
/// and looked up via `app_handle.tray_by_id(TRAY_ICON_ID)` whenever the
/// menu needs rebuilding.
const TRAY_ICON_ID: &str = "jawata-tray";

/// Sprint 13 (v0.13.0): how often to refresh the tray menu in the
/// background. Catches both (a) status changes from external events
/// (workspace's jawata process killed from a shell) and (b) workspace
/// composition changes from the main window (rename / add / delete).
///
/// Set to 1 second so a rename in the dashboard propagates to the tray
/// within ~1 s, matching user expectation that "the tray follows what I
/// just changed". Per-tick cost is sub-50 ms (one
/// `workspace_status_summary` + one GTK menu rebuild + one D-Bus
/// `set_menu` to AppIndicator), well under 1 % CPU.
const TRAY_REFRESH_INTERVAL_SECS: u64 = 1;

/// Sprint 28b (D6): how long the canary waits before its first round, so a
/// resident that is still booting (OSGi + JDT, ~30 s) is not called degraded
/// for being slow to start.
///
/// A CEILING, NOT A FLOOR — and until now it was a floor. It was a plain
/// `thread::sleep`, so the wake channel could not reach it: a wake that arrives
/// while it is sleeping is buffered and consumed at the END of the first loop
/// iteration, which is after the first round has already run. The tray therefore
/// stayed amber for the full 45 s even when every resident was ready in ten, and
/// the one signal that says "booting finished" could not shorten it.
///
/// The deferral itself is right: without it a resident mid-boot is reported
/// degraded for being slow. What was wrong is that it ignored evidence. It is a
/// `recv_timeout` now, so "a resident just started" starts the first round at
/// once and the 45 s is only what happens when nothing says otherwise.
const CANARY_FIRST_DELAY_SECS: u64 = 45;

/// And how often it asks again WHEN THE LAST ANSWER WAS GOOD. Two real
/// round-trips per resident is not free, and a healthy channel that dies stays
/// dead — five minutes is soon enough to notice and rare enough to be invisible.
const CANARY_INTERVAL_SECS: u64 = 300;

/// How soon it asks again when the last answer was NOT good (studio#21).
///
/// Five minutes is the right cadence for confirming health and the wrong one
/// for confirming RECOVERY. Measured live on 2026-08-18: after a resident
/// restart both residents answered the canary's own probes correctly while the
/// tray was still amber, because the verdict was up to five minutes old. The
/// light's whole job is to be current when someone looks at it, so an unhappy
/// verdict is re-checked quickly and a happy one is not.
const CANARY_RECHECK_SECS: u64 = 15;

impl field_view::CanaryHealth {
    /// How long to wait before asking again, given what this round just learned.
    ///
    /// EXHAUSTIVE ON PURPOSE — there is no `_` arm, so adding a variant to
    /// `CanaryHealth` does not compile until someone decides its cadence. That is the
    /// property this method exists for, and it is the one a catch-all destroys.
    ///
    /// It is here rather than at the call site because the answer is a fact about the
    /// VERDICT, not about the loop: `StoreHealth::word` already states the same
    /// principle one type over. Written as a `match` in the thread, `Reduced` fell
    /// through `_` into the 15-second recheck the moment it was introduced — so a
    /// machine with one workspace deliberately switched off probed every remaining
    /// resident twenty times more often than intended, permanently, in exactly the
    /// configuration studio#48 exists to render correctly. Nothing failed; the light
    /// was right; only the cost was wrong, which is why no test and no user could see
    /// it.
    pub(crate) fn recheck_after_secs(self) -> u64 {
        match self {
            // Nothing is wrong, so there is nothing to confirm the recovery of.
            field_view::CanaryHealth::Green => CANARY_INTERVAL_SECS,
            // Nor here: a workspace switched off is a DECISION, not a fault. It earns
            // the slow cadence for the same reason Green does, and giving it the fast
            // one would charge a user for having configured their machine.
            field_view::CanaryHealth::Reduced => CANARY_INTERVAL_SECS,
            // These three are the ones a user is most likely to be staring at while
            // they are already out of date.
            field_view::CanaryHealth::Degraded
            | field_view::CanaryHealth::Loading
            | field_view::CanaryHealth::Unknown => CANARY_RECHECK_SECS,
        }
    }
}

/// How often the studio re-asks the one CHEAP question: can each resident read
/// its own workspace?
///
/// studio#26. Five seconds, and it is not a compromise between load and
/// freshness — it is measured. The question costs about twenty milliseconds
/// against the largest workspace on this machine (twenty-nine projects),
/// because the resident keeps the answer current and merely hands it over. The
/// five-minute spacing above protects the OTHER probe, which asks each resident
/// two real questions to prove its engine still answers.
const READABILITY_INTERVAL_SECS: u64 = 5;

#[derive(Clone, Copy, Debug)]
enum TrayIconVariant {
    /// The jawata arch mark (Sprint 22b brand) on the batik-indigo circle.
    ArchCircle,
    CoffeeCircle,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuitPromptEvent {
    source: String,
    running_services: usize,
    tray_enabled: bool,
}

fn selected_tray_icon_variant() -> TrayIconVariant {
    match std::env::var("JAWATA_TRAY_ICON")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "coffee" | "cup" => TrayIconVariant::CoffeeCircle,
        _ => TrayIconVariant::ArchCircle,
    }
}

/// The branded tray icon: the glyph on the batik-indigo disc. Used on EVERY
/// platform, macOS included.
///
/// Sprint 28 (v3.6.3): macOS gets this one, not a template image. v3.6.0
/// introduced a macOS-only template variant on the reasoning that an opaque
/// dark disc would be invisible against a dark menu bar. **That reasoning was
/// false, and the counterexample was in this product's own predecessor:**
/// javalens-manager drew the same full-bleed opaque disc (`#1c3a74`, white
/// glyph), passed it with no template flag, and had zero macOS-specific code
/// in its tray path — and its icon was visible on the same Mac. macOS renders
/// a non-template status-item image in full colour; template mode uses the
/// alpha channel alone, and that is what made the icon disappear. v3.6.1 then
/// changed the template's SHAPE, which was fixing the wrong thing twice.
fn build_tray_icon(variant: TrayIconVariant) -> Image<'static> {
    build_tray_icon_for(variant, field_view::CanaryHealth::Green)
}

/// The same mark, tinted by the canary verdict (Sprint 28b, D6).
fn build_tray_icon_for(
    variant: TrayIconVariant,
    health: field_view::CanaryHealth,
) -> Image<'static> {
    let mut rgba = vec![0u8; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];
    match tray_disc_style(health) {
        DiscStyle::Filled(fill) => {
            draw_base_circle_in(&mut rgba, fill);
            match variant {
                TrayIconVariant::ArchCircle => draw_arch_glyph(&mut rgba),
                TrayIconVariant::CoffeeCircle => draw_coffee_glyph(&mut rgba),
            }
        }
        DiscStyle::Hollow(stroke) => {
            draw_base_ring_in(&mut rgba, stroke);
            match variant {
                TrayIconVariant::ArchCircle => draw_arch_glyph_in(&mut rgba, stroke),
                TrayIconVariant::CoffeeCircle => draw_coffee_glyph_in(&mut rgba, stroke),
            }
        }
    }
    Image::new_owned(rgba, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
}

/// Raise the main window, and on macOS make Studio a foreground app first.
///
/// Sprint 28 (v3.6.1): the ONE way the window is shown, so the activation
/// policy and the window state can never disagree. macOS ties Dock presence and
/// the Cmd+Tab list to `NSApplicationActivationPolicy`: `Regular` puts the app
/// in both, `Accessory` in neither while still allowing a status item and
/// windows. Studio is a service manager, so it is a foreground app only while
/// its window is open — see `hide_main_window` for the other half.
fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    // Bring the process itself forward — with no visible window macOS does not
    // activate the app on its own, so the freshly shown window would open
    // behind whatever the user was in.
    #[cfg(target_os = "macos")]
    let _ = app.show();
}

/// Hide the main window to the tray, and on macOS drop out of the foreground.
///
/// Sprint 28 (v3.6.1): the counterpart to `show_main_window`. Once the window
/// is hidden Studio is doing exactly what the jawata residents do — running
/// services in the background — so it stops occupying a Cmd+Tab slot. A pinned
/// Dock item is unaffected: that is a Launch Services shortcut, not the
/// running-app list, and clicking it reaches the running instance through
/// `RunEvent::Reopen`.
fn hide_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

fn set_px(rgba: &mut [u8], x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= TRAY_ICON_SIZE as i32 || y >= TRAY_ICON_SIZE as i32 {
        return;
    }
    let idx = ((y as u32 * TRAY_ICON_SIZE + x as u32) * 4) as usize;
    rgba[idx] = color[0];
    rgba[idx + 1] = color[1];
    rgba[idx + 2] = color[2];
    rgba[idx + 3] = color[3];
}

fn draw_rect(rgba: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            set_px(rgba, x, y, color);
        }
    }
}

fn draw_line_v(rgba: &mut [u8], x: i32, y0: i32, y1: i32, thickness: i32, color: [u8; 4]) {
    for dx in 0..thickness {
        draw_rect(rgba, x + dx, y0, x + dx, y1, color);
    }
}

fn draw_ring(rgba: &mut [u8], cx: i32, cy: i32, radius: i32, thickness: i32, color: [u8; 4]) {
    let min = -radius - thickness;
    let max = radius + thickness;
    let inner = (radius - thickness).max(0);
    let outer2 = radius * radius;
    let inner2 = inner * inner;
    for dy in min..=max {
        for dx in min..=max {
            let d2 = dx * dx + dy * dy;
            if d2 <= outer2 && d2 >= inner2 {
                set_px(rgba, cx + dx, cy + dy, color);
            }
        }
    }
}

/// How the tray disc is drawn for a given canary verdict: a colour, and whether the
/// disc is filled or an outline.
///
/// studio#48, and the palette is Harald's (2026-09-10). Two things changed at once and
/// they are separate claims.
///
/// **Healthy stopped being the brand colour.** It was batik-indigo `#1d2f4e`, which made
/// health the LOGO rather than a signal: meaning existed only in the departure from it,
/// so the tray carried one bit, and a rebrand would silently change what it meant.
/// Semantics must not hang off a brand asset. Every state now has a colour chosen for
/// meaning.
///
/// **`Unknown` moved to GREY rather than to amber.** Unknown is "not probed yet" — not
/// measured, not broken. Painted amber, every studio start would show a fault until the
/// first round landed, which is the complaint this issue came from. So grey is the
/// catch-all and what it catches is *we do not know*; amber is reserved for *we looked,
/// and it is wrong*, which is what makes amber worth reacting to.
///
/// It also honours a promise the enum makes and the tray used to break: `Unknown`
/// documents itself as "never rendered as green" while being painted byte-identical to
/// [`field_view::CanaryHealth::Green`].
///
/// **The reduced state is a SHAPE, not a fourth hue.** A hollow disc reads as "less"
/// without reading as "wrong", needs no extra colour at 22 px, and survives both a
/// rebrand and a colour-blind reader. Grass green rather than the dark green for that
/// one is practical: a hollow disc in the dark green nearly vanishes against a dark
/// shell panel, and a ring needs contrast to read as a ring.
///
/// The one weak pair is green against amber, which deuteranopia flattens and which —
/// unlike the two greens — has no shape difference to fall back on. That is why the
/// state is also carried in WORDS in the tray tooltip: it is the channel that still
/// works when the colour channel fails, not a nicety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscStyle {
    /// A filled disc, with the glyph in its own contrast colour.
    Filled([u8; 4]),
    /// An outline, with the glyph drawn in the same stroke — see `draw_arch_glyph_in`
    /// for why the glyph cannot keep its cream here.
    Hollow([u8; 4]),
}

fn tray_disc_style(health: field_view::CanaryHealth) -> DiscStyle {
    match health {
        // Dark green (#1b5e20) — legible against a light and a dark menu bar alike.
        field_view::CanaryHealth::Green => DiscStyle::Filled([27, 94, 32, 255]),
        // Grass green (#7cb342), hollow: running as configured, and visibly less.
        field_view::CanaryHealth::Reduced => DiscStyle::Hollow([124, 179, 66, 255]),
        // Amber (#a8621a) — we looked, and it is wrong.
        field_view::CanaryHealth::Degraded => DiscStyle::Filled([168, 98, 26, 255]),
        // Grey (#6e7278) — we do not know yet: mid-import, or not yet probed.
        field_view::CanaryHealth::Loading | field_view::CanaryHealth::Unknown => {
            DiscStyle::Filled([110, 114, 120, 255])
        }
    }
}

/// The ring the hollow variant draws, at the filled disc's own outer radius so the two
/// marks occupy the same slot and swapping between them does not appear to resize.
fn draw_base_ring_in(rgba: &mut [u8], stroke: [u8; 4]) {
    let center = (TRAY_ICON_SIZE as i32) / 2;
    draw_ring(rgba, center, center, center, 2, stroke);
}

fn draw_base_circle_in(rgba: &mut [u8], fill: [u8; 4]) {
    let center = (TRAY_ICON_SIZE as i32) / 2;
    // Draw slightly beyond the nominal radius so the circle nearly fills the tray slot.
    let radius = center + 1;
    for y in 0..TRAY_ICON_SIZE as i32 {
        for x in 0..TRAY_ICON_SIZE as i32 {
            let dx = x - center;
            let dy = y - center;
            let d2 = dx * dx + dy * dy;
            let r2 = radius * radius;
            if d2 <= r2 {
                set_px(rgba, x, y, fill);
            }
        }
    }
}

fn draw_disc(rgba: &mut [u8], cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
    let r2 = radius * radius;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= r2 {
                set_px(rgba, cx + dx, cy + dy, color);
            }
        }
    }
}

/// Sprint 22b: the jawata brand mark — the handwritten "arch" (Harald's monoline
/// abstraction of Javanese *ja*, concept sheet rev 5) — rasterized for the tray:
/// the cubic segments + flat tail of the agreed path (native box ~8..160 x 0..188)
/// sampled and stroked as discs, scaled into the 32×32 tray slot.
/// The arch, in the disc's own contrast colour.
///
/// studio#48 parameterised this: on a HOLLOW disc there is no dark fill behind the
/// glyph, so cream-on-transparent is legible over a dark menu bar and vanishes over a
/// light one. The hollow variant passes its own green instead, so the mark reads as an
/// outlined version of itself on either.
fn draw_arch_glyph(rgba: &mut [u8]) {
    draw_arch_glyph_in(rgba, [234, 227, 210, 255]); // #EAE3D2, per the concept sheet
}

fn draw_arch_glyph_in(rgba: &mut [u8], cream: [u8; 4]) {
    // Control points of the agreed path, cubic segments in order.
    let segs: [[(f32, f32); 4]; 6] = [
        [(52.0, 174.0), (40.0, 170.0), (33.0, 160.0), (33.0, 146.0)],
        [(33.0, 146.0), (30.0, 112.0), (30.0, 78.0), (35.0, 56.0)],
        [(35.0, 56.0), (42.0, 22.0), (60.0, 12.0), (75.0, 12.0)],
        [(75.0, 12.0), (92.0, 12.0), (110.0, 24.0), (113.0, 54.0)],
        [(113.0, 54.0), (115.0, 92.0), (113.0, 124.0), (113.0, 146.0)],
        [(113.0, 146.0), (113.0, 164.0), (119.0, 174.0), (131.0, 174.0)],
    ];
    // Map the native box (x 20..160, y 0..190) into the tray slot with padding.
    let scale = 20.0 / 190.0;
    let map = |p: (f32, f32)| -> (i32, i32) {
        (
            ((p.0 - 20.0) * scale + 8.5).round() as i32,
            (p.1 * scale + 6.0).round() as i32,
        )
    };
    for pts in &segs {
        for i in 0..=24 {
            let t = i as f32 / 24.0;
            let u = 1.0 - t;
            let x = u * u * u * pts[0].0
                + 3.0 * u * u * t * pts[1].0
                + 3.0 * u * t * t * pts[2].0
                + t * t * t * pts[3].0;
            let y = u * u * u * pts[0].1
                + 3.0 * u * u * t * pts[1].1
                + 3.0 * u * t * t * pts[2].1
                + t * t * t * pts[3].1;
            let (px, py) = map((x, y));
            draw_disc(rgba, px, py, 1, cream);
        }
    }
    // The flat tail: L149,173.
    for i in 0..=6 {
        let t = i as f32 / 6.0;
        let (px, py) = map((131.0 + t * 18.0, 174.0 - t));
        draw_disc(rgba, px, py, 1, cream);
    }
}

fn draw_coffee_glyph(rgba: &mut [u8]) {
    draw_coffee_glyph_in(rgba, [20, 24, 30, 255]);
}

fn draw_coffee_glyph_in(rgba: &mut [u8], white: [u8; 4]) {
    // Slightly larger coffee cup glyph for parity with the "J" icon.
    draw_rect(rgba, 7, 12, 21, 14, white);
    draw_rect(rgba, 7, 21, 21, 23, white);
    draw_line_v(rgba, 7, 12, 23, 2, white);
    draw_line_v(rgba, 20, 12, 23, 2, white);
    draw_ring(rgba, 24, 18, 5, 1, white);
    draw_line_v(rgba, 10, 7, 11, 1, white);
    draw_line_v(rgba, 14, 6, 10, 1, white);
    draw_line_v(rgba, 18, 7, 11, 1, white);
}

pub fn run() {
    // Sprint 16.2 (bugs.md #20): disable WebKitGTK accelerated compositing on
    // Linux. On hybrid Intel+NVIDIA (and other partial-GPU) stacks the AC path
    // is half-initialised in the WRY webview — present enough to be used, broken
    // enough to make scrolling jump (a native GTK app like yelp scrolls fine on
    // the same WebKitGTK, which is how we isolated it; see tauri#10566). Turning
    // AC off entirely drops to a clean, consistently-smooth path. Set before the
    // webview is created so WebKitGTK reads it at compositor init. Covers dev and
    // every Linux package in one place (the CI bake only touches AppImage/.deb).
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    let config_store = ConfigStore::new().expect("failed to initialize config store");
    let release_manager = ReleaseManager::new().expect("failed to initialize release manager");
    let runtime_manager = RuntimeManager::new(config_store.paths());
    let manager_service = ManagerService::new(config_store, release_manager, runtime_manager);

    // Sprint 21b (item E): backups are plumbing — sweep historically scattered
    // .bak-<ms> files into the managed area once per launch, automatically. Only
    // recognized jawata-studio patterns move; unrecognized files are never touched.
    let gc = manager_service.backups_gc(false);
    if !gc.items.is_empty() || gc.unrecognized_skipped > 0 {
        eprintln!(
            "jawata-studio: backup GC — {} recognized backup(s) swept into the managed area, {} unrecognized left untouched ({} dirs scanned)",
            gc.moved, gc.unrecognized_skipped, gc.scanned_dirs
        );
    }

    tauri::Builder::default()
        // Sprint 14 (v0.14.0, bugs.md #3): single-instance MUST be the first
        // plugin in the chain so the duplicate process exits before any
        // expensive setup (tray icon registration, config-store init). The
        // callback fires on the *original* running process when the second
        // launch is rejected — raise its main window so the user sees the
        // dashboard they expected from re-clicking the app menu / dock entry.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Sprint 28 (v3.6.1): through the one show path, so a second launch
            // restores the foreground activation policy as well as the window.
            // On macOS this is the route a Dock-icon click takes when Launch
            // Services starts a second process instead of reopening the first.
            show_main_window(app);
        }))
        // Sprint 14 (v0.14.0): autostart-on-boot. Per-OS plumbing
        // (Linux ~/.config/autostart/, macOS LaunchAgent, Windows
        // registry) lives in the plugin; ManagerSettings.autostart_on_boot
        // is the persisted state, reconciled on every launch (see the
        // setup block) and on every toggle (set_autostart_on_boot
        // command + tray on_menu_event arm).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { manager_service })
        .setup(|app| {
            // Sprint 13 (v0.13.0): native dynamic menu — the libappindicator
            // path on GNOME hard-routes left-click to the menu and strips
            // per-item icons. Workspace status is shown via monochrome
            // unicode bullets in the menu label (●/◐/○/✗). The webview
            // popover stays as a "rich dashboard" reachable via the
            // "Open dashboard" menu item; "Open manager" raises the full
            // configuration window.
            let initial_menu = rebuild_tray_menu(app.handle())?;

            // Sprint 28 (v3.6.3): NO icon_as_template — see build_tray_icon. The
            // predecessor shipped exactly this call shape and its icon was visible
            // on macOS; the template flag added in v3.6.0 is what removed it.
            let _tray = TrayIconBuilder::with_id(TRAY_ICON_ID)
                .icon(build_tray_icon(selected_tray_icon_variant()))
                .menu(&initial_menu)
                .on_menu_event(|tray, event| {
                    let app_handle = tray.app_handle();
                    let id = event.id().as_ref();
                    match id {
                        "tray_open_dashboard" => {
                            // "Dashboard" is the default view of the main
                            // manager window — raise + focus it. Used both
                            // for routine "I want to glance at status" and
                            // for one-off configuration.
                            show_main_window(app_handle);
                        }
                        "tray_start_all_services" => {
                            let state = app_handle.state::<AppState>();
                            let _ = state.manager_service.start_all_runtimes();
                            refresh_tray_menu(app_handle);
                        }
                        "tray_reload_all_services" => {
                            // Sprint 14 (v0.14.0): sequenced stop+start
                            // across every workspace. Blocks the menu
                            // event for up to ~30 s while polling;
                            // the periodic refresh ticker keeps the
                            // bullets next to each workspace current
                            // during the gap.
                            let state = app_handle.state::<AppState>();
                            let _ = state.manager_service.reload_all_runtimes();
                            refresh_tray_menu(app_handle);
                        }
                        "tray_stop_all_services" => {
                            let state = app_handle.state::<AppState>();
                            let _ = state.manager_service.stop_all_runtimes();
                            refresh_tray_menu(app_handle);
                        }
                        "tray_autostart_on_boot" => {
                            // Sprint 14 (v0.14.0): toggle the persisted
                            // setting AND reconcile OS-level autostart
                            // via tauri-plugin-autostart in one click.
                            use tauri_plugin_autostart::ManagerExt;
                            let state = app_handle.state::<AppState>();
                            let next = !state
                                .manager_service
                                .get_settings()
                                .autostart_on_boot;
                            let _ = state.manager_service.set_autostart_on_boot(next);
                            let autolaunch = app_handle.autolaunch();
                            let _ = if next {
                                autolaunch.enable()
                            } else {
                                autolaunch.disable()
                            };
                            refresh_tray_menu(app_handle);
                            // v0.14.1 (bugs.md #4): notify the frontend
                            // that a backend-driven settings write just
                            // happened, so the Settings UI reloads its
                            // locally-cached Svelte variables. Payload
                            // is unit — the listener just calls
                            // getDashboard() to pull the fresh state.
                            let _ = app_handle.emit("jawata://settings-changed", ());
                        }
                        "tray_quit" => {
                            // The quit prompt is a window dialog, so the window
                            // must be foreground-visible before it is emitted —
                            // on macOS that means restoring the Regular policy
                            // too, or the prompt appears behind everything.
                            show_main_window(app_handle);
                            emit_quit_prompt_event(app_handle, "tray");
                        }
                        other => {
                            if let Some(workspace_name) =
                                other.strip_prefix("tray_workspace_toggle:")
                            {
                                let state = app_handle.state::<AppState>();
                                let _ = state.manager_service.toggle_workspace(workspace_name);
                                refresh_tray_menu(app_handle);
                            }
                        }
                    }
                })
                .build(app)?;

            // Periodic refresh — same cadence and rationale as Sprint 12:
            // catches external state changes (e.g. a workspace's jawata
            // process killed from a shell) so the bullet next to each
            // workspace stays current.
            let refresh_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(
                    TRAY_REFRESH_INTERVAL_SECS,
                ));
                refresh_tray_menu(&refresh_handle);
            });

            // Sprint 28b (D6): the canary. Every resident is asked ONE real
            // recall and ONE real compiler question against a fixture every
            // Java workspace has; a resident that cannot answer both flips the
            // tray icon and the dashboard to non-green.
            //
            // Its own thread because each probe is two blocking round-trips
            // against a JVM, and the first round is deferred so a just-launched
            // resident is not called dead while OSGi and JDT are still booting.
            let canary_handle = app.handle().clone();
            let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel::<()>(1);
            let _ = CANARY_WAKE.set(wake_tx);
            std::thread::spawn(move || {
                // The first wait is INTERRUPTIBLE, for the reason on
                // CANARY_FIRST_DELAY_SECS: a wake means a resident just came up, which
                // is exactly the news that makes deferring pointless.
                let _ = wake_rx.recv_timeout(
                    std::time::Duration::from_secs(CANARY_FIRST_DELAY_SECS));
                loop {
                    let health = run_canary_round(&canary_handle);
                    // studio#21: how long to wait depends on WHAT WE JUST LEARNED.
                    // A good answer keeps the slow cadence; anything else is
                    // re-checked quickly, because a stale unhappy verdict is the
                    // one a user is actually staring at. A wake request (a
                    // resident just started, the Field view just opened) cuts
                    // either wait short — and `sync_channel(1)` collapses a burst
                    // of them into a single extra round.
                    let _ = wake_rx.recv_timeout(
                        std::time::Duration::from_secs(health.recheck_after_secs()));
                }
            });

            // studio#26: the fast lane. It asks ONLY whether each resident can
            // read its workspace, and it repaints only when that answer changed
            // — so a healthy machine does no work here beyond one cheap
            // question per resident per five seconds, and a workspace that goes
            // bad is on screen within that.
            let readability_handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(
                        READABILITY_INTERVAL_SECS,
                    ));
                    let state = readability_handle.state::<AppState>();
                    // studio#48: this asks for no server list any more — the refresher
                    // derives the population itself, so this loop and the deep round
                    // cannot disagree about which residents are supposed to be running.
                    if let Some((health, switched_off)) =
                        state.manager_service.refresh_workspace_readability()
                    {
                        let board = state.manager_service.canary_board();
                        let tooltip = field_view::canary_tooltip(
                            health,
                            &board,
                            &switched_off,
                            field_view::now_millis(),
                        );
                        apply_canary_health_to_tray(&readability_handle, health, &tooltip);
                    }
                }
            });

            // Sprint 14 (v0.14.0): reconcile OS-level autostart with
            // the saved `autostart_on_boot` setting at every launch.
            // Best-effort — errors here don't block startup; the next
            // launch reconciles again.
            {
                use tauri_plugin_autostart::ManagerExt;
                let want_autostart = app
                    .state::<AppState>()
                    .manager_service
                    .get_settings()
                    .autostart_on_boot;
                let autolaunch = app.autolaunch();
                let _ = if want_autostart {
                    autolaunch.enable()
                } else {
                    autolaunch.disable()
                };
            }

            // v0.14.1 (bugs.md #7, redesign 2026-06-04): if
            // autostart_on_boot is set, restore the workspaces that
            // were running at last shutdown. "Running at last shutdown"
            // = any project with phase Running | Starting | Failed in
            // the persisted runtime-state.json. (Failed counts because
            // it represents "user wanted this running; it died — retry
            // on next launch".) Deferred ~2 s + separate thread so the
            // tray icon, main window, and event listeners register
            // first.
            //
            // If the user cleanly stopped everything via tray
            // "Stop and Quit", the snapshot reflects Stopped → no
            // restoration. If they Quit (or close-to-tray + Quit, or
            // crash), the snapshot keeps the Running phases →
            // restoration fires.
            if app
                .state::<AppState>()
                .manager_service
                .get_settings()
                .autostart_on_boot
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let state = app_handle.state::<AppState>();
                    let workspaces = state.manager_service.workspaces_to_auto_restore();
                    if workspaces.is_empty() {
                        return;
                    }
                    if let Err(error) =
                        state.manager_service.start_specific_workspaces(&workspaces)
                    {
                        eprintln!("auto-restore workspaces failed: {error}");
                    }
                });
            }

            // Sprint 28 (v3.6.2): check for a runtime update OFF the main thread.
            //
            // This work used to happen inside load_dashboard, which every dashboard read
            // and nine mutating operations call, and which runs on the main thread. With
            // the update policy set to install, launching the app and pressing "stop the
            // services" each blocked the UI for a 112 MB download, and overlapping calls
            // ran several at once. The window painted white and the OS offered to force
            // quit it.
            //
            // Deferred a few seconds so the tray, window and listeners register first —
            // the same reason the auto-restore below waits.
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    let state = app_handle.state::<AppState>();
                    match state.manager_service.sync_releases_now() {
                        // Only nudge the UI when something actually changed; a routine
                        // "already current" must not churn the dashboard.
                        Ok(true) => {
                            let _ = app_handle.emit("jawata://settings-changed", ());
                        }
                        Ok(false) => {}
                        Err(error) => {
                            eprintln!("[jawata-studio] runtime update check failed: {error}")
                        }
                    }
                });
            }

            // Sprint 28 (v3.6.1): seat the macOS activation policy on the
            // window's ACTUAL initial visibility rather than assuming it. Today
            // the main window is configured visible, so this resolves to
            // `Regular` — Tauri's default — and changes nothing. It is here so
            // that a later "start hidden to the tray" or autostart-launch change
            // cannot leave Studio in the Cmd+Tab list with no window, which is
            // exactly the dead-entry state this release fixes.
            #[cfg(target_os = "macos")]
            {
                let visible = app
                    .get_webview_window("main")
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(true);
                let _ = app.set_activation_policy(if visible {
                    tauri::ActivationPolicy::Regular
                } else {
                    tauri::ActivationPolicy::Accessory
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.manager_service.should_close_to_tray() {
                    api.prevent_close();
                    hide_main_window(window.app_handle());
                } else {
                    api.prevent_close();
                    emit_quit_prompt_event(window.app_handle(), "window");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dashboard,
            commands::add_project,
            commands::set_project_workspace,
            commands::rename_workspace,
            commands::delete_workspace,
            commands::set_workspace_max_heap,
            commands::set_workspace_debuggable,
            commands::rename_project,
            commands::delete_project,
            commands::start_all_runtimes,
            commands::stop_all_runtimes,
            commands::reload_all_runtimes,
            commands::set_autostart_on_boot,
            commands::delete_all_projects,
            commands::discover_workspace_projects,
            commands::scan_folder_for_projects,
            commands::import_workspace_projects,
            commands::update_settings,
            commands::redetect_mcp_client_paths,
            commands::download_or_update_jawata,
            commands::start_runtime,
            commands::stop_runtime,
            commands::get_runtime_status,
            commands::get_services_inventory,
            commands::clean_logs,
            commands::clean_workspaces,
            commands::clean_generated_data,
            commands::probe_services,
            commands::deploy_to_agents,
            commands::knowledge_status,
            commands::resolution_status,
            commands::experience_verb,
            commands::field_status,
            commands::workspace_readability,
            commands::field_set_silence,
            commands::get_quit_prompt_context,
            commands::perform_quit_action,
        ])
        .build(tauri::generate_context!())
        .expect("error while running jawata-studio")
        .run(|_app_handle, _event| {
            // Sprint 28 (v3.6.1): macOS "reopen" — clicking the Dock icon, or
            // selecting the app, while it runs with no visible window. Without
            // this the event was dropped and the window never came back, so a
            // Studio closed to the tray was a dead entry: selectable, and doing
            // nothing when selected. `.run(context)` takes no callback at all,
            // which is why nothing handled it.
            //
            // macOS-only: `RunEvent::Reopen` does not exist on other targets.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = _event
            {
                if !has_visible_windows {
                    show_main_window(_app_handle);
                }
            }
        });
}

/// Ask the canary thread for a round NOW (studio#21).
///
/// Bounded to one pending request: a burst — several residents starting at
/// once, a window regaining focus while the Field view mounts — costs one extra
/// round, not one per event. Silent when the thread is not up yet; a missed
/// wake only means the periodic cadence applies, never a wrong verdict.
static CANARY_WAKE: std::sync::OnceLock<std::sync::mpsc::SyncSender<()>> =
    std::sync::OnceLock::new();

pub(crate) fn request_canary_round() {
    if let Some(tx) = CANARY_WAKE.get() {
        let _ = tx.try_send(());
    }
}

/// One canary round: probe every resident, publish the verdict, and let the
/// tray wear it. Passive throughout — the only user-visible effect is a colour.
///
/// Returns the verdict so the caller can decide how soon to ask again.
fn run_canary_round<R: Runtime>(app: &AppHandle<R>) -> field_view::CanaryHealth {
    let population = app.state::<AppState>().manager_service.canary_population();
    if population.probe.is_empty() && population.switched_off.is_empty() {
        // Nothing deployed yet — an absence, not a failure. Reported as Unknown
        // rather than swallowed, so the caller does not read it as health.
        return field_view::CanaryHealth::Unknown;
    }
    // studio#48: only the residents that are SUPPOSED to be running are probed. A
    // stopped one is not asked and so cannot be classified — which is the fix, rather
    // than probing it and giving its silence a gentler name.
    let results = ManagerService::canary_round_for(&population.probe);
    let probed = results.len();
    let health = app.state::<AppState>().manager_service.publish_canary(results);
    let health = field_view::fold_switched_off(health, probed, population.switched_off.len());
    // The board is read back rather than kept, so the words describe the SAME rows the
    // verdict was taken over — publish_canary stitches loading runs into them, and a
    // tooltip built from the pre-stitch copy would report a fresh import every round.
    let board = app.state::<AppState>().manager_service.canary_board();
    let tooltip = field_view::canary_tooltip(
        health,
        &board,
        &population.switched_off,
        field_view::now_millis(),
    );
    apply_canary_health_to_tray(app, health, &tooltip);
    health
}

/// Swap the tray icon when — and only when — the verdict changed. Every swap is
/// a D-Bus message the shell re-renders, and the tray menu ticker already
/// taught this file what a per-second swap looks like to a user.
fn apply_canary_health_to_tray<R: Runtime>(
    app: &AppHandle<R>,
    health: field_view::CanaryHealth,
    tooltip: &str,
) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<field_view::CanaryHealth>> = Mutex::new(None);
    static LAST_TOOLTIP: Mutex<Option<String>> = Mutex::new(None);

    // studio#48: THE TOOLTIP IS TRACKED SEPARATELY FROM THE VERDICT, because its text
    // moves while the verdict does not — a load's minute count climbs, and a second
    // workspace can break while the tray is already amber. Gating the words on the
    // colour would freeze them at whatever was true when the colour last changed, which
    // is the same staleness studio#21 fixed one layer up.
    let tooltip_changed = {
        let last = LAST_TOOLTIP.lock().unwrap();
        last.as_deref() != Some(tooltip)
    };
    if tooltip_changed {
        if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
            if let Err(e) = tray.set_tooltip(Some(tooltip)) {
                // Not fatal and deliberately not an early return: the COLOUR is the
                // load-bearing half, and a platform that refuses tooltips must not cost
                // the user their icon as well.
                eprintln!("jawata-studio: tray.set_tooltip failed: {e}");
            } else {
                *LAST_TOOLTIP.lock().unwrap() = Some(tooltip.to_string());
            }
        }
    }

    {
        let last = LAST.lock().unwrap();
        if *last == Some(health) {
            return;
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        let icon = build_tray_icon_for(selected_tray_icon_variant(), health);
        if let Err(e) = tray.set_icon(Some(icon)) {
            eprintln!("jawata-studio: tray.set_icon failed: {e}");
            return;
        }
    } else {
        eprintln!("jawata-studio: tray_by_id({TRAY_ICON_ID}) returned None");
        return;
    }
    *LAST.lock().unwrap() = Some(health);
}

fn emit_quit_prompt_event(app_handle: &tauri::AppHandle, source: &str) {
    let state = app_handle.state::<AppState>();
    let payload = QuitPromptEvent {
        source: source.to_string(),
        running_services: state.manager_service.running_services_count(),
        tray_enabled: state.manager_service.is_system_tray_enabled(),
    };
    let _ = app_handle.emit("jawata://quit-requested", payload);
}

/// Sprint 13 (v0.13.0): monochrome status glyph for the workspace menu
/// rows. `IconMenuItem` images are stripped at the AppIndicator D-Bus
/// boundary on GNOME, so the menu label is the only place we can show
/// status — and emoji glyphs (🟢/🟡) come from the system emoji font at
/// a fixed pixel size unrelated to the menu's point size, which makes
/// them dominate the row. These monochrome shapes (●/◐/○/✗) render in
/// the menu's own font, sized 1× with the surrounding text.
fn phase_glyph(phase: &RuntimePhase) -> &'static str {
    match phase {
        RuntimePhase::Running => "●",   // solid       — running
        RuntimePhase::Starting => "◐",  // half-filled — transitioning
        RuntimePhase::Stopped => "○",   // hollow      — off
        RuntimePhase::Failed => "✗",    // ballot X    — error
    }
}

/// Sprint 13 (v0.13.0): build the tray menu reflecting the current set
/// of workspaces.
///
/// Menu shape:
///   Open dashboard            (raises the full manager window)
///   ─────
///   Workspaces                (disabled header — only when ≥1 workspace)
///     ●  Jawata_WS          (toggle on click)
///     ○  BETA-WS
///   ─────
///   Start all services
///   Stop all services
///   ─────
///   Quit
///
/// Per-workspace items have id `tray_workspace_toggle:<workspace_name>`;
/// the tray's `on_menu_event` handler parses the suffix and toggles.
fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    use tauri::menu::MenuItemBuilder;

    let summaries = app
        .state::<AppState>()
        .manager_service
        .workspace_status_summary();

    // Sprint 14 (v0.14.0): pulled out separately because the tray-side
    // toggle for "Autostart on boot" needs its CURRENT value to render
    // the checkmark correctly each rebuild. Tray refresh is keyed off
    // both the workspace snapshot AND this bool (see refresh_tray_menu).
    let autostart_on_boot = app
        .state::<AppState>()
        .manager_service
        .get_settings()
        .autostart_on_boot;

    let mut builder = MenuBuilder::new(app)
        .text("tray_open_dashboard", "Open dashboard")
        .separator();

    if !summaries.is_empty() {
        let header = MenuItemBuilder::new("Workspaces")
            .id("tray_workspaces_header")
            .enabled(false)
            .build(app)?;
        builder = builder.item(&header);

        for summary in &summaries {
            let label = format!(
                "  {}  {}",
                phase_glyph(&summary.phase),
                summary.workspace_name
            );
            let id = format!("tray_workspace_toggle:{}", summary.workspace_name);
            builder = builder.text(id, label);
        }

        builder = builder.separator();
    }

    // Sprint 14 (v0.14.0): tray-side autostart toggle. CheckMenuItemBuilder
    // surfaces a native checkmark on most platforms; GNOME-via-AppIndicator
    // strips per-item icons but still honours the menu protocol's checked
    // state, so the toggle stays legible.
    use tauri::menu::CheckMenuItemBuilder;
    let autostart_item = CheckMenuItemBuilder::with_id(
        "tray_autostart_on_boot",
        "Autostart on boot",
    )
    .checked(autostart_on_boot)
    .build(app)?;

    builder
        .text("tray_start_all_services", "Start all services")
        .text("tray_reload_all_services", "Reload all services")
        .text("tray_stop_all_services", "Stop all services")
        .separator()
        .item(&autostart_item)
        .separator()
        .text("tray_quit", "Quit")
        .build()
}

/// Rebuild the tray menu and swap it onto the live tray icon — but only
/// when the underlying state changed since the last tick. Skipping the
/// swap on steady state matters because every `tray.set_menu` swap fires
/// a D-Bus message that the GNOME shell extension re-renders, which is
/// visible to the user as menu flicker. With change-detection, the menu
/// only re-renders when something actually changed (rename / add /
/// delete / phase transition).
///
/// The cached `LAST` value is the snapshot of `(workspace_name, phase)`
/// pairs that drove the most recent successful menu swap.
fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    use std::sync::Mutex;
    // Sprint 14 (v0.14.0): cache key extended with `autostart_on_boot`
    // so toggling the tray checkable from elsewhere (Settings checkbox
    // saves through update_settings) forces a rebuild — otherwise the
    // periodic refresh would see the workspace snapshot unchanged and
    // skip the swap, leaving the checkmark stale.
    type CacheKey = (Vec<(String, RuntimePhase)>, bool);
    static LAST: Mutex<Option<CacheKey>> = Mutex::new(None);

    let workspace_snapshot: Vec<(String, RuntimePhase)> = app
        .state::<AppState>()
        .manager_service
        .workspace_status_summary()
        .into_iter()
        .map(|s| (s.workspace_name, s.phase))
        .collect();
    let autostart_on_boot = app
        .state::<AppState>()
        .manager_service
        .get_settings()
        .autostart_on_boot;
    let snapshot: CacheKey = (workspace_snapshot, autostart_on_boot);

    {
        let last = LAST.lock().unwrap();
        if last.as_ref() == Some(&snapshot) {
            return; // unchanged — no rebuild, no swap, no flicker
        }
    }

    let menu = match rebuild_tray_menu(app) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("jawata-studio: rebuild_tray_menu failed: {e}");
            return;
        }
    };
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        if let Err(e) = tray.set_menu(Some(menu)) {
            eprintln!("jawata-studio: tray.set_menu failed: {e}");
            return;
        }
    } else {
        eprintln!("jawata-studio: tray_by_id({TRAY_ICON_ID}) returned None");
        return;
    }

    *LAST.lock().unwrap() = Some(snapshot);
}


#[cfg(test)]
mod tray_icon_tests {
    /// The disc colour alone, for assertions that are about the hue and not the shape.
    ///
    /// It lived in production until the tray gained a hollow style: the only caller that
    /// wanted a bare colour was a fixed-green wrapper that the parameterised drawing
    /// superseded, and once that went this had none. The hollow-wiring gate is what said
    /// so — dead without `cfg(test)`, alive with it, which is its definition of a
    /// function the product does not run and the tests keep alive.
    fn tray_disc_colour(health: super::field_view::CanaryHealth) -> [u8; 4] {
        match super::tray_disc_style(health) {
            super::DiscStyle::Filled(colour) | super::DiscStyle::Hollow(colour) => colour,
        }
    }

    use super::*;

    /// Sprint 28 (v3.6.3): the tray icon is the BRANDED, FULL-COLOUR mark on every
    /// platform — macOS included — and is handed to the tray with no template flag.
    ///
    /// This replaces two tests that pinned a macOS-only template image. The premise
    /// behind that image was wrong: v3.6.0 assumed an opaque dark disc would be
    /// invisible on a dark menu bar, and the predecessor product disproved it —
    /// javalens-manager drew the same full-bleed opaque disc, passed it with no
    /// template flag, carried zero macOS-specific tray code, and its icon was visible
    /// on the same machine. Template mode uses the alpha channel alone, which is what
    /// made the icon disappear; v3.6.1 then changed the template's shape, fixing the
    /// wrong thing a second time.
    ///
    /// So what needs pinning is the opposite of what was pinned before: the icon must
    /// stay OPAQUE and COLOURED, because an alpha-only or transparent-background icon
    /// is the failure.
    #[test]
    fn tray_icon_is_opaque_and_coloured_on_every_platform() {
        for variant in [TrayIconVariant::ArchCircle, TrayIconVariant::CoffeeCircle] {
            let icon = build_tray_icon(variant);
            let rgba = icon.rgba();
            let total = (TRAY_ICON_SIZE * TRAY_ICON_SIZE) as usize;

            // The disc is opaque across the middle — not a silhouette, not a stroke.
            let centre = ((TRAY_ICON_SIZE / 2) * TRAY_ICON_SIZE + TRAY_ICON_SIZE / 2) as usize;
            assert_eq!(
                rgba[centre * 4 + 3],
                255,
                "{variant:?}: the disc must be opaque at its centre"
            );

            // And it carries COLOUR. A template image would be alpha-only, so its
            // colour channels convey nothing; that is the state this test forbids.
            let coloured = (0..total).any(|i| {
                let (r, g, b, a) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]);
                a == 255 && !(r == g && g == b)
            });
            assert!(
                coloured,
                "{variant:?}: the tray icon must carry brand colour — an alpha-only \
                 image is what vanished from the macOS menu bar in v3.6.0/v3.6.1"
            );
        }
    }

    /// Sprint 28b (D6): a degraded canary changes the tray icon, and it changes
    /// it PASSIVELY — a different colour on the same mark, nothing else.
    ///
    /// The verdict half of this is driven with no resident, no agent session and
    /// no network: `judge_canary` is pure, so "the resident could not answer"
    /// is expressible as data. That matters because the state this guards is
    /// precisely the one where nothing is running to ask.
    #[test]
    fn a_degraded_resident_flips_the_tray_icon() {
        let degraded = field_view::judge_canary(
            "alpha",
            "http://127.0.0.1:65000/mcp",
            Err("request failed: connection refused".into()),
            Err("request failed: connection refused".into()),
            0, // never answered, so no latency to report
            0,
        );
        assert!(!degraded.green);
        let health = field_view::canary_health(std::slice::from_ref(&degraded), 0);
        assert_eq!(field_view::CanaryHealth::Degraded, health);

        let alarmed = tray_disc_colour(health);
        let healthy = tray_disc_colour(field_view::CanaryHealth::Green);
        assert_ne!(healthy, alarmed, "the tray must LOOK different, not merely know");

        // The centre pixel of the built icon carries the alarmed colour, so the
        // flip reaches the image the tray is handed and not just a helper.
        let icon = build_tray_icon_for(TrayIconVariant::ArchCircle, health);
        let rgba = icon.rgba();
        let centre = ((TRAY_ICON_SIZE / 2) * TRAY_ICON_SIZE + TRAY_ICON_SIZE / 2) as usize;
        let corner_of_disc = ((2 * TRAY_ICON_SIZE) + TRAY_ICON_SIZE / 2) as usize;
        assert_eq!(255, rgba[centre * 4 + 3], "still an opaque disc");
        assert_eq!(
            [alarmed[0], alarmed[1], alarmed[2]],
            [
                rgba[corner_of_disc * 4],
                rgba[corner_of_disc * 4 + 1],
                rgba[corner_of_disc * 4 + 2]
            ],
            "the disc wears the alarmed colour"
        );

        // studio#48 CHANGED WHAT THESE TWO CLAIM, and the change is the point.
        //
        // They used to assert that "not looked yet" and "still starting up" are painted
        // the SAME as healthy — true then, because all three shared the brand colour, and
        // the claim being made was only "this is not an alarm". That is the weaker half of
        // what a reader needs: it says the tray will not cry wolf, and says nothing about
        // whether the tray can tell them apart. It could not. Three states, one colour.
        //
        // Now each is its own state, so both halves are asserted: NOT the alarm, and NOT
        // healthy either. Grey is the catch-all and what it catches is "we do not know" —
        // which is also the promise `CanaryHealth::Unknown` makes in its own doc comment
        // ("never rendered as green") and which the tray used to break by painting it
        // byte-identical to Green.
        let unknown = tray_disc_colour(field_view::CanaryHealth::Unknown);
        let loading = tray_disc_colour(field_view::CanaryHealth::Loading);
        assert_ne!(alarmed, unknown, "not looked yet is not an alarm");
        assert_ne!(healthy, unknown, "and it must not claim health either");
        // issue #16: every healthy launch of a large workspace used to wear the alarm for
        // five minutes. It must still not — and it must still not read as finished.
        assert_ne!(alarmed, loading, "still starting up is not an alarm");
        assert_ne!(healthy, loading, "nor is it done");
        assert_eq!(unknown, loading, "both are the one state: we do not know yet");
    }

    /// studio#48: the reduced state is a SHAPE, and the shape is what carries it.
    ///
    /// A hollow disc reads as "less" without reading as "wrong". The test is therefore
    /// about PIXELS rather than about the enum: the centre of a reduced icon is
    /// transparent where every other state's centre is opaque, and the ring is present
    /// at the edge — so the two claims a hollow mark makes (it is still there; it is
    /// visibly less) are checked separately.
    #[test]
    fn the_reduced_state_is_drawn_hollow_and_the_others_are_not() {
        let centre = ((TRAY_ICON_SIZE / 2) * TRAY_ICON_SIZE + TRAY_ICON_SIZE / 2) as usize;
        let edge = ((TRAY_ICON_SIZE / 2) * TRAY_ICON_SIZE + 1) as usize;

        let reduced = build_tray_icon_for(
            TrayIconVariant::ArchCircle,
            field_view::CanaryHealth::Reduced,
        );
        let rgba = reduced.rgba();
        assert_eq!(
            0,
            rgba[centre * 4 + 3],
            "the middle of a hollow disc is empty — that IS the signal"
        );
        assert_eq!(
            255,
            rgba[edge * 4 + 3],
            "and the ring is drawn, or the mark would simply be missing"
        );

        // THE CONTROL. Without it, a build that drew NOTHING for every state would
        // satisfy the assertion above.
        for filled in [
            field_view::CanaryHealth::Green,
            field_view::CanaryHealth::Degraded,
            field_view::CanaryHealth::Loading,
            field_view::CanaryHealth::Unknown,
        ] {
            let icon = build_tray_icon_for(TrayIconVariant::ArchCircle, filled);
            assert_eq!(
                255,
                icon.rgba()[centre * 4 + 3],
                "{filled:?} is a filled disc"
            );
        }
    }

    /// studio#48: healthy stopped being the company colour, deliberately.
    ///
    /// Harald, 2026-09-10, on why: *"if we give it real meaning all the time and not only
    /// showing company color which might change."* With healthy painted in the brand, the
    /// tray carried ONE bit — meaning lived only in the departure from the logo — and a
    /// rebrand would silently change what the tray meant. Semantics must not hang off a
    /// brand asset.
    #[test]
    fn healthy_is_not_the_brand_colour() {
        let batik_indigo = [29, 47, 78, 255];
        for health in [
            field_view::CanaryHealth::Green,
            field_view::CanaryHealth::Reduced,
            field_view::CanaryHealth::Degraded,
            field_view::CanaryHealth::Loading,
            field_view::CanaryHealth::Unknown,
        ] {
            assert_ne!(
                batik_indigo,
                tray_disc_colour(health),
                "{health:?} must mean something on its own, not borrow the logo"
            );
        }
    }

    /// studio#21: an UNHAPPY verdict is re-checked soon, a happy one is not.
    ///
    /// Measured live on 2026-08-18: after a resident restart both residents
    /// answered the canary's own probes correctly while the tray was still
    /// amber, because the verdict was up to five minutes old. Harald's ruling:
    /// "Cold start is ok until it is up, but not 5 minutes." The colour during
    /// startup is honest; the staleness afterwards is the defect.
    ///
    /// This pins the RULE (which wait follows which verdict) rather than the
    /// thread, because the thread's sleep is not observable from a test — and a
    /// rule nothing asserts is how a constant drifts back.
    /// The FIRST wait must be interruptible, and the wake must survive arriving early.
    ///
    /// The canary defers its first round by 45 s so a resident still booting is not
    /// called degraded. That deferral was a plain `thread::sleep`, which the wake
    /// channel could not reach — so the tray stayed amber for the full 45 s even when
    /// every resident was ready in ten, and the signal saying "booting finished" could
    /// not shorten it. Reported as a couple of minutes of amber after an upgrade.
    ///
    /// What this pins is the CHANNEL BEHAVIOUR the fix rests on, not the thread — the
    /// thread's wait is no more observable from a test than the sleep was. The crux is
    /// ordering: the wake is sent when a resident comes up, which is BEFORE the canary
    /// thread reaches its wait. `sync_channel(1)` buffers exactly that one send, so the
    /// wait returns at once instead of losing the wake and sitting out the timeout.
    #[test]
    fn a_wake_sent_before_the_first_wait_is_not_lost() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

        // The resident comes up FIRST — the ordering that matters.
        tx.send(()).expect("the buffered slot takes one wake");

        let started = std::time::Instant::now();
        let woke = rx.recv_timeout(std::time::Duration::from_secs(CANARY_FIRST_DELAY_SECS));
        let waited = started.elapsed();

        assert!(woke.is_ok(), "the early wake must be delivered, not dropped");
        assert!(
            waited < std::time::Duration::from_secs(1),
            "the first wait must END on the wake rather than run out the {}s deferral - it waited {:?}",
            CANARY_FIRST_DELAY_SECS,
            waited
        );
    }

    /// The control: with no wake, the wait is the deferral and not zero.
    ///
    /// Without this the assertion above passes against a channel that returns
    /// immediately whatever happens, which would defeat the deferral entirely and
    /// report a booting resident as degraded — the defect the 45 s exists to prevent.
    #[test]
    fn with_no_wake_the_first_wait_actually_waits() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

        let started = std::time::Instant::now();
        let woke = rx.recv_timeout(std::time::Duration::from_millis(150));

        assert!(woke.is_err(), "no wake means the wait times out rather than returning");
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(140),
            "and it must actually have waited"
        );
    }

    #[test]
    fn an_unhappy_verdict_is_rechecked_soon_a_happy_one_is_not() {
        // IT CALLS THE PRODUCTION RULE. The previous version declared its own copy of
        // the `match` and looped over three of the five variants, so it was blind twice
        // over: a changed arm would not have reached it, and `Reduced` was simply not in
        // the list. Both halves were live — `Reduced` fell through the production `_`
        // into the fast cadence and this test stayed green.
        let wait_for = field_view::CanaryHealth::recheck_after_secs;

        assert_eq!(CANARY_INTERVAL_SECS, wait_for(field_view::CanaryHealth::Green));
        assert_eq!(
            CANARY_INTERVAL_SECS,
            wait_for(field_view::CanaryHealth::Reduced),
            "a workspace switched off is a DECISION, not a fault, so it earns the slow \
             cadence — charging a user twenty times the probe load for having configured \
             their machine is what the catch-all arm did"
        );
        for unhappy in [
            field_view::CanaryHealth::Degraded,
            field_view::CanaryHealth::Loading,
            field_view::CanaryHealth::Unknown,
        ] {
            assert_eq!(
                CANARY_RECHECK_SECS,
                wait_for(unhappy),
                "{unhappy:?} must be re-checked on the short cadence — a user \
                 staring at amber is staring at the verdict that is most likely \
                 to be out of date"
            );
        }
        assert!(
            CANARY_RECHECK_SECS * 4 <= CANARY_INTERVAL_SECS,
            "the recheck cadence must be materially faster than the healthy one, \
             or recovery is still invisible: recheck={CANARY_RECHECK_SECS}s \
             healthy={CANARY_INTERVAL_SECS}s"
        );
        assert!(
            CANARY_RECHECK_SECS >= 5,
            "but not so fast that two blocking round-trips per resident become a \
             load generator"
        );
    }

    /// The branded (Linux/Windows) icon keeps its filled disc — the template
    /// change must not have flattened the brand mark everywhere.
    #[test]
    fn branded_icon_keeps_its_opaque_disc() {
        let branded = build_tray_icon(TrayIconVariant::ArchCircle);
        let alphas: Vec<u8> = branded.rgba().iter().skip(3).step_by(4).copied().collect();
        let center = ((TRAY_ICON_SIZE / 2) * TRAY_ICON_SIZE + TRAY_ICON_SIZE / 2) as usize;
        assert_eq!(alphas[center], 255, "the branded disc stays opaque");
    }
}
