use crate::{ui,util};
use crate::util::arg;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("install") => install(args),
        Some("enable-monitoring") => {util::require_root()?; util::status("systemctl", &["enable","--now","pgaegis-monitor.timer"])},
        Some("resources") => resources(),
        Some("swap") => swap(args),
        _ => Err("usage: pgaegis system <install postgres18|install patroni|install etcd --sha256 HASH [--version V]|enable-monitoring|resources|swap status|swap setup --size-gb N [--file /swapfile] [--allow-additional]>".into())
    }
}

fn install(args:&[String])->Result<(),String>{
    util::require_root()?;
    let component=args.get(1).map(String::as_str).ok_or("component required")?;
    match component {
        "postgres18" => run_helper("install-postgresql18-debian13.sh", &[]),
        "patroni" => {
            let version=arg(args,"--version").unwrap_or("4.1.5"); util::validate_safe_name("--version",version)?;
            run_helper_env("install-patroni-debian13.sh", &[("PATRONI_VERSION",version)], &[])
        },
        "etcd" => {
            let version=arg(args,"--version").unwrap_or("3.7.4"); util::validate_safe_name("--version",version)?; let sha=arg(args,"--sha256").ok_or("--sha256 from the official etcd release is required")?;
            if sha.len()!=64 || !sha.chars().all(|c|c.is_ascii_hexdigit()){return Err("--sha256 must be a 64-character SHA-256 hex digest".into());}
            run_helper_env("install-etcd-debian13.sh", &[("ETCD_VERSION",version),("ETCD_SHA256",sha)], &[])
        },
        _ => Err(format!("unsupported component: {component}"))
    }
}

fn resources()->Result<(),String>{
    let mem=fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field=|key:&str|->u64{mem.lines().find(|l|l.starts_with(key)).and_then(|l|l.split_whitespace().nth(1)).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0)};
    let total_kib=field("MemTotal:"); let avail_kib=field("MemAvailable:"); let swap_kib=field("SwapTotal:");
    let cpus=util::output("nproc",&[]).unwrap_or_else(|_|"unknown".into());
    let root=util::output("df", &["-hP","/"]).unwrap_or_else(|_|"unavailable".into());
    let root_line=root.lines().last().unwrap_or("unavailable").to_string();
    ui::card("System Resources", &[("CPU threads",cpus.trim().to_string()),("RAM total",format!("{:.1} GiB",total_kib as f64/1048576.0)),("RAM available",format!("{:.1} GiB",avail_kib as f64/1048576.0)),("Swap total",format!("{:.1} GiB",swap_kib as f64/1048576.0)),("Root filesystem",root_line)]);
    if swap_kib>0 { ui::note("PostgreSQL should not depend on swap for normal capacity; sustained swap activity indicates memory pressure."); }
    Ok(())
}

fn swap(args:&[String])->Result<(),String>{
    match args.get(1).map(String::as_str){
        Some("status")=>swap_status(),
        Some("setup")=>swap_setup(args),
        _=>Err("usage: pgaegis system swap <status|setup --size-gb N [--file /swapfile] [--allow-additional]>".into())
    }
}

fn swap_status()->Result<(),String>{
    let s=fs::read_to_string("/proc/swaps").unwrap_or_default();
    if s.lines().count()<=1 { ui::card("Swap", &[("Status",ui::badge_warn("Not configured"))]); }
    else { let rows=s.lines().skip(1).map(|l|vec![l.to_string()]).collect::<Vec<_>>(); ui::table("Swap Devices / Files", &["/proc/swaps"], &rows); }
    Ok(())
}

fn swap_setup(args:&[String])->Result<(),String>{
    util::require_root()?;
    let gb=arg(args,"--size-gb").ok_or("--size-gb is required")?.parse::<u64>().map_err(|_|"--size-gb must be an integer")?;
    if !(1..=64).contains(&gb){return Err("--size-gb must be between 1 and 64 GiB".into());}
    let file=arg(args,"--file").unwrap_or("/swapfile"); util::validate_plain_path("--file",file)?;
    if fs::read_to_string("/proc/swaps").unwrap_or_default().lines().count()>1 && !args.iter().any(|x|x=="--allow-additional"){
        return Err("swap already exists; refusing to add another swap area without --allow-additional".into());
    }
    if Path::new(file).exists(){return Err(format!("{file} already exists; refusing overwrite"));}
    let bytes=gb.checked_mul(1024*1024*1024).ok_or("swap size overflow")?;
    let fallocate=Command::new("fallocate").args(["-l",&bytes.to_string(),file]).status().map(|s|s.success()).unwrap_or(false);
    if !fallocate {
        util::status("dd", &["if=/dev/zero",&format!("of={file}"),"bs=1M",&format!("count={}",gb*1024),"status=progress"])?;
    }
    fs::set_permissions(file,fs::Permissions::from_mode(0o600)).map_err(|e|format!("chmod swap: {e}"))?;
    util::status("mkswap", &[file])?; util::status("swapon", &[file])?;
    let fstab=fs::read_to_string("/etc/fstab").unwrap_or_default();
    let entry=format!("{file} none swap sw 0 0");
    if !fstab.lines().any(|l|l.split_whitespace().next()==Some(file)){
        let backup=util::backup_file("/etc/fstab")?; if let Some(b)=backup{ui::note(&format!("Backed up /etc/fstab to {b}"));}
        let mut next=fstab; if !next.ends_with('\n'){next.push('\n');} next.push_str(&entry); next.push('\n');
        util::atomic_write("/etc/fstab",&next,0o644).map_err(|e|format!("write /etc/fstab: {e}"))?;
    }
    ui::success(&format!("Configured {gb} GiB swap at {file}")); ui::note("No PostgreSQL memory setting was changed automatically.");
    Ok(())
}

fn helper_path(name:&str)->Result<String,String>{ util::asset_path("scripts",name) }
fn run_helper(name:&str,args:&[&str])->Result<(),String>{
    let p=helper_path(name)?; let mut c=Command::new("bash"); c.arg(&p).args(args);
    let st=c.status().map_err(|e|e.to_string())?; if st.success(){Ok(())}else{Err(format!("{name} failed with {st}"))}
}
fn run_helper_env(name:&str,envs:&[(&str,&str)],args:&[&str])->Result<(),String>{
    let p=helper_path(name)?; let mut c=Command::new("bash"); c.arg(&p).args(args); for &(k,v) in envs{c.env(k,v);} let st=c.status().map_err(|e|e.to_string())?; if st.success(){Ok(())}else{Err(format!("{name} failed with {st}"))}
}
