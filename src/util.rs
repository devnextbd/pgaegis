use std::fs;
use std::process::{Command, Stdio};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;

pub fn command_exists(name: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {} >/dev/null 2>&1", shell_quote(name))]).status().map(|s|s.success()).unwrap_or(false)
}

pub fn shell_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

pub fn output(program: &str, args: &[&str]) -> Result<String,String> {
    let out=Command::new(program).args(args).output().map_err(|e|format!("{program}: {e}"))?;
    if !out.status.success() { return Err(format!("{} {} failed: {}", program, args.join(" "), String::from_utf8_lossy(&out.stderr).trim())); }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn status(program: &str, args: &[&str]) -> Result<(),String> {
    let s=Command::new(program).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().map_err(|e|format!("{program}: {e}"))?;
    if !s.success() { return Err(format!("{} {} exited with {}",program,args.join(" "),s)); }
    Ok(())
}

pub fn command_version(program: &str, args: &[&str]) -> String {
    if !command_exists(program) { return "not installed".into(); }
    output(program,args).unwrap_or_else(|_|"installed (version unavailable)".into()).lines().next().unwrap_or("").to_string()
}

pub fn os_pretty_name() -> String {
    let Ok(s)=fs::read_to_string("/etc/os-release") else { return std::env::consts::OS.into(); };
    for l in s.lines() { if let Some(v)=l.strip_prefix("PRETTY_NAME=") { return v.trim_matches('"').into(); } }
    std::env::consts::OS.into()
}

pub fn read(path:&str)->String { fs::read_to_string(path).unwrap_or_default() }

pub fn contains_all(path:&str, needles:&[&str])->bool {
    let s=read(path); needles.iter().all(|n|s.contains(n))
}

pub fn unix_now()->u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }

