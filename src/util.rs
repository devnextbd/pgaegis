use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub fn command_exists(name: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {} >/dev/null 2>&1", shell_quote(name))]).status().map(|s|s.success()).unwrap_or(false)
}

pub fn shell_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

pub fn output(program: &str, args: &[&str]) -> Result<String,String> {
    output_with(program, args, &[], None)
}

/// Run a program capturing stdout. `envs` are added to the child environment
/// (use this instead of argv for secrets: /proc/<pid>/environ is owner-only,
/// argv is world-readable). `stdin` is written to the child if given.
pub fn output_with(program: &str, args: &[&str], envs: &[(&str,&str)], stdin: Option<&str>) -> Result<String,String> {
    let mut cmd=Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k,v) in envs { cmd.env(k,v); }
    cmd.stdin(if stdin.is_some(){Stdio::piped()}else{Stdio::null()});
    let mut child=cmd.spawn().map_err(|e|format!("{program}: {e}"))?;
    if let Some(data)=stdin {
        let mut si=child.stdin.take().ok_or("stdin unavailable")?;
        si.write_all(data.as_bytes()).map_err(|e|format!("{program}: write stdin: {e}"))?;
    }
    let out=child.wait_with_output().map_err(|e|format!("{program}: {e}"))?;
    if !out.status.success() { return Err(format!("{} {} failed: {}", program, args.join(" "), String::from_utf8_lossy(&out.stderr).trim())); }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn status(program: &str, args: &[&str]) -> Result<(),String> {
    let s=Command::new(program).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().map_err(|e|format!("{program}: {e}"))?;
    if !s.success() { return Err(format!("{} {} exited with {}",program,args.join(" "),s)); }
    Ok(())
}

/// Like `status`, but feeds `stdin` to the child and adds `envs`.
pub fn status_with(program: &str, args: &[&str], envs: &[(&str,&str)], stdin: Option<&str>) -> Result<(),String> {
    let mut cmd=Command::new(program);
    cmd.args(args).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    for (k,v) in envs { cmd.env(k,v); }
    cmd.stdin(if stdin.is_some(){Stdio::piped()}else{Stdio::inherit()});
    let mut child=cmd.spawn().map_err(|e|format!("{program}: {e}"))?;
    if let Some(data)=stdin {
        let mut si=child.stdin.take().ok_or("stdin unavailable")?;
        si.write_all(data.as_bytes()).map_err(|e|format!("{program}: write stdin: {e}"))?;
    }
    let s=child.wait().map_err(|e|format!("{program}: {e}"))?;
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

/// Value following a `--flag` in an argument list.
pub fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}

pub fn has_flag(a:&[String],k:&str)->bool{a.iter().any(|x|x==k)}

/// Installed asset first (/usr/share/pgaegis/<kind>/<name>), then the source tree.
pub fn asset_path(kind:&str,name:&str)->Result<String,String>{
    for base in [format!("/usr/share/pgaegis/{kind}"),kind.to_string()] {
        let p=format!("{base}/{name}");
        if Path::new(&p).exists(){return Ok(p);}
    }
    Err(format!("pgaegis {kind} asset not found: {name}"))
}

/// systemd unit is enabled (and, if `require_active`, currently active).
pub fn systemd_unit_ok(unit:&str,require_active:bool)->bool{
    if !command_exists("systemctl"){return false;}
    let q=|verb:&str|Command::new("systemctl").args([verb,"--quiet",unit]).status().map(|s|s.success()).unwrap_or(false);
    q("is-enabled") && (!require_active || q("is-active"))
}

