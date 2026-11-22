use crate::util;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("report") => report(arg(args,"--db").unwrap_or("postgres")),
        Some("recommend") => recommend(),
        Some("analyze") => analyze(args),
        Some("autovacuum") => autovacuum(args),
        Some("queries") => queries(arg(args,"--db").unwrap_or("postgres")),
        Some("locks") => locks(arg(args,"--db").unwrap_or("postgres")),
        _ => Err("usage: pgaegis optimize <report|recommend|analyze|autovacuum|queries|locks> [options]".into())
    }
}

fn report(db:&str)->Result<(),String>{
    validate_ident(db)?;
    println!("PostgreSQL configuration");
    println!("{}",util::output("psql", &["-d",db,"-x","-c","SELECT name,setting,unit,source FROM pg_settings WHERE name IN ('shared_buffers','effective_cache_size','work_mem','maintenance_work_mem','wal_buffers','checkpoint_completion_target','random_page_cost','effective_io_concurrency','max_connections','autovacuum','autovacuum_vacuum_scale_factor','autovacuum_analyze_scale_factor','log_min_duration_statement') ORDER BY name;"])?);
    println!("\nTop dead-tuple tables");
    println!("{}",util::output("psql", &["-d",db,"-c","SELECT schemaname,relname,n_live_tup,n_dead_tup,last_autovacuum,last_analyze FROM pg_stat_user_tables ORDER BY n_dead_tup DESC NULLS LAST LIMIT 20;"])?);
    println!("\nIndex scan statistics");
    println!("{}",util::output("psql", &["-d",db,"-c","SELECT schemaname,relname,indexrelname,idx_scan,pg_size_pretty(pg_relation_size(indexrelid)) AS size FROM pg_stat_user_indexes ORDER BY idx_scan ASC,pg_relation_size(indexrelid) DESC LIMIT 30;"])?);
    Ok(())
}

fn recommend()->Result<(),String>{
    let mem_kb=std::fs::read_to_string("/proc/meminfo").unwrap_or_default().lines().find_map(|l|l.strip_prefix("MemTotal:").and_then(|x|x.split_whitespace().next()).and_then(|x|x.parse::<u64>().ok())).unwrap_or(0);
    let cores=std::thread::available_parallelism().map(|x|x.get()).unwrap_or(1);
    if mem_kb==0{return Err("unable to detect RAM".into());}
    let mib=mem_kb/1024; let shared=mib/4; let cache=mib*3/4; let maint=(mib/16).clamp(64,2048);
    println!("Hardware: {} MiB RAM, {} CPU threads",mib,cores);
    println!("Conservative dedicated-server starting points (review workload before applying):");
    println!("  shared_buffers              = {}MB",shared);
    println!("  effective_cache_size        = {}MB",cache);
    println!("  maintenance_work_mem        = {}MB",maint);
    println!("  checkpoint_completion_target = 0.9");
    println!("  random_page_cost            = 1.1 (SSD/NVMe only)");
    println!("  effective_io_concurrency    = 200 (verify storage)");
    println!("work_mem is intentionally not auto-sized: it is per operation and must account for concurrency.");
    Ok(())
}

fn analyze(args:&[String])->Result<(),String>{
    util::require_root()?; ensure_primary()?;
    let db=arg(args,"--db").unwrap_or("postgres"); validate_ident(db)?;
    if let Some(table)=arg(args,"--table") { let q=quote_qualified(table)?; util::status("runuser", &["-u","postgres","--","psql","-d",db,"-v","ON_ERROR_STOP=1","-c",&format!("ANALYZE VERBOSE {q};")]) }
    else { util::status("runuser", &["-u","postgres","--","psql","-d",db,"-v","ON_ERROR_STOP=1","-c","ANALYZE VERBOSE;"]) }
}

