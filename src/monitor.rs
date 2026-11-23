use crate::{backup,config::Config,ha,routing,scram,ui,util};
use crate::util::has_flag;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("check")=>health_check(has_flag(args,"--alert")),
        Some("test-alert")=>test_alert(),
        _=>Err("usage: pgaegis monitor <check [--alert]|test-alert>".into())
    }
}

/// POST `msg` to the webhook. The URL (often containing a token) goes to curl
/// through a config on stdin, never argv; bounded timeouts keep a hung
/// receiver from blocking the oneshot service and stalling the timer.
pub fn send_alert(webhook:&str,msg:&str)->Result<(),String>{
    let conf=format!("url = {}\ndata = {}\n",util::curl_config_quote(webhook),util::curl_config_quote(msg));
    util::output_with("curl",&["--fail","--silent","--show-error","--connect-timeout","5","--max-time","15","--retry","2","--retry-max-time","30","-X","POST","-H","Content-Type: text/plain","-K","-"],&[],Some(&conf)).map(|_|())
}

fn host()->String { util::output("hostname", &["-f"]).unwrap_or_else(|_|"unknown".into()) }

fn test_alert()->Result<(),String>{
    let cfg=Config::load();
    if cfg.alert_webhook.is_empty(){return Err("ALERT_WEBHOOK is not configured".into());}
    send_alert(&cfg.alert_webhook,&format!("pgaegis test alert from {}",host())).map_err(|e|format!("alert webhook rejected the test alert: {e}"))?;
    std::fs::create_dir_all(&cfg.certification_dir).map_err(|e|e.to_string())?;
    util::atomic_write(&format!("{}/alert.env",cfg.certification_dir),&format!("status=PASS\ntested_epoch={}\n",util::unix_now()),0o640)?;
    ui::success("Alert delivery test: PASS"); Ok(())
}

const STATE_FILE:&str="/var/lib/pgaegis/monitor-alert.state";
const REPEAT_SECS:u64=30*60;

/// Alert dedup: send when the failure set changes, or every 30 minutes while
/// it persists. Returns true when an alert should be sent now.
pub fn should_alert(prev:&str,fingerprint:&str,now:u64)->bool{
    let mut fp=""; let mut sent=0u64;
    for l in prev.lines() {
        if let Some(v)=l.strip_prefix("fingerprint="){fp=v;}
        else if let Some(v)=l.strip_prefix("sent_epoch="){sent=v.parse().unwrap_or(0);}
    }
    fp!=fingerprint || now.saturating_sub(sent)>=REPEAT_SECS
}

fn fingerprint(failures:&[String])->String{
    scram::sha256(failures.join("\n").as_bytes()).iter().take(12).map(|b|format!("{b:02x}")).collect()
}

pub fn health_check(alert:bool)->Result<(),String>{
    let cfg=Config::load(); let mut failures:Vec<String>=Vec::new();
    if util::output("pg_isready", &["-h",&cfg.writer_host,"-p",&cfg.writer_port.to_string()]).is_err(){failures.push("writer endpoint unavailable".into());}
    if util::command_exists("patronictl") {
        match ha::patroni_members(&cfg) {
            Ok(m)=>failures.extend(ha::topology_failures(&m)),
            Err(e)=>failures.push(format!("Patroni cluster state unavailable: {e}")),
        }
    }
    if !cfg.monitor_dsn.is_empty() {
        match routing::psql_dsn(&cfg.monitor_dsn,"SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)),0)::bigint FROM pg_stat_replication;") {
            Ok(v) => if v.trim().parse::<u64>().unwrap_or(u64::MAX) > cfg.max_replication_lag_bytes { failures.push("replication lag exceeds configured threshold".into()); },
            Err(_) => failures.push("replication lag query failed".into()),
        }
        match routing::psql_dsn(&cfg.monitor_dsn,"SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)),0)::bigint FROM pg_replication_slots WHERE slot_type='physical' AND restart_lsn IS NOT NULL;") {
            Ok(v) => if v.trim().parse::<u64>().unwrap_or(u64::MAX) > cfg.max_slot_retained_wal_bytes { failures.push("physical replication slots retain too much WAL".into()); },
            Err(_) => failures.push("replication-slot retained WAL query failed".into()),
        }
    } else { failures.push("MONITOR_DSN is not configured; replication lag cannot be measured".into()); }
    // Light check: metadata + archiver status only. `pgbackrest check` forces a
    // WAL switch and is reserved for `pgaegis backup check` / certification.
    if util::command_exists("pgbackrest") { if let Err(e)=backup::check_light(){failures.push(format!("pgBackRest health/backup metadata check failed: {e}"));} }
    for path in [&cfg.pgdata[..], "/var/spool/pgbackrest"] {
        if !std::path::Path::new(path).exists(){continue;}
        let disk=util::output("df", &["-P",path]).unwrap_or_default();
        if let Some(last)=disk.lines().last(){
            if let Some(pct)=last.split_whitespace().nth(4).and_then(|x|x.trim_end_matches('%').parse::<u32>().ok()){
                if pct>=cfg.disk_usage_max_percent { failures.push(if path==cfg.pgdata.as_str(){"PostgreSQL data filesystem usage exceeds threshold".into()}else{"pgBackRest spool filesystem usage exceeds threshold".into()}); }
            }
        }
    }
    let prev=util::read(STATE_FILE);
    if failures.is_empty(){
        if alert && !prev.is_empty() {
            if !cfg.alert_webhook.is_empty() { if let Err(e)=send_alert(&cfg.alert_webhook,&format!("pgaegis RECOVERED on {}: all monitoring checks pass",host())) { ui::error(&format!("recovery notice not delivered: {e}")); } }
            let _=std::fs::remove_file(STATE_FILE);
        }
        ui::success("MONITOR: PASS"); return Ok(());
    }
    for f in &failures {ui::error(&format!("ALERT: {f}"));}
    if alert {
        if cfg.alert_webhook.is_empty() { ui::error("ALERT_WEBHOOK is not configured; alert not delivered"); }
        else {
            let fp=fingerprint(&failures); let now=util::unix_now();
            if should_alert(&prev,&fp,now) {
                let msg=format!("pgaegis alert on {}: {}",host(),failures.join("; "));
                match send_alert(&cfg.alert_webhook,&msg) {
                    Ok(())=>{ let _=std::fs::create_dir_all("/var/lib/pgaegis"); let _=util::atomic_write(STATE_FILE,&format!("fingerprint={fp}\nsent_epoch={now}\n"),0o640); },
                    Err(e)=>ui::error(&format!("alert webhook delivery FAILED: {e}")),
                }
            } else { ui::note("Same failure set already alerted recently; not re-sending yet"); }
        }
    }
    Err(format!("{} monitoring check(s) failed",failures.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_dedup() {
        assert!(should_alert("","abc",100));
        let st="fingerprint=abc\nsent_epoch=100\n";
        assert!(!should_alert(st,"abc",200));
        assert!(should_alert(st,"def",200));
        assert!(should_alert(st,"abc",100+REPEAT_SECS));
    }
}
