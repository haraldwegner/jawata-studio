//! Sprint 28k, Stage 20 (P20) — the one door an end-to-end run uses to reach studio's own Eclipse code.
//!
//! The end-to-end test (`tests/p20_real_eclipse_notes.rs`) lives outside the crate, so it can only name public
//! items. This module makes exactly three questions public — the rows the dashboard shows, the lines the tray
//! shows, and what studio remembers — and answers each with the PRODUCTION code: the real settings store (over a
//! folder the caller names, so the user's own settings are never touched), the real note reader with the real OS
//! liveness check, and the one function the list and the tray both build their rows from.
//!
//! This module exists for the P20 test only. It is hidden from the documentation and nothing in studio calls it.
//!
//! Nothing here can start, stop, delete or write anything of an Eclipse workspace; it is read-only by construction.

use crate::{
    config::ConfigStore, manager_service::ManagerService, release_manager::ReleaseManager,
    runtime_manager::RuntimeManager,
};
use std::path::Path;

pub use crate::eclipse_notes::{EclipseProject, EclipseWorkspaceRow, RememberedEclipseWorkspace};

/// One Eclipse line of the tray menu, as a platform would build the menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayEclipseLine {
    pub id: String,
    pub label: String,
    /// False for every Eclipse line: a greyed statement, nothing to click.
    pub enabled: bool,
}

/// Studio's Eclipse code over a settings folder and a notes folder the caller names.
pub struct EclipseProbe {
    service: ManagerService,
}

impl EclipseProbe {
    /// Open studio's real settings store in `settings_dir` (created if absent, loaded if it holds a settings file)
    /// and point the note reader at `notes_dir`. The liveness check stays the real one.
    pub fn open(settings_dir: &Path, notes_dir: &Path) -> Result<Self, String> {
        let store = ConfigStore::in_directory(settings_dir)?;
        let paths = store.paths();
        let service = ManagerService::new(store, ReleaseManager::new()?, RuntimeManager::new(paths))
            .with_eclipse_notes_dir(notes_dir.to_path_buf());
        Ok(Self { service })
    }

    /// The Eclipse rows of the dashboard, from a real dashboard build.
    pub fn dashboard_rows(&self) -> Result<Vec<EclipseWorkspaceRow>, String> {
        Ok(self.service.load_dashboard()?.eclipse_workspaces)
    }

    /// The Eclipse lines of the tray menu, built by the tray's own function from the same rows function the
    /// dashboard uses.
    pub fn tray_lines(&self) -> Vec<TrayEclipseLine> {
        let rows = self.service.eclipse_workspace_rows();
        crate::tray_menu_rows(None, &[], &rows, false, None)
            .into_iter()
            .filter_map(|row| match row {
                crate::TrayRow::Item { id, label, enabled } if id.starts_with("tray_eclipse:") => {
                    Some(TrayEclipseLine { id, label, enabled })
                }
                _ => None,
            })
            .collect()
    }

    /// Every Eclipse workspace studio has seen, as its own settings hold them.
    pub fn remembered(&self) -> Vec<RememberedEclipseWorkspace> {
        self.service.get_settings().eclipse_workspaces
    }
}
