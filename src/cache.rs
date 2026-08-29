//! On-disk cache under XDG_CACHE_HOME.
//!
//! cicdbar is re-exec'd by waybar every 60s, so all state lives here. The
//! contract that matters: a failed fetch never blanks the widget while any
//! previous value survives -- it is served and flagged stale.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum Freshness {
    /// Fetched just now.
    Fresh,
    /// Served from cache, still within its TTL.
    Cached { age_secs: u64 },
    /// The fetch failed; this is the last good value.
    Stale { age_secs: u64, reason: String },
}

impl Freshness {
    pub fn age_secs(&self) -> u64 {
        match self {
            Freshness::Fresh => 0,
            Freshness::Cached { age_secs } | Freshness::Stale { age_secs, .. } => *age_secs,
        }
    }
    pub fn is_stale(&self) -> bool {
        matches!(self, Freshness::Stale { .. })
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            Freshness::Stale { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

#[derive(Serialize, serde::Deserialize)]
struct Entry<T> {
    written_at: u64,
    value: T,
}

pub struct Cache {
    dir: PathBuf,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Cache {
    pub fn new(dir: PathBuf) -> Cache {
        let _ = std::fs::create_dir_all(&dir);
        Cache { dir }
    }

    pub fn default_dir() -> PathBuf {
        let base = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache")
            });
        base.join("cicdbar")
    }

    pub fn path_for(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn read<T: DeserializeOwned>(&self, key: &str) -> Option<(T, u64)> {
        let path = self.path_for(key);
        let raw = std::fs::read_to_string(&path).ok()?;
        // A truncated or malformed file is simply a cache miss.
        let entry: Entry<T> = serde_json::from_str(&raw).ok()?;
        let age = now_secs().saturating_sub(entry.written_at);
        Some((entry.value, age))
    }

    fn write<T: Serialize>(&self, key: &str, value: &T) {
        let entry = Entry {
            written_at: now_secs(),
            value,
        };
        let Ok(raw) = serde_json::to_string(&entry) else {
            return;
        };
        let path = self.path_for(key);
        // Write-then-rename so a crash cannot leave a half-written file that
        // the next exec would have to recover from.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, raw).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Serve `key` from cache if within `ttl_secs`, else call `fetch`.
    /// If `fetch` fails, fall back to any cached value regardless of age.
    pub fn get_or_refresh<T, E, F>(
        &self,
        key: &str,
        ttl_secs: u64,
        fetch: F,
    ) -> Result<(T, Freshness), E>
    where
        T: Serialize + DeserializeOwned,
        E: std::fmt::Display,
        F: FnOnce() -> Result<T, E>,
    {
        if let Some((value, age)) = self.read::<T>(key) {
            if age < ttl_secs {
                return Ok((value, Freshness::Cached { age_secs: age }));
            }
        }
        match fetch() {
            Ok(value) => {
                self.write(key, &value);
                Ok((value, Freshness::Fresh))
            }
            Err(e) => match self.read::<T>(key) {
                Some((value, age)) => Ok((
                    value,
                    Freshness::Stale {
                        age_secs: age,
                        reason: e.to_string(),
                    },
                )),
                None => Err(e),
            },
        }
    }

    /// Read a value with no TTL semantics, for state we manage ourselves.
    pub fn read_raw<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.read::<T>(key).map(|(v, _)| v)
    }

    /// Write a value with no TTL semantics.
    pub fn write_raw<T: Serialize>(&self, key: &str, value: &T) {
        self.write(key, value);
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
