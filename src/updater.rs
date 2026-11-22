use crate::{ui, util};
use std::fs;
use std::process::Command;

const REPO: &str = "devnextbd/pgaegis";

pub fn dispatch(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|x| x == "--check");
    let force = args.iter().any(|x| x == "--force");
    let latest = latest_version()?;
    let current = env!("CARGO_PKG_VERSION");

    ui::card(
        "pgaegis Update",
        &[("Installed", current.into()), ("Latest", latest.clone())],
    );

    if check {
        return Ok(());
    }
    if latest == current && !force {
        ui::success("pgaegis is already up to date");
        return Ok(());
    }

    install_version(&latest)
}

fn latest_version() -> Result<String, String> {
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = fetch_text(&api)?;
    let key = "\"tag_name\"";
    let pos = body
        .find(key)
        .ok_or("latest release response has no tag_name")?;
    let rest = &body[pos + key.len()..];
    let colon = rest.find(':').ok_or("invalid tag_name JSON")?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest
        .strip_prefix('"')
        .ok_or("invalid tag_name value")?;
    let end = rest.find('"').ok_or("invalid tag_name value")?;
    let version = rest[..end].trim_start_matches('v');
    validate_version(version)?;
    Ok(version.to_string())
}

fn validate_version(version: &str) -> Result<(), String> {
    if !version.is_empty()
        && version.len() < 32
        && version
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c.is_ascii_alphabetic())
    {
        Ok(())
    } else {
        Err("release version contains unsupported characters".into())
    }
}

fn install_version(version: &str) -> Result<(), String> {
    util::require_root()?;
    validate_version(version)?;

    if !util::command_exists("curl") && !util::command_exists("wget") {
        return Err("curl or wget is required for pgaegis self-update".into());
    }
    if !util::command_exists("sha256sum") {
        return Err("sha256sum is required for pgaegis self-update".into());
    }

    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let dir = format!("/tmp/pgaegis-update-{}", std::process::id());
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let installer = format!("{dir}/install.sh");
    let sums = format!("{dir}/SHA256SUMS");

    let result = (|| {
        download(&format!("{base}/install.sh"), &installer)?;
        download(&format!("{base}/SHA256SUMS"), &sums)?;
        verify_release_file(&installer, &sums, "install.sh")?;

        let status = Command::new("sh")
            .arg(&installer)
            .env("PGAEGIS_VERSION", version)
            .env("PGAEGIS_REPO", REPO)
            .status()
            .map_err(|e| format!("run verified installer: {e}"))?;

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
        util::output(
            "curl",
            &["-fsSL", "--retry", "3", "--proto", "=https", "--tlsv1.2", url],
        )
    } else if util::command_exists("wget") {
        util::output("wget", &["-qO-", url])
    } else {
        Err("curl or wget is required".into())
    }
}

fn download(url: &str, destination: &str) -> Result<(), String> {
    if util::command_exists("curl") {
        util::status(
            "curl",
            &[
                "-fL",
                "--retry",
                "3",
                "--proto",
                "=https",
                "--tlsv1.2",
                url,
                "-o",
                destination,
            ],
        )
    } else if util::command_exists("wget") {
        util::status("wget", &["-qO", destination, url])
    } else {
        Err("curl or wget is required".into())
    }
}

fn verify_release_file(file: &str, manifest_path: &str, name: &str) -> Result<(), String> {
    let manifest = fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
    let expected = manifest
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let filename = parts.next()?.trim_start_matches('*');
            (filename == name).then(|| hash.to_string())
        })
        .ok_or("release checksum manifest does not contain install.sh")?;

    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid SHA256 in release manifest".into());
    }

    let output = util::output("sha256sum", &[file])?;
    let actual = output.split_whitespace().next().unwrap_or_default();
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err("verified installer SHA256 mismatch".into());
    }

    Ok(())
}
