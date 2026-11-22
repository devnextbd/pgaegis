use crate::{ui,util};
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("check") => check_db(args),
        Some("create") => create_db(args),
        Some("role") if args.get(1).map(String::as_str)==Some("create") => create_role_public(&args[1..]),
        Some("backup") => logical_backup(args),
        Some("restore") => logical_restore(args),
        _ => Err("usage: pgaegis database <list|check|create|role create|backup|restore> [options]".into())
    }
}

fn list()->Result<(),String>{
    let sql="SELECT d.datname||'|'||pg_get_userbyid(d.datdba)||'|'||pg_size_pretty(pg_database_size(d.datname))||'|'||(SELECT count(*) FROM pg_stat_activity a WHERE a.datname=d.datname)||'|'||CASE WHEN d.datallowconn THEN 'online' ELSE 'disabled' END FROM pg_database d WHERE d.datistemplate=false ORDER BY d.datname;";
    let out=util::output("runuser", &["-u","postgres","--","psql","-Atqc",sql])?;
    let rows=out.lines().filter(|l|!l.trim().is_empty()).map(|line|{
        let p:Vec<&str>=line.split('|').collect();
        vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"").to_string(),p.get(2).unwrap_or(&"-").to_string(),p.get(3).unwrap_or(&"0").to_string(),if p.get(4).copied()==Some("online"){ui::badge_ok("Online")}else{ui::badge_warn("Disabled")} ]
    }).collect::<Vec<_>>();
    ui::table("PostgreSQL Databases", &["Database","Owner","Size","Connections","Status"], &rows);
    ui::success(&format!("{} database(s) found",rows.len()));
    Ok(())
}

fn check_db(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").ok_or("--db required")?; validate_ident_public(db)?;
    let sql=format!("SELECT d.datname||'|'||pg_get_userbyid(d.datdba)||'|'||pg_size_pretty(pg_database_size(d.datname))||'|'||pg_encoding_to_char(d.encoding)||'|'||d.datcollate||'|'||(SELECT count(*) FROM pg_stat_activity a WHERE a.datname=d.datname)||'|'||d.datconnlimit||'|'||d.datallowconn FROM pg_database d WHERE d.datname={};",quote_literal_public(db));
    let out=util::output("runuser", &["-u","postgres","--","psql","-Atqc",&sql])?;
    if out.trim().is_empty(){return Err(format!("database {db} does not exist"));}
    let p:Vec<&str>=out.split('|').collect();
    ui::card("PostgreSQL Database", &[
        ("Database",p.first().unwrap_or(&db).to_string()),("Owner",p.get(1).unwrap_or(&"-").to_string()),
        ("Size",p.get(2).unwrap_or(&"-").to_string()),("Encoding",p.get(3).unwrap_or(&"-").to_string()),
        ("Collation",p.get(4).unwrap_or(&"-").to_string()),("Connections",p.get(5).unwrap_or(&"0").to_string()),
        ("Connection limit",p.get(6).unwrap_or(&"-1").to_string()),("Status",if p.get(7).copied()==Some("t"){ui::badge_ok("Online")}else{ui::badge_warn("Connections disabled")}),
    ]); Ok(())
}

fn create_db(args:&[String])->Result<(),String>{
    util::require_root()?; ensure_writable_primary()?;
    let db=arg(args,"--db").ok_or("--db required")?;
    let owner=arg(args,"--owner").ok_or("--owner required")?;
    validate_ident_public(db)?; validate_ident_public(owner)?;
    if !role_exists(owner)? {
        let pw=util::prompt_secret_confirm(&format!("Password for new PostgreSQL role {owner}"))?;
        create_role(owner,&pw)?;
    }
    if db_exists(db)? {return Err(format!("database {db} already exists; pgaegis will not overwrite it"));}
    psql(&format!("CREATE DATABASE {} OWNER {} ENCODING 'UTF8';",quote_ident(db),quote_ident(owner)))?;
    ui::success(&format!("Database \"{db}\" created successfully"));
    ui::card("PostgreSQL", &[("Database",db.to_string()),("Owner",owner.to_string()),("Password","hidden by design".into()),("Status",ui::badge_ok("Ready"))]); Ok(())
}

pub fn create_role_public(args:&[String])->Result<(),String>{
    util::require_root()?; ensure_writable_primary()?;
    let name=arg(args,"--name").ok_or("--name required")?; validate_ident_public(name)?;
    if role_exists(name)? {return Err(format!("role {name} already exists; password was not changed"));}
    let pw=util::prompt_secret_confirm(&format!("Password for PostgreSQL role {name}"))?;
    create_role(name,&pw)
}

fn create_role(name:&str,password:&str)->Result<(),String>{
    validate_ident_public(name)?; util::reject_control_chars("password",password)?;
    let sql=format!("CREATE ROLE {} LOGIN PASSWORD {} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;",quote_ident(name),quote_literal_public(password));
    psql(&sql)?; ui::success(&format!("Role {name} created with least-privilege defaults")); Ok(())
}

