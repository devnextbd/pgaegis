use crate::{backup,config::Config,ui,util};
use std::process::Command;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("check")=>health_check(args.iter().any(|x|x=="--alert")),
        Some("test-alert")=>test_alert(),
        _=>Err("usage: pgaegis monitor <check [--alert]|test-alert>".into())
    }
}

fn test_alert()->Result<(),String>{
    let cfg=Config::load();
    if cfg.alert_webhook.is_empty(){return Err("ALERT_WEBHOOK is not configured".into());}
    let msg=format!("pgaegis test alert from {}",util::output("hostname", &["-f"]).unwrap_or_else(|_|"unknown".into()));
    let s=Command::new("curl").args(["--fail","--silent","--show-error","-X","POST","-H","Content-Type: text/plain","--data",&msg,&cfg.alert_webhook]).status().map_err(|e|e.to_string())?;
    if !s.success(){return Err("alert webhook rejected the test alert".into());}
    std::fs::create_dir_all(&cfg.certification_dir).map_err(|e|e.to_string())?;
    std::fs::write(format!("{}/alert.env",cfg.certification_dir),format!("status=PASS\ntested_epoch={}\n",util::unix_now())).map_err(|e|e.to_string())?;
    ui::success("Alert delivery test: PASS"); Ok(())
}

pub fn health_check(alert:bool)->Result<(),String>{
    let cfg=Config::load(); let mut failures=Vec::new();
    if util::output("pg_isready", &["-h",&cfg.writer_host,"-p",&cfg.writer_port.to_string()]).is_err(){failures.push("writer endpoint unavailable");}
    if util::command_exists("patronictl") {
        let list=util::output("patronictl", &["-c",&cfg.patroni_config,"list",&cfg.cluster_name,"-f","tsv"]).unwrap_or_default();
        let member_lines:Vec<&str>=list.lines().filter(|l|!l.trim().is_empty()).collect();
        let members=member_lines.len();
        let leaders=member_lines.iter().filter(|l|l.to_ascii_lowercase().contains("leader")||l.to_ascii_lowercase().contains("primary")).count();
        if leaders!=1 { failures.push("cluster does not have exactly one leader"); }
        if members<3 { failures.push("fewer than 3 Patroni members are visible"); }
        if member_lines.iter().any(|l| { let x=l.to_ascii_lowercase(); x.contains("stopped")||x.contains("unknown")||x.contains("uninitialized")||x.contains("start failed") }) {
            failures.push("one or more Patroni members are unhealthy");
        }
    }
    if !cfg.monitor_dsn.is_empty() {
        match util::output("psql", &[&cfg.monitor_dsn,"-Atc","SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)),0)::bigint FROM pg_stat_replication;"]) {
            Ok(v) => if v.trim().parse::<u64>().unwrap_or(u64::MAX) > cfg.max_replication_lag_bytes { failures.push("replication lag exceeds configured threshold"); },
            Err(_) => failures.push("replication lag query failed"),
        }
        match util::output("psql", &[&cfg.monitor_dsn,"-Atc","SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)),0)::bigint FROM pg_replication_slots WHERE slot_type='physical' AND restart_lsn IS NOT NULL;"]) {
            Ok(v) => if v.trim().parse::<u64>().unwrap_or(u64::MAX) > cfg.max_slot_retained_wal_bytes { failures.push("physical replication slots retain too much WAL"); },
            Err(_) => failures.push("replication-slot retained WAL query failed"),
        }
    } else { failures.push("MONITOR_DSN is not configured; replication lag cannot be measured"); }
    if util::command_exists("pgbackrest") && backup::check().is_err(){failures.push("pgBackRest health/backup metadata check failed");}
    for path in [&cfg.pgdata[..], "/var/spool/pgbackrest"] {
        if !std::path::Path::new(path).exists(){continue;}
        let disk=util::output("df", &["-P",path]).unwrap_or_default();
        if let Some(last)=disk.lines().last(){
            if let Some(pct)=last.split_whitespace().nth(4).and_then(|x|x.trim_end_matches('%').parse::<u32>().ok()){
                if pct>=cfg.disk_usage_max_percent { failures.push(if path==cfg.pgdata.as_str(){"PostgreSQL data filesystem usage exceeds threshold"}else{"pgBackRest spool filesystem usage exceeds threshold"}); }
            }
        }
    }
    if failures.is_empty(){ui::success("MONITOR: PASS"); return Ok(());}
    let msg=format!("pgaegis alert on {}: {}",util::output("hostname", &["-f"]).unwrap_or_else(|_|"unknown".into()),failures.join("; "));
    for f in &failures {ui::error(&format!("ALERT: {f}"));}
    if alert && !cfg.alert_webhook.is_empty(){ let _=Command::new("curl").args(["--fail","--silent","--show-error","-X","POST","-H","Content-Type: text/plain","--data",&msg,&cfg.alert_webhook]).status(); }
    Err(format!("{} monitoring check(s) failed",failures.len()))
}
