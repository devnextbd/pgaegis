use crate::{config::Config,json,ui,util};
use crate::util::arg;
use std::fs;
use std::path::Path;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("setup-s3") => setup_s3(args),
        Some("schedule-enable") => schedule_enable(),
        Some("full") => run_backup("full"),
        Some("diff") => run_backup("diff"),
        Some("restore-drill") => restore_drill(args),
        _ => Err("usage: pgaegis backup <setup-s3|schedule-enable|check|full|diff|restore-drill>".into())
    }
}

/// Full backup health check, including `pgbackrest check` (which forces a WAL
/// switch to prove archiving end-to-end). Use for certification/manual runs.
pub fn check()->Result<(),String>{ health(true) }

/// Same as `check` minus `pgbackrest check`: safe to run every minute from the
/// monitor without generating a WAL segment per run.
pub fn check_light()->Result<(),String>{ health(false) }

fn pgbackrest(cfg:&Config,args:&[&str])->Vec<String>{
    let mut a=vec!["-u".to_string(),"postgres".into(),"--".into(),"pgbackrest".into(),"--stanza".into(),cfg.pgbackrest_stanza.clone()];
    a.extend(args.iter().map(|s|s.to_string())); a
}

/// True when pgbackrest.conf defines an off-host repository (S3/GCS/Azure or a repo host).
pub fn has_offhost_repo(conf:&str)->bool{
    conf.lines().filter_map(|l|{let l=l.trim(); if l.starts_with('#')||l.starts_with(';'){return None;} l.split_once('=').map(|(k,v)|(k.trim().to_string(),v.trim().to_string()))})
        .any(|(k,v)|{
            let repo_key=k.strip_prefix("repo").and_then(|r|r.split_once('-')).filter(|(n,_)|!n.is_empty()&&n.chars().all(|c|c.is_ascii_digit())).map(|(_,rest)|rest.to_string());
            match repo_key.as_deref() { Some("type")=>matches!(v.as_str(),"s3"|"gcs"|"azure"), Some("host")=>!v.is_empty(), _=>false }
        })
}

/// Latest backup stop epoch for `stanza` from `pgbackrest info --output=json`.
pub fn latest_backup_stop(info:&str,stanza:&str)->Result<u64,String>{
    let v=json::parse(info).map_err(|e|format!("pgBackRest info JSON: {e}"))?;
    let stanzas=v.as_array().ok_or("pgBackRest info: expected a JSON array")?;
    let st=stanzas.iter().find(|s|s.get("name").and_then(json::Value::as_str)==Some(stanza)).or_else(||if stanzas.len()==1{stanzas.first()}else{None})
        .ok_or_else(||format!("pgBackRest info has no stanza {stanza}"))?;
    st.get("backup").and_then(json::Value::as_array).unwrap_or(&[]).iter()
        .filter_map(|b|b.get("timestamp").and_then(|t|t.get("stop")).and_then(json::Value::as_u64)).max()
        .ok_or_else(||"pgBackRest has no completed backup".to_string())
}

fn health(full:bool)->Result<(),String>{
    util::require_root()?;
    let cfg=Config::load();
    if full {
        let a=pgbackrest(&cfg,&["check"]); let a:Vec<&str>=a.iter().map(String::as_str).collect();
        util::status("runuser",&a)?;
    }
    if !has_offhost_repo(&util::read("/etc/pgbackrest/pgbackrest.conf")) {
        return Err("production certification requires an off-host pgBackRest repository (S3/GCS/Azure or repo host)".into());
    }
    let a=pgbackrest(&cfg,&["info","--output=json"]); let a:Vec<&str>=a.iter().map(String::as_str).collect();
    let info=util::output("runuser",&a)?;
    let latest=latest_backup_stop(&info,&cfg.pgbackrest_stanza)?;
    let age=util::unix_now().saturating_sub(latest);
    if age > cfg.backup_max_age_hours.saturating_mul(3600) { return Err(format!("latest backup is {}h old (limit {}h)",age/3600,cfg.backup_max_age_hours)); }
    // A failed query is a failure, never a silent "healthy".
    let recovery=crate::pg::query("postgres","SELECT pg_is_in_recovery();").map_err(|e|format!("cannot determine node role for WAL archive check: {e}"))?;
    let archive_state=match recovery.trim() {
        "f" => {
            let archive=crate::pg::query("postgres","SELECT CASE WHEN failed_count=0 OR last_failed_time IS NULL OR (last_archived_time IS NOT NULL AND last_archived_time > last_failed_time) THEN 'ok' ELSE 'fail' END FROM pg_stat_archiver;")?;
            if archive.trim()!="ok" { return Err("WAL archiver reports a failure newer than the last successful archive".into()); }
            ui::badge_ok("Healthy")
        }
        "t" => ui::dim("Standby (archiving is verified on the Primary)"),
        other => return Err(format!("unexpected pg_is_in_recovery() result: {other}")),
    };
    ui::card("Backup Health", &[("pgBackRest",ui::badge_ok(if full{"Healthy (check passed)"}else{"Healthy (metadata)"})),("WAL archive",archive_state),("Latest backup age",format!("{}h",age/3600)),("Repository",ui::badge_ok("Off-host configured"))]); Ok(())
}

