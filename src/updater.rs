use crate::{json, ui, util};
use crate::util::has_flag;
use std::fs;
use std::path::Path;
use std::process::Command;

const REPO: &str = "devnextbd/pgaegis";
/// Release-signing public keys (PEM). The first existing file is used; the
/// /etc override lets operators pin/rotate without reinstalling.
const PUBKEY_PATHS: [&str; 2] = ["/etc/pgaegis/release-signing.pub", "/usr/share/pgaegis/release-signing.pub"];

pub fn dispatch(args: &[String]) -> Result<(), String> {
    let check = has_flag(args, "--check");
    let force = has_flag(args, "--force");
    let allow_unsigned = has_flag(args, "--allow-unsigned");
    let latest = latest_version()?;
    let current = env!("CARGO_PKG_VERSION");

    ui::card("pgaegis Update", &[("Installed", current.into()), ("Latest", latest.clone())]);

    if check {
        return Ok(());
    }
    match compare_versions(&latest, current) {
        std::cmp::Ordering::Equal if !force => {
            ui::success("pgaegis is already up to date");
            return Ok(());
        }
        std::cmp::Ordering::Less if !force => {
            ui::note("Installed version is newer than the latest release; nothing to do (use --force to reinstall the release).");
            return Ok(());
        }
        _ => {}
    }

    install_version(&latest, allow_unsigned)
}

/// Compare dotted numeric versions; non-numeric suffixes sort before the plain release.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> (Vec<u64>, bool) {
        let (core, pre) = v.split_once('-').map(|(c, p)| (c, !p.is_empty())).unwrap_or((v, false));
        (core.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect(), pre)
    };
    let (mut av, ap) = parse(a);
    let (mut bv, bp) = parse(b);
    let n = av.len().max(bv.len());
    av.resize(n, 0);
    bv.resize(n, 0);
    av.cmp(&bv).then(bp.cmp(&ap))
}

fn latest_version() -> Result<String, String> {
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = fetch_text(&api)?;
    let v = json::parse(&body).map_err(|e| format!("latest release JSON: {e}"))?;
    let tag = v.get("tag_name").and_then(json::Value::as_str).ok_or("latest release response has no tag_name")?;
    let version = tag.trim_start_matches('v');
    validate_version(version)?;
    Ok(version.to_string())
}

fn validate_version(version: &str) -> Result<(), String> {
    if !version.is_empty()
        && version.len() < 32
        && version.chars().next().is_some_and(|c| c.is_ascii_digit())
        && version.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c.is_ascii_alphabetic())
    {
        Ok(())
    } else {
        Err("release version contains unsupported characters".into())
    }
}

fn release_pubkey() -> Option<&'static str> {
    PUBKEY_PATHS.iter().copied().find(|p| Path::new(p).is_file())
}

