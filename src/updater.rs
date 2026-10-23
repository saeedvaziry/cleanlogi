//! Self-update from GitHub releases. Each release publishes `update.json`
//! next to its disk image. An installed, Developer ID signed release checks it
//! hourly, downloads and verifies a newer image in the background, and swaps
//! the app bundle in place when the user picks the update from the menu.
//!
//! Verification: the image must match the feed's size and SHA-256, and both
//! the image and the app inside must be signed by the same team as the
//! running app, with the same bundle ID and the advertised version.

use crate::worker::log;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// Set by `scripts/build-app.sh --release`. Local builds don't update.
pub const VERSION: Option<&str> = option_env!("CLEANLOGI_VERSION");

const FEED: &str = "https://github.com/saeedvaziry/cleanlogi/releases/latest/download/update.json";
const RELEASES: &str = "https://github.com/saeedvaziry/cleanlogi/releases/download";
const CHECK_EVERY: Duration = Duration::from_secs(3600);
const MAX_FEED: usize = 64 * 1024;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
/// Next to the installed bundle, so the swap is a rename on one volume.
const STAGING: &str = ".CleanLogi-update";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Unavailable,
    Checking,
    UpToDate,
    Downloading(String),
    Ready(String),
    Installing,
    /// The new bundle is in place; quit so it can start.
    Restarting,
    Failed,
}

/// Runs until the app quits. Each message on `clicks` is the user picking the
/// update menu item: install the downloaded update if there is one, else
/// check now.
pub fn run(clicks: Receiver<()>, notify: impl Fn(State)) {
    let Some(current) = VERSION else { return };
    let app = match Installation::detect(current) {
        Ok(app) => app,
        Err(e) => {
            log!("updates unavailable: {e:#}");
            return notify(State::Unavailable);
        }
    };
    app.clear_staging();
    let mut ready: Option<Prepared> = None;
    let mut clicked = false;
    loop {
        match ready.take() {
            Some(update) if clicked => {
                notify(State::Installing);
                match app.install(&update) {
                    Ok(()) => return notify(State::Restarting),
                    Err(e) => {
                        log!("update failed: {e:#}");
                        app.clear_staging();
                        notify(State::Failed);
                    }
                }
            }
            prepared => {
                ready = prepared;
                if ready.is_none() {
                    notify(State::Checking);
                }
                if let Err(e) = app.check(current, &mut ready, &notify) {
                    log!("update check failed: {e:#}");
                    match &ready {
                        Some(r) => notify(State::Ready(r.version.clone())),
                        None => {
                            app.clear_staging();
                            notify(State::Failed);
                        }
                    }
                }
            }
        }
        clicked = match clicks.recv_timeout(CHECK_EVERY) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
    }
}

#[derive(Debug, PartialEq)]
struct Release {
    version: String,
    url: String,
    size: u64,
    sha256: String,
}

impl Release {
    /// The feed's release, if it is newer than `current`.
    fn parse(bytes: &[u8], current: &str) -> Result<Option<Self>> {
        #[derive(Deserialize)]
        struct Feed {
            schema: u32,
            version: String,
            url: String,
            size: u64,
            sha256: String,
        }
        let feed: Feed = serde_json::from_slice(bytes).context("invalid update feed")?;
        ensure!(
            feed.schema == 1,
            "unsupported update feed schema {}",
            feed.schema
        );
        let next = parse_version(&feed.version).context("invalid update version")?;
        if next <= parse_version(current).context("invalid app version")? {
            return Ok(None);
        }
        let v = &feed.version;
        ensure!(
            feed.url == format!("{RELEASES}/v{v}/CleanLogi-{v}.dmg"),
            "unexpected download location {}",
            feed.url
        );
        ensure!(
            feed.size > 0 && feed.size <= MAX_DOWNLOAD,
            "invalid download size {}",
            feed.size
        );
        ensure!(
            feed.sha256.len() == 64 && feed.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid checksum"
        );
        Ok(Some(Self {
            version: feed.version,
            url: feed.url,
            size: feed.size,
            sha256: feed.sha256.to_ascii_lowercase(),
        }))
    }
}

