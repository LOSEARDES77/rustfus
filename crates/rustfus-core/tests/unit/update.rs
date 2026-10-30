use super::*;

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

#[test]
fn parses_versions() {
    assert_eq!(
        v("v1.2.3"),
        Version {
            major: 1,
            minor: 2,
            patch: 3,
            pre: String::new()
        }
    );
    assert_eq!(v("0.2.0-beta.1+build.5").pre, "beta.1");
    assert_eq!(v("0.2.0-rc.1").to_string(), "0.2.0-rc.1");
    for bad in [
        "",
        "1.2",
        "1.2.3.4",
        "1.x.3",
        "1.2.3-",
        "1.2.3-a..b",
        "latest",
    ] {
        assert!(bad.parse::<Version>().is_err(), "{bad:?} parsed");
    }
    assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn orders_like_semver() {
    // Precedence example from semver.org, section 11.
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.1",
        "1.1.0",
        "2.0.0",
    ];
    for pair in chain.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
    }
    assert_eq!(v("v1.0.0"), v("1.0.0+meta"));
}

const RELEASES: &str = r#"[
  {"tag_name": "v0.3.0-beta.1", "html_url": "https://x/v0.3.0-beta.1", "body": "beta", "draft": false, "prerelease": true},
  {"tag_name": "v0.4.0", "html_url": "https://x/v0.4.0", "body": "draft", "draft": true, "prerelease": false},
  {"tag_name": "nightly", "html_url": "https://x/nightly", "body": null, "draft": false, "prerelease": false},
  {"tag_name": "v0.2.1", "html_url": "https://x/v0.2.1", "body": "fixes", "draft": false, "prerelease": false},
  {"tag_name": "v0.2.0", "html_url": "https://x/v0.2.0", "draft": false, "prerelease": false}
]"#;

#[test]
fn picks_newest_release() {
    let stable = newest_release(RELEASES.as_bytes(), false).unwrap().unwrap();
    assert_eq!(stable.version, v("0.2.1"));
    assert_eq!(stable.url, "https://x/v0.2.1");
    assert_eq!(stable.notes, "fixes");
    let beta = newest_release(RELEASES.as_bytes(), true).unwrap().unwrap();
    assert_eq!(beta.version, v("0.3.0-beta.1"));
    assert_eq!(newest_release(b"[]", true).unwrap(), None);
    assert!(newest_release(b"{\"message\": \"Not Found\"}", true).is_err());
}

#[test]
fn settings_round_trip() {
    let s = Settings {
        interval: Interval::Weekly,
        include_betas: true,
        last_check: 1_700_000_000,
    };
    assert_eq!(s.to_string().parse::<Settings>().unwrap(), s);
    let off = Settings {
        interval: Interval::Disabled,
        ..Settings::default()
    };
    assert_eq!(off.to_string().parse::<Settings>().unwrap(), off);
    assert_eq!(
        "garbage\nIncludeBetas=x".parse::<Settings>().unwrap(),
        Settings::default()
    );
}

#[test]
fn checks_when_due() {
    let s = Settings {
        last_check: 1_000_000,
        ..Settings::default()
    };
    assert!(Settings::default().is_due(now()));
    assert!(!s.is_due(1_000_000 + 3600));
    assert!(s.is_due(1_000_000 + 86_400));
    // A clock set back in time must not trigger a check on every launch.
    assert!(!s.is_due(0));
    let off = Settings {
        interval: Interval::Disabled,
        ..Settings::default()
    };
    assert!(!off.is_due(now()));
}

/// Network test: run with `cargo test net_live -- --ignored`.
#[test]
fn net_live_github_releases() {
    check(true).unwrap();
}
