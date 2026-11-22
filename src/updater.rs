use crate::{ui,util};
use std::fs;
use std::process::Command;

const REPO:&str="mohammadanarul/pgaegis";

pub fn dispatch(args:&[String])->Result<(),String>{
    let check=args.iter().any(|x|x=="--check"); let force=args.iter().any(|x|x=="--force");
    let latest=latest_version()?; let current=env!("CARGO_PKG_VERSION");
    ui::card("pgaegis Update", &[("Installed",current.into()),("Latest",latest.clone())]);
    if check{return Ok(());}
    if latest==current && !force {ui::success("pgaegis is already up to date");return Ok(());}
    install_version(&latest)
}

fn latest_version()->Result<String,String>{
    let api=format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body=util::output("curl", &["-fsSL","-H","Accept: application/vnd.github+json",&api])?;
    let key="\"tag_name\""; let pos=body.find(key).ok_or("latest release response has no tag_name")?; let rest=&body[pos+key.len()..];
    let colon=rest.find(':').ok_or("invalid tag_name JSON")?; let rest=rest[colon+1..].trim_start();
    let rest=rest.strip_prefix('"').ok_or("invalid tag_name value")?; let end=rest.find('"').ok_or("invalid tag_name value")?;
    let v=rest[..end].trim_start_matches('v'); validate_version(v)?; Ok(v.to_string())
}
fn validate_version(v:&str)->Result<(),String>{if !v.is_empty()&&v.len()<32&&v.chars().all(|c|c.is_ascii_digit()||c=='.'||c=='-'||c.is_ascii_alphabetic()){Ok(())}else{Err("release version contains unsupported characters".into())}}
fn install_version(version:&str)->Result<(),String>{
    util::require_root()?; validate_version(version)?;
    let arch=match std::env::consts::ARCH{"x86_64"=>"amd64","aarch64"=>"arm64",x=>return Err(format!("unsupported release architecture: {x}"))};
    if !util::command_exists("dpkg-query"){return Err("self-update is production-certified only for Debian packages in this release".into());}
    let base=format!("https://github.com/{REPO}/releases/download/v{version}");
    let name=format!("pgaegis_{version}_{arch}.deb"); let dir=format!("/tmp/pgaegis-update-{}",std::process::id()); fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
    let deb=format!("{dir}/{name}"); let sums=format!("{dir}/SHA256SUMS");
    let result=(||{
        util::status("curl", &["-fL","--proto","=https","--tlsv1.2",&format!("{base}/{name}"),"-o",&deb])?;
        util::status("curl", &["-fL","--proto","=https","--tlsv1.2",&format!("{base}/SHA256SUMS"),"-o",&sums])?;
        let manifest=fs::read_to_string(&sums).map_err(|e|e.to_string())?;
        let expected=manifest.lines().find_map(|l|{let mut p=l.split_whitespace();let h=p.next()?;let n=p.next()?.trim_start_matches('*');if n==name{Some(h.to_string())}else{None}}).ok_or("release checksum manifest does not contain package")?;
        if expected.len()!=64||!expected.chars().all(|c|c.is_ascii_hexdigit()){return Err("invalid SHA256 in release manifest".into());}
        let actual=util::output("sha256sum", &[&deb])?.split_whitespace().next().unwrap_or("").to_string(); if !actual.eq_ignore_ascii_case(&expected){return Err("pgaegis package SHA256 mismatch".into());}
        let st=Command::new("apt-get").args(["install","-y",&deb]).status().map_err(|e|e.to_string())?; if !st.success(){return Err(format!("apt-get install failed with {st}"));}
        ui::success(&format!("pgaegis updated to {version}")); Ok(())
    })();
    let _=fs::remove_dir_all(&dir); result
}
