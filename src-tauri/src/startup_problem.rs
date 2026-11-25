//! The studio that could not load its own files at start-up: it opens its window, says which file and why, and does
//! nothing else.
//!
//! It deliberately has no manager service. Going on with default values would act on choices the person never made
//! (deploying into agents they had switched off, taking the autostart entry away) and would sooner or later write
//! those defaults over their file. So nothing here reads settings or writes a file: the person repairs the file, or
//! moves it away, and starts the studio again. Before this, a settings.json without a `version` field made the
//! studio panic with no window and no message.

use tauri::{AppHandle, Manager, Runtime};

// The page's half is src/lib/components/StartupProblem.svelte, chosen in src/main.ts.

/// Why this start stopped short. Managed only by the app [`run`] builds.
struct StartupProblem(String);

/// What the page asks before it draws anything: `None` is a studio that started normally.
#[tauri::command]
pub fn startup_problem<R: Runtime>(app: AppHandle<R>) -> Option<String> {
    app.try_state::<StartupProblem>().map(|stopped| stopped.0.clone())
}

/// What the stopped studio holds and answers, on whichever runtime the caller builds on.
fn app<R: Runtime>(builder: tauri::Builder<R>, problem: String) -> tauri::Builder<R> {
    builder
        .manage(StartupProblem(problem))
        .invoke_handler(tauri::generate_handler![startup_problem])
}

/// Opens the window that tells the person `problem`, and returns when they close it.
pub fn run(problem: String, context: tauri::Context) {
    crate::studio_log::say(&format!("studio could not start: {problem}"));
    app(
        // A second launch raises this window: the person has to close it before a repaired file is read.
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            crate::show_main_window(app);
        })),
        problem,
    )
    .setup(|app| {
        // Shown also for a `--background` launch: there is no tray here, so a hidden window would be a process
        // nobody can see or end.
        crate::show_main_window(app.handle());
        Ok(())
    })
    .run(context)
    .expect("error while showing why jawata-studio could not start");
}

#[cfg(test)]
mod tests {
    /// `run` cannot be run without a display. What can be held is the wiring: a config store that does not load
    /// leads here and not to a panic, and the page asks before it draws.
    #[test]
    fn a_config_store_that_does_not_load_opens_the_window_instead_of_panicking() {
        let source = include_str!("lib.rs");
        let from = source.find("pub fn run() {").unwrap();
        let run = &source[from..from + source[from..].find("tauri::Builder::default()").unwrap()];
        assert!(!run.contains("failed to initialize config store"), "run() must not panic on a file it cannot read");
        assert!(run.contains("startup_problem::run("), "run() must show the person why");
        assert!(
            source.contains("startup_problem::startup_problem,"),
            "the normal studio answers the page's question too"
        );
        let page = include_str!("../../src/main.ts");
        assert!(page.contains("startupProblem()") && page.contains("StartupProblemView"), "the page shows the answer");
    }

    // Not built on Windows: see the note on `canary_duty_tests` in lib.rs.
    #[cfg(not(windows))]
    #[test]
    fn a_settings_file_without_a_version_is_named_with_its_reason_and_left_as_it_was() {
        let dir = crate::testdir::TestDir::new("jawata-studio-startup-problem");
        let settings = dir.join("settings.json");
        let written = "{\n  \"updatePolicy\": \"ask\",\n  \"autoCheckForUpdates\": false\n}\n";
        std::fs::write(&settings, written).unwrap();
        let reason = crate::config::ConfigStore::in_directory(&dir)
            .err()
            .expect("control: this file does not load");

        let stopped = super::app(tauri::test::mock_builder(), reason)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("the stopped studio builds");
        let told = super::startup_problem(stopped.handle().clone()).expect("the page is told");

        assert!(told.contains(&settings.display().to_string()), "it names the file: {told}");
        assert!(told.contains("version"), "and the reason: {told}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), written, "the person's file is as they left it");
        let normal = tauri::test::mock_app();
        assert_eq!(super::startup_problem(normal.handle().clone()), None, "a studio that started has nothing to say");
    }
}
