//! The stable-channel version comparison, which decides whether a published
//! release is an update at all.
//!
//! The published ladder runs behind the working tree: every 0.x.x release is on
//! GitHub while the suites build 1.0.0 locally. A check that treated "tag
//! differs from my version" as an update would pull 0.9.0 binaries over a 1.0.0
//! install, so the direction of the difference is the whole contract here.
use fennec_rs::updater::is_newer;

#[test]
fn a_higher_release_is_an_update() {
    assert_eq!(is_newer("1.0.1", "1.0.0"), Some(true));
    assert_eq!(is_newer("1.1.0", "1.0.9"), Some(true));
    assert_eq!(is_newer("2.0.0", "1.9.9"), Some(true));
    assert_eq!(is_newer("0.10.0", "0.9.0"), Some(true), "0.10 is not 0.1");
}

#[test]
fn the_same_release_or_an_older_one_is_not() {
    assert_eq!(is_newer("1.0.0", "1.0.0"), Some(false));
    assert_eq!(is_newer("0.9.0", "1.0.0"), Some(false), "never downgrade an install");
    assert_eq!(is_newer("0.9.0", "1.0.0-rc1"), None, "a pre-release is not a number");
}

#[test]
fn a_tag_the_comparator_does_not_understand_offers_nothing() {
    assert_eq!(is_newer("v1.0.0", "1.0.0"), Some(false), "the leading v is the tag spelling");
    assert_eq!(is_newer("dev-abc1234", "1.0.0"), None, "the experimental channel is not a version");
    assert_eq!(is_newer("", "1.0.0"), None);
    assert_eq!(is_newer("1.0", "1.0.0"), Some(false), "a missing patch field reads as zero");
}

#[test]
fn the_asset_name_follows_the_platform() {
    let platform = fennec_rs::updater::Platform {
        os: "windows".to_string(),
        arch: "x86_64".to_string(),
        ext: "exe".to_string(),
    };
    assert_eq!(platform.asset_name("fennec-cli"), "fennec-cli-windows-x86_64.exe");
    let linux = fennec_rs::updater::Platform {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
        ext: "bin".to_string(),
    };
    assert_eq!(linux.asset_name("fennec-cli"), "fennec-cli-linux-aarch64");
}
