use super::*;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

fn rel(version: &str) -> Release {
    Release {
        version: version.into(),
        date: Some("2026-11-02".into()),
        body: format!("notes of {version}"),
    }
}

fn ts(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

#[test]
fn versions_are_major_minor_patch_compared_as_numbers() {
    assert_eq!(v("0.5.8"), Version(0, 5, 8));
    assert_eq!(v("v1.2.3"), Version(1, 2, 3));
    assert!(v("0.5.10") > v("0.5.9"));
    assert!(v("0.10.0") > v("0.9.99"));
    assert!(v("1.0.0") > v("0.99.99"));
    for bad in [
        "",
        "0.5",
        "0.5.8.1",
        "0.5.x",
        "0.5.8-rc1",
        "v",
        "0..8",
        "+1.2.3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad}");
    }
    assert_eq!(v("0.5.9").to_string(), "0.5.9");
    assert_eq!(Version::current().to_string(), CURRENT);
}

#[test]
fn the_changelog_sections_are_release_bodies() {
    let text = "# Changelog\n\nintro\n\n## v0.5.9 — 2026-11-02\n\n### New\n\n- **News** (#91).\n  - nested\n\n```\n## v9.9.9 — in a fence\n### kept\n```\n\n## v0.5.8 — 2026-10-31\n\n### Fixed\n\n- a fix\n\n## Notes\n\n## vNext\n\ndropped\n";
    let r = parse_changelog(text);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].version, "0.5.9");
    assert_eq!(r[0].date.as_deref(), Some("2026-11-02"));
    assert_eq!(
        r[0].body,
        "## New\n\n- **News** (#91).\n  - nested\n\n```\n## v9.9.9 — in a fence\n### kept\n```"
    );
    assert_eq!(r[1].version, "0.5.8");
    // A `## v` heading that is no version ends the section, as the release script does.
    assert_eq!(r[1].body, "## Fixed\n\n- a fix\n\n## Notes");
}

#[test]
fn the_embedded_changelog_holds_this_version() {
    let all = embedded();
    assert!(
        all.iter()
            .any(|r| r.version == CURRENT && !r.body.is_empty()),
        "CHANGELOG.md has a section for {CURRENT}"
    );
    assert!(all.windows(2).all(|w| w[0].parsed() > w[1].parsed()));
}

#[test]
fn github_releases_leave_out_drafts_prereleases_and_other_tags() {
    let list: Vec<GithubRelease> = serde_json::from_value(serde_json::json!([
        {"tag_name": "v0.6.0", "body": "draft", "draft": true, "prerelease": false},
        {"tag_name": "v0.5.10", "body": "pre", "draft": false, "prerelease": true},
        {"tag_name": "v0.5.8", "body": "eight\r\n", "draft": false, "prerelease": false,
         "published_at": "2026-10-31T10:00:00Z"},
        {"tag_name": "v0.5.9", "body": null, "draft": false, "prerelease": false},
        {"tag_name": "nightly", "body": "x", "draft": false, "prerelease": false}
    ]))
    .unwrap();
    let r = from_github(list);
    assert_eq!(
        r.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
        ["0.5.9", "0.5.8"]
    );
    assert_eq!(r[1].date.as_deref(), Some("2026-10-31"));
    assert_eq!(r[1].body, "eight");
    assert_eq!(r[0].body, "");
}

#[test]
fn the_cache_keeps_the_newer_releases_and_the_attempt_time() {
    let now = ts("2026-11-01T10:00:00Z");
    let current = Version::current();
    let up = Version(current.0, current.1, current.2 + 1);
    let releases = vec![rel(&up.to_string()), rel(CURRENT)];
    let c = Cache::from_fetch(&releases, now);
    assert_eq!(c.latest.as_deref(), Some(up.to_string().as_str()));
    assert_eq!(c.newer, vec![rel(&up.to_string())]);
    assert_eq!(c.update_for(current), Some(up));
    assert_eq!(c.update_for(up), None);
    assert!(c.newer_than(up).is_empty());
    assert!(c.attempted_recently(ts("2026-11-02T09:59:59Z")));
    assert!(!c.attempted_recently(ts("2026-11-02T10:00:00Z")));
    assert!(
        !c.attempted_recently(ts("2026-11-01T09:00:00Z")),
        "a clock gone back"
    );

    // A failed attempt keeps what was found, with the new attempt time.
    let later = ts("2026-11-03T10:00:00Z");
    let f = Cache::failed(Some(c.clone()), later);
    assert_eq!((f.attempted_at, f.fetched_at), (later, Some(now)));
    assert_eq!(f.newer, c.newer);
    assert!(f.attempted_recently(later) && !f.fetched_recently(later));
    let first = Cache::failed(None, later);
    assert_eq!((first.fetched_at, first.latest), (None, None));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub/update.json");
    c.save(&path).unwrap();
    assert_eq!(Cache::load(&path), Some(c));
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(Cache::load(&path), None);
}

#[test]
fn a_version_change_is_noted_once_and_a_first_start_says_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("illumio-tui/last-version");
    assert_eq!(note_start(&path, "0.5.8"), None, "fresh install");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "0.5.8\n");
    assert_eq!(note_start(&path, "0.5.8"), None);
    assert_eq!(note_start(&path, "0.5.9"), Some("0.5.9".into()));
    assert_eq!(note_start(&path, "0.5.9"), None, "once");
}

