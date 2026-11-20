//! Checking for a newer release and installing it.
//!
//! Releases are published on GitHub as a zip of the program folder. The
//! check asks GitHub for the latest release and compares its version with
//! this build's. Installing downloads that zip, checks it against the
//! SHA-256 the release states, unpacks it beside the running program and
//! swaps the files in. Windows will not overwrite a running executable but
//! does let it be renamed, so each replaced file is moved aside (as
//! `<name>.openacmi-old`) and the new one moved into its place; the old ones
//! are deleted then, or at the next start for those still in use. The new
//! version runs after a restart.
//!
//! Nothing is installed unless all of this holds: the download comes from
//! this project's release page, its hash matches, every path in the zip
//! stays inside the program folder, and the program runs from an unpacked
//! release (an `assets` folder beside the executable) that can be written
//! to. Files that are not part of the release are never touched. Otherwise
//! the user is pointed at the release page instead.
//!
//! All network and disk work happens on a thread of its own; the UI reads
//! [`Updater::status`].

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;
use sha2::{Digest, Sha256};

/// The project's repository; releases are trusted only from here.
const REPOSITORY: &str = "Spearhead2/OpenACMI";
/// Name of the executable inside a release.
const PACKAGE_EXECUTABLE: &str = "OpenACMI.exe";
/// Ending of the release zip for this platform.
const PACKAGE_SUFFIX: &str = "-windows-x64.zip";
/// Suffix of files moved aside by an update, deleted once no longer in use.
const OLD_SUFFIX: &str = "openacmi-old";
/// Folder inside the program folder where a download is unpacked.
const STAGING_FOLDER: &str = ".openacmi-update";
/// Releases are about 13 MB; anything far beyond that is not one.
const MAX_PACKAGE_BYTES: u64 = 300 * 1024 * 1024;

/// A release version: `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub [u32; 3]);

impl Version {
    /// The version of this build.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version([0; 3]))
    }

    /// Reads `0.2.1` or `v0.2.1`; anything else (pre-releases included) is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().trim_start_matches(['v', 'V']).split('.');
        let mut numbers = [0u32; 3];
        for number in &mut numbers {
            *number = parts.next()?.parse().ok()?;
        }
        parts.next().is_none().then_some(Version(numbers))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [major, minor, patch] = self.0;
        write!(f, "{major}.{minor}.{patch}")
    }
}

/// The zip of a release that can be installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub url: String,
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the zip, as the release states it.
    pub sha256: String,
}

/// A published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Web page of the release, with its notes and downloads.
    pub page: String,
    /// The zip for this platform, when there is one with a known hash.
    pub package: Option<Package>,
}

