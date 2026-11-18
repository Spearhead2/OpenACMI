//! Making OpenACMI the program that opens `.acmi` files.
//!
//! Windows goes by the last extension only, so associating `.acmi` covers
//! `recording.acmi`, `recording.zip.acmi` and `recording.txt.acmi` alike.
//!
//! The association is made for the current user (under
//! `HKEY_CURRENT_USER\Software\Classes`), which needs no administrator
//! rights and is undone completely by [`remove`]. One thing is outside any
//! program's reach: if the user has picked a default for `.acmi` in
//! Explorer's "Open with" or in Settings, Windows keeps that choice. OpenACMI
//! then still appears in "Open with" and can be chosen there.
//!
//! On other systems associations belong to the desktop environment and the
//! package that installs the program, so nothing is offered here.

/// Who opens `.acmi` files at the moment, as far as the registry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Association {
    /// This executable.
    Ours,
    /// This executable is registered, but the user has chosen another
    /// program for `.acmi` in Windows, and Windows honours that instead.
    Overridden,
    /// OpenACMI, but a copy somewhere else (its path): moved or reinstalled.
    OtherCopy(String),
    /// Some other program, or none.
    NotOurs,
    /// Not something this build can find out or change.
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported,
}

/// The command Windows runs to open a file with `executable`.
fn open_command(executable: &str) -> String {
    format!("\"{executable}\" \"%1\"")
}

/// The executable an [`open_command`] names.
fn executable_of(command: &str) -> Option<&str> {
    command.strip_prefix('"')?.split('"').next()
}

#[cfg(windows)]
mod platform {
    use std::io;

    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    use super::{executable_of, open_command, Association};

    const CLASSES: &str = r"Software\Classes";
    const EXTENSION: &str = ".acmi";
    /// Our file type's name in the registry.
    const PROG_ID: &str = "OpenACMI.Recording";
    /// Where the extension's previous default is kept, to put back on removal.
    const PREVIOUS: &str = "PreviousDefault";
    /// Where Explorer records the program the user picked for an extension.
    const USER_CHOICE: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";

    fn executable() -> io::Result<String> {
        Ok(std::env::current_exe()?.to_string_lossy().into_owned())
    }

    pub fn status() -> Association {
        let classes = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(CLASSES, KEY_READ) {
            Ok(classes) => classes,
            Err(_) => return Association::NotOurs,
        };
        let default_of = |path: &str| classes.open_subkey(path).and_then(|key| key.get_value::<String, _>("")).ok();
        if default_of(EXTENSION).as_deref() != Some(PROG_ID) {
            return Association::NotOurs;
        }
        let command = default_of(&format!(r"{PROG_ID}\shell\open\command")).unwrap_or_default();
        // The choice made through "Open with" or Settings, which wins.
        let chosen: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(r"{USER_CHOICE}\{EXTENSION}\UserChoice"))
            .and_then(|key| key.get_value("ProgId"))
            .ok();
        let overridden = chosen.is_some_and(|chosen| chosen != PROG_ID);
        match (executable_of(&command), executable()) {
            (Some(registered), Ok(ours)) if registered.eq_ignore_ascii_case(&ours) && overridden => Association::Overridden,
            (Some(registered), Ok(ours)) if registered.eq_ignore_ascii_case(&ours) => Association::Ours,
            (Some(registered), _) => Association::OtherCopy(registered.to_owned()),
            _ => Association::NotOurs,
        }
    }

    pub fn register() -> io::Result<()> {
        let executable = executable()?;
        let (classes, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(CLASSES)?;

        // The file type: what it is called, its icon and how to open it.
        let (prog, _) = classes.create_subkey(PROG_ID)?;
        prog.set_value("", &"ACMI flight recording")?;
        prog.create_subkey("DefaultIcon")?.0.set_value("", &format!("\"{executable}\",0"))?;
        prog.create_subkey(r"shell\open\command")?.0.set_value("", &open_command(&executable))?;

        // The extension points at it, remembering what it pointed at before.
        let (extension, _) = classes.create_subkey(EXTENSION)?;
        let previous: String = extension.get_value("").unwrap_or_default();
        if previous != PROG_ID {
            prog.set_value(PREVIOUS, &previous)?;
        }
        extension.set_value("", &PROG_ID)?;
        // Listed under "Open with" even where the user's own choice wins.
        extension.create_subkey("OpenWithProgids")?.0.set_value(PROG_ID, &"")?;
        notify_shell();
        Ok(())
    }

    pub fn remove() -> io::Result<()> {
        let classes = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(CLASSES, KEY_READ | KEY_WRITE)?;
        if let Ok(extension) = classes.open_subkey_with_flags(EXTENSION, KEY_READ | KEY_WRITE) {
            if extension.get_value::<String, _>("").ok().as_deref() == Some(PROG_ID) {
                let previous = classes.open_subkey(PROG_ID).and_then(|prog| prog.get_value::<String, _>(PREVIOUS)).unwrap_or_default();
                if previous.is_empty() {
                    let _ = extension.delete_value("");
                } else {
                    extension.set_value("", &previous)?;
                }
            }
            if let Ok(open_with) = extension.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
                let _ = open_with.delete_value(PROG_ID);
            }
        }
        match classes.delete_subkey_all(PROG_ID) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            _ => {}
        }
        notify_shell();
        Ok(())
    }

    pub fn open_default_apps_settings() {
        let _ = std::process::Command::new("explorer.exe").arg("ms-settings:defaultapps").spawn();
    }

    /// Tells Explorer that associations changed, so icons and double-clicks
    /// follow without signing out.
    fn notify_shell() {
        use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        // SAFETY: with SHCNE_ASSOCCHANGED both item pointers must be null.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io;

    use super::Association;

    pub fn status() -> Association {
        Association::Unsupported
    }

    pub fn register() -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "file associations are set by the desktop environment on this system"))
    }

    pub fn remove() -> io::Result<()> {
        register()
    }

    pub fn open_default_apps_settings() {}
}

/// Who opens `.acmi` files now.
pub fn status() -> Association {
    platform::status()
}

/// Makes this executable open `.acmi` files for the current user.
pub fn register() -> std::io::Result<()> {
    platform::register()
}

/// Opens the system's page for choosing default programs, where the user
/// can pick OpenACMI for `.acmi` when their earlier choice stands in the way.
pub fn open_default_apps_settings() {
    platform::open_default_apps_settings()
}

/// Undoes [`register`], putting back whatever opened `.acmi` files before.
pub fn remove() -> std::io::Result<()> {
    platform::remove()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_command_quotes_both_paths() {
        // Spaces in either path must not split the command.
        let command = open_command(r"C:\Program Files\OpenACMI\OpenACMI.exe");
        assert_eq!(command, r#""C:\Program Files\OpenACMI\OpenACMI.exe" "%1""#);
        assert_eq!(executable_of(&command), Some(r"C:\Program Files\OpenACMI\OpenACMI.exe"));
        assert_eq!(executable_of("notepad %1"), None);
    }
}
