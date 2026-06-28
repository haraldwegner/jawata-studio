mod commands;
mod config;
mod lombok;
mod manager_service;
mod release_manager;
mod resident;
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
const TRAY_ICON_ID: &str = "goja-tray";

/// Sprint 13 (v0.13.0): how often to refresh the tray menu in the
/// background. Catches both (a) status changes from external events
/// (workspace's goja process killed from a shell) and (b) workspace
/// composition changes from the main window (rename / add / delete).
///
/// Set to 1 second so a rename in the dashboard propagates to the tray
/// within ~1 s, matching user expectation that "the tray follows what I
/// just changed". Per-tick cost is sub-50 ms (one
/// `workspace_status_summary` + one GTK menu rebuild + one D-Bus
/// `set_menu` to AppIndicator), well under 1 % CPU.
const TRAY_REFRESH_INTERVAL_SECS: u64 = 1;

#[derive(Clone, Copy)]
enum TrayIconVariant {
    GCircle,
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
    match std::env::var("GOJA_TRAY_ICON")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "coffee" | "cup" => TrayIconVariant::CoffeeCircle,
        _ => TrayIconVariant::GCircle,
    }
}

fn build_tray_icon(variant: TrayIconVariant) -> Image<'static> {
    let mut rgba = vec![0u8; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];
    draw_base_circle(&mut rgba);
    match variant {
        TrayIconVariant::GCircle => draw_g_glyph(&mut rgba),
        TrayIconVariant::CoffeeCircle => draw_coffee_glyph(&mut rgba),
    }
    Image::new_owned(rgba, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
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

fn draw_base_circle(rgba: &mut [u8]) {
    let center = (TRAY_ICON_SIZE as i32) / 2;
    // Draw slightly beyond the nominal radius so the circle nearly fills the tray slot.
    let radius = center + 1;
    // Brand indigo circle background (#4F46E5)
    let fill = [79, 70, 229, 255]; // #4F46E5
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

fn draw_g_glyph(rgba: &mut [u8]) {
    let white = [255, 255, 255, 255];
    
    // Blocky "G" sized to stay readable at small tray sizes: an open "C"
    // (top / left / bottom strokes), closed on the lower-right, with the
    // tongue stroke into the middle — the top-right stays open.
    draw_rect(rgba, 8, 7, 23, 9, white);    // top stroke
    draw_rect(rgba, 8, 7, 10, 24, white);   // left stroke
    draw_rect(rgba, 8, 22, 23, 24, white);  // bottom stroke
    draw_rect(rgba, 21, 16, 23, 24, white); // lower-right edge
    draw_rect(rgba, 16, 15, 23, 17, white); // inner tongue
}

fn draw_coffee_glyph(rgba: &mut [u8]) {
    let white = [20, 24, 30, 255];
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

    tauri::Builder::default()
        // Sprint 14 (v0.14.0, bugs.md #3): single-instance MUST be the first
        // plugin in the chain so the duplicate process exits before any
        // expensive setup (tray icon registration, config-store init). The
        // callback fires on the *original* running process when the second
        // launch is rejected — raise its main window so the user sees the
        // dashboard they expected from re-clicking the app menu / dock entry.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
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
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
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
                            let _ = app_handle.emit("goja://settings-changed", ());
                        }
                        "tray_quit" => {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
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
            // catches external state changes (e.g. a workspace's goja
            // process killed from a shell) so the bullet next to each
            // workspace stays current.
            let refresh_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(
                    TRAY_REFRESH_INTERVAL_SECS,
                ));
                refresh_tray_menu(&refresh_handle);
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

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.manager_service.should_close_to_tray() {
                    api.prevent_close();
                    let _ = window.hide();
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
            commands::download_or_update_goja,
            commands::start_runtime,
            commands::stop_runtime,
            commands::get_runtime_status,
            commands::get_services_inventory,
            commands::clean_logs,
            commands::clean_workspaces,
            commands::clean_generated_data,
            commands::probe_services,
            commands::deploy_to_agents,
            commands::get_quit_prompt_context,
            commands::perform_quit_action,
        ])
        .run(tauri::generate_context!())
        .expect("error while running goja-studio");
}

fn emit_quit_prompt_event(app_handle: &tauri::AppHandle, source: &str) {
    let state = app_handle.state::<AppState>();
    let payload = QuitPromptEvent {
        source: source.to_string(),
        running_services: state.manager_service.running_services_count(),
        tray_enabled: state.manager_service.is_system_tray_enabled(),
    };
    let _ = app_handle.emit("goja://quit-requested", payload);
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
///     ●  Goja_WS          (toggle on click)
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
            eprintln!("goja-studio: rebuild_tray_menu failed: {e}");
            return;
        }
    };
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        if let Err(e) = tray.set_menu(Some(menu)) {
            eprintln!("goja-studio: tray.set_menu failed: {e}");
            return;
        }
    } else {
        eprintln!("goja-studio: tray_by_id({TRAY_ICON_ID}) returned None");
        return;
    }

    *LAST.lock().unwrap() = Some(snapshot);
}