fn setup_s3(args:&[String])->Result<(),String>{
    util::require_root()?;
    let endpoint=arg(args,"--endpoint").ok_or("--endpoint required")?;
    let bucket=arg(args,"--bucket").ok_or("--bucket required")?;
    let region=arg(args,"--region").ok_or("--region required")?;
    let key=arg(args,"--key").ok_or("--key required")?;
    let pgdata=arg(args,"--pgdata").unwrap_or("/var/lib/postgresql/18/main");
    let stanza=arg(args,"--stanza").unwrap_or("main");
    let uri_style=arg(args,"--uri-style").unwrap_or("host");
    util::validate_safe_name("--stanza",stanza)?;
    util::validate_abs_path("--pgdata",pgdata)?;
    for (label,value) in [("--endpoint",endpoint),("--bucket",bucket),("--region",region),("--key",key)] { util::reject_control_chars(label,value)?; if value.contains("${"){return Err(format!("{label} may not contain '${{'"));} }
    if uri_style!="host" && uri_style!="path" {return Err("--uri-style must be host or path".into());}
    let secret=util::prompt_secret_confirm("S3 secret key")?;
    let cipher=util::prompt_secret_confirm("pgBackRest repository encryption passphrase")?;
    let template=util::asset_path("templates","pgbackrest.production.conf")?;
    let c=fs::read_to_string(&template).map_err(|e|format!("template: {e}"))?;
    let c=util::render_template(&c,&[("S3_ENDPOINT",endpoint),("S3_BUCKET",bucket),("S3_REGION",region),("S3_URI_STYLE",uri_style),("S3_KEY",key),("S3_SECRET",secret.as_str()),("REPO_CIPHER_PASS",cipher.as_str()),("PGDATA",pgdata)])?;
    let _=util::backup_file("/etc/pgbackrest/pgbackrest.conf")?;
    util::write_secret_file("/etc/pgbackrest/pgbackrest.conf",&c)?;
    util::status("chown", &["root:postgres","/etc/pgbackrest/pgbackrest.conf"])?;
    util::status("chmod", &["640","/etc/pgbackrest/pgbackrest.conf"])?;
    fs::create_dir_all("/var/spool/pgbackrest").map_err(|e|e.to_string())?;
    util::status("chown", &["postgres:postgres","/var/spool/pgbackrest"])?;
    util::upsert_kv_file("/etc/pgaegis/pgaegis.conf", &[("PGBACKREST_STANZA",stanza)])?;
    ui::success("pgBackRest S3 configuration installed"); ui::note(&format!("Next: sudo -u postgres pgbackrest --stanza={stanza} stanza-create && sudo pgaegis backup check"));
    Ok(())
}

fn schedule_enable()->Result<(),String>{
    util::require_root()?;
    util::status("systemctl", &["enable","--now","pgaegis-backup-full.timer","pgaegis-backup-diff.timer"])?;
    ui::success("Backup timers enabled"); ui::note("Standby nodes safely skip cluster backups; the current Primary owns the scheduled backup.");
    Ok(())
}

fn run_backup(kind:&str)->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    let recovery=crate::pg::query("postgres","SELECT pg_is_in_recovery();").map_err(|e|format!("cannot determine node role: {e}"))?;
    if recovery.trim()=="t" { ui::note("Backup skipped on standby; the current Primary owns the scheduled cluster backup."); return Ok(()); }
    let a=pgbackrest(&cfg,&["--type",kind,"backup"]); let a:Vec<&str>=a.iter().map(String::as_str).collect();
    util::status("runuser",&a)
}

const DRILL_ROOT:&str="/var/lib/pgaegis/restore-drill/";

