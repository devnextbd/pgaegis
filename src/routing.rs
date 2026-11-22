use crate::{config::Config,ui,util};
use std::fs;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("render") => render(args),
        _ => Err("usage: pgaegis routing <check|render>".into())
    }
}

pub fn check()->Result<(),String>{
    let cfg=Config::load();
    util::output("pg_isready", &["-h",&cfg.writer_host,"-p",&cfg.writer_port.to_string()])?;
    if cfg.monitor_dsn.is_empty(){return Err("MONITOR_DSN must point through the PgBouncer writer endpoint for certification".into());}
    let writable=util::output("psql", &[&cfg.monitor_dsn,"-Atc","SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';"])?;
    if writable.trim()!="t" {return Err("writer endpoint is reachable but is not writable primary".into());}
    if util::command_exists("systemctl") {
        util::status("systemctl", &["is-active","--quiet","haproxy"])?;
        util::status("systemctl", &["is-active","--quiet","pgbouncer"])?;
    }
    ui::card("Application Routing", &[("Writer endpoint",format!("{}:{}",cfg.writer_host,cfg.writer_port)),("Writable Primary",ui::badge_ok("Verified")),("HAProxy",ui::badge_ok("Running")),("PgBouncer",ui::badge_ok("Running"))]); Ok(())
}

fn render(args:&[String])->Result<(),String>{
    util::require_root()?;
    let nodes=arg(args,"--nodes").ok_or("--nodes pg1=10.0.0.11,pg2=10.0.0.12,pg3=10.0.0.13 required")?;
    let patroni_ca=arg(args,"--patroni-ca").unwrap_or("/etc/haproxy/tls/patroni-ca.crt");
    let patroni_client=arg(args,"--patroni-client-pem").unwrap_or("/etc/haproxy/tls/patroni-client.pem");
    if !std::path::Path::new(patroni_ca).exists() || !std::path::Path::new(patroni_client).exists(){return Err("Patroni mTLS CA/client PEM missing for HAProxy health checks".into());}
    let mut servers=String::new();
    for item in nodes.split(',') {
        let (name,host)=item.split_once('=').ok_or("invalid --nodes entry")?;
        servers.push_str(&format!("    server {name} {host}:5432 check port 8008 check-ssl verify required ca-file {patroni_ca} crt {patroni_client} inter 2s fall 3 rise 2 on-marked-down shutdown-sessions\n"));
    }
    let hap=format!(r#"global
    log /dev/log local0
    maxconn 5000

defaults
    log global
    mode tcp
    timeout connect 5s
    timeout client  60s
    timeout server  60s

frontend pg_writer
    bind *:5000
    default_backend patroni_primary

backend patroni_primary
    option httpchk GET /primary
    http-check expect status 200
{servers}

frontend pg_reader
    bind *:5001
    default_backend patroni_replicas

backend patroni_replicas
    balance roundrobin
    option httpchk GET /replica
    http-check expect status 200
{servers}
"#);
    fs::create_dir_all("/etc/haproxy").map_err(|e|e.to_string())?;
    fs::write("/etc/haproxy/haproxy.cfg",hap).map_err(|e|e.to_string())?;
    let pgb=r#"[databases]
* = host=127.0.0.1 port=5000

[pgbouncer]
listen_addr = 0.0.0.0
listen_port = 6432
pool_mode = transaction
max_client_conn = 2000
default_pool_size = 50
reserve_pool_size = 10
server_reset_query = DISCARD ALL
server_lifetime = 300
server_idle_timeout = 60
server_fast_close = 1
ignore_startup_parameters = extra_float_digits
server_tls_sslmode = verify-ca
server_tls_ca_file = /etc/postgresql/tls/ca.crt
server_tls_cert_file = /etc/pgbouncer/tls/client.crt
server_tls_key_file = /etc/pgbouncer/tls/client.key
client_tls_sslmode = require
client_tls_ca_file = /etc/pgbouncer/tls/ca.crt
client_tls_cert_file = /etc/pgbouncer/tls/server.crt
client_tls_key_file = /etc/pgbouncer/tls/server.key
auth_type = scram-sha-256
auth_file = /etc/pgbouncer/userlist.txt
admin_users = postgres
stats_users = postgres
"#;
    fs::create_dir_all("/etc/pgbouncer").map_err(|e|e.to_string())?;
    fs::write("/etc/pgbouncer/pgbouncer.ini",pgb).map_err(|e|e.to_string())?;
    util::status("haproxy", &["-c","-f","/etc/haproxy/haproxy.cfg"])?;
    ui::success("Rendered HAProxy writer/reader endpoints and PgBouncer transaction pool"); ui::warning("Configure /etc/pgbouncer/userlist.txt before starting PgBouncer."); Ok(())
}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{ a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str) }