/// "X.Y.Z", digits only.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.split('.').map(|p| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    });
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// A verified app bundle waiting in the staging folder.
struct Prepared {
    version: String,
    app: PathBuf,
}

struct Installation {
    bundle: PathBuf,
    team: String,
}

impl Installation {
    fn detect(current: &str) -> Result<Self> {
        let bundle = crate::bundle_path().context("not running from an app bundle")?;
        ensure!(
            !bundle.starts_with("/Volumes")
                && !bundle
                    .components()
                    .any(|c| c.as_os_str() == "AppTranslocation"),
            "move {} to Applications and reopen it to get updates",
            crate::APP_NAME
        );
        let dir = bundle.parent().context("app bundle has no folder")?;
        let dir_c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes())?;
        ensure!(
            unsafe { libc::access(dir_c.as_ptr(), libc::W_OK) } == 0,
            "{} is not writable",
            dir.display()
        );
        let out = Command::new("/usr/bin/codesign")
            .args(["--display", "--verbose=4"])
            .arg(&bundle)
            .output()?;
        ensure!(out.status.success(), "the app has no valid signature");
        let team = String::from_utf8_lossy(&out.stderr)
            .lines()
            .find_map(|l| l.strip_prefix("TeamIdentifier="))
            .filter(|t| {
                t.len() == 10
                    && t.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            .context("the app is not signed with a Developer ID")?
            .to_owned();
        let app = Self { bundle, team };
        app.verify_app(&app.bundle, current)?;
        Ok(app)
    }

    fn staging(&self) -> PathBuf {
        self.bundle.with_file_name(STAGING)
    }

    /// Removes leftovers: an unused download, or the previous app after an
    /// update.
    fn clear_staging(&self) {
        let staging = self.staging();
        if staging.exists() {
            Mount::detach(&staging.join("mount"));
            if let Err(e) = std::fs::remove_dir_all(&staging) {
                log!("cannot remove {}: {e}", staging.display());
            }
        }
    }

    fn check(
        &self,
        current: &str,
        ready: &mut Option<Prepared>,
        notify: &impl Fn(State),
    ) -> Result<()> {
        let have = ready.as_ref().map_or(current, |r| r.version.as_str());
        let Some(release) = latest(have)? else {
            notify(match ready {
                Some(r) => State::Ready(r.version.clone()),
                None => State::UpToDate,
            });
            return Ok(());
        };
        notify(State::Downloading(release.version.clone()));
        *ready = None;
        let update = self.prepare(release)?;
        notify(State::Ready(update.version.clone()));
        *ready = Some(update);
        Ok(())
    }

    fn prepare(&self, release: Release) -> Result<Prepared> {
        self.clear_staging();
        let staging = self.staging();
        std::fs::create_dir(&staging)
            .with_context(|| format!("cannot create {}", staging.display()))?;
        let dmg = staging.join("update.dmg");
        download(&release, &dmg)?;
        self.verify_signature(&dmg, false)?;
        let app = staging.join(format!("{}.app", crate::APP_NAME));
        {
            let mount = Mount::attach(&dmg, staging.join("mount"))?;
            exec(
                Command::new("/usr/bin/ditto")
                    .arg(mount.0.join(format!("{}.app", crate::APP_NAME)))
                    .arg(&app),
            )?;
        }
        std::fs::remove_file(&dmg)?;
        self.verify_app(&app, &release.version)?;
        Ok(Prepared {
            version: release.version,
            app,
        })
    }

    /// Swaps the new bundle in and arranges for it to open once this process
    /// exits. The running app is kept in the staging folder until the new one
    /// starts and clears it.
    fn install(&self, update: &Prepared) -> Result<()> {
        self.verify_app(&update.app, &update.version)?;
        let backup = self.staging().join("previous.app");
        replace(&self.bundle, &update.app, &backup)?;
        if let Err(e) = crate::reopen_after_exit(&self.bundle) {
            replace(&self.bundle, &backup, &update.app)?;
            return Err(e).context("cannot restart; the previous app was restored");
        }
        Ok(())
    }

    fn verify_signature(&self, path: &Path, app: bool) -> Result<()> {
        let mut requirement = format!(
            "anchor apple generic and certificate leaf[subject.OU] = \"{}\"",
            self.team
        );
        if app {
            requirement.push_str(&format!(" and identifier \"{}\"", crate::BUNDLE_ID));
        }
        exec(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--strict", "--deep", "-R"])
                .arg(format!("={requirement}"))
                .arg(path),
        )?;
        Ok(())
    }

    fn verify_app(&self, app: &Path, version: &str) -> Result<()> {
        self.verify_signature(app, true)?;
        let plist = app.join("Contents/Info.plist");
        for (key, expected) in [
            ("CFBundleIdentifier", crate::BUNDLE_ID),
            ("CFBundleExecutable", "cleanlogi"),
            ("CFBundleShortVersionString", version),
        ] {
            let actual = exec(
                Command::new("/usr/bin/plutil")
                    .args(["-extract", key, "raw", "-o", "-"])
                    .arg(&plist),
            )?;
            ensure!(
                actual.trim() == expected,
                "{} has {key} {:?}, expected {expected:?}",
                app.display(),
                actual.trim()
            );
        }
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        exec(
            Command::new("/usr/bin/lipo")
                .arg(app.join("Contents/MacOS/cleanlogi"))
                .args(["-verify_arch", arch]),
        )?;
        Ok(())
    }
}

/// Moves `current` to `backup` and `next` into its place, restoring `current`
/// if the second move fails.
fn replace(current: &Path, next: &Path, backup: &Path) -> Result<()> {
    std::fs::rename(current, backup)
        .with_context(|| format!("cannot move {} aside", current.display()))?;
    if let Err(e) = std::fs::rename(next, current) {
        std::fs::rename(backup, current).with_context(|| {
            format!(
                "cannot install the update ({e}) or restore the previous app, which is at {}",
                backup.display()
            )
        })?;
        return Err(e).context("cannot install the update; the previous app was restored");
    }
    Ok(())
}

fn curl(url: &str) -> Command {
    let mut c = Command::new("/usr/bin/curl");
    c.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "30",
        "--user-agent",
    ])
    .arg(format!("CleanLogi/{}", VERSION.unwrap_or("dev")))
    .arg(url);
    c
}

