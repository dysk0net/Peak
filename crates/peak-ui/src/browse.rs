//! "Pick the firmware in File Explorer" for the locked panel.
//!
//! On Windows the button opens the standard Open dialog (the File Explorer
//! picker), filtered to `.syx` files and starting in the user's Downloads
//! folder, where the Novation download normally lands. The chosen path is then
//! linked exactly as if it had been typed into the path field.
//!
//! The dialog is modal and blocks until the user answers, so it runs on its own
//! thread; a plugin editor must never block the host's UI thread. The panel
//! polls a [`Pick`] each frame for the answer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Label for the file-type filter in the dialog.
pub const FILTER_NAME: &str = "Novation Peak firmware (*.syx)";
/// Extensions the dialog offers. Both the firmware and the FPGA image are `.syx`.
pub const FILTER_EXTENSIONS: &[&str] = &["syx"];

/// True on platforms where [`open_firmware_dialog`] can show a dialog. The panel
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
pub fn open_firmware_dialog() -> Result<Pick, String> {
    let pick = Pick::new();
    let answer = pick.clone();
    std::thread::Builder::new()
        .name("peak-firmware-dialog".into())
        .spawn(move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Select the Novation Peak firmware (.syx)")
                .add_filter(FILTER_NAME, FILTER_EXTENSIONS);
            if let Some(dir) = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .and_then(|profile| downloads_dir(&profile))
            {
                dialog = dialog.set_directory(dir);
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
pub fn open_firmware_dialog() -> Result<Pick, String> {
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
    fn filter_matches_the_images_the_linker_accepts() {
        assert_eq!(FILTER_EXTENSIONS, ["syx"]);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_report_unsupported() {
        assert!(!browse_supported());
        assert!(open_firmware_dialog().is_err());
    }
}