fn restore_drill(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    let target=arg(args,"--target-dir").ok_or("--target-dir required")?;
    util::validate_plain_path("--target-dir",target)?;
    if !target.starts_with(DRILL_ROOT) || target.len()<=DRILL_ROOT.len() { return Err(format!("restore drill target must be a directory under {DRILL_ROOT}")); }
    let port=util::validate_port("--port",arg(args,"--port").unwrap_or("55432"))?.to_string();
    if Path::new(target).exists() { return Err("restore target already exists; refusing to overwrite".into()); }
    fs::create_dir_all(DRILL_ROOT).map_err(|e|e.to_string())?;
    let root_canon=fs::canonicalize(DRILL_ROOT).map_err(|e|e.to_string())?;
    fs::create_dir_all(target).map_err(|e|e.to_string())?;
    let canon=fs::canonicalize(target).map_err(|e|e.to_string())?;
    if !canon.starts_with(&root_canon) || canon==root_canon { return Err(format!("restore drill target resolves outside {DRILL_ROOT}")); }
    util::status("chown", &["postgres:postgres",target])?;
    util::status("chmod", &["0700",target])?;
    // promote: pgBackRest's default target action is "pause", which would leave
    // the drill instance in recovery forever.
    let a=pgbackrest(&cfg,&["--pg1-path",target,"--type=immediate","--target-action=promote","restore"]); let a:Vec<&str>=a.iter().map(String::as_str).collect();
    util::status("runuser",&a)?;
    let pgctl=find_pg_ctl()?;
    let sock=if Path::new("/var/run/postgresql").is_dir(){"/var/run/postgresql"}else{target};
    let log=format!("{target}/pgaegis-drill.log");
    // archive_mode=off: the drill must never push its new timeline/history
    // into the production repository. synchronous_standby_names is cleared
    // because the restored Patroni config may require a sync standby.
    let opts=format!("-p {port} -c listen_addresses='' -c unix_socket_directories={sock} -c archive_mode=off -c synchronous_standby_names='' -c hot_standby=on");
    util::status("runuser", &["-u","postgres","--",&pgctl,"-D",target,"-l",&log,"-o",&opts,"-w","-t","300","start"])?;
    let result=validate_drill(sock,&port);
    let _=util::status("runuser", &["-u","postgres","--",&pgctl,"-D",target,"-m","fast","-w","stop"]);
    result.map_err(|e|format!("{e}. Server log: {log}"))?;
    ui::success(&format!("Independent restore drill: PASS ({target})")); Ok(())
}

fn validate_drill(sock:&str,port:&str)->Result<(),String>{
    let q=|sql:&str|util::output("runuser", &["-u","postgres","--","psql","-X","-h",sock,"-p",port,"-d","postgres","-Atc",sql]);
    let mut promoted=false;
    for _ in 0..120 {
        if q("SELECT pg_is_in_recovery();").map(|v|v.trim()=="f").unwrap_or(false) { promoted=true; break; }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if !promoted { return Err("restored instance did not finish recovery and promote within 120s".into()); }
    let v=q("SELECT current_setting('server_version_num')::int >= 180000;")?;
    if v.trim()!="t" { return Err(format!("restore drill validation unexpected: {v}")); }
    Ok(())
}

fn find_pg_ctl()->Result<String,String>{
    if util::command_exists("pg_ctl") { return Ok("pg_ctl".into()); }
    let p=util::output("sh", &["-c","ls -1 /usr/lib/postgresql/*/bin/pg_ctl 2>/dev/null | sort -V | tail -1"])?;
    if p.is_empty(){Err("pg_ctl not found".into())}else{Ok(p)}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_latest_stop_epoch() {
        let s=r#"[{"name":"main","backup":[{"timestamp":{"start":90,"stop":100}},{"timestamp":{"start":200,"stop":250}}],"archive":[{"max":"x"}]}]"#;
        assert_eq!(latest_backup_stop(s,"main"), Ok(250));
        assert!(latest_backup_stop(r#"[{"name":"main","backup":[]}]"#,"main").is_err());
        assert!(latest_backup_stop(r#"[{"name":"a","backup":[]},{"name":"b","backup":[]}]"#,"main").is_err());
    }

    #[test]
    fn offhost_detection_tolerates_spacing() {
        assert!(has_offhost_repo("[global]\nrepo1-type = s3\n"));
        assert!(has_offhost_repo("repo2-host=backup.example\n"));
        assert!(!has_offhost_repo("repo1-path=/var/lib/pgbackrest\n# repo1-type=s3\n"));
        assert!(!has_offhost_repo("repo1-type=posix\n"));
    }
}
