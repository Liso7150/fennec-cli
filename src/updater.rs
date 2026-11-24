//! Auto-update system: silent background updates from GitHub releases,
//! plus `fennec update --experimental` for devs to pull from main branch.
//!
//! Normal mode checks tagged releases and updates silently in the background.
//! Experimental mode pulls the latest commit from main/origin for developers.
//! Every download is verified against a SHA-256 checksum before applying.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// GitHub repository for releases
const GITHUB_REPO: &str = "Liso7150/fennec-cli";

/// Update channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    /// Stable releases from GitHub tags
    Stable,
    /// Latest commits from main branch
    Experimental,
}

/// Platform triple for binary selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub ext: String,
}

impl Platform {
    /// Detect the current platform
    pub fn current() -> Self {
        let os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        }
        .to_string();

        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "unknown"
        }
        .to_string();

        let ext = if cfg!(target_os = "windows") {
            "exe"
        } else {
            "bin"
        }
        .to_string();

        Platform { os, arch, ext }
    }

    /// Asset name for this platform (e.g., "fennec-windows-x86_64.exe")
    pub fn asset_name(&self, base: &str) -> String {
        if self.ext == "exe" {
            format!("{}-{}-{}.{}", base, self.os, self.arch, self.ext)
        } else {
            format!("{}-{}-{}", base, self.os, self.arch)
        }
    }
}

/// Release information from GitHub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub published_at: String,
    pub assets: Vec<Asset>,
    pub checksums_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
}

/// Update status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateStatus {
    /// No update available
    UpToDate,
    /// Update downloaded and ready to apply
    UpdateReady { version: String, path: PathBuf },
    /// Update applied, restart required
    Applied { version: String },
    /// Error during update
    Error(String),
}

/// `Some(true)` when `candidate` is a strictly newer dotted version than
/// `current`, `Some(false)` when it is the same or older, and `None` when
/// either side is not three plain numbers (an `dev-<sha>` tag, say) so the
/// caller refuses to guess.
///
/// This exists because the published releases run *behind* a working tree: a
/// 1.0.0 install must not pull the 0.9.0 assets just because the tag differs.
pub fn is_newer(candidate: &str, current: &str) -> Option<bool> {
    fn parts(version: &str) -> Option<[u64; 3]> {
        let version = version.trim().trim_start_matches('v');
        let mut numbers = [0u64; 3];
        for (field, slot) in version.split('.').zip(numbers.iter_mut()) {
            *slot = field.parse().ok()?;
        }
        Some(numbers)
    }
    let (candidate, current) = (parts(candidate)?, parts(current)?);
    Some(candidate > current)
}

/// Check for updates from GitHub releases
pub async fn check_stable(current_version: &str) -> Result<Option<Release>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("fennec-updater")
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub API: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| format!("Parse release: {e}"))?;

    // Only a release newer than the running binary is an update. The published
    // ladder runs behind a working tree, so an unequal tag is not enough.
    let version = release.tag_name.trim_start_matches('v').to_string();
    if is_newer(&version, current_version) != Some(true) {
        return Ok(None);
    }

    let assets: Vec<Asset> = release
        .assets
        .into_iter()
        .map(|a| Asset {
            name: a.name,
            download_url: a.browser_download_url,
            size: a.size,
        })
        .collect();
    // The publisher's digest list, when the release attached one. Absent means
    // the download cannot be verified, which `checksum_for` reports rather than
    // silently installing whatever came back.
    let checksums_url = assets
        .iter()
        .find(|a| a.name == CHECKSUMS_ASSET)
        .map(|a| a.download_url.clone());

    Ok(Some(Release {
        version,
        tag: release.tag_name,
        published_at: release.published_at,
        assets,
        checksums_url,
    }))
}

/// Release asset that carries the SHA-256 digests of the other assets.
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS";

/// The expected digest of `asset_name`, read from the release's checksums
/// asset. A missing checksums asset or an unreadable manifest is an error;
/// `None` means the manifest has no entry for that exact basename. Binary
/// callers must reject both.
pub async fn checksum_for(release: &Release, asset_name: &str) -> Result<Option<String>, String> {
    let url = release
        .checksums_url
        .as_deref()
        .ok_or_else(|| format!("Release is missing {CHECKSUMS_ASSET}"))?;
    let text = fetch_text(url).await?;
    parse_checksum(&text, asset_name)
}