fn logical_backup(args:&[String])->Result<(),String>{
    util::require_root()?;
    let file=arg(args,"--file").ok_or("--file required")?;
    util::validate_abs_path("--file",file)?;
    if Path::new(file).exists(){return Err("backup destination already exists; refusing overwrite".into());}
    if let Some(parent)=Path::new(file).parent(){
        if !parent.exists(){fs::create_dir_all(parent).map_err(|e|e.to_string())?; util::status("chown", &["postgres:postgres",parent.to_str().ok_or("backup parent path is not UTF-8")?])?;}
        let parent_s=parent.to_str().ok_or("backup parent path is not UTF-8")?;
        let writable=Command::new("runuser").args(["-u","postgres","--","test","-w",parent_s]).status().map(|x|x.success()).unwrap_or(false);
        if !writable{return Err(format!("backup directory is not writable by postgres: {parent_s}"));}
    }
    if args.iter().any(|x|x=="--all-sql") {
        util::status("runuser", &["-u","postgres","--","pg_dumpall","--clean","--if-exists","--file",file])?;
        ui::success(&format!("Full logical cluster SQL written to {file}")); return Ok(());
    }
    let db=arg(args,"--db").ok_or("--db required unless --all-sql is used")?; validate_ident_public(db)?;
    util::status("runuser", &["-u","postgres","--","pg_dump","--format=custom","--compress=6","--file",file,db])?;
    ui::success(&format!("Logical database backup written to {file}")); Ok(())
}

fn logical_restore(args:&[String])->Result<(),String>{
    util::require_root()?; ensure_writable_primary()?;
    let file=arg(args,"--file").ok_or("--file required")?; util::validate_abs_path("--file",file)?;
    if !Path::new(file).is_file(){return Err(format!("restore file not found: {file}"));}
    if args.iter().any(|x|x=="--full-sql") {
        if !args.iter().any(|x|x=="--acknowledge-cluster-sql"){return Err("full cluster SQL can create/drop roles and databases; add --acknowledge-cluster-sql after reviewing the file".into());}
        util::status("runuser", &["-u","postgres","--","psql","-v","ON_ERROR_STOP=1","-d","postgres","-f",file])?;
        ui::success("Full SQL restore completed. Review roles, ownership and extensions."); return Ok(());
    }
    let db=arg(args,"--db").ok_or("--db required")?; let owner=arg(args,"--owner").ok_or("--owner required")?;
    validate_ident_public(db)?; validate_ident_public(owner)?;
    if !role_exists(owner)? {
        let pw=util::prompt_secret_confirm(&format!("Password for new PostgreSQL role {owner}"))?;
        create_role(owner,&pw)?;
    }
    if db_exists(db)? {
        if !args.iter().any(|x|x=="--replace"){return Err(format!("target database {db} exists. Restore to a new name or explicitly add --replace"));}
        psql(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname={} AND pid<>pg_backend_pid();",quote_literal_public(db)))?;
        psql(&format!("DROP DATABASE {};",quote_ident(db)))?;
    }
    psql(&format!("CREATE DATABASE {} OWNER {} ENCODING 'UTF8';",quote_ident(db),quote_ident(owner)))?;
    let custom=Command::new("runuser").args(["-u","postgres","--","pg_restore","--list",file]).output().map(|o|o.status.success()).unwrap_or(false);
    let restore=if custom {
        util::status("runuser", &["-u","postgres","--","pg_restore","--exit-on-error","--no-owner","--role",owner,"--dbname",db,file])
    } else {
        util::status("runuser", &["-u","postgres","--","psql","-v","ON_ERROR_STOP=1","--dbname",db,"--file",file])
    };
    if let Err(e)=restore { return Err(format!("restore failed after creating target database {db}; target was preserved for inspection: {e}")); }
    ui::success(&format!("Restore completed into {db} with owner {owner}")); Ok(())
}

fn ensure_writable_primary()->Result<(),String>{
    let out=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';"])?;
    if out.trim()!="t" {return Err("database mutation is allowed only on the current writable Primary".into());}
    Ok(())
}
fn role_exists(name:&str)->Result<bool,String>{Ok(util::output("runuser", &["-u","postgres","--","psql","-Atqc",&format!("SELECT 1 FROM pg_roles WHERE rolname={};",quote_literal_public(name))])?.trim()=="1")}
fn db_exists(name:&str)->Result<bool,String>{Ok(util::output("runuser", &["-u","postgres","--","psql","-Atqc",&format!("SELECT 1 FROM pg_database WHERE datname={};",quote_literal_public(name))])?.trim()=="1")}
fn psql(sql:&str)->Result<(),String>{util::status("runuser", &["-u","postgres","--","psql","-v","ON_ERROR_STOP=1","-d","postgres","-c",sql])}
fn quote_ident(s:&str)->String{format!("\"{}\"",s.replace('"',"\"\""))}
pub fn quote_literal_public(s:&str)->String{format!("'{}'",s.replace('\'',"''"))}
pub fn validate_ident_public(s:&str)->Result<(),String>{
    if s.is_empty()||s.len()>63{return Err("PostgreSQL identifier must be 1..63 bytes".into());}
    let mut it=s.chars(); let first=it.next().unwrap();
    if !(first.is_ascii_alphabetic()||first=='_') || !it.all(|c|c.is_ascii_alphanumeric()||c=='_'){return Err(format!("invalid PostgreSQL identifier: {s}"));}
    Ok(())
}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}
