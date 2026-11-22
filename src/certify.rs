use crate::{backup,config::Config,ha,monitor,routing,ui,util};
use std::fs;
use std::path::Path;

#[derive(Clone)] struct Gate { name:&'static str, status:&'static str, detail:String }

pub fn dispatch(args:&[String])->Result<(),String>{
    let role=arg(args,"--role").unwrap_or("all");
    if !matches!(role,"all"|"database"|"router"){return Err("--role must be all, database, or router".into());}
    match args.first().map(String::as_str) {
        Some("check") => { let g=run_checks(role); print_table(role,&g); ensure_pass(&g) },
        Some("report") => report(role),
        Some("lab") => lab(),
        _ => Err("usage: pgaegis certify <check|report> [--role all|database|router] | pgaegis certify lab".into())
    }
}

fn run_checks(role:&str)->Vec<Gate>{
    let cfg=Config::load(); let mut g=Vec::new();
    if role=="all" || role=="database" {
        add(&mut g,"PostgreSQL 18", version_ok("psql","--version"," 18."), "psql major version must be 18");
        add(&mut g,"Patroni", util::command_exists("patroni"), "Patroni must be installed");
        add(&mut g,"3-member etcd quorum", etcd_quorum(&cfg), "etcdctl member list/endpoint health must show at least 3 members and healthy quorum");
        add_result(&mut g,"Patroni/DCS safeguards", ha::audit());
        add_result(&mut g,"pgBackRest + WAL archive", backup::check());
        add(&mut g,"Database TLS and credential permissions", tls_permissions(&cfg,"database"), "database/Patroni/etcd certificates, keys and secret file modes must pass");
        add(&mut g,"Backup schedule", backup_timers(), "full and differential backup timers must be enabled and active");
        add(&mut g,"Monitoring timer", monitor_timer(), "pgaegis-monitor.timer must be enabled and active");
        add_result(&mut g,"Monitoring health checks", monitor::health_check(false));
        add(&mut g,"Alert delivery evidence", evidence_pass(&cfg,"alert.env"), "run pgaegis monitor test-alert against the operations receiver");
        add(&mut g,"Monitoring fault-detection evidence", evidence_fields_pass(&cfg,"monitoring-faults.env", &["backup_failure_detected=PASS","replication_lag_detected=PASS","node_failure_detected=PASS","alerts_emitted=PASS"]), "failure injection must prove backup, lag and node failures are detected and alerted");
        add(&mut g,"Production HA evidence", evidence_fields_pass(&cfg,"ha-e2e.env", &["automatic_failover=PASS","routing_reconnect=PASS","quorum_loss_safe=1","planned_switchover=PASS","former_primary_rejoin=PASS","pg_rewind=PASS","three_node_patroni=PASS","three_member_etcd=PASS"]), "run disposable/VM HA certification suite");
        add(&mut g,"Split-brain evidence", evidence_fields_pass(&cfg,"split-brain.env", &["split_brain=0"]), "network partition test must record split_brain=0");
        add(&mut g,"Watchdog/fencing evidence", signed_evidence_pass(&cfg,"watchdog-fencing.env","watchdog-fencing.sig"), "real watchdog reset and independent fencing isolation must be signed and current");
        add(&mut g,"Independent PITR evidence", evidence_fields_pass(&cfg,"pitr.env", &["independent_restore_host=PASS","wal_replay_boundary=PASS","before_target_present=PASS","after_target_absent=PASS"]), "restore on a distinct server identity must prove the target-time boundary");
        add(&mut g,"Disk/WAL-full evidence", evidence_fields_pass(&cfg,"storage-faults.env", &["disk_full=PASS","wal_full=PASS","data_disk_failure_observed=PASS","wal_disk_failure_observed=PASS"]), "data-filesystem and WAL-filesystem ENOSPC tests must pass");
        add(&mut g,"Rust quality evidence", evidence_fields_pass(&cfg,"rust-quality.env", &["rust_clippy=PASS","rust_tests=PASS","rust_release_build=PASS"]), "cargo clippy/test/release build must pass for this release");
        add(&mut g,"Package lifecycle evidence", evidence_fields_pass(&cfg,"debian13-package.env", &["clean_install=PASS","upgrade=PASS","config_preserved=PASS","repair=PASS","uninstall=PASS"]), "Debian 13 install/upgrade/repair/uninstall lifecycle must pass");
        add(&mut g,"RTO/RPO evidence", rto_rpo(&cfg), "RTO/RPO must be measured and within configured acceptance targets");
    }
    if role=="all" || role=="router" {
        add_result(&mut g,"HAProxy/PgBouncer writer endpoint", routing::check());
        add(&mut g,"Router TLS and credential permissions", tls_permissions(&cfg,"router"), "HAProxy/PgBouncer certificates, keys and credentials must pass");
    }
    g
}

