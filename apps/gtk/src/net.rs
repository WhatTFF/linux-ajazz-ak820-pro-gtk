//! Small HTTP helpers for GIPHY search and GitHub release checks.
//!
//! All requests go through `ureq` (rustls). Background threads call these
//! functions; the GTK side only ever sees plain data.

use std::time::Duration;

use serde::Deserialize;

/// GitHub Releases API, newest first.
const RELEASES: &str =
    "https://api.github.com/repos/WhatTFF/linux-ajazz-ak820-pro-gtk/releases?per_page=10";
/// GIPHY public API.
const GIPHY: &str = "https://api.giphy.com/v1/gifs/";
/// Refuse uncompressed bodies larger than this (release assets are handled by
/// streaming, not through this limit).
const MAX_JSON: u64 = 4 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(45)))
            .build(),
    )
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut response = agent()
        .get(url)
        .header("User-Agent", "ak820-gtk")
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_JSON)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// A release asset that is relevant to this build.
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// One GitHub release.
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// Fetch the newest non-draft releases.
pub fn releases() -> Result<Vec<Release>, String> {
    let bytes = get_bytes(RELEASES)?;
    let mut releases: Vec<Release> =
        serde_json::from_slice(&bytes).map_err(|e| format!("GitHub release JSON: {e}"))?;
    releases.retain(|r| !r.draft);
    Ok(releases)
}

/// A GIPHY result, reduced to what the picker needs.
#[derive(Clone, Debug)]
pub struct Gif {
    pub id: String,
    pub title: String,
    pub preview_url: String,
    pub url: String,
}

#[derive(Deserialize)]
struct GiphyResponse {
    data: Vec<GiphyItem>,
}

#[derive(Deserialize)]
struct GiphyItem {
    id: String,
    #[serde(default)]
    title: String,
    images: GiphyImages,
}

#[derive(Deserialize)]
struct GiphyImages {
    fixed_width_small: GiphyImage,
    original: GiphyImage,
}

#[derive(Deserialize)]
struct GiphyImage {
    url: String,
}

/// Percent-encode a query value for GIPHY.
fn encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}

/// Search GIPHY (or fetch trending when `query` is empty).
pub fn giphy_search(
    key: &str,
    query: &str,
    rating: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<Gif>, String> {
    if key.trim().is_empty() {
        return Err("no GIPHY API key: add one in Settings".into());
    }
    let rating = match rating {
        "g" | "pg" | "pg-13" | "r" => rating,
        _ => "r",
    };
    let mut url = format!(
        "{GIPHY}{}?api_key={}&limit={limit}&offset={offset}&rating={rating}",
        if query.trim().is_empty() {
            "trending"
        } else {
            "search"
        },
        encode(key.trim()),
    );
    if !query.trim().is_empty() {
        url.push_str("&q=");
        url.push_str(&encode(query.trim()));
    }
    let bytes = get_bytes(&url)?;
    let parsed: GiphyResponse =
        serde_json::from_slice(&bytes).map_err(|e| format!("GIPHY JSON: {e}"))?;
    Ok(parsed
        .data
        .into_iter()
        .map(|item| Gif {
            id: item.id,
            title: item.title,
            preview_url: item.images.fixed_width_small.url,
            url: item.images.original.url,
        })
        .collect())
}

/// Only allow HTTPS media downloads from GIPHY.
fn giphy_url_ok(url: &str) -> bool {
    let rest = match url.strip_prefix("https://") {
        Some(rest) => rest,
        None => return false,
    };
    let host = rest.split('/').next().unwrap_or_default();
    host == "media.giphy.com"
        || host.ends_with(".giphy.com")
        || host == "i.giphy.com"
        || host == "media0.giphy.com"
        || host == "media1.giphy.com"
        || host == "media2.giphy.com"
        || host == "media3.giphy.com"
        || host == "media4.giphy.com"
}

/// Download one GIPHY media file (preview or full GIF).
pub fn giphy_download(url: &str) -> Result<Vec<u8>, String> {
    if !giphy_url_ok(url) {
        return Err(format!("refusing to download from {url}"));
    }
    let mut response = agent()
        .get(url)
        .header("User-Agent", "ak820-gtk")
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(24 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// Only allow release downloads from GitHub's own hosts.
fn github_url_ok(url: &str) -> bool {
    let rest = match url.strip_prefix("https://") {
        Some(rest) => rest,
        None => return false,
    };
    let host = rest.split('/').next().unwrap_or_default();
    host == "github.com"
        || host == "objects.githubusercontent.com"
        || host == "release-assets.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

/// Stream a release asset to `path`.
pub fn download_release(url: &str, path: &std::path::Path) -> Result<(), String> {
    if !github_url_ok(url) {
        return Err(format!("refusing to download from {url}"));
    }
    let mut response = agent()
        .get(url)
        .header("User-Agent", "ak820-gtk")
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Fetch an image (used for GIPHY thumbnails).
pub fn image_bytes(url: &str) -> Result<Vec<u8>, String> {
    giphy_download(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(encode("hello world"), "hello%20world");
        assert_eq!(encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(encode("Ä"), "%C3%84");
    }

    #[test]
    fn only_https_giphy_hosts() {
        assert!(giphy_url_ok("https://media.giphy.com/media/x/giphy.gif"));
        assert!(giphy_url_ok("https://i.giphy.com/x.webp"));
        assert!(!giphy_url_ok("http://media.giphy.com/x.gif"));
        assert!(!giphy_url_ok("https://example.com/x.gif"));
    }

    #[test]
    fn only_github_release_hosts() {
        assert!(github_url_ok(
            "https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases/download/v1/x.rpm"
        ));
        assert!(github_url_ok("https://objects.githubusercontent.com/x"));
        assert!(!github_url_ok("https://evil.example/x.rpm"));
        assert!(!github_url_ok("http://github.com/x"));
    }
}
