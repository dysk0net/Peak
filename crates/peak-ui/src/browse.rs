//! "Pick a file in File Explorer" for the firmware link and the preset import.
//!
//! On Windows the buttons open the standard Open dialog (the File Explorer
//! picker), filtered to `.syx` files. The firmware picker starts in the user's
//! Downloads folder, where the Novation download normally lands; the preset
//! import leaves the start folder to Windows, which remembers the last one. The
//! chosen path is then used exactly as if it had been typed into a path field.
//!
//! The dialog is modal and blocks until the user answers, so it runs on its own
//! thread; a plugin editor must never block the host's UI thread. The panel
//! polls a [`Pick`] each frame for the answer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Extensions the dialogs offer. Firmware, FPGA images and presets are all `.syx`.
pub const FILTER_EXTENSIONS: &[&str] = &["syx"];

/// What the file dialog is being opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseKind {
    /// The Novation Peak firmware or FPGA image.
    Firmware,
    /// A Peak preset or bank to import.
    Preset,
}

impl BrowseKind {
    /// Title bar text of the dialog.
    pub fn title(self) -> &'static str {
        match self {
            BrowseKind::Firmware => "Select the Novation Peak firmware (.syx)",
            BrowseKind::Preset => "Import a Peak preset (.syx)",
        }
    }

    /// Label of the file-type filter.
    pub fn filter_name(self) -> &'static str {
        match self {
            BrowseKind::Firmware => "Novation Peak firmware (*.syx)",
            BrowseKind::Preset => "Peak presets (*.syx)",
        }
    }

    /// Whether the dialog should open in Downloads. Only the firmware does:
    /// presets live wherever the user keeps them, so Windows' own memory of the
    /// last folder serves better.
    pub fn starts_in_downloads(self) -> bool {
        matches!(self, BrowseKind::Firmware)
    }
}

/// True on platforms where [`open_file_dialog`] can show a dialog. The panel
/// hides the button elsewhere instead of offering one that always fails.
pub const fn browse_supported() -> bool {
    cfg!(windows)
}

/// What a [`Pick`] currently holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickPoll {
    /// The dialog is still open.
    Pending,
    /// The user dismissed the dialog, or it could not finish.
    Cancelled,
    /// The user chose this file.
    Chosen(String),
}

/// The answer to a file dialog that is running on another thread.
///
/// `Arc<Mutex<..>>` rather than a channel receiver because the panel state must
/// be `Sync` for the plugin editor, and `mpsc::Receiver` is not.
#[derive(Clone, Default)]
pub struct Pick(Arc<Mutex<Option<Option<String>>>>);

impl Pick {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the answer: `Some(path)` for a chosen file, `None` for cancel.
    pub fn finish(&self, path: Option<String>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(path);
        }
    }

    pub fn poll(&self) -> PickPoll {
        let Ok(slot) = self.0.lock() else {
            return PickPoll::Cancelled;
        };
        match &*slot {
            Some(Some(path)) => PickPoll::Chosen(path.clone()),
            Some(None) => PickPoll::Cancelled,
            // The dialog thread holds the only other handle. If it is gone
            // without an answer it panicked: report a cancel instead of
            // leaving the button stuck on "waiting" for ever.
            None if Arc::strong_count(&self.0) == 1 => PickPoll::Cancelled,
            None => PickPoll::Pending,
        }
    }
}

/// `<profile>\Downloads`, if it exists.
pub fn downloads_dir(profile: &Path) -> Option<PathBuf> {
    let dir = profile.join("Downloads");
    dir.is_dir().then_some(dir)
}

/// Show the file dialog on a background thread and return at once.
#[cfg(windows)]
pub fn open_file_dialog(kind: BrowseKind) -> Result<Pick, String> {
    let pick = Pick::new();
    let answer = pick.clone();
    std::thread::Builder::new()
        .name("peak-firmware-dialog".into())
        .spawn(move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title(kind.title())
                .add_filter(kind.filter_name(), FILTER_EXTENSIONS);
            if kind.starts_in_downloads() {
                if let Some(dir) = std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .and_then(|profile| downloads_dir(&profile))
                {
                    dialog = dialog.set_directory(dir);
                }
            }
            answer.finish(
                dialog
                    .pick_file()
                    .map(|path| path.to_string_lossy().into_owned()),
            );
        })
        .map_err(|error| format!("could not open the file dialog: {error}"))?;
    Ok(pick)
}

/// There is no dialog here; see [`browse_supported`].
#[cfg(not(windows))]
pub fn open_file_dialog(_kind: BrowseKind) -> Result<Pick, String> {
    Err("the file dialog is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_pick_is_pending_while_the_dialog_thread_holds_it() {
        let pick = Pick::new();
        let _dialog_thread_handle = pick.clone();
        assert_eq!(pick.poll(), PickPoll::Pending);
    }

    #[test]
    fn a_chosen_file_is_reported() {
        let pick = Pick::new();
        let answer = pick.clone();
        answer.finish(Some(
            r"C:\Users\me\Downloads\peak-firmware-2.2-build2108.syx".into(),
        ));
        assert_eq!(
            pick.poll(),
            PickPoll::Chosen(r"C:\Users\me\Downloads\peak-firmware-2.2-build2108.syx".into())
        );
    }

    #[test]
    fn cancelling_is_reported_as_cancelled() {
        let pick = Pick::new();
        let answer = pick.clone();
        answer.finish(None);
        assert_eq!(pick.poll(), PickPoll::Cancelled);
    }

    #[test]
    fn a_dialog_thread_that_died_does_not_leave_the_button_waiting() {
        let pick = Pick::new();
        let answer = pick.clone();
        drop(answer); // the thread panicked: its handle is gone, no answer given
        assert_eq!(pick.poll(), PickPoll::Cancelled);
    }

    #[test]
    fn downloads_dir_only_when_it_exists() {
        let root = std::env::temp_dir().join(format!("peak-browse-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(downloads_dir(&root), None);
        std::fs::create_dir_all(root.join("Downloads")).unwrap();
        assert_eq!(downloads_dir(&root), Some(root.join("Downloads")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn filter_matches_the_files_the_panel_accepts() {
        assert_eq!(FILTER_EXTENSIONS, ["syx"]);
    }

    #[test]
    fn each_kind_has_its_own_title_and_filter() {
        assert_ne!(BrowseKind::Firmware.title(), BrowseKind::Preset.title());
        assert_ne!(
            BrowseKind::Firmware.filter_name(),
            BrowseKind::Preset.filter_name()
        );
        assert!(BrowseKind::Preset.filter_name().contains("*.syx"));
    }

    #[test]
    fn only_the_firmware_dialog_starts_in_downloads() {
        assert!(BrowseKind::Firmware.starts_in_downloads());
        assert!(!BrowseKind::Preset.starts_in_downloads());
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_report_unsupported() {
        assert!(!browse_supported());
        assert!(open_file_dialog(BrowseKind::Preset).is_err());
    }
}