pub fn systemd_active(unit:&str)->bool{
    command_exists("systemctl") && Command::new("systemctl").args(["is-active","--quiet",unit]).status().map(|s|s.success()).unwrap_or(false)
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

pub fn validate_host(label:&str,value:&str)->Result<(),String>{
    if value.parse::<std::net::IpAddr>().is_ok(){return Ok(());}
    if value.is_empty() || value.len()>253 || !value.chars().all(|c|c.is_ascii_alphanumeric()||matches!(c,'.'|'-')) || value.starts_with('-') {
        return Err(format!("{label} must be an IP address or DNS name"));
    }
    Ok(())
}

pub fn validate_cidr(label:&str,value:&str)->Result<(),String>{
    if matches!(value,"0.0.0.0/0"|"::/0") { return Err(format!("{label} may not allow the entire Internet")); }
    let (ip,prefix)=value.split_once('/').ok_or_else(||format!("{label} must be CIDR notation"))?;
    let parsed=ip.parse::<std::net::IpAddr>().map_err(|_|format!("{label} contains an invalid IP"))?;
    let bits=match parsed {std::net::IpAddr::V4(_)=>32u8,std::net::IpAddr::V6(_)=>128u8};
    let p=prefix.parse::<u8>().map_err(|_|format!("{label} contains an invalid prefix"))?;
    if p>bits {return Err(format!("{label} prefix exceeds {bits}"));}
    if p==0 {return Err(format!("{label} may not allow the entire Internet"));}
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
    let p=Path::new(value);
    if !p.is_absolute(){return Err(format!("{label} must be an absolute path"));}
    if p.components().any(|c|matches!(c,Component::ParentDir)){return Err(format!("{label} may not contain '..'"));}
    Ok(())
}

/// Absolute path that must also be safe to embed in whitespace-separated
/// config files such as /etc/fstab.
pub fn validate_plain_path(label:&str,value:&str)->Result<(),String>{
    validate_abs_path(label,value)?;
    if value.chars().any(char::is_whitespace){return Err(format!("{label} may not contain whitespace"));}
    Ok(())
}

pub fn validate_port(label:&str,value:&str)->Result<u16,String>{
    match value.parse::<u16>() { Ok(p) if p>0 => Ok(p), _ => Err(format!("{label} must be a TCP port number (1-65535)")) }
}

pub fn yaml_single_quote(value:&str)->String{
    format!("'{}'",value.replace('\'',"''"))
}

pub fn reject_control_chars(label:&str,value:&str)->Result<(),String>{
    if value.chars().any(|c|c.is_control()) {return Err(format!("{label} contains control characters"));}
    Ok(())
}

fn read_stdin_line()->Result<String,String>{
    let mut s=String::new();
    let n=io::stdin().read_line(&mut s).map_err(|e|e.to_string())?;
    if n==0 { return Err("input closed (EOF); run pgaegis with an interactive terminal or use the CLI subcommands".into()); }
    Ok(s)
}

pub fn prompt_secret(label:&str)->Result<String,String>{
    print!("{label}: "); io::stdout().flush().map_err(|e|e.to_string())?;
    let _=Command::new("stty").arg("-echo").status();
    let r=read_stdin_line();
    let _=Command::new("stty").arg("echo").status(); println!();
    let v=r?.trim_end_matches(['\r','\n']).to_string();
    if v.len()<12{return Err(format!("{label} must be at least 12 characters"));}
    if v.trim()!=v{return Err(format!("{label} may not start or end with whitespace"));}
    reject_control_chars(label,&v)?;
    Ok(v)
}

pub fn prompt_secret_confirm(label:&str)->Result<String,String>{
    let first=prompt_secret(label)?;
    let second=prompt_secret(&format!("Confirm {label}"))?;
    if first!=second { return Err(format!("{label} confirmation does not match")); }
    Ok(first)
}

pub fn backup_file(path:&str)->Result<Option<String>,String>{
    let p=Path::new(path);
    if !p.exists(){return Ok(None);}
    let backup=format!("{path}.bak-{}",unix_now());
    fs::copy(path,&backup).map_err(|e|format!("backup {path}: {e}"))?;
    Ok(Some(backup))
}

pub fn random_bytes(n:usize)->Result<Vec<u8>,String>{
    let mut f=fs::File::open("/dev/urandom").map_err(|e|format!("/dev/urandom: {e}"))?;
    let mut b=vec![0u8;n]; f.read_exact(&mut b).map_err(|e|format!("/dev/urandom: {e}"))?;
    Ok(b)
}

fn random_suffix()->Result<String,String>{
    Ok(random_bytes(8)?.iter().map(|b|format!("{b:02x}")).collect())
}

/// Atomically replace `path`: the temp file is created exclusively with the
/// final mode (never briefly world-readable), fsynced, then renamed.
pub fn atomic_write(path:&str,content:&str,mode:u32)->Result<(),String>{
    let target=Path::new(path);
    let parent=target.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e|format!("create {}: {e}",parent.display()))?;
    let name=target.file_name().and_then(|n|n.to_str()).ok_or("invalid file name")?;
    let tmp=parent.join(format!(".{name}.tmp-{}",random_suffix()?));
    let res=(||{
        let mut f=fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp).map_err(|e|format!("create {}: {e}",tmp.display()))?;
        f.set_permissions(fs::Permissions::from_mode(mode)).map_err(|e|e.to_string())?;
        f.write_all(content.as_bytes()).map_err(|e|e.to_string())?;
        f.sync_all().map_err(|e|e.to_string())?;
        fs::rename(&tmp,target).map_err(|e|format!("rename to {path}: {e}"))?;
        if let Ok(d)=fs::File::open(parent){let _=d.sync_all();}
        Ok(())
    })();
    if res.is_err(){let _=fs::remove_file(&tmp);}
    res
}

pub fn write_secret_file(path:&str,content:&str)->Result<(),String>{ atomic_write(path,content,0o600) }

