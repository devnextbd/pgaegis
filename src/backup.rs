use crate::{config::Config,ui,util};
use std::fs;

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

pub fn check()->Result<(),String>{
    util::require_root()?;
    let cfg=Config::load();
    util::status("runuser", &["-u","postgres","--","pgbackrest","--stanza",&cfg.pgbackrest_stanza,"check"])?;
    let brcfg=util::read("/etc/pgbackrest/pgbackrest.conf");
    if !(brcfg.contains("repo1-type=s3") || brcfg.contains("repo1-host=") || brcfg.contains("repo2-type=s3") || brcfg.contains("repo2-host=")) {
        return Err("production certification requires an off-host pgBackRest repository (S3 or repo host)".into());
    }
    let info=util::output("runuser", &["-u","postgres","--","pgbackrest","--stanza",&cfg.pgbackrest_stanza,"info","--output=json"])?;
    if !info.contains("\"backup\"") { return Err("pgBackRest has no backup metadata".into()); }
    let latest=latest_json_epoch(&info,"\"stop\":").ok_or("unable to determine latest backup stop timestamp")?;
    let age=util::unix_now().saturating_sub(latest);
    if age > cfg.backup_max_age_hours.saturating_mul(3600) { return Err(format!("latest backup is {}h old (limit {}h)",age/3600,cfg.backup_max_age_hours)); }
    let recovery=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT pg_is_in_recovery();"]).unwrap_or_default();
    if recovery.trim()=="f" {
        let archive=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT CASE WHEN failed_count=0 OR last_failed_time IS NULL OR (last_archived_time IS NOT NULL AND last_archived_time > last_failed_time) THEN 'ok' ELSE 'fail' END FROM pg_stat_archiver;"])?;
        if archive.trim()!="ok" { return Err("WAL archiver reports a failure newer than the last successful archive".into()); }
    }
    ui::card("Backup Health", &[("pgBackRest",ui::badge_ok("Healthy")),("WAL archive",ui::badge_ok("Healthy")),("Latest backup age",format!("{}h",age/3600)),("Repository",ui::badge_ok("Off-host configured"))]); Ok(())
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
    for (label,value) in [("--endpoint",endpoint),("--bucket",bucket),("--region",region),("--key",key)] { util::reject_control_chars(label,value)?; }
    if uri_style!="host" && uri_style!="path" {return Err("--uri-style must be host or path".into());}
    let secret=util::prompt_secret_confirm("S3 secret key")?;
    let cipher=util::prompt_secret_confirm("pgBackRest repository encryption passphrase")?;
    let template=if std::path::Path::new("/usr/share/pgaegis/templates/pgbackrest.production.conf").exists(){"/usr/share/pgaegis/templates/pgbackrest.production.conf"}else{"templates/pgbackrest.production.conf"};
    let mut c=fs::read_to_string(template).map_err(|e|format!("template: {e}"))?;
    for (k,v) in [("${S3_ENDPOINT}",endpoint),("${S3_BUCKET}",bucket),("${S3_REGION}",region),("${S3_URI_STYLE}",uri_style),("${S3_KEY}",key),("${S3_SECRET}",secret.as_str()),("${REPO_CIPHER_PASS}",cipher.as_str()),("${PGDATA}",pgdata)] {c=c.replace(k,v);}
    if c.contains("${"){return Err("unresolved pgBackRest template variables remain".into());}
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
    let recovery=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT pg_is_in_recovery();"]).unwrap_or_default();
    if recovery.trim()=="t" { ui::note("Backup skipped on standby; the current Primary owns the scheduled cluster backup."); return Ok(()); }
    util::status("runuser", &["-u","postgres","--","pgbackrest","--stanza",&cfg.pgbackrest_stanza,"--type",kind,"backup"])
}

fn restore_drill(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    let target=arg(args,"--target-dir").ok_or("--target-dir required")?;
    if !target.starts_with("/var/lib/pgaegis/restore-drill/") { return Err("restore drill target must be under /var/lib/pgaegis/restore-drill/".into()); }
    let port=arg(args,"--port").unwrap_or("55432");
    if std::path::Path::new(target).exists() { return Err("restore target already exists; refusing to overwrite".into()); }
    fs::create_dir_all(target).map_err(|e|e.to_string())?;
    util::status("chown", &["postgres:postgres",target])?;
    util::status("runuser", &["-u","postgres","--","pgbackrest","--stanza",&cfg.pgbackrest_stanza,"--pg1-path",target,"--type=immediate","restore"])?;
    let pgctl=find_pg_ctl()?;
    util::status("runuser", &["-u","postgres","--",&pgctl,"-D",target,"-o",&format!("-p {port} -c listen_addresses=127.0.0.1"),"-w","start"])?;
    let result=util::output("psql", &["-h","127.0.0.1","-p",port,"-U","postgres","-d","postgres","-Atc","SELECT pg_is_in_recovery(), current_setting('server_version_num')::int >= 180000;"]);
    let _=util::status("runuser", &["-u","postgres","--",&pgctl,"-D",target,"-m","fast","-w","stop"]);
    let out=result?; if !out.contains("f|t") { return Err(format!("restore drill validation unexpected: {out}")); }
    ui::success(&format!("Independent restore drill: PASS ({target})")); Ok(())
}
fn find_pg_ctl()->Result<String,String>{
    if util::command_exists("pg_ctl") { return Ok("pg_ctl".into()); }
    let p=util::output("sh", &["-c","ls -1 /usr/lib/postgresql/*/bin/pg_ctl 2>/dev/null | sort -V | tail -1"])?;
    if p.is_empty(){Err("pg_ctl not found".into())}else{Ok(p)}
}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{ a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str) }


fn latest_json_epoch(s:&str,key:&str)->Option<u64>{
    let mut rest=s; let mut max=None;
    while let Some(i)=rest.find(key){
        rest=&rest[i+key.len()..];
        let digits:String=rest.chars().skip_while(|c|c.is_whitespace()).take_while(|c|c.is_ascii_digit()).collect();
        if let Ok(v)=digits.parse::<u64>(){max=Some(max.map(|m:u64|m.max(v)).unwrap_or(v));}
        if rest.is_empty(){break;}
    }
    max
}

#[cfg(test)]
mod tests {
    use super::latest_json_epoch;
    #[test]
    fn extracts_latest_stop_epoch() {
        let s=r#"[{"backup":[{"timestamp":{"stop":100}},{"timestamp":{"stop":250}}]}]"#;
        assert_eq!(latest_json_epoch(s, "\"stop\":"), Some(250));
    }
}