fn add(g:&mut Vec<Gate>,name:&'static str,ok:bool,detail:&str){g.push(Gate{name,status:if ok{"PASS"}else{"FAIL"},detail:detail.into()})}
fn add_result(g:&mut Vec<Gate>,name:&'static str,r:Result<(),String>){match r{Ok(_)=>add(g,name,true,"verified"),Err(e)=>add(g,name,false,&e)}}
fn version_ok(c:&str,arg:&str,needle:&str)->bool{util::output(c,&[arg]).map(|x|x.contains(needle)).unwrap_or(false)}
fn etcd_quorum(cfg:&Config)->bool{
    if !util::command_exists("etcdctl"){return false;}
    let members=ha::etcd_output(cfg, &["member","list"]).unwrap_or_default().lines().filter(|l|!l.trim().is_empty()).count();
    let health=ha::etcd_output(cfg, &["endpoint","health","--cluster","-w","json"]).unwrap_or_default();
    let health_compact:String=health.chars().filter(|c|!c.is_whitespace()).collect();
    let json=ha::etcd_output(cfg, &["member","list","-w","json"]).unwrap_or_default();
    members>=3 && members%2==1 && !json.contains("\"isLearner\":true") && health_compact.matches("\"health\":true").count()>=((members/2)+1)
}
fn tls_permissions(cfg:&Config,role:&str)->bool{
    let mut certs:Vec<&str>=Vec::new();
    let mut strict_private:Vec<&str>=Vec::new();
    let mut group_read:Vec<&str>=Vec::new();
    let mut pairs:Vec<(&str,&str)>=Vec::new();
    let mut chains:Vec<(&str,&str)>=Vec::new();
    if role=="all" || role=="database" {
        certs.extend(["/etc/patroni/tls/ca.crt","/etc/patroni/tls/node.crt","/etc/patroni/tls/admin.crt","/etc/etcd/tls/ca.crt","/etc/etcd/tls/patroni.crt","/etc/postgresql/tls/ca.crt","/etc/postgresql/tls/server.crt"]);
        strict_private.extend(["/etc/patroni/patroni.yml","/etc/patroni/tls/node.key","/etc/patroni/tls/admin.key","/etc/etcd/tls/patroni.key","/etc/postgresql/tls/server.key"]);
        group_read.extend(["/etc/pgbackrest/pgbackrest.conf","/etc/pgaegis/pgaegis.conf"]);
        pairs.extend([("/etc/patroni/tls/node.crt","/etc/patroni/tls/node.key"),("/etc/patroni/tls/admin.crt","/etc/patroni/tls/admin.key"),("/etc/etcd/tls/patroni.crt","/etc/etcd/tls/patroni.key"),("/etc/postgresql/tls/server.crt","/etc/postgresql/tls/server.key")]);
        chains.extend([("/etc/patroni/tls/ca.crt","/etc/patroni/tls/node.crt"),("/etc/patroni/tls/ca.crt","/etc/patroni/tls/admin.crt"),("/etc/etcd/tls/ca.crt","/etc/etcd/tls/patroni.crt"),("/etc/postgresql/tls/ca.crt","/etc/postgresql/tls/server.crt")]);
    }
    if role=="all" || role=="router" {
        certs.extend(["/etc/pgbouncer/tls/ca.crt","/etc/pgbouncer/tls/server.crt","/etc/pgbouncer/tls/client.crt","/etc/haproxy/tls/patroni-ca.crt"]);
        strict_private.extend(["/etc/pgbouncer/tls/server.key","/etc/pgbouncer/tls/client.key","/etc/haproxy/tls/patroni-client.pem"]);
        group_read.push("/etc/pgbouncer/userlist.txt");
        pairs.extend([("/etc/pgbouncer/tls/server.crt","/etc/pgbouncer/tls/server.key"),("/etc/pgbouncer/tls/client.crt","/etc/pgbouncer/tls/client.key")]);
        chains.extend([("/etc/pgbouncer/tls/ca.crt","/etc/pgbouncer/tls/server.crt"),("/etc/postgresql/tls/ca.crt","/etc/pgbouncer/tls/client.crt"),("/etc/haproxy/tls/patroni-ca.crt","/etc/haproxy/tls/patroni-client.pem")]);
        certs.push("/etc/postgresql/tls/ca.crt");
    }
    let checkend=cfg.certificate_min_valid_days.saturating_mul(86400).to_string();
    for cert in certs { if !Path::new(cert).exists(){return false;} if !std::process::Command::new("openssl").args(["x509","-checkend",&checkend,"-noout","-in",cert]).status().map(|x|x.success()).unwrap_or(false){return false;} }
    for p in strict_private {if !safe_mode(p,false){return false;}}
    for p in group_read {if !safe_mode(p,true){return false;}}
    for (cert,key) in pairs {if !cert_key_match(cert,key){return false;}}
    for (ca,cert) in chains {if !cert_chain_ok(ca,cert){return false;}}
    if role=="all" || role=="router" { if !cert_key_match("/etc/haproxy/tls/patroni-client.pem","/etc/haproxy/tls/patroni-client.pem"){return false;} }
    true
}

fn safe_mode(path:&str,allow_group_read:bool)->bool{
    if !Path::new(path).exists(){return false;}
    let out=util::output("stat", &["-c","%a",path]).unwrap_or_default();
    let Ok(mode)=u32::from_str_radix(out.trim(),8) else{return false;};
    if allow_group_read { mode & 0o037 == 0 } else { mode & 0o077 == 0 }
}


fn cert_chain_ok(ca:&str,cert:&str)->bool{
    std::process::Command::new("openssl").args(["verify","-CAfile",ca,cert]).status().map(|s|s.success()).unwrap_or(false)
}

fn cert_key_match(cert:&str,key:&str)->bool{
    let cert_pub=util::output("openssl", &["x509","-pubkey","-noout","-in",cert]).unwrap_or_default();
    let key_pub=util::output("openssl", &["pkey","-pubout","-in",key]).unwrap_or_default();
    !cert_pub.is_empty() && cert_pub==key_pub
}

fn backup_timers()->bool{
    if !util::command_exists("systemctl"){return false;}
    ["pgaegis-backup-full.timer","pgaegis-backup-diff.timer"].iter().all(|unit| {
        std::process::Command::new("systemctl").args(["is-enabled","--quiet",*unit]).status().map(|s|s.success()).unwrap_or(false) &&
        std::process::Command::new("systemctl").args(["is-active","--quiet",*unit]).status().map(|s|s.success()).unwrap_or(false)
    })
}
fn monitor_timer()->bool{ if !util::command_exists("systemctl"){return false;} let enabled=std::process::Command::new("systemctl").args(["is-enabled","--quiet","pgaegis-monitor.timer"]).status().map(|s|s.success()).unwrap_or(false); let active=std::process::Command::new("systemctl").args(["is-active","--quiet","pgaegis-monitor.timer"]).status().map(|s|s.success()).unwrap_or(false); enabled && active }
fn evidence_pass(cfg:&Config,name:&str)->bool{
    let p=format!("{}/{}",cfg.certification_dir,name);
    let s=fs::read_to_string(p).unwrap_or_default();
    if !s.lines().any(|l|l.trim()=="status=PASS"){return false;}
    let tested=s.lines().find_map(|l|l.strip_prefix("tested_epoch=")).and_then(|v|v.trim().parse::<u64>().ok()).unwrap_or(0);
    tested>0 && util::unix_now().saturating_sub(tested) <= cfg.certification_evidence_max_age_hours.saturating_mul(3600)
}

fn evidence_fields_pass(cfg:&Config,name:&str,required:&[&str])->bool{
    let p=format!("{}/{}",cfg.certification_dir,name);
    let s=fs::read_to_string(p).unwrap_or_default();
    evidence_pass(cfg,name) && required.iter().all(|needle| s.lines().any(|l|l.trim()==*needle))
}

fn signed_evidence_pass(cfg:&Config,name:&str,sig:&str)->bool{
    let evidence=format!("{}/{}",cfg.certification_dir,name);
    let signature=format!("{}/{}",cfg.certification_dir,sig);
    let body=fs::read_to_string(&evidence).unwrap_or_default();
    if !body.lines().any(|l|l.trim()=="status=PASS"){return false;}
    if !body.lines().any(|l|l.trim()=="watchdog_reset=PASS"){return false;}
    if !body.lines().any(|l|l.trim()=="fence_isolation=PASS"){return false;}
    if !body.lines().any(|l|l.starts_with("node=") && l.len()>5){return false;}
    let tested=body.lines().find_map(|l|l.strip_prefix("tested_epoch=")).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
    if tested==0 || util::unix_now().saturating_sub(tested)>cfg.certification_evidence_max_age_hours.saturating_mul(3600){return false;}
    let exp=body.lines().find_map(|l|l.strip_prefix("expires_epoch=")).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
    if exp < util::unix_now(){return false;}
    std::process::Command::new("openssl").args(["dgst","-sha256","-verify",&cfg.fence_public_key,"-signature",&signature,&evidence]).status().map(|s|s.success()).unwrap_or(false)
}
fn rto_rpo(cfg:&Config)->bool{
    let p=format!("{}/rto-rpo.env",cfg.certification_dir); let s=fs::read_to_string(p).unwrap_or_default();
    let mut status=false;let mut rto=None;let mut lost=None;let mut tested=0;
    for l in s.lines(){if l=="status=PASS"{status=true;} if let Some(v)=l.strip_prefix("rto_seconds="){rto=v.parse::<u64>().ok();} if let Some(v)=l.strip_prefix("lost_committed_rows="){lost=v.parse::<u64>().ok();} if let Some(v)=l.strip_prefix("tested_epoch="){tested=v.parse::<u64>().unwrap_or(0);}}
    status && tested>0 && util::unix_now().saturating_sub(tested)<=cfg.certification_evidence_max_age_hours.saturating_mul(3600) && rto.map(|x|x<=cfg.rto_target_seconds).unwrap_or(false) && lost.map(|x|x<=cfg.rpo_max_lost_rows).unwrap_or(false)
}
fn print_table(role:&str,g:&[Gate]){
    let rows=g.iter().map(|x|vec![x.name.to_string(),if x.status=="PASS"{ui::badge_ok("PASS")}else{ui::badge_bad("FAIL")},x.detail.clone()]).collect::<Vec<_>>();
    ui::table(&format!("Production Certification / {role}"), &["Gate","Status","Evidence / Requirement"], &rows);
    let pass=g.iter().all(|x|x.status=="PASS");
    if pass{ui::success(&format!("PRODUCTION READINESS ({role}): PASS"));}else{ui::error(&format!("PRODUCTION READINESS ({role}): FAIL"));}
}
fn ensure_pass(g:&[Gate])->Result<(),String>{if g.iter().all(|x|x.status=="PASS"){Ok(())}else{Err("production certification gates are not all PASS".into())}}
fn report(role:&str)->Result<(),String>{let g=run_checks(role); let cfg=Config::load(); fs::create_dir_all(&cfg.certification_dir).map_err(|e|e.to_string())?; let mut out=format!("{{
  \"pgaegis_version\": \"{}\",
  \"role\": \"{}\",
  \"gates\": [
",env!("CARGO_PKG_VERSION"),esc(role));for(i,x)in g.iter().enumerate(){out.push_str(&format!("    {{\"name\":\"{}\",\"status\":\"{}\",\"detail\":\"{}\"}}{}
",esc(x.name),x.status,esc(&x.detail),if i+1==g.len(){""}else{","}));}out.push_str("  ]
}
");let p=format!("{}/latest-report-{}.json",cfg.certification_dir,role);fs::write(&p,out).map_err(|e|e.to_string())?;println!("{p}");Ok(())}
fn lab()->Result<(),String>{ if std::env::var("PGAEGIS_DISPOSABLE_LAB").ok().as_deref()!=Some("YES"){return Err("set PGAEGIS_DISPOSABLE_LAB=YES; certification fault tests are destructive".into());} if !Path::new("tests/cert/run-production-suite.sh").exists(){return Err("run this command from the source tree".into());} util::status("bash", &["tests/cert/run-production-suite.sh"]) }
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}
fn esc(s:&str)->String{s.replace('\\',"\\\\").replace('"',"\\\"").replace('\n',"\\n")}