pub fn upsert_kv_file(path:&str,pairs:&[(&str,&str)])->Result<(),String>{
    for (k,v) in pairs { reject_control_chars(k,k)?; reject_control_chars(k,v)?; }
    let existing=fs::read_to_string(path).unwrap_or_default();
    let mut lines:Vec<String>=existing.lines().map(|x|x.to_string()).collect();
    for (key,val) in pairs {
        let prefix=format!("{key}="); let mut found=false;
        for line in &mut lines { if line.trim_start().starts_with(&prefix){*line=format!("{key}={val}"); found=true; break;} }
        if !found { lines.push(format!("{key}={val}")); }
    }
    atomic_write(path,&format!("{}\n",lines.join("\n")),0o640)
}

/// Private (0700) directory created exclusively; fails if the name exists,
/// so a pre-planted directory or symlink in /tmp can never be reused.
pub fn secure_temp_dir(prefix:&str)->Result<PathBuf,String>{
    let base=std::env::temp_dir();
    for _ in 0..8 {
        let p=base.join(format!("{prefix}-{}",random_suffix()?));
        match fs::DirBuilder::new().mode(0o700).create(&p) {
            Ok(())=>{
                let md=fs::symlink_metadata(&p).map_err(|e|e.to_string())?;
                if !md.is_dir() || md.permissions().mode() & 0o077 != 0 { return Err(format!("unsafe temp dir {}",p.display())); }
                return Ok(p);
            }
            Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>continue,
            Err(e)=>return Err(format!("create temp dir: {e}")),
        }
    }
    Err("could not create a unique temp dir".into())
}

/// Split a libpq connection string (URI or key=value form) into the string
/// without its password and the password itself, so the password can be
/// passed via PGPASSWORD (environment) instead of world-readable argv.
pub fn split_dsn_password(dsn:&str)->(String,Option<String>){
    let t=dsn.trim();
    if let Some(rest)=t.strip_prefix("postgresql://").map(|r|("postgresql://",r)).or_else(||t.strip_prefix("postgres://").map(|r|("postgres://",r))) {
        let (scheme,body)=rest;
        let auth_end=body.find(['/','?']).unwrap_or(body.len());
        let (authority,tail)=body.split_at(auth_end);
        if let Some(at)=authority.rfind('@') {
            let (userinfo,hosts)=authority.split_at(at);
            if let Some((user,pw))=userinfo.split_once(':') {
                return (format!("{scheme}{user}{hosts}{tail}"),Some(percent_decode(pw)));
            }
        }
        return (t.to_string(),None);
    }
    // key=value form; values may be single-quoted with backslash escapes.
    let b=t.as_bytes(); let mut i=0; let mut kept:Vec<String>=Vec::new(); let mut pw=None;
    while i<b.len() {
        while i<b.len() && b[i].is_ascii_whitespace(){i+=1;}
        if i>=b.len(){break;}
        let ks=i; while i<b.len() && b[i]!=b'=' && !b[i].is_ascii_whitespace(){i+=1;}
        let key=&t[ks..i];
        while i<b.len() && b[i].is_ascii_whitespace(){i+=1;}
        if i<b.len() && b[i]==b'=' {i+=1;}
        while i<b.len() && b[i].is_ascii_whitespace(){i+=1;}
        let vs=i; let mut val:Vec<u8>=Vec::new();
        if i<b.len() && b[i]==b'\'' {
            i+=1;
            while i<b.len() && b[i]!=b'\'' { if b[i]==b'\\' && i+1<b.len(){i+=1;} val.push(b[i]); i+=1; }
            i+=1;
        } else {
            while i<b.len() && !b[i].is_ascii_whitespace(){ if b[i]==b'\\' && i+1<b.len(){i+=1;} val.push(b[i]); i+=1; }
        }
        let raw=&t[vs..i.min(t.len())];
        if key=="password" { pw=Some(String::from_utf8_lossy(&val).into_owned()); } else if !key.is_empty() { kept.push(format!("{key}={raw}")); }
    }
    (kept.join(" "),pw)
}

