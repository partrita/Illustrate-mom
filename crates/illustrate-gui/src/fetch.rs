//! RCSB structure download with on-disk caching (see ADR-0005).
//!
//! All decision logic is network-free: callers inject the HTTP fetcher so URL building, ID
//! validation, `.pdb` → `.cif` fallback, and caching are testable headlessly.

use std::fs;
use std::path::Path;
use std::time::Duration;

/// Failure modes of a single download attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The server answered HTTP 404 (e.g. no legacy `.pdb` file for a large structure).
    NotFound,
    /// Any other transport or protocol failure.
    Other(String),
}

/// Structure text together with the file name it should be loaded under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedStructure {
    pub filename: String,
    pub content: String,
}

/// Validates a 4-character PDB ID (`^[0-9][A-Za-z0-9]{3}$`) and returns it upper-cased.
pub fn validate_pdb_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    let bytes = id.as_bytes();
    let valid = bytes.len() == 4
        && bytes[0].is_ascii_digit()
        && bytes[1..].iter().all(u8::is_ascii_alphanumeric);
    if valid {
        Ok(id.to_ascii_uppercase())
    } else {
        Err(format!(
            "'{id}' is not a valid PDB ID (expected 4 characters such as 2HHB)"
        ))
    }
}

/// Builds the RCSB download URL for a validated upper-case `id` and extension (`pdb` / `cif`).
pub fn rcsb_url(id: &str, ext: &str) -> String {
    format!("https://files.rcsb.org/download/{id}.{ext}")
}

/// Resolves `id` from the cache or via `fetcher`, preferring `.pdb` and falling back to `.cif`.
pub fn fetch_structure(
    id: &str,
    cache_dir: &Path,
    fetcher: &dyn Fn(&str) -> Result<String, FetchError>,
) -> Result<FetchedStructure, String> {
    let id = validate_pdb_id(id)?;

    for ext in ["pdb", "cif"] {
        let cached = cache_dir.join(format!("{id}.{ext}"));
        if let Ok(content) = fs::read_to_string(&cached)
            && !content.trim().is_empty()
        {
            return Ok(FetchedStructure {
                filename: format!("{id}.{ext}"),
                content,
            });
        }
    }

    for ext in ["pdb", "cif"] {
        match fetcher(&rcsb_url(&id, ext)) {
            Ok(content) => {
                let filename = format!("{id}.{ext}");
                if fs::create_dir_all(cache_dir).is_ok() {
                    let _ = fs::write(cache_dir.join(&filename), &content);
                }
                return Ok(FetchedStructure { filename, content });
            }
            Err(FetchError::NotFound) => {}
            Err(FetchError::Other(msg)) => {
                return Err(format!(
                    "Could not download {id} ({msg}). Check your connection or open a file instead."
                ));
            }
        }
    }
    Err(format!("PDB ID {id} was not found at RCSB"))
}

/// Downloads `url` as text over HTTPS with a 60 s overall timeout and a 200 MiB body limit.
pub fn http_fetch(url: &str) -> Result<String, FetchError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    match agent.get(url).call() {
        Ok(mut response) => response
            .body_mut()
            .with_config()
            .limit(200 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| FetchError::Other(e.to_string())),
        Err(ureq::Error::StatusCode(404)) => Err(FetchError::NotFound),
        Err(e) => Err(FetchError::Other(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp_cache(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("illustrate_fetch_{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn validates_and_normalizes_ids() {
        assert_eq!(validate_pdb_id(" 2hhb ").as_deref(), Ok("2HHB"));
        assert!(validate_pdb_id("HHB2").is_err());
        assert!(validate_pdb_id("2hh").is_err());
        assert!(validate_pdb_id("2hh/").is_err());
        assert!(validate_pdb_id("../etc").is_err());
    }

    #[test]
    fn falls_back_to_cif_and_caches_result() {
        let cache = temp_cache("fallback");
        let urls = RefCell::new(Vec::new());
        let fetcher = |url: &str| {
            urls.borrow_mut().push(url.to_string());
            if url.ends_with(".pdb") {
                Err(FetchError::NotFound)
            } else {
                Ok("data_7XYZ\n".to_string())
            }
        };
        let got = fetch_structure("7xyz", &cache, &fetcher).expect("cif fallback");
        assert_eq!(got.filename, "7XYZ.cif");
        assert_eq!(urls.borrow().len(), 2);

        let never = |_: &str| -> Result<String, FetchError> { panic!("cache must be used") };
        let again = fetch_structure("7XYZ", &cache, &never).expect("cache hit");
        assert_eq!(again, got);
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn reports_not_found_and_network_errors() {
        let cache = temp_cache("errors");
        let missing = |_: &str| Err(FetchError::NotFound);
        let err = fetch_structure("1abc", &cache, &missing).unwrap_err();
        assert!(err.contains("not found"));

        let offline = |_: &str| Err(FetchError::Other("offline".into()));
        let err = fetch_structure("1abc", &cache, &offline).unwrap_err();
        assert!(err.contains("offline"));
        let _ = fs::remove_dir_all(&cache);
    }
}