fn parse_checksum(text: &str, asset_name: &str) -> Result<Option<String>, String> {
    let mut found = None;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let digest = fields
            .next()
            .ok_or_else(|| format!("Parse {CHECKSUMS_ASSET} line {}", index + 1))?;
        let file = fields
            .next()
            .ok_or_else(|| format!("Parse {CHECKSUMS_ASSET} line {}", index + 1))?;
        if fields.next().is_some() {
            return Err(format!(
                "Parse {CHECKSUMS_ASSET} line {}: expected two fields",
                index + 1
            ));
        }
        let file = file.strip_prefix('*').unwrap_or(file);
        let digest = normalize_checksum(digest)
            .map_err(|e| format!("Parse {CHECKSUMS_ASSET} line {}: {e}", index + 1))?;
        if file == asset_name {
            if found.is_some() {
                return Err(format!(
                    "Duplicate {CHECKSUMS_ASSET} entry for {asset_name}"
                ));
            }
            found = Some(digest);
        }
    }
    Ok(found)
}

fn normalize_checksum(checksum: &str) -> Result<String, String> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Checksum must be exactly 64 hexadecimal characters".to_string());
    }
    Ok(checksum.to_ascii_lowercase())
}

/// Download a release binary only after its exact manifest entry and digest
/// have been verified. No caller needs to stage an unverified release asset.
pub async fn download_release_asset(
    release: &Release,
    asset_name: &str,
) -> Result<Vec<u8>, String> {
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("No asset for platform: {asset_name}"))?;
    let checksum = checksum_for(release, asset_name)
        .await?
        .ok_or_else(|| format!("{CHECKSUMS_ASSET} has no entry for {asset_name}"))?;
    download_asset(&asset.download_url, Some(&checksum)).await
}

async fn fetch_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("fennec-updater")
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Download {CHECKSUMS_ASSET} returned {}",
            resp.status()
        ));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("Read body: {e}"))?;
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("Parse {CHECKSUMS_ASSET}: {e}"))
}

/// Check for experimental updates from main branch
pub async fn check_experimental() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{}/commits/main", GITHUB_REPO);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("fennec-updater")
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub API: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let commit: GitHubCommit = resp
        .json()
        .await
        .map_err(|e| format!("Parse commit: {e}"))?;

    // For experimental, we always "update" to the latest commit
    let short_sha = &commit.sha[..7];

    Ok(Some(Release {
        version: format!("dev-{}", short_sha),
        tag: commit.sha.clone(),
        published_at: commit.commit.author.date,
        assets: vec![],
        checksums_url: None,
    }))
}

/// Download an asset, requiring a valid digest whenever one is supplied.
/// `None` is only for metadata such as the checksums manifest itself.
pub async fn download_asset(url: &str, expected_checksum: Option<&str>) -> Result<Vec<u8>, String> {
    let expected_checksum = expected_checksum.map(normalize_checksum).transpose()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("fennec-updater")
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Download returned {}", resp.status()));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Read bytes: {e}"))?
        .to_vec();

    if let Some(expected) = expected_checksum {
        let actual = sha256_hex(&bytes);
        if actual != expected {
            return Err(format!(
                "Checksum mismatch: expected {}, got {}",
                expected, actual
            ));
        }
    }

    Ok(bytes)
}

/// Compute SHA-256 checksum of bytes
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Install `bytes` over the running executable, in place of the file this
/// process was started from.
///
/// Windows refuses to write to an in-use image, so the live binary is renamed
/// aside first and the new one copied into its place; the `.old` sibling is
/// what survives if the update turns out to be bad. Unix replaces the inode
/// and restores the execute bits a downloaded asset cannot carry.
pub fn install_bytes(bytes: &[u8], asset_name: &str) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("Current exe: {e}"))?;
    let temp_path = std::env::temp_dir().join(asset_name);
    std::fs::write(&temp_path, bytes).map_err(|e| format!("Write temp: {e}"))?;
    let result = replace_binary(&temp_path, &current_exe);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn replace_binary(new_path: &std::path::Path, current_exe: &std::path::Path) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        let backup = current_exe.with_extension("exe.old");
        if backup.exists() {
            std::fs::remove_file(&backup).map_err(|e| format!("Remove backup: {e}"))?;
        }
        std::fs::rename(current_exe, &backup).map_err(|e| format!("Backup current: {e}"))?;
        std::fs::copy(new_path, current_exe).map_err(|e| format!("Install new: {e}"))?;
    } else {
        std::fs::copy(new_path, current_exe).map_err(|e| format!("Replace: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(current_exe, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("Chmod: {e}"))?;
        }
    }
    Ok(())
}

// GitHub API response types
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Deserialize)]
struct GitHubCommit {
    sha: String,
    commit: GitHubCommitDetail,
}

#[derive(Deserialize)]
struct GitHubCommitDetail {
    author: GitHubAuthor,
}

#[derive(Deserialize)]
struct GitHubAuthor {
    date: String,
}