fn install_version(version: &str, allow_unsigned: bool) -> Result<(), String> {
    util::require_root()?;
    validate_version(version)?;

    if !util::command_exists("curl") && !util::command_exists("wget") {
        return Err("curl or wget is required for pgaegis self-update".into());
    }
    if !util::command_exists("sha256sum") {
        return Err("sha256sum is required for pgaegis self-update".into());
    }
    let pubkey = release_pubkey();
    if pubkey.is_none() && !allow_unsigned {
        return Err(format!(
            "no release-signing public key found ({}); refusing an unsigned self-update. Install the key or rerun with --allow-unsigned to accept checksum-only verification",
            PUBKEY_PATHS.join(" or ")
        ));
    }

    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    // Private 0700 directory created exclusively: a local user cannot pre-create
    // it, plant symlinks, or swap install.sh between verification and execution.
    let dir = util::secure_temp_dir("pgaegis-update")?;
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let (installer, sums, sig) = (p("install.sh"), p("SHA256SUMS"), p("SHA256SUMS.sig"));

    let result = (|| {
        download(&format!("{base}/install.sh"), &installer)?;
        download(&format!("{base}/SHA256SUMS"), &sums)?;
        if let Some(key) = pubkey {
            download(&format!("{base}/SHA256SUMS.sig"), &sig).map_err(|e| format!("release signature download failed: {e}"))?;
            let ok = Command::new("openssl")
                .args(["dgst", "-sha256", "-verify", key, "-signature", &sig, &sums])
                .stdout(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err("release SHA256SUMS signature verification FAILED; update aborted".into());
            }
            ui::success("Release signature verified");
        } else {
            ui::warning("Unsigned update accepted (--allow-unsigned): integrity is checksum-only");
        }
        verify_release_file(&installer, &sums, "install.sh")?;

        let mut cmd = Command::new("sh");
        cmd.arg(&installer)
            .env("PGAEGIS_VERSION", version)
            .env("PGAEGIS_REPO", REPO)
            // The installer must use the SHA256SUMS that was just verified,
            // not download a second, unverified copy.
            .env("PGAEGIS_SHA256SUMS_FILE", &sums);
        if let Some(key) = pubkey {
            cmd.env("PGAEGIS_RELEASE_PUBKEY", key).env("PGAEGIS_SHA256SUMS_SIG_FILE", &sig);
        }
        let status = cmd.status().map_err(|e| format!("run verified installer: {e}"))?;
        if !status.success() {
            return Err(format!("pgaegis installer failed with {status}"));
        }
        ui::success(&format!("pgaegis updated to {version}"));
        Ok(())
    })();

    let _ = fs::remove_dir_all(&dir);
    result
}

fn fetch_text(url: &str) -> Result<String, String> {
    if util::command_exists("curl") {
        util::output("curl", &["-fsSL", "--retry", "3", "--connect-timeout", "15", "--max-time", "60", "--proto", "=https", "--tlsv1.2", url])
    } else if util::command_exists("wget") {
        util::output("wget", &["--https-only", "--timeout=30", "-qO-", url])
    } else {
        Err("curl or wget is required".into())
    }
}

fn download(url: &str, destination: &str) -> Result<(), String> {
    if util::command_exists("curl") {
        util::status("curl", &["-fL", "--retry", "3", "--connect-timeout", "15", "--max-time", "300", "--proto", "=https", "--tlsv1.2", url, "-o", destination])
    } else if util::command_exists("wget") {
        util::status("wget", &["--https-only", "--timeout=30", "-qO", destination, url])
    } else {
        Err("curl or wget is required".into())
    }
}

pub fn expected_hash(manifest: &str, name: &str) -> Result<String, String> {
    let expected = manifest
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let filename = parts.next()?.trim_start_matches('*');
            (filename == name).then(|| hash.to_string())
        })
        .ok_or_else(|| format!("release checksum manifest does not contain {name}"))?;
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid SHA256 in release manifest".into());
    }
    Ok(expected)
}

fn verify_release_file(file: &str, manifest_path: &str, name: &str) -> Result<(), String> {
    let manifest = fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
    let expected = expected_hash(&manifest, name)?;
    let output = util::output("sha256sum", &[file])?;
    let actual = output.split_whitespace().next().unwrap_or_default();
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err("verified installer SHA256 mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn version_ordering() {
        assert_eq!(compare_versions("1.0.10", "1.0.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.1.0-rc1", "1.1.0"), Ordering::Less);
        assert!(validate_version("1.0.4").is_ok());
        assert!(validate_version("../x").is_err());
        assert!(validate_version("v1").is_err());
    }

    #[test]
    fn manifest_lookup() {
        let h = "a".repeat(64);
        let m = format!("{h}  install.sh\n{h} *pgaegis_1.0.4_amd64.deb\n");
        assert_eq!(expected_hash(&m, "install.sh").unwrap(), h);
        assert!(expected_hash(&m, "pgaegis_1.0.4_amd64.deb").is_ok());
        assert!(expected_hash(&m, "missing").is_err());
        assert!(expected_hash("zz  install.sh\n", "install.sh").is_err());
    }
}