fn autovacuum(args:&[String])->Result<(),String>{
    util::require_root()?; ensure_primary()?;
    let db=arg(args,"--db").ok_or("--db required")?; validate_ident(db)?;
    let table=arg(args,"--table").ok_or("--table schema.table required")?; let q=quote_qualified(table)?;
    let vac=parse_factor(arg(args,"--vacuum-scale").unwrap_or("0.02"),"--vacuum-scale")?;
    let ana=parse_factor(arg(args,"--analyze-scale").unwrap_or("0.01"),"--analyze-scale")?;
    let sql=format!("ALTER TABLE {q} SET (autovacuum_vacuum_scale_factor={vac}, autovacuum_analyze_scale_factor={ana}); ANALYZE {q};");
    util::status("runuser", &["-u","postgres","--","psql","-d",db,"-v","ON_ERROR_STOP=1","-c",&sql])
}

fn queries(db:&str)->Result<(),String>{
    validate_ident(db)?;
    let exists=util::output("psql", &["-d",db,"-Atqc","SELECT 1 FROM pg_extension WHERE extname='pg_stat_statements';"])?;
    if exists.trim()!="1" {return Err("pg_stat_statements is not enabled in this database. Use pgaegis extension preload pg_stat_statements, perform a controlled rolling restart, then pgaegis extension enable pg_stat_statements --db <db>".into());}
    println!("{}",util::output("psql", &["-d",db,"-c","SELECT calls,round(total_exec_time::numeric,2) AS total_ms,round(mean_exec_time::numeric,2) AS mean_ms,rows,left(query,160) AS query FROM pg_stat_statements ORDER BY total_exec_time DESC LIMIT 25;"])?); Ok(())
}

fn locks(db:&str)->Result<(),String>{
    validate_ident(db)?;
    println!("{}",util::output("psql", &["-d",db,"-c","SELECT blocked.pid AS blocked_pid,blocking.pid AS blocking_pid,blocked.wait_event_type,blocked.wait_event,left(blocked.query,120) AS blocked_query,left(blocking.query,120) AS blocking_query FROM pg_stat_activity blocked JOIN pg_locks bl ON bl.pid=blocked.pid AND NOT bl.granted JOIN pg_locks kl ON kl.locktype=bl.locktype AND kl.database IS NOT DISTINCT FROM bl.database AND kl.relation IS NOT DISTINCT FROM bl.relation AND kl.page IS NOT DISTINCT FROM bl.page AND kl.tuple IS NOT DISTINCT FROM bl.tuple AND kl.virtualxid IS NOT DISTINCT FROM bl.virtualxid AND kl.transactionid IS NOT DISTINCT FROM bl.transactionid AND kl.classid IS NOT DISTINCT FROM bl.classid AND kl.objid IS NOT DISTINCT FROM bl.objid AND kl.objsubid IS NOT DISTINCT FROM bl.objsubid AND kl.granted JOIN pg_stat_activity blocking ON blocking.pid=kl.pid WHERE blocked.pid<>blocking.pid;"])?); Ok(())
}

fn ensure_primary()->Result<(),String>{let x=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';"])?;if x.trim()=="t"{Ok(())}else{Err("maintenance mutation is allowed only on writable Primary".into())}}
fn parse_factor(v:&str,label:&str)->Result<f64,String>{let x=v.parse::<f64>().map_err(|_|format!("{label} must be a number"))?;if (0.0..=1.0).contains(&x){Ok(x)}else{Err(format!("{label} must be between 0 and 1"))}}
fn validate_ident(s:&str)->Result<(),String>{if !s.is_empty()&&s.len()<=63&&s.chars().all(|c|c.is_ascii_alphanumeric()||c=='_'){Ok(())}else{Err(format!("invalid identifier: {s}"))}}
fn quote_qualified(s:&str)->Result<String,String>{let parts:Vec<&str>=s.split('.').collect();if parts.is_empty()||parts.len()>2{return Err("table must be table or schema.table".into());}for p in &parts{validate_ident(p)?;}Ok(parts.into_iter().map(|x|format!("\"{}\"",x)).collect::<Vec<_>>().join("."))}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}