pub fn require_root()->Result<(),String> {
    let uid=output("id", &["-u"])?;
    if uid.trim()!="0" { return Err("this operation requires root".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::shell_quote;
    #[test]
    fn quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}



pub fn validate_safe_name(label:&str,value:&str)->Result<(),String>{
    if value.is_empty() || value.len()>128 || !value.chars().all(|c|c.is_ascii_alphanumeric() || matches!(c,'-'|'_'|'.')) {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(())
}

pub fn validate_ip(label:&str,value:&str)->Result<(),String>{
    value.parse::<std::net::IpAddr>().map(|_|()).map_err(|_|format!("{label} must be a valid IP address"))
}

pub fn validate_cidr(label:&str,value:&str)->Result<(),String>{
    if matches!(value,"0.0.0.0/0"|"::/0") { return Err(format!("{label} may not allow the entire Internet")); }
    let (ip,prefix)=value.split_once('/').ok_or_else(||format!("{label} must be CIDR notation"))?;
    let parsed=ip.parse::<std::net::IpAddr>().map_err(|_|format!("{label} contains an invalid IP"))?;
    let bits=match parsed {std::net::IpAddr::V4(_)=>32u8,std::net::IpAddr::V6(_)=>128u8};
    let p=prefix.parse::<u8>().map_err(|_|format!("{label} contains an invalid prefix"))?;
    if p>bits {return Err(format!("{label} prefix exceeds {bits}"));}
    Ok(())
}


pub fn validate_host_port_list(label:&str,value:&str)->Result<(),String>{
    for item in value.split(',') {
        let ep=item.trim().trim_start_matches("https://").trim_start_matches("http://");
        if ep.is_empty() || ep.chars().any(|c|c.is_whitespace() || matches!(c,'#'|'\''|'"'|'{'|'}')) {
            return Err(format!("{label} contains an invalid endpoint"));
        }
        let (host,port)=if let Some(rest)=ep.strip_prefix('[') {
            let Some((host,port))=rest.split_once("]:") else{return Err(format!("{label} invalid IPv6 endpoint: {ep}"));};
            (host,port)
        } else {
            let Some((host,port))=ep.rsplit_once(':') else{return Err(format!("{label} endpoint must include port: {ep}"));};
            (host,port)
        };
        if host.is_empty() || !host.chars().all(|c|c.is_ascii_alphanumeric() || matches!(c,'.'|'-'|'_'|':')) {return Err(format!("{label} invalid host: {host}"));}
        let p=port.parse::<u16>().map_err(|_|format!("{label} invalid port: {port}"))?;
        if p==0{return Err(format!("{label} port may not be zero"));}
    }
    Ok(())
}

pub fn validate_abs_path(label:&str,value:&str)->Result<(),String>{
    reject_control_chars(label,value)?;
    if !std::path::Path::new(value).is_absolute(){return Err(format!("{label} must be an absolute path"));}
    Ok(())
}

pub fn yaml_single_quote(value:&str)->String{
    format!("'{}'",value.replace('\'',"''"))
}

pub fn reject_control_chars(label:&str,value:&str)->Result<(),String>{
    if value.chars().any(|c|c.is_control()) {return Err(format!("{label} contains control characters"));}
    Ok(())
}

pub fn prompt_secret(label:&str)->Result<String,String>{
    print!("{label}: "); io::stdout().flush().map_err(|e|e.to_string())?;
    let _=Command::new("stty").arg("-echo").status();
    let mut s=String::new(); let r=io::stdin().read_line(&mut s);
    let _=Command::new("stty").arg("echo").status(); println!();
    r.map_err(|e|e.to_string())?; let v=s.trim().to_string();
    if v.len()<12{return Err(format!("{label} must be at least 12 characters"));}
    Ok(v)
}


pub fn prompt_secret_confirm(label:&str)->Result<String,String>{
    let first=prompt_secret(label)?;
    let second=prompt_secret(&format!("Confirm {label}"))?;
    if first!=second { return Err(format!("{label} confirmation does not match")); }
    Ok(first)
}

pub fn backup_file(path:&str)->Result<Option<String>,String>{
    let p=std::path::Path::new(path);
    if !p.exists(){return Ok(None);}
    let backup=format!("{path}.bak-{}",unix_now());
    fs::copy(path,&backup).map_err(|e|format!("backup {path}: {e}"))?;
    Ok(Some(backup))
}

pub fn write_secret_file(path:&str,content:&str)->Result<(),String>{
    if let Some(parent)=std::path::Path::new(path).parent(){fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    let tmp=format!("{path}.tmp-{}",std::process::id());
    fs::write(&tmp,content).map_err(|e|e.to_string())?;
    fs::set_permissions(&tmp,fs::Permissions::from_mode(0o600)).map_err(|e|e.to_string())?;
    fs::rename(&tmp,path).map_err(|e|e.to_string())?;
    Ok(())
}


pub fn upsert_kv_file(path:&str,pairs:&[(&str,&str)])->Result<(),String>{
    let existing=fs::read_to_string(path).unwrap_or_default();
    let mut lines:Vec<String>=existing.lines().map(|x|x.to_string()).collect();
    for (key,val) in pairs {
        let prefix=format!("{key}="); let mut found=false;
        for line in &mut lines { if line.trim_start().starts_with(&prefix){*line=format!("{key}={val}"); found=true; break;} }
        if !found { lines.push(format!("{key}={val}")); }
    }
    if let Some(parent)=std::path::Path::new(path).parent(){fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    fs::write(path,format!("{}\n",lines.join("\n"))).map_err(|e|e.to_string())?;
    fs::set_permissions(path,fs::Permissions::from_mode(0o640)).map_err(|e|e.to_string())?;
    Ok(())
}


pub fn prompt_line(label:&str,default:Option<&str>)->Result<String,String>{
    match default { Some(d)=>print!("{label} [{d}]: "), None=>print!("{label}: ") }
    io::stdout().flush().map_err(|e|e.to_string())?;
    let mut s=String::new(); io::stdin().read_line(&mut s).map_err(|e|e.to_string())?;
    let v=s.trim();
    if v.is_empty(){ if let Some(d)=default{return Ok(d.to_string());} }
    if v.is_empty(){return Err(format!("{label} is required"));}
    Ok(v.to_string())
}

pub fn prompt_yes_no(label:&str,default_yes:bool)->Result<bool,String>{
    let hint=if default_yes{"Y/n"}else{"y/N"};
    loop {
        print!("{label} [{hint}]: "); io::stdout().flush().map_err(|e|e.to_string())?;
        let mut s=String::new(); io::stdin().read_line(&mut s).map_err(|e|e.to_string())?;
        match s.trim().to_ascii_lowercase().as_str(){
            ""=>return Ok(default_yes), "y"|"yes"=>return Ok(true), "n"|"no"=>return Ok(false), _=>println!("Please answer yes or no.")
        }
    }
}