fn percent_decode(s:&str)->String{
    let b=s.as_bytes(); let mut out=Vec::with_capacity(b.len()); let mut i=0;
    while i<b.len() {
        if b[i]==b'%' && i+2<b.len() {
            if let Some(v)=std::str::from_utf8(&b[i+1..i+3]).ok().and_then(|h|u8::from_str_radix(h,16).ok()){out.push(v); i+=3; continue;}
        }
        out.push(b[i]); i+=1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Quote a value for a curl `--config` file (double quotes, backslash escapes).
pub fn curl_config_quote(v:&str)->String{
    format!("\"{}\"",v.replace('\\',"\\\\").replace('"',"\\\"").replace('\n',"\\n").replace('\r',"\\r").replace('\t',"\\t"))
}

/// Single-pass `${NAME}` substitution: values are never re-scanned, so a
/// secret that happens to contain `${...}` cannot pull in another value.
pub fn render_template(t:&str,vars:&[(&str,&str)])->Result<String,String>{
    let mut out=String::with_capacity(t.len()); let mut rest=t;
    while let Some(i)=rest.find("${") {
        out.push_str(&rest[..i]);
        let after=&rest[i+2..];
        let end=after.find('}').ok_or("unterminated ${ in template")?;
        let name=&after[..end];
        let v=vars.iter().find(|(k,_)|*k==name).map(|(_,v)|*v).ok_or_else(||format!("unresolved template variable ${{{name}}}"))?;
        out.push_str(v); rest=&after[end+1..];
    }
    out.push_str(rest); Ok(out)
}

pub fn prompt_line(label:&str,default:Option<&str>)->Result<String,String>{
    match default { Some(d)=>print!("{label} [{d}]: "), None=>print!("{label}: ") }
    io::stdout().flush().map_err(|e|e.to_string())?;
    let s=read_stdin_line()?;
    let v=s.trim();
    if v.is_empty(){ if let Some(d)=default{return Ok(d.to_string());} }
    if v.is_empty(){return Err(format!("{label} is required"));}
    Ok(v.to_string())
}

pub fn prompt_yes_no(label:&str,default_yes:bool)->Result<bool,String>{
    let hint=if default_yes{"Y/n"}else{"y/N"};
    loop {
        print!("{label} [{hint}]: "); io::stdout().flush().map_err(|e|e.to_string())?;
        let s=read_stdin_line()?;
        match s.trim().to_ascii_lowercase().as_str(){
            ""=>return Ok(default_yes), "y"|"yes"=>return Ok(true), "n"|"no"=>return Ok(false), _=>println!("Please answer yes or no.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn cidr_rules() {
        assert!(validate_cidr("x","10.0.0.0/8").is_ok());
        assert!(validate_cidr("x","fd00::/64").is_ok());
        for bad in ["0.0.0.0/0","::/0","10.0.0.0/0","10.0.0.0/33","10.0.0.0","nope/8"] { assert!(validate_cidr("x",bad).is_err(),"{bad}"); }
    }

    #[test]
    fn host_port_lists() {
        assert!(validate_host_port_list("x","https://10.0.0.1:2379,etcd2:2379,[fd00::1]:2379").is_ok());
        for bad in ["10.0.0.1","a:0","a b:1","a:99999","'x':1"] { assert!(validate_host_port_list("x",bad).is_err(),"{bad}"); }
    }

    #[test]
    fn paths() {
        assert!(validate_abs_path("x","/var/lib/x").is_ok());
        assert!(validate_abs_path("x","relative").is_err());
        assert!(validate_abs_path("x","/var/lib/pgaegis/restore-drill/../../etc").is_err());
        assert!(validate_plain_path("x","/swap file").is_err());
    }

    #[test]
    fn dsn_password_is_split_out() {
        assert_eq!(split_dsn_password("postgresql://mon:s%40cret@writer:6432/postgres?sslmode=require"),
            ("postgresql://mon@writer:6432/postgres?sslmode=require".to_string(),Some("s@cret".to_string())));
        assert_eq!(split_dsn_password("postgresql://mon@writer/postgres"),("postgresql://mon@writer/postgres".to_string(),None));
        assert_eq!(split_dsn_password("host=w port=6432 password='a b\\'c' user=mon"),
            ("host=w port=6432 user=mon".to_string(),Some("a b'c".to_string())));
        assert_eq!(split_dsn_password("host=w password=xyz"),("host=w".to_string(),Some("xyz".to_string())));
    }

    #[test]
    fn atomic_write_sets_mode_and_temp_dir_is_private() {
        let d=secure_temp_dir("pgaegis-test").unwrap();
        assert_eq!(fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        let f=d.join("secret.conf"); let fs_=f.to_str().unwrap();
        atomic_write(fs_,"a=1\n",0o600).unwrap();
        assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        upsert_kv_file(fs_,&[("a","2"),("b","3")]).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(),"a=2\nb=3\n");
        assert!(upsert_kv_file(fs_,&[("a","x\nINJECT=1")]).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn template_rendering_is_single_pass() {
        assert_eq!(render_template("a=${A} b=${B}",&[("A","${B}"),("B","x")]).unwrap(),"a=${B} b=x");
        assert!(render_template("${MISSING}",&[]).is_err());
    }

    #[test]
    fn curl_quote() {
        assert_eq!(curl_config_quote("https://h/a\"b\\c"), "\"https://h/a\\\"b\\\\c\"");
    }
}
