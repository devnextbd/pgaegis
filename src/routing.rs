use crate::{config::Config,ui,util};
use crate::util::arg;
use std::collections::HashSet;
use std::fs;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("render") => render(args),
        _ => Err("usage: pgaegis routing <check|render --nodes pg1=IP,pg2=IP,pg3=IP [--listen-addr IP]>".into())
    }
}

/// Run psql against MONITOR_DSN with the password moved into PGPASSWORD
/// (owner-only /proc/<pid>/environ) instead of world-readable argv.
pub fn psql_dsn(dsn:&str,sql:&str)->Result<String,String>{
    let (conn,pw)=util::split_dsn_password(dsn);
    let envs:Vec<(&str,&str)>=pw.as_deref().map(|p|vec![("PGPASSWORD",p)]).unwrap_or_default();
    util::output_with("psql",&["-X","-At","-v","ON_ERROR_STOP=1","-d",&conn,"-f","-"],&envs,Some(sql))
}

pub fn check()->Result<(),String>{
    let cfg=Config::load();
    util::output("pg_isready", &["-h",&cfg.writer_host,"-p",&cfg.writer_port.to_string()])?;
    if cfg.monitor_dsn.is_empty(){return Err("MONITOR_DSN must point through the PgBouncer writer endpoint for certification".into());}
    let writable=psql_dsn(&cfg.monitor_dsn,"SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';")?;
    if writable.trim()!="t" {return Err("writer endpoint is reachable but is not writable primary".into());}
    if util::command_exists("systemctl") {
        util::status("systemctl", &["is-active","--quiet","haproxy"])?;
        util::status("systemctl", &["is-active","--quiet","pgbouncer"])?;
    }
    ui::card("Application Routing", &[("Writer endpoint",format!("{}:{}",cfg.writer_host,cfg.writer_port)),("Writable Primary",ui::badge_ok("Verified")),("HAProxy",ui::badge_ok("Running")),("PgBouncer",ui::badge_ok("Running"))]); Ok(())
}

/// Parse and validate `--nodes name=host,...`.
pub fn parse_nodes(nodes:&str)->Result<Vec<(String,String)>,String>{
    let mut out=Vec::new(); let mut seen=HashSet::new();
    for item in nodes.split(',').map(str::trim).filter(|x|!x.is_empty()) {
        let (name,host)=item.split_once('=').ok_or_else(||format!("invalid --nodes entry (expected name=host): {item}"))?;
        util::validate_safe_name("--nodes member name",name)?;
        util::validate_host("--nodes host",host)?;
        if !seen.insert(name.to_string()){return Err(format!("duplicate --nodes member: {name}"));}
        out.push((name.to_string(),host.to_string()));
    }
    if out.is_empty(){return Err("--nodes must list at least one member".into());}
    Ok(out)
}

pub fn render_haproxy(template:&str,nodes:&[(String,String)],ca:&str,client:&str,listen:Option<&str>)->Result<String,String>{
    let servers=nodes.iter().map(|(n,h)|{
        let addr=if h.contains(':'){format!("[{h}]")}else{h.clone()};
        format!("    server {n} {addr}:5432 check port 8008 check-ssl verify required ca-file {ca} crt {client} inter 2s fall 3 rise 2 on-marked-down shutdown-sessions")
    }).collect::<Vec<_>>().join("\n");
    let reader_bind=match listen { Some(ip) if ip!="127.0.0.1" => format!("    bind {}:5001",if ip.contains(':'){format!("[{ip}]")}else{ip.to_string()}), _ => String::new() };
    util::render_template(template,&[("SERVERS",&servers),("READER_BIND",&reader_bind)])
}

fn install_config(path:&str,content:&str,mode:u32)->Result<(),String>{
    if let Some(b)=util::backup_file(path)? { ui::note(&format!("Backed up {path} to {b}")); }
    util::atomic_write(path,content,mode)
}