/// Reads GitHub's description of a release. `None` when it is not a
/// release of this project or not a version this build understands.
pub fn parse_release(json: &str) -> Option<Release> {
    let release: serde_json::Value = serde_json::from_str(json).ok()?;
    if release["draft"].as_bool() == Some(true) || release["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = Version::parse(release["tag_name"].as_str()?)?;
    let page = release["html_url"].as_str()?.to_owned();
    if !page.starts_with(&format!("https://github.com/{REPOSITORY}/releases/")) {
        return None;
    }
    let downloads = format!("https://github.com/{REPOSITORY}/releases/download/");
    let package = release["assets"].as_array().into_iter().flatten().find_map(|asset| {
        let url = asset["browser_download_url"].as_str()?;
        if !cfg!(windows) || !asset["name"].as_str()?.ends_with(PACKAGE_SUFFIX) || !url.starts_with(&downloads) {
            return None;
        }
        // GitHub states the hash of an upload; older releases have it in
        // their notes only.
        let stated = asset["digest"].as_str().and_then(|digest| digest.strip_prefix("sha256:")).map(str::to_owned);
        let sha256 = stated.or_else(|| hash_in_notes(release["body"].as_str()?)).filter(|hash| is_sha256(hash))?;
        Some(Package { url: url.to_owned(), bytes: asset["size"].as_u64()?, sha256: sha256.to_ascii_lowercase() })
    });
    Some(Release { version, page, package })
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The hash a release's notes give after the words "SHA-256".
fn hash_in_notes(notes: &str) -> Option<String> {
    let after = &notes[notes.find("SHA-256")?..];
    after.split(|c: char| !c.is_ascii_hexdigit()).find(|word| is_sha256(word)).map(str::to_owned)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where an update is in its course.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Status {
    /// Nothing has been asked yet.
    #[default]
    Idle,
    Checking,
    /// The latest release is not newer than this build.
    UpToDate,
    /// The check could not be made.
    CheckFailed(String),
    Available(Release),
    Downloading {
        release: Release,
        received: u64,
    },
    /// The files are in place; the new version runs after a restart.
    Installed(Release),
    /// The update could not be installed; the release page still works.
    Failed {
        release: Release,
        message: String,
    },
}

/// Checks for and installs updates in the background.
#[derive(Default)]
pub struct Updater {
    status: Arc<Mutex<Status>>,
    /// What this build is taken to be (a development aid can lower it).
    pretend: Option<Version>,
}

impl Updater {
    pub fn new(pretend: Option<Version>) -> Self {
        Self { status: Arc::default(), pretend }
    }

    pub fn current_version(&self) -> Version {
        self.pretend.unwrap_or_else(Version::current)
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Whether a check or an installation is under way.
    pub fn busy(&self) -> bool {
        matches!(*self.status.lock().unwrap(), Status::Checking | Status::Downloading { .. })
    }

    /// Forgets the outcome of the last check, so nothing is shown for it.
    pub fn dismiss(&self) {
        let mut status = self.status.lock().unwrap();
        if !matches!(*status, Status::Checking | Status::Downloading { .. }) {
            *status = Status::Idle;
        }
    }

    /// Asks GitHub for the latest release.
    pub fn check(&self, ctx: &egui::Context) {
        if self.busy() || matches!(self.status(), Status::Installed(_)) {
            return;
        }
        *self.status.lock().unwrap() = Status::Checking;
        let (status, ctx, current) = (self.status.clone(), ctx.clone(), self.current_version());
        std::thread::spawn(move || {
            let outcome = match latest_release_parsed() {
                Ok(release) if release.version > current => Status::Available(release),
                Ok(_) => Status::UpToDate,
                Err(message) => {
                    log::warn!("update check failed: {message}");
                    Status::CheckFailed(message)
                }
            };
            *status.lock().unwrap() = outcome;
            ctx.request_repaint();
        });
    }

    /// Downloads and installs `release`.
    pub fn install(&self, ctx: &egui::Context, release: Release) {
        if self.busy() {
            return;
        }
        *self.status.lock().unwrap() = Status::Downloading { release: release.clone(), received: 0 };
        let (status, ctx) = (self.status.clone(), ctx.clone());
        std::thread::spawn(move || {
            let progress = |received: u64| {
                *status.lock().unwrap() = Status::Downloading { release: release.clone(), received };
                ctx.request_repaint();
            };
            let outcome = match download_and_install(&release, &progress) {
                Ok(()) => {
                    log::info!("updated to {}; it runs after a restart", release.version);
                    Status::Installed(release.clone())
                }
                Err(message) => {
                    log::error!("update to {} failed: {message}", release.version);
                    Status::Failed { release: release.clone(), message }
                }
            };
            *status.lock().unwrap() = outcome;
            ctx.request_repaint();
        });
    }
}

/// Why this copy of the program cannot update itself, if it cannot.
pub fn cannot_install() -> Option<&'static str> {
    if !cfg!(windows) {
        return Some("automatic updates are for the Windows release");
    }
    install_folder().is_none().then_some("this copy does not run from an unpacked release")
}

/// The folder of an unpacked release this program runs from: the one with
/// the executable and its `assets`. A development build has no such folder.
fn install_folder() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_owned();
    folder.join("assets").join("acmi_colors.json").is_file().then_some(folder)
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .https_only(true)
        .user_agent(concat!("OpenACMI/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

fn latest_release() -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = agent().get(&url).header("Accept", "application/vnd.github+json").call().map_err(|err| err.to_string())?;
    response.body_mut().read_to_string().map_err(|err| err.to_string())
}

fn latest_release_parsed() -> Result<Release, String> {
    parse_release(&latest_release()?).ok_or_else(|| "the latest release could not be understood".to_owned())
}

fn download_and_install(release: &Release, progress: &dyn Fn(u64)) -> Result<(), String> {
    let package = release.package.as_ref().ok_or("this release has no download for this system")?;
    let folder = install_folder().ok_or("this copy does not run from an unpacked release")?;
    let executable = std::env::current_exe().map_err(|err| err.to_string())?;

    let mut response = agent().get(&package.url).call().map_err(|err| format!("download failed: {err}"))?;
    let mut reader = response.body_mut().as_reader().take(MAX_PACKAGE_BYTES + 1);
    let mut bytes = Vec::with_capacity(package.bytes.min(MAX_PACKAGE_BYTES) as usize);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let read = reader.read(&mut chunk).map_err(|err| format!("download failed: {err}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        progress(bytes.len() as u64);
    }
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("the download is far larger than a release".to_owned());
    }
    if sha256_hex(&bytes) != package.sha256 {
        return Err("the download does not match the release's checksum".to_owned());
    }
    install_package(&bytes, &folder, &executable)
}

/// The place in the program folder of a path inside a release zip: the
/// zip's own top folder is dropped. `None` for anything that would land
/// outside the folder.
fn place_in_folder(zipped: &Path) -> Option<PathBuf> {
    let mut parts = zipped.components();
    // `OpenACMI-v0.2.1-windows-x64/…`
    parts.next()?;
    let rest = parts.as_path();
    let inside = rest.components().all(|part| matches!(part, Component::Normal(_)));
    (inside && rest.file_name().is_some()).then(|| rest.to_owned())
}

/// Path with the suffix that marks a file moved aside by an update.
fn moved_aside(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{OLD_SUFFIX}"));
    path.with_file_name(name)
}

/// Unpacks the release in `zip` into `folder` and swaps its files in.
/// `executable` is the running program inside `folder`, which takes the
/// release's executable whatever it is called.
pub fn install_package(zip: &[u8], folder: &Path, executable: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|err| format!("the download is not a zip: {err}"))?;

    // Unpack everything first: nothing in the folder changes until the
    // whole release is known to be readable and to fit on the disk.
    let staging = folder.join(STAGING_FOLDER);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|err| format!("cannot write to {}: {err}", folder.display()))?;
    let staged = unpack(&mut archive, &staging).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })?;
    if !staged.iter().any(|file| file == Path::new(PACKAGE_EXECUTABLE)) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("the download does not contain the program".to_owned());
    }

    // Swap the files in, keeping what each replaced until all are done.
    let mut swapped: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = staged.iter().try_for_each(|file| {
        let target = if file == Path::new(PACKAGE_EXECUTABLE) { executable.to_owned() } else { folder.join(file) };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
        let aside = moved_aside(&target);
        let previous = if target.exists() {
            let _ = std::fs::remove_file(&aside);
            std::fs::rename(&target, &aside).map_err(|err| format!("cannot replace {}: {err}", target.display()))?;
            Some(aside)
        } else {
            None
        };
        swapped.push((target.clone(), previous));
        std::fs::rename(staging.join(file), &target).map_err(|err| format!("cannot write {}: {err}", target.display()))
    });
    let _ = std::fs::remove_dir_all(&staging);

    match result {
        Ok(()) => {
            // Old files not in use go now; the running program's at the next start.
            for (_, previous) in swapped {
                if let Some(previous) = previous {
                    let _ = std::fs::remove_file(previous);
                }
            }
            Ok(())
        }
        Err(message) => {
            // Put back what was there, so the folder holds one whole version.
            for (target, previous) in swapped.into_iter().rev() {
                let _ = std::fs::remove_file(&target);
                if let Some(previous) = previous {
                    let _ = std::fs::rename(previous, &target);
                }
            }
            Err(message)
        }
    }
}

