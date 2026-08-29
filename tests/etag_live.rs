//! Conditional requests against the real GitHub API.
//!
//! 304 responses do not count against the REST rate limit, which is the
//! cheapest available defence against the burst limit that bit this widget.

use cicdbar::http::Http;
use cicdbar::token::TokenSource;

fn store_dir(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("cicdbar-etag-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
#[ignore = "hits the live GitHub API; needs a gh token"]
fn a_repeated_request_is_answered_304_by_the_real_api() {
    let http = Http::new(TokenSource::GhCli.resolve().expect("token"))
        .expect("client")
        .with_etag_store(store_dir("basic"));

    let path = "/repos/DataZooDE/heron/actions/runs?per_page=5";
    let first: serde_json::Value = http.get_json(path).expect("first");
    assert_eq!(http.not_modified_count(), 0, "nothing cached yet");

    let second: serde_json::Value = http.get_json(path).expect("second");
    assert_eq!(http.not_modified_count(), 1, "second request must be a 304");
    assert_eq!(first, second, "304 must replay the stored body verbatim");
}

#[test]
#[ignore = "hits the live GitHub API; needs a gh token"]
fn a_different_url_does_not_reuse_another_urls_etag() {
    let http = Http::new(TokenSource::GhCli.resolve().expect("token"))
        .expect("client")
        .with_etag_store(store_dir("distinct"));
    let _: serde_json::Value = http.get_json("/repos/DataZooDE/heron").expect("a");
    let b: serde_json::Value = http.get_json("/repos/DataZooDE/erpl-proto").expect("b");
    assert_eq!(http.not_modified_count(), 0);
    assert_eq!(b["name"], "erpl-proto");
}

#[test]
#[ignore = "hits the live GitHub API; needs a gh token"]
fn the_etag_store_survives_across_processes() {
    // waybar re-execs the binary; the store is only useful if it persists.
    let dir = store_dir("across");
    let path = "/repos/DataZooDE/heron/actions/runs?per_page=3";
    {
        let http = Http::new(TokenSource::GhCli.resolve().unwrap())
            .unwrap()
            .with_etag_store(dir.clone());
        let _: serde_json::Value = http.get_json(path).expect("prime");
    }
    let http2 = Http::new(TokenSource::GhCli.resolve().unwrap())
        .unwrap()
        .with_etag_store(dir);
    let _: serde_json::Value = http2.get_json(path).expect("reuse");
    assert_eq!(
        http2.not_modified_count(),
        1,
        "a fresh process must reuse the stored etag"
    );
}

#[test]
#[ignore = "hits the live GitHub API; needs a gh token"]
fn a_corrupt_etag_entry_falls_back_to_a_normal_request() {
    let dir = store_dir("corrupt");
    let http = Http::new(TokenSource::GhCli.resolve().unwrap())
        .unwrap()
        .with_etag_store(dir.clone());
    let path = "/repos/DataZooDE/heron/actions/runs?per_page=3";
    let _: serde_json::Value = http.get_json(path).expect("prime");
    for e in std::fs::read_dir(&dir).unwrap() {
        std::fs::write(e.unwrap().path(), "{truncated").unwrap();
    }
    let v: serde_json::Value = http.get_json(path).expect("must recover");
    assert!(v.get("workflow_runs").is_some());
}
