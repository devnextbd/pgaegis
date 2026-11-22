use crate::{database, ui, util};

pub fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("list") => list(args),
        Some("check") => check(args),
        _ => Err("usage: pgaegis table <list|check> --db DATABASE [--schema public] [--table TABLE]".into()),
    }
}

fn list(args: &[String]) -> Result<(), String> {
    let db=arg(args,"--db").ok_or("--db required")?;
    database::validate_ident_public(db)?;
    let schema=arg(args,"--schema").unwrap_or("public");
    database::validate_ident_public(schema)?;
    let schema_lit=database::quote_literal_public(schema);
    let sql=format!(r#"SELECT schemaname||'|'||relname||'|'||n_live_tup||'|'||pg_size_pretty(pg_total_relation_size(format('%I.%I',schemaname,relname)::regclass))||'|'||n_dead_tup||'|'||COALESCE(to_char(last_autovacuum,'YYYY-MM-DD HH24:MI'),'never')||'|'||COALESCE(to_char(last_analyze,'YYYY-MM-DD HH24:MI'),'never') FROM pg_stat_user_tables WHERE schemaname={schema_lit} ORDER BY pg_total_relation_size(format('%I.%I',schemaname,relname)::regclass) DESC;"#);
    let out=psql_db(db,&sql)?;
    let rows=out.lines().filter(|l|!l.trim().is_empty()).map(|line|{
        let p:Vec<&str>=line.split('|').collect();
        let dead=p.get(4).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
        let live=p.get(2).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
        let health=if dead==0 || (live>0 && dead*100/live<5){ui::badge_ok("Healthy")} else if live>0 && dead*100/live<20{ui::badge_warn("Vacuum")} else {ui::badge_bad("Bloat risk")};
        vec![
            p.first().unwrap_or(&"").to_string(), p.get(1).unwrap_or(&"").to_string(),
            p.get(2).unwrap_or(&"0").to_string(), p.get(3).unwrap_or(&"-").to_string(),
            p.get(4).unwrap_or(&"0").to_string(), health,
        ]
    }).collect::<Vec<_>>();
    ui::table(&format!("{db} / Tables"), &["Schema","Table","Live rows","Size","Dead rows","Health"], &rows);
    ui::note(&format!("Use `pgaegis table check --db {db} --table NAME` for vacuum/analyze details."));
    Ok(())
}

fn check(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").ok_or("--db required")?; let table=arg(args,"--table").ok_or("--table required")?;
    let schema=arg(args,"--schema").unwrap_or("public");
    for v in [db,table,schema]{database::validate_ident_public(v)?;}
    let sl=database::quote_literal_public(schema); let tl=database::quote_literal_public(table);
    let sql=format!(r#"SELECT schemaname||'|'||relname||'|'||n_live_tup||'|'||n_dead_tup||'|'||pg_size_pretty(pg_total_relation_size(format('%I.%I',schemaname,relname)::regclass))||'|'||COALESCE(to_char(last_vacuum,'YYYY-MM-DD HH24:MI'),'never')||'|'||COALESCE(to_char(last_autovacuum,'YYYY-MM-DD HH24:MI'),'never')||'|'||COALESCE(to_char(last_analyze,'YYYY-MM-DD HH24:MI'),'never')||'|'||COALESCE(to_char(last_autoanalyze,'YYYY-MM-DD HH24:MI'),'never') FROM pg_stat_user_tables WHERE schemaname={sl} AND relname={tl};"#);
    let out=psql_db(db,&sql)?; if out.trim().is_empty(){return Err(format!("table {schema}.{table} not found in {db}"));}
    let p:Vec<&str>=out.split('|').collect();
    ui::card("Table Health", &[
        ("Database",db.into()),("Table",format!("{}.{}",p.first().unwrap_or(&schema),p.get(1).unwrap_or(&table))),
        ("Live rows",p.get(2).unwrap_or(&"0").to_string()),("Dead rows",p.get(3).unwrap_or(&"0").to_string()),
        ("Total size",p.get(4).unwrap_or(&"-").to_string()),("Last vacuum",p.get(5).unwrap_or(&"never").to_string()),
        ("Last autovacuum",p.get(6).unwrap_or(&"never").to_string()),("Last analyze",p.get(7).unwrap_or(&"never").to_string()),
        ("Last autoanalyze",p.get(8).unwrap_or(&"never").to_string()),
    ]);
    Ok(())
}

fn psql_db(db:&str,sql:&str)->Result<String,String>{util::output("runuser", &["-u","postgres","--","psql","-d",db,"-Atqc",sql])}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}
