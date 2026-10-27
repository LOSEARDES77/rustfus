// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Update check against the project's GitHub releases, like Rufus' check
//! against rufus.ie, with Rufus' update policy: a check interval, an
//! "include beta versions" switch and the time of the last check.
//!
//! Unlike Rufus, Rustfus doesn't replace its own executable: it ships as
//! .deb, .rpm, AppImage, .dmg and .exe, so it points to the release page.

use crate::net;
use serde::Deserialize;
use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const RELEASES_API: &str = "https://api.github.com/repos/LOSEARDES77/rustfus/releases?per_page=30";

/// A `MAJOR.MINOR.PATCH[-PRE]` version, as used by the `v*` release tags.
/// Build metadata (`+...`) is accepted and ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Pre-release identifiers, such as `beta.1`; empty for a stable release.
    pub pre: String,
}

impl Version {
    /// The version of this build.
    #[must_use]
    pub fn current() -> Version {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("Cargo package versions are valid semver")
    }

    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl FromStr for Version {
    type Err = String;

    fn from_str(s: &str) -> Result<Version, String> {
        let bad = || format!("invalid version {s:?}");
        let s = s.trim();
        let core = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let core = core.split_once('+').map_or(core, |(c, _)| c);
        let (core, pre) = match core.split_once('-') {
            Some((core, pre)) if !pre.split('.').any(str::is_empty) => (core, pre),
            Some(_) => return Err(bad()),
            None => (core, ""),
        };
        let nums: Result<Vec<u64>, _> = core.split('.').map(str::parse).collect();
        let Ok([major, minor, patch]) = nums.as_deref() else {
            return Err(bad());
        };
        let (major, minor, patch) = (*major, *minor, *patch);
        Ok(Version {
            major,
            minor,
            patch,
            pre: pre.to_string(),
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.is_prerelease() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

/// SemVer precedence: a pre-release sorts before its release, and
/// pre-release identifiers compare numerically when both are numbers.
impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => compare_pre(&self.pre, &other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut a_ids = a.split('.');
    let mut b_ids = b.split('.');
    loop {
        let order = match (a_ids.next(), b_ids.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                // Numeric identifiers sort before alphanumeric ones.
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => x.cmp(y),
            },
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

/// A published release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Release page, where the packages for every OS are listed.
    pub url: String,
    /// Release notes (Markdown).
    pub notes: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Newest release in a GitHub `/releases` response. Drafts and tags that
/// aren't versions are skipped, and so are pre-releases unless `include_betas`.
pub fn newest_release(json: &[u8], include_betas: bool) -> Result<Option<Release>, String> {
    let list: Vec<GithubRelease> =
        serde_json::from_slice(json).map_err(|e| format!("bad release data: {e}"))?;
    let mut newest: Option<Release> = None;
    for r in list {
        let Ok(version) = r.tag_name.parse::<Version>() else {
            continue;
        };
        let beta = r.prerelease || version.is_prerelease();
        if r.draft || (beta && !include_betas) {
            continue;
        }
        if newest.as_ref().is_none_or(|n| version > n.version) {
            newest = Some(Release {
                version,
                url: r.html_url,
                notes: r.body.unwrap_or_default(),
            });
        }
    }
    Ok(newest)
}

/// Asks GitHub for the newest release, returning it only when it is newer
/// than this build.
pub fn check(include_betas: bool) -> Result<Option<Release>, String> {
    let json = net::fetch_with(
        RELEASES_API,
        &[
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
        ],
    )?;
    let current = Version::current();
    Ok(newest_release(&json, include_betas)?.filter(|r| r.version > current))
}

/// How often to check for updates; Rufus' choices, daily by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interval {
    Disabled,
    #[default]
    Daily,
    Weekly,
    Monthly,
}

impl Interval {
    pub const ALL: [Interval; 4] = [
        Interval::Disabled,
        Interval::Daily,
        Interval::Weekly,
        Interval::Monthly,
    ];

    /// Seconds between checks, or `None` when disabled.
    #[must_use]
    pub fn seconds(self) -> Option<u64> {
        const DAY: u64 = 24 * 60 * 60;
        match self {
            Interval::Disabled => None,
            Interval::Daily => Some(DAY),
            Interval::Weekly => Some(7 * DAY),
            Interval::Monthly => Some(30 * DAY),
        }
    }

    /// Rufus' translation id for the interval's name.
    #[must_use]
    pub fn loc_id(self) -> &'static str {
        match self {
            Interval::Disabled => "MSG_013",
            Interval::Daily => "MSG_014",
            Interval::Weekly => "MSG_015",
            Interval::Monthly => "MSG_016",
        }
    }

    /// Rufus' registry value: the interval in seconds, -1 when disabled.
    fn from_seconds(s: i64) -> Interval {
        Interval::ALL
            .into_iter()
            .find(|i| i.seconds().map_or(-1, |n| n as i64) == s)
            .unwrap_or_default()
    }
}

/// The update policy, saved as `key=value` lines in `update.ini` under the
/// per-user config directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub interval: Interval,
    pub include_betas: bool,
    /// Unix time of the last successful check.
    pub last_check: u64,
}

impl Settings {
    /// Saved settings, or `None` before the user was first asked, like
    /// Rufus' missing registry key.
    #[must_use]
    pub fn load() -> Option<Settings> {
        std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| s.parse().ok())
    }

    pub fn save(&self) -> Result<(), String> {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, self.to_string()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Whether an automatic check is due at Unix time `now`.
    #[must_use]
    pub fn is_due(&self, now: u64) -> bool {
        self.interval
            .seconds()
            .is_some_and(|s| now.saturating_sub(self.last_check) >= s)
    }
}

impl FromStr for Settings {
    type Err = String;

    /// Unknown keys and bad values are ignored, keeping the defaults.
    fn from_str(s: &str) -> Result<Settings, String> {
        let mut settings = Settings::default();
        for line in s.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "UpdateCheckInterval" => {
                    if let Ok(v) = value.parse() {
                        settings.interval = Interval::from_seconds(v);
                    }
                }
                "IncludeBetas" => settings.include_betas = value == "1",
                "LastUpdate" => settings.last_check = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        Ok(settings)
    }
}

impl fmt::Display for Settings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let interval = self.interval.seconds().map_or(-1, |s| s as i64);
        writeln!(f, "UpdateCheckInterval={interval}")?;
        writeln!(f, "IncludeBetas={}", u8::from(self.include_betas))?;
        writeln!(f, "LastUpdate={}", self.last_check)
    }
}

/// Current Unix time in seconds.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn settings_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("rustfus")
        .join("update.ini")
}

#[cfg(test)]
#[path = "../tests/unit/update.rs"]
mod tests;