/// Writes the files of `archive` under `staging`; returns their places in
/// the program folder.
fn unpack(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, staging: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| format!("the download is damaged: {err}"))?;
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` refuses absolute paths and `..`.
        let place = entry.enclosed_name().as_deref().and_then(place_in_folder);
        let place = place.ok_or_else(|| format!("the download holds a file outside the program folder (entry {index})"))?;
        let path = staging.join(&place);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut file = std::fs::File::create(&path).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
        std::io::copy(&mut entry, &mut file).map_err(|err| format!("cannot unpack {}: {err}", place.display()))?;
        files.push(place);
    }
    Ok(files)
}

/// Deletes what an earlier update moved aside. Called at startup, when the
/// previous version no longer runs.
pub fn clean_up() {
    let Some(folder) = install_folder() else { return };
    let _ = std::fs::remove_dir_all(folder.join(STAGING_FOLDER));
    remove_moved_aside(&folder, 0);
}

fn remove_moved_aside(folder: &Path, depth: usize) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // A release is a few folders deep; a user's own folders are not searched far.
            if depth < 3 {
                remove_moved_aside(&path, depth + 1);
            }
        } else if path.extension().is_some_and(|extension| extension == OLD_SUFFIX) {
            match std::fs::remove_file(&path) {
                Ok(()) => log::info!("removed {} left by an update", path.display()),
                Err(err) => log::warn!("cannot remove {}: {err}", path.display()),
            }
        }
    }
}

/// Starts the program again, as it was started, and returns whether that
/// worked; the caller then closes this one.
pub fn restart() -> bool {
    let Ok(executable) = std::env::current_exe() else { return false };
    // A recording given on the command line is opened again; development
    // options are not passed on.
    let file = std::env::args_os().nth(1).filter(|arg| !arg.to_string_lossy().starts_with("--"));
    let mut command = std::process::Command::new(executable);
    command.args(file);
    command.spawn().inspect_err(|err| log::error!("cannot restart: {err}")).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn versions_are_read_and_compared() {
        assert_eq!(Version::parse("v0.2.1"), Some(Version([0, 2, 1])));
        assert_eq!(Version::parse("1.10.0"), Some(Version([1, 10, 0])));
        assert!(Version::parse("v0.10.0") > Version::parse("v0.9.9"));
        assert!(Version::parse("v1.0.0") > Version::parse("v0.99.99"));
        for odd in ["", "v1.2", "1.2.3.4", "v1.2.x", "v1.2.3-beta", "latest"] {
            assert_eq!(Version::parse(odd), None, "{odd}");
        }
        assert_eq!(Version([0, 2, 1]).to_string(), "0.2.1");
        assert!(Version::current() > Version([0, 0, 0]));
    }

    fn release_json(tag: &str, asset_url: &str, digest: Option<&str>, notes: &str) -> String {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/Spearhead2/OpenACMI/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
            "body": notes,
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://github.com/Spearhead2/OpenACMI/releases/download/x/notes.txt", "size": 5 },
                { "name": format!("OpenACMI-{tag}-windows-x64.zip"), "browser_download_url": asset_url, "size": 12_676_507, "digest": digest },
            ],
        })
        .to_string()
    }

    const HASH: &str = "86e16d552b18b8e9468ce94ec934f1e99f41cc21952635f35a7ea616308b8897";
    const DOWNLOAD: &str = "https://github.com/Spearhead2/OpenACMI/releases/download/v0.2.1/OpenACMI-v0.2.1-windows-x64.zip";

    #[test]
    fn a_release_is_read_from_githubs_description() {
        let release = parse_release(&release_json("v0.2.1", DOWNLOAD, Some(&format!("sha256:{HASH}")), "Notes.")).unwrap();
        assert_eq!(release.version, Version([0, 2, 1]));
        assert_eq!(release.page, "https://github.com/Spearhead2/OpenACMI/releases/tag/v0.2.1");
        if cfg!(windows) {
            assert_eq!(release.package, Some(Package { url: DOWNLOAD.to_owned(), bytes: 12_676_507, sha256: HASH.to_owned() }));
        }

        // Without GitHub's digest the hash comes from the release notes.
        let notes = format!("Stars.\n\nSHA-256 of the zip: `{}`\n", HASH.to_uppercase());
        let from_notes = parse_release(&release_json("v0.2.1", DOWNLOAD, None, &notes)).unwrap();
        if cfg!(windows) {
            assert_eq!(from_notes.package.unwrap().sha256, HASH);
        }
        // No hash anywhere: the release is known, but cannot be installed.
        let unknown = parse_release(&release_json("v0.2.1", DOWNLOAD, None, "No checksum here.")).unwrap();
        assert_eq!(unknown.package, None);
    }

    #[test]
    fn only_this_projects_releases_are_trusted() {
        // A download hosted anywhere else is not offered.
        let elsewhere = release_json("v9.0.0", "https://example.com/OpenACMI-v9.0.0-windows-x64.zip", Some(&format!("sha256:{HASH}")), "");
        assert_eq!(parse_release(&elsewhere).unwrap().package, None);
        let lookalike = "https://github.com/Spearhead2/OpenACMI-evil/releases/download/v9/OpenACMI-v9.0.0-windows-x64.zip";
        assert_eq!(parse_release(&release_json("v9.0.0", lookalike, Some(&format!("sha256:{HASH}")), "")).unwrap().package, None);
        // Drafts, pre-releases and replies that are not a release at all.
        let draft = release_json("v9.0.0", DOWNLOAD, None, "").replace("\"draft\":false", "\"draft\":true");
        assert_eq!(parse_release(&draft), None);
        let pre = release_json("v9.0.0", DOWNLOAD, None, "").replace("\"prerelease\":false", "\"prerelease\":true");
        assert_eq!(parse_release(&pre), None);
        assert_eq!(parse_release("{\"message\":\"API rate limit exceeded\"}"), None);
        assert_eq!(parse_release("not json"), None);
    }

    /// A fresh folder under the temp directory, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("openacmi-update-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        fn read(&self, relative: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(relative)).ok()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_release_replaces_its_own_files_and_nothing_else() {
        let folder = Scratch::new("install");
        folder.write("MyViewer.exe", "old program");
        folder.write("assets/acmi_colors.json", "old colours");
        folder.write("assets/meshes/Core.Cube.obj", "old cube");
        folder.write("my-models/F-16.obj", "the user's own");
        folder.write("notes.txt", "the user's own");

        let zip = zip_of(&[
            ("OpenACMI-v9.9.9-windows-x64/OpenACMI.exe", "new program"),
            ("OpenACMI-v9.9.9-windows-x64/assets/acmi_colors.json", "new colours"),
            ("OpenACMI-v9.9.9-windows-x64/assets/stars.bin", "new stars"),
            ("OpenACMI-v9.9.9-windows-x64/README.md", "new readme"),
        ]);
        // The program was renamed by the user: it is still the one replaced.
        install_package(&zip, &folder.0, &folder.0.join("MyViewer.exe")).unwrap();

        assert_eq!(folder.read("MyViewer.exe").as_deref(), Some("new program"));
        assert_eq!(folder.read("OpenACMI.exe"), None);
        assert_eq!(folder.read("assets/acmi_colors.json").as_deref(), Some("new colours"));
        assert_eq!(folder.read("assets/stars.bin").as_deref(), Some("new stars"));
        assert_eq!(folder.read("README.md").as_deref(), Some("new readme"));
        // Files the release does not have stay as they were.
        assert_eq!(folder.read("assets/meshes/Core.Cube.obj").as_deref(), Some("old cube"));
        assert_eq!(folder.read("my-models/F-16.obj").as_deref(), Some("the user's own"));
        assert_eq!(folder.read("notes.txt").as_deref(), Some("the user's own"));
        // Nothing of the update's own is left behind.
        assert!(!folder.0.join(STAGING_FOLDER).exists());
        assert!(!moved_aside(&folder.0.join("MyViewer.exe")).exists());
    }

    #[test]
    fn a_bad_download_changes_nothing() {
        let folder = Scratch::new("refuse");
        folder.write("OpenACMI.exe", "old program");
        folder.write("assets/acmi_colors.json", "old colours");
        let executable = folder.0.join("OpenACMI.exe");

        // Not a zip at all.
        assert!(install_package(b"<html>not found</html>", &folder.0, &executable).is_err());
        // A zip without the program in it.
        let partial = zip_of(&[("OpenACMI-v9/assets/acmi_colors.json", "new colours")]);
        assert!(install_package(&partial, &folder.0, &executable).is_err());
        // A zip that reaches outside the program folder.
        for escape in ["OpenACMI-v9/../../evil.exe", "OpenACMI-v9/assets/../../evil.exe", "/evil.exe"] {
            let zip = zip_of(&[("OpenACMI-v9/OpenACMI.exe", "new program"), (escape, "evil")]);
            assert!(install_package(&zip, &folder.0, &executable).is_err(), "{escape}");
        }

        assert_eq!(folder.read("OpenACMI.exe").as_deref(), Some("old program"));
        assert_eq!(folder.read("assets/acmi_colors.json").as_deref(), Some("old colours"));
        assert!(!folder.0.join(STAGING_FOLDER).exists());
        assert!(!folder.0.parent().unwrap().join("evil.exe").exists());
    }

    #[test]
    fn paths_in_a_release_lose_the_top_folder() {
        assert_eq!(place_in_folder(Path::new("OpenACMI-v1/assets/meshes/a.obj")), Some(PathBuf::from("assets/meshes/a.obj")));
        assert_eq!(place_in_folder(Path::new("OpenACMI-v1/OpenACMI.exe")), Some(PathBuf::from("OpenACMI.exe")));
        // A file not inside a top folder has no place.
        assert_eq!(place_in_folder(Path::new("OpenACMI.exe")), None);
        assert_eq!(place_in_folder(Path::new("OpenACMI-v1/../x")), None);
    }

    #[test]
    fn the_hash_is_the_usual_sha256() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn leftovers_of_an_update_are_removed() {
        let folder = Scratch::new("cleanup");
        folder.write("OpenACMI.exe.openacmi-old", "previous program");
        folder.write("assets/meshes/Core.Cube.obj.openacmi-old", "previous cube");
        folder.write("assets/meshes/Core.Cube.obj", "cube");
        folder.write("backup.old", "the user's own");
        remove_moved_aside(&folder.0, 0);
        assert_eq!(folder.read("OpenACMI.exe.openacmi-old"), None);
        assert_eq!(folder.read("assets/meshes/Core.Cube.obj.openacmi-old"), None);
        assert_eq!(folder.read("assets/meshes/Core.Cube.obj").as_deref(), Some("cube"));
        assert_eq!(folder.read("backup.old").as_deref(), Some("the user's own"));
    }
}
