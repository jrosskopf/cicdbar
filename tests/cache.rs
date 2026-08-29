//! Cache tests use the real filesystem, and the degraded path is proved with
//! a real HTTP client hitting a real closed port -- not a simulated error.

use cicdbar::cache::{Cache, Freshness};
use cicdbar::http::Http;
use cicdbar::providers::github_billing;
use std::sync::atomic::{AtomicUsize, Ordering};

fn cache_dir(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("cicdbar-cache-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn a_fresh_entry_is_served_without_refetching() {
    let c = Cache::new(cache_dir("fresh"));
    let calls = AtomicUsize::new(0);
    let fetch = || {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok::<_, String>(vec![1u32, 2, 3])
    };
    let (v, f) = c.get_or_refresh("k", 900, fetch).unwrap();
    assert_eq!(v, vec![1, 2, 3]);
    assert!(matches!(f, Freshness::Fresh));
    let (v2, f2) = c.get_or_refresh("k", 900, fetch).unwrap();
    assert_eq!(v2, vec![1, 2, 3]);
    assert!(matches!(f2, Freshness::Cached { .. }), "second call must hit the cache");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no second network call");
}

#[test]
fn an_expired_entry_is_refetched() {
    let c = Cache::new(cache_dir("expired"));
    let calls = AtomicUsize::new(0);
    let fetch = || {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        Ok::<_, String>(vec![n as u32])
    };
    c.get_or_refresh("k", 900, fetch).unwrap();
    // ttl of 0 means everything already written is expired.
    let (v, f) = c.get_or_refresh("k", 0, fetch).unwrap();
    assert_eq!(v, vec![1]);
    assert!(matches!(f, Freshness::Fresh));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_corrupt_cache_file_is_recovered_from() {
    let dir = cache_dir("corrupt");
    let c = Cache::new(dir.clone());
    c.get_or_refresh("k", 900, || Ok::<_, String>(vec![7u32])).unwrap();
    // Something truncated the file mid-write.
    let path = c.path_for("k");
    std::fs::write(&path, "{not json").unwrap();
    let (v, f) = c.get_or_refresh("k", 900, || Ok::<_, String>(vec![9u32])).unwrap();
    assert_eq!(v, vec![9]);
    assert!(matches!(f, Freshness::Fresh));
}

#[test]
fn a_real_unreachable_api_serves_stale_data_and_marks_it() {
    let c = Cache::new(cache_dir("stale"));
    // Seed the cache with something good.
    c.get_or_refresh("billing", 900, || Ok::<_, String>(vec![42u32])).unwrap();

    // A real client against a real closed port on localhost.
    let dead = Http::with_base("token".into(), "http://127.0.0.1:1".into()).unwrap();
    let (v, f) = c
        .get_or_refresh("billing", 0, || {
            github_billing::fetch(&dead, "DataZooDE", 2026, 8)
                .map(|_| vec![0u32])
                .map_err(|e| e.short())
        })
        .expect("stale data must rescue a failed fetch");
    assert_eq!(v, vec![42], "stale value served");
    match f {
        Freshness::Stale { reason, .. } => assert_eq!(reason, "unreachable"),
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn a_failed_fetch_with_no_cache_at_all_is_an_error() {
    let c = Cache::new(cache_dir("nocache"));
    let dead = Http::with_base("token".into(), "http://127.0.0.1:1".into()).unwrap();
    let r = c.get_or_refresh("billing", 0, || {
        github_billing::fetch(&dead, "DataZooDE", 2026, 8)
            .map(|_| vec![0u32])
            .map_err(|e| e.short())
    });
    assert!(r.is_err());
}

#[test]
fn entries_written_by_one_instance_are_read_by_the_next() {
    // The binary is stateless and re-execed every 60s: this is the real path.
    let dir = cache_dir("across");
    let calls = AtomicUsize::new(0);
    {
        let c = Cache::new(dir.clone());
        c.get_or_refresh("k", 900, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(vec![5u32])
        })
        .unwrap();
    }
    let c2 = Cache::new(dir);
    let (v, _) = c2
        .get_or_refresh("k", 900, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(vec![6u32])
        })
        .unwrap();
    assert_eq!(v, vec![5]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// ---- 403 classification ----

#[test]
fn a_throttling_403_is_not_mistaken_for_a_permissions_403() {
    use cicdbar::http::classify;
    // GitHub returns 403 for both "you may not read this" and "you asked too
    // fast". Confusing them makes the widget claim you lost billing access
    // during a burst.
    let throttled = classify(
        403,
        "API rate limit exceeded for user ID 851749. If you reach out to GitHub Support…".into(),
    );
    assert!(throttled.is_rate_limited(), "got {throttled:?}");
    assert!(!throttled.is_access_denied());

    let secondary = classify(403, "You have exceeded a secondary rate limit".into());
    assert!(secondary.is_rate_limited());

    let denied = classify(403, "No access to billing usage data.".into());
    assert!(denied.is_access_denied(), "got {denied:?}");
    assert!(!denied.is_rate_limited());
    assert_eq!(denied.short(), "no billing access");

    assert!(classify(404, "Not Found".into()).is_not_found());
    assert!(classify(429, "too many requests".into()).is_rate_limited());
}
