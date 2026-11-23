use crate::{pg,util};
use crate::util::arg;

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
    pg::validate_ident(db)?;
    println!("PostgreSQL configuration");
    println!("{}",pg::report(db,"SELECT name,setting,unit,source FROM pg_settings WHERE name IN ('shared_buffers','effective_cache_size','work_mem','maintenance_work_mem','wal_buffers','checkpoint_completion_target','random_page_cost','effective_io_concurrency','max_connections','autovacuum','autovacuum_vacuum_scale_factor','autovacuum_analyze_scale_factor','log_min_duration_statement') ORDER BY name;",true)?);
    println!("\nTop dead-tuple tables");
    println!("{}",pg::report(db,"SELECT schemaname,relname,n_live_tup,n_dead_tup,last_autovacuum,last_analyze FROM pg_stat_user_tables ORDER BY n_dead_tup DESC NULLS LAST LIMIT 20;",false)?);
    println!("\nIndex scan statistics");
    println!("{}",pg::report(db,"SELECT schemaname,relname,indexrelname,idx_scan,pg_size_pretty(pg_relation_size(indexrelid)) AS size FROM pg_stat_user_indexes ORDER BY idx_scan ASC,pg_relation_size(indexrelid) DESC LIMIT 30;",false)?);
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
    util::require_root()?; pg::ensure_writable_primary("maintenance mutation")?;
    let db=arg(args,"--db").unwrap_or("postgres"); pg::validate_ident(db)?;
    if let Some(table)=arg(args,"--table") { let q=pg::quote_qualified(table)?; pg::exec(db,&format!("ANALYZE VERBOSE {q};")) }
    else { pg::exec(db,"ANALYZE VERBOSE;") }
}

fn autovacuum(args:&[String])->Result<(),String>{
    util::require_root()?; pg::ensure_writable_primary("maintenance mutation")?;
    let db=arg(args,"--db").ok_or("--db required")?; pg::validate_ident(db)?;
    let table=arg(args,"--table").ok_or("--table schema.table required")?; let q=pg::quote_qualified(table)?;
    let vac=parse_factor(arg(args,"--vacuum-scale").unwrap_or("0.02"),"--vacuum-scale")?;
    let ana=parse_factor(arg(args,"--analyze-scale").unwrap_or("0.01"),"--analyze-scale")?;
    let sql=format!("ALTER TABLE {q} SET (autovacuum_vacuum_scale_factor={vac}, autovacuum_analyze_scale_factor={ana}); ANALYZE {q};");
    pg::exec(db,&sql)
}

fn queries(db:&str)->Result<(),String>{
    pg::validate_ident(db)?;
    let exists=pg::query(db,"SELECT 1 FROM pg_extension WHERE extname='pg_stat_statements';")?;
    if exists.trim()!="1" {return Err("pg_stat_statements is not enabled in this database. Use pgaegis extension preload pg_stat_statements, perform a controlled rolling restart, then pgaegis extension enable pg_stat_statements --db <db>".into());}
    println!("{}",pg::report(db,"SELECT calls,round(total_exec_time::numeric,2) AS total_ms,round(mean_exec_time::numeric,2) AS mean_ms,rows,left(query,160) AS query FROM pg_stat_statements ORDER BY total_exec_time DESC LIMIT 25;",false)?); Ok(())
}

fn locks(db:&str)->Result<(),String>{
    pg::validate_ident(db)?;
    println!("{}",pg::report(db,"SELECT blocked.pid AS blocked_pid,blocking.pid AS blocking_pid,blocked.wait_event_type,blocked.wait_event,left(blocked.query,120) AS blocked_query,left(blocking.query,120) AS blocking_query FROM pg_stat_activity blocked JOIN pg_locks bl ON bl.pid=blocked.pid AND NOT bl.granted JOIN pg_locks kl ON kl.locktype=bl.locktype AND kl.database IS NOT DISTINCT FROM bl.database AND kl.relation IS NOT DISTINCT FROM bl.relation AND kl.page IS NOT DISTINCT FROM bl.page AND kl.tuple IS NOT DISTINCT FROM bl.tuple AND kl.virtualxid IS NOT DISTINCT FROM bl.virtualxid AND kl.transactionid IS NOT DISTINCT FROM bl.transactionid AND kl.classid IS NOT DISTINCT FROM bl.classid AND kl.objid IS NOT DISTINCT FROM bl.objid AND kl.objsubid IS NOT DISTINCT FROM bl.objsubid AND kl.granted JOIN pg_stat_activity blocking ON blocking.pid=kl.pid WHERE blocked.pid<>blocking.pid;",false)?); Ok(())
}

fn parse_factor(v:&str,label:&str)->Result<f64,String>{let x=v.parse::<f64>().map_err(|_|format!("{label} must be a number"))?;if (0.0..=1.0).contains(&x){Ok(x)}else{Err(format!("{label} must be between 0 and 1"))}}