fn latest(current: &str) -> Result<Option<Release>> {
    let body = exec(
        curl(FEED)
            .args(["--max-time", "60", "--max-filesize"])
            .arg(MAX_FEED.to_string())
            .args(["--header", "Cache-Control: no-cache"]),
    )?;
    ensure!(body.len() <= MAX_FEED, "update feed is too large");
    Release::parse(body.as_bytes(), current)
}

fn download(release: &Release, path: &Path) -> Result<()> {
    exec(
        curl(&release.url)
            .args(["--max-time", "900", "--max-filesize"])
            .arg(release.size.to_string())
            .arg("--output")
            .arg(path),
    )?;
    let size = std::fs::metadata(path)?.len();
    ensure!(
        size == release.size,
        "downloaded {size} bytes, expected {}",
        release.size
    );
    let sum = exec(
        Command::new("/usr/bin/shasum")
            .args(["-a", "256"])
            .arg(path),
    )?;
    ensure!(
        sum.split_whitespace()
            .next()
            .is_some_and(|h| h.eq_ignore_ascii_case(&release.sha256)),
        "download checksum mismatch"
    );
    Ok(())
}

/// A read-only disk image mounted for as long as this lives.
struct Mount(PathBuf);

impl Mount {
    fn attach(dmg: &Path, at: PathBuf) -> Result<Self> {
        std::fs::create_dir(&at)?;
        exec(
            Command::new("/usr/bin/hdiutil")
                .args([
                    "attach",
                    "-readonly",
                    "-nobrowse",
                    "-noautoopen",
                    "-mountpoint",
                ])
                .arg(&at)
                .arg(dmg)
                .stdin(Stdio::null()),
        )?;
        Ok(Self(at))
    }