#[test]
fn ranges_are_after_a_up_to_b() {
    let r = Range::parse("0.5.6..").unwrap();
    assert!(!r.contains(v("0.5.6")) && r.contains(v("0.5.7")) && r.contains(v("9.0.0")));
    let r = Range::parse("0.5.6..0.5.8").unwrap();
    assert!(r.contains(v("0.5.8")) && !r.contains(v("0.5.9")));
    assert!(r.reaches_past(v("0.5.7")) && !r.reaches_past(v("0.5.8")));
    for bad in ["0.5.6", "..0.5.8", "x..", "0.5.8..0.5.6", "0.5.8..0.5.8"] {
        assert!(Range::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn news_without_a_range_is_what_is_newer_else_this_version() {
    let current = v("0.5.8");
    let embedded = vec![rel("0.5.8"), rel("0.5.7"), rel("0.5.6")];
    let newer = vec![rel("0.5.10"), rel("0.5.9"), rel("0.5.8")];
    let names = |r: Vec<Release>| r.into_iter().map(|r| r.version).collect::<Vec<_>>();
    assert_eq!(
        names(select(None, current, &embedded, &newer)),
        ["0.5.10", "0.5.9"]
    );
    assert_eq!(names(select(None, current, &embedded, &[])), ["0.5.8"]);
    let r = Range::parse("0.5.6..").unwrap();
    assert_eq!(
        names(select(Some(&r), current, &embedded, &newer)),
        ["0.5.10", "0.5.9", "0.5.8", "0.5.7"]
    );
    let r = Range::parse("0.5.6..0.5.9").unwrap();
    assert_eq!(
        names(select(Some(&r), current, &embedded, &newer)),
        ["0.5.9", "0.5.8", "0.5.7"]
    );
}

#[test]
fn markdown_heads_each_release() {
    let text = markdown(&[rel("0.5.9"), rel("0.5.8")]);
    assert_eq!(
        text,
        "# v0.5.9 — 2026-11-02\n\nnotes of 0.5.9\n\n# v0.5.8 — 2026-11-02\n\nnotes of 0.5.8\n"
    );
}

#[test]
fn the_install_follows_the_binary_path() {
    let cases = [
        (
            "/opt/homebrew/Cellar/illumio-tui/0.5.8/bin/illumio-tui",
            Install::Homebrew,
        ),
        (
            r"C:\Users\alex\scoop\apps\illumio-tui\current\illumio-tui.exe",
            Install::Scoop,
        ),
        (
            r"C:\Users\alex\AppData\Local\Microsoft\WinGet\Packages\alexgoller.illumio-tui_x\illumio-tui.exe",
            Install::Winget,
        ),
        (
            r"C:\Program Files\illumio-tui\illumio-tui.exe",
            Install::Msi,
        ),
        ("/usr/bin/illumio-tui", Install::Package),
        ("/Users/alex/.cargo/bin/illumio-tui", Install::Other),
    ];
    for (path, want) in cases {
        assert_eq!(Install::from_path(path), want, "{path}");
    }
    assert_eq!(
        Install::Homebrew.upgrade_hint("0.5.9"),
        "brew upgrade illumio-tui"
    );
    assert_eq!(
        Install::Scoop.upgrade_hint("0.5.9"),
        "scoop update illumio-tui"
    );
    assert_eq!(
        Install::Winget.upgrade_hint("0.5.9"),
        "winget upgrade illumio-tui"
    );
    assert!(
        Install::Msi
            .upgrade_hint("0.5.9")
            .starts_with("winget upgrade illumio-tui")
    );
    let page = "https://github.com/alexgoller/illumio-tui/releases/tag/v0.5.9";
    for i in [Install::Msi, Install::Package, Install::Other] {
        assert!(i.upgrade_hint("0.5.9").contains(page), "{i:?}");
    }
    assert!(Install::Package.upgrade_hint("0.5.9").contains(".deb"));
}

#[tokio::test]
async fn one_fetch_reads_the_release_list_and_records_it() {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let current = Version::current();
    let up = Version(current.0, current.1 + 1, 0).to_string();
    Mock::given(method("GET"))
        .and(path("/repos/alexgoller/illumio-tui/releases"))
        .and(query_param("per_page", "30"))
        .and(header("user-agent", concat!("illumio-tui/", env!("CARGO_PKG_VERSION"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"tag_name": format!("v{up}"), "body": "new things", "draft": false, "prerelease": false}
        ])))
        .expect(2)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("update.json");
    // A first start fetches; a second within the day reads the cache only.
    assert_eq!(startup_check(&server.uri(), &path).await, Some(v(&up)));
    assert_eq!(startup_check(&server.uri(), &path).await, Some(v(&up)));
    // A day later it asks again.
    let mut c = Cache::load(&path).unwrap();
    c.attempted_at = ts("2020-01-01T00:00:00Z");
    c.save(&path).unwrap();
    assert_eq!(startup_check(&server.uri(), &path).await, Some(v(&up)));
}

#[tokio::test]
async fn a_failed_startup_check_shows_nothing_and_waits_a_day() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("update.json");
    assert_eq!(startup_check(&server.uri(), &path).await, None);
    assert_eq!(
        startup_check(&server.uri(), &path).await,
        None,
        "no retry within the day"
    );
    let c = Cache::load(&path).unwrap();
    assert_eq!((c.fetched_at, c.latest), (None, None));
}