fn render(args:&[String])->Result<(),String>{
    util::require_root()?;
    let nodes=parse_nodes(arg(args,"--nodes").ok_or("--nodes pg1=10.0.0.11,pg2=10.0.0.12,pg3=10.0.0.13 required")?)?;
    let patroni_ca=arg(args,"--patroni-ca").unwrap_or("/etc/haproxy/tls/patroni-ca.crt");
    let patroni_client=arg(args,"--patroni-client-pem").unwrap_or("/etc/haproxy/tls/patroni-client.pem");
    util::validate_plain_path("--patroni-ca",patroni_ca)?; util::validate_plain_path("--patroni-client-pem",patroni_client)?;
    let listen=arg(args,"--listen-addr");
    if let Some(ip)=listen { util::validate_ip("--listen-addr",ip)?; if matches!(ip,"0.0.0.0"|"::"){return Err("--listen-addr must be a specific internal interface address, not a wildcard".into());} }
    if !std::path::Path::new(patroni_ca).exists() || !std::path::Path::new(patroni_client).exists(){return Err("Patroni mTLS CA/client PEM missing for HAProxy health checks".into());}

    let hap_t=fs::read_to_string(util::asset_path("templates","haproxy.production.cfg")?).map_err(|e|format!("haproxy template: {e}"))?;
    let pgb_t=fs::read_to_string(util::asset_path("templates","pgbouncer.production.ini")?).map_err(|e|format!("pgbouncer template: {e}"))?;
    let hap=render_haproxy(&hap_t,&nodes,patroni_ca,patroni_client,listen)?;
    let listen_addr=match listen { Some(ip) if ip!="127.0.0.1" => format!("127.0.0.1,{ip}"), _ => "127.0.0.1".into() };
    let pgb=util::render_template(&pgb_t,&[("LISTEN_ADDR",&listen_addr)])?;

    // Validate the candidate HAProxy config before touching the live file.
    let tmp=util::secure_temp_dir("pgaegis-routing")?;
    let cand=tmp.join("haproxy.cfg"); let cand_s=cand.to_str().ok_or("temp path not UTF-8")?;
    util::atomic_write(cand_s,&hap,0o644)?;
    let valid=util::status("haproxy", &["-c","-f",cand_s]);
    let _=fs::remove_dir_all(&tmp);
    valid.map_err(|e|format!("rendered HAProxy config is invalid; live config untouched: {e}"))?;

    install_config("/etc/haproxy/haproxy.cfg",&hap,0o644)?;
    install_config("/etc/pgbouncer/pgbouncer.ini",&pgb,0o644)?;
    ui::success("Rendered HAProxy writer/reader endpoints and PgBouncer transaction pool");
    ui::note(&format!("PgBouncer listens on {listen_addr}:6432; HAProxy writer on 127.0.0.1:5000 only."));
    if listen.is_none(){ ui::note("Add --listen-addr <internal IP> if applications connect from other hosts; restrict it with a host firewall."); }
    ui::warning("Configure /etc/pgbouncer/userlist.txt before starting PgBouncer, then reload haproxy/pgbouncer."); Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_validated() {
        assert_eq!(parse_nodes("pg1=10.0.0.11,pg2=db2.internal").unwrap().len(),2);
        for bad in ["pg1","pg1=10.0.0.1 backup","pg 1=10.0.0.1","pg1=10.0.0.1\n  bind *:9","pg1=a,pg1=b",""] { assert!(parse_nodes(bad).is_err(),"{bad:?}"); }
    }

    #[test]
    fn haproxy_render_uses_template() {
        let t=include_str!("../templates/haproxy.production.cfg");
        let n=parse_nodes("pg1=10.0.0.11,pg2=fd00::2").unwrap();
        let out=render_haproxy(t,&n,"/ca.crt","/c.pem",Some("10.1.1.1")).unwrap();
        assert!(out.contains("    server pg1 10.0.0.11:5432 check port 8008"));
        assert!(out.contains("server pg2 [fd00::2]:5432"));
        assert!(out.contains("bind 127.0.0.1:5000") && !out.contains("bind *:"));
        assert!(out.contains("bind 10.1.1.1:5001"));
        assert!(!out.contains("${"));
        let p=util::render_template(include_str!("../templates/pgbouncer.production.ini"),&[("LISTEN_ADDR","127.0.0.1")]).unwrap();
        assert!(p.contains("listen_addr = 127.0.0.1\n") && p.contains("ignore_startup_parameters"));
    }
}