    fn detach(at: &Path) {
        if at.exists() {
            let _ = Command::new("/usr/bin/hdiutil")
                .args(["detach", "-force"])
                .arg(at)
                .output();
        }
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        Self::detach(&self.0);
    }
}

/// Runs a tool and returns its stdout, or its stderr as the error.
fn exec(cmd: &mut Command) -> Result<String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .with_context(|| format!("cannot run {program}"))?;
    ensure!(
        out.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8(out.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(version: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "version": version,
            "url": format!("{RELEASES}/v{version}/CleanLogi-{version}.dmg"),
            "size": 123,
            "sha256": "a".repeat(64),
        })
    }

    fn parse(feed: &serde_json::Value, current: &str) -> Result<Option<Release>> {
        Release::parse(feed.to_string().as_bytes(), current)
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("1.20.3"), Some((1, 20, 3)));
        for bad in ["1.2", "1.2.3.4", "1.x.3", "1.+2.3", "1..3", "", "v1.2.3"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn updates_only_to_newer_releases() {
        for v in ["0.1.10", "0.2.0", "1.0.0"] {
            assert!(parse(&feed(v), "0.1.9").unwrap().is_some(), "{v}");
        }
        for v in ["0.1.9", "0.1.1", "0.0.99"] {
            assert!(parse(&feed(v), "0.1.9").unwrap().is_none(), "{v}");
        }
        assert!(parse(&feed("0.2"), "0.1.9").is_err());
    }

    #[test]
    fn rejects_bad_feeds() {
        let mut f = feed("0.2.0");
        f["schema"] = 2.into();
        assert!(parse(&f, "0.1.0").is_err());
        for url in [
            "http://github.com/saeedvaziry/cleanlogi/releases/download/v0.2.0/CleanLogi-0.2.0.dmg",
            "https://example.com/CleanLogi-0.2.0.dmg",
            "https://github.com/saeedvaziry/cleanlogi/releases/download/v0.1.0/CleanLogi-0.1.0.dmg",
        ] {
            let mut f = feed("0.2.0");
            f["url"] = url.into();
            assert!(parse(&f, "0.1.0").is_err(), "{url}");
        }
        for size in [0, MAX_DOWNLOAD + 1] {
            let mut f = feed("0.2.0");
            f["size"] = size.into();
            assert!(parse(&f, "0.1.0").is_err(), "{size}");
        }
        for sum in ["abcd".to_string(), "z".repeat(64)] {
            let mut f = feed("0.2.0");
            f["sha256"] = sum.into();
            assert!(parse(&f, "0.1.0").is_err());
        }
    }

    #[test]
    fn failed_replacement_keeps_the_current_app() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("cleanlogi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let current = dir.join("current.app");
        let backup = dir.join("previous.app");
        std::fs::create_dir_all(&current)?;
        std::fs::write(current.join("v"), "old")?;
        assert!(replace(&current, &dir.join("missing.app"), &backup).is_err());
        assert_eq!(std::fs::read_to_string(current.join("v"))?, "old");
        assert!(!backup.exists());

        let next = dir.join("next.app");
        std::fs::create_dir_all(&next)?;
        std::fs::write(next.join("v"), "new")?;
        replace(&current, &next, &backup)?;
        assert_eq!(std::fs::read_to_string(current.join("v"))?, "new");
        assert_eq!(std::fs::read_to_string(backup.join("v"))?, "old");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
