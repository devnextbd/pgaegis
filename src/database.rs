use crate::pg::{self, quote_ident, quote_literal, validate_ident};
use crate::{scram, ui, util};
use crate::util::{arg, has_flag};
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
    let out=pg::query("postgres",sql)?;
    let rows=out.lines().filter(|l|!l.trim().is_empty()).map(|line|{
        let p:Vec<&str>=line.split('|').collect();
        vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"").to_string(),p.get(2).unwrap_or(&"-").to_string(),p.get(3).unwrap_or(&"0").to_string(),if p.get(4).copied()==Some("online"){ui::badge_ok("Online")}else{ui::badge_warn("Disabled")} ]
    }).collect::<Vec<_>>();
    ui::table("PostgreSQL Databases", &["Database","Owner","Size","Connections","Status"], &rows);
    ui::success(&format!("{} database(s) found",rows.len()));
    Ok(())
}

fn check_db(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").ok_or("--db required")?; validate_ident(db)?;
    let sql=format!("SELECT d.datname||'|'||pg_get_userbyid(d.datdba)||'|'||pg_size_pretty(pg_database_size(d.datname))||'|'||pg_encoding_to_char(d.encoding)||'|'||d.datcollate||'|'||(SELECT count(*) FROM pg_stat_activity a WHERE a.datname=d.datname)||'|'||d.datconnlimit||'|'||d.datallowconn FROM pg_database d WHERE d.datname={};",quote_literal(db));
    let out=pg::query("postgres",&sql)?;
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
    util::require_root()?; pg::ensure_writable_primary("database mutation")?;
    let db=arg(args,"--db").ok_or("--db required")?;
    let owner=arg(args,"--owner").ok_or("--owner required")?;
    validate_ident(db)?; validate_ident(owner)?;
    if pg::db_exists(db)? {return Err(format!("database {db} already exists; pgaegis will not overwrite it"));}
    ensure_owner_role(owner)?;
    pg::exec("postgres",&format!("CREATE DATABASE {} OWNER {} ENCODING 'UTF8';",quote_ident(db),quote_ident(owner)))?;
    ui::success(&format!("Database \"{db}\" created successfully"));
    ui::card("PostgreSQL", &[("Database",db.to_string()),("Owner",owner.to_string()),("Password","hidden by design".into()),("Status",ui::badge_ok("Ready"))]); Ok(())
}

pub fn create_role_public(args:&[String])->Result<(),String>{
    util::require_root()?; pg::ensure_writable_primary("role creation")?;
    let name=arg(args,"--name").ok_or("--name required")?; validate_ident(name)?;
    if pg::role_exists(name)? {return Err(format!("role {name} already exists; password was not changed"));}
    let pw=util::prompt_secret_confirm(&format!("Password for PostgreSQL role {name}"))?;
    create_role(name,&pw)
}

fn ensure_owner_role(owner:&str)->Result<(),String>{
    if !pg::role_exists(owner)? {
        let pw=util::prompt_secret_confirm(&format!("Password for new PostgreSQL role {owner}"))?;
        create_role(owner,&pw)?;
    }
    Ok(())
}

/// The password is hashed client-side into a SCRAM-SHA-256 verifier and the
/// statement is sent over stdin: the plaintext never appears in argv, the
/// server log (log_statement=ddl) or pg_stat_activity.
fn create_role(name:&str,password:&str)->Result<(),String>{
    validate_ident(name)?; util::reject_control_chars("password",password)?;
    let verifier=scram::verifier(password)?;
    let sql=format!("CREATE ROLE {} LOGIN PASSWORD {} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;",quote_ident(name),quote_literal(&verifier));
    pg::exec("postgres",&sql)?; ui::success(&format!("Role {name} created with least-privilege defaults (SCRAM-SHA-256)")); Ok(())
}

fn logical_backup(args:&[String])->Result<(),String>{
    util::require_root()?;
    let file=arg(args,"--file").ok_or("--file required")?;
    util::validate_abs_path("--file",file)?;
    if Path::new(file).exists(){return Err("backup destination already exists; refusing overwrite".into());}
    if let Some(parent)=Path::new(file).parent(){
        let parent_s=parent.to_str().ok_or("backup parent path is not UTF-8")?;
        if !parent.exists(){fs::create_dir_all(parent).map_err(|e|e.to_string())?; util::status("chown", &["postgres:postgres",parent_s])?;}
        let writable=Command::new("runuser").args(["-u","postgres","--","test","-w",parent_s]).status().map(|x|x.success()).unwrap_or(false);
        if !writable{return Err(format!("backup directory is not writable by postgres: {parent_s}"));}
    }
    if has_flag(args,"--all-sql") {
        util::status("runuser", &["-u","postgres","--","pg_dumpall","--clean","--if-exists","--file",file])?;
        ui::success(&format!("Full logical cluster SQL written to {file}")); return Ok(());
    }
    let db=arg(args,"--db").ok_or("--db required unless --all-sql is used")?; validate_ident(db)?;
    util::status("runuser", &["-u","postgres","--","pg_dump","--format=custom","--compress=6","--file",file,db])?;
    ui::success(&format!("Logical database backup written to {file}")); Ok(())
}

fn is_custom_dump(file:&str)->bool{
    Command::new("runuser").args(["-u","postgres","--","pg_restore","--list",file]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s|s.success()).unwrap_or(false)
}

/// Name for a temporary database that is guaranteed to fit in 63 bytes.
fn side_name(kind:&str,db:&str)->String{
    let n=format!("{db}_pgaegis_{kind}_{}",util::unix_now());
    if n.len()<=63 { n } else { format!("pgaegis_{kind}_{}",util::unix_now()) }
}

fn restore_into(target:&str,owner:&str,file:&str,custom:bool)->Result<(),String>{
    if custom {
        util::status("runuser", &["-u","postgres","--","pg_restore","--exit-on-error","--single-transaction","--no-owner","--role",owner,"--dbname",target,file])
    } else {
        // Objects are created as the owner role, matching the custom-format path.
        let opts=format!("-c role={owner}");
        util::status("runuser", &["-u","postgres","--","env",&format!("PGOPTIONS={opts}"),"psql","-X","-v","ON_ERROR_STOP=1","--single-transaction","--dbname",target,"--file",file])
    }
}

fn logical_restore(args:&[String])->Result<(),String>{
    util::require_root()?; pg::ensure_writable_primary("database restore")?;
    let file=arg(args,"--file").ok_or("--file required")?; util::validate_abs_path("--file",file)?;
    if !Path::new(file).is_file(){return Err(format!("restore file not found: {file}"));}
    let readable=Command::new("runuser").args(["-u","postgres","--","test","-r",file]).status().map(|x|x.success()).unwrap_or(false);
    if !readable{return Err(format!("restore file is not readable by postgres: {file}"));}
    if has_flag(args,"--full-sql") {
        if !has_flag(args,"--acknowledge-cluster-sql"){return Err("full cluster SQL can create/drop roles and databases; add --acknowledge-cluster-sql after reviewing the file".into());}
        util::status("runuser", &["-u","postgres","--","psql","-X","-v","ON_ERROR_STOP=1","-d","postgres","-f",file])?;
        ui::success("Full SQL restore completed. Review roles, ownership and extensions."); return Ok(());
    }
    let db=arg(args,"--db").ok_or("--db required")?; let owner=arg(args,"--owner").ok_or("--owner required")?;
    validate_ident(db)?; validate_ident(owner)?;

    // Validate the input BEFORE touching any existing database.
    let custom=is_custom_dump(file);
    if !custom && !has_flag(args,"--acknowledge-plain-sql") {
        return Err("file is not a pg_dump custom archive, so it would be executed as plain SQL by psql (meta-commands such as \\! run OS commands as the postgres user). Review the file, then add --acknowledge-plain-sql".into());
    }
    let exists=pg::db_exists(db)?;
    let replace=has_flag(args,"--replace");
    if exists && !replace {return Err(format!("target database {db} exists. Restore to a new name or explicitly add --replace"));}
    ensure_owner_role(owner)?;

    // Restore into a staging database; the live database is only swapped
    // after the restore succeeded completely.
    let target=if exists { side_name("restore",db) } else { db.to_string() };
    if exists && pg::db_exists(&target)? {return Err(format!("staging database {target} already exists"));}
    // template0 is the documented clean template for restoring pg_dump output.
    pg::exec("postgres",&format!("CREATE DATABASE {} OWNER {} ENCODING 'UTF8' TEMPLATE template0;",quote_ident(&target),quote_ident(owner)))?;
    if let Err(e)=restore_into(&target,owner,file,custom) {
        return Err(if exists {
            format!("restore failed; the existing database {db} was NOT modified. Staging database {target} kept for inspection (drop it when done): {e}")
        } else {
            format!("restore failed after creating target database {db}; target was preserved for inspection: {e}")
        });
    }
    if exists {
        let old=side_name("replaced",db);
        let (dbq,oldq,tq,dbl)=(quote_ident(db),quote_ident(&old),quote_ident(&target),quote_literal(db));
        pg::exec("postgres",&format!(
            "ALTER DATABASE {dbq} ALLOW_CONNECTIONS false;\n\
             SELECT pg_terminate_backend(pid, 10000) FROM pg_stat_activity WHERE datname={dbl} AND pid<>pg_backend_pid();\n\
             ALTER DATABASE {dbq} RENAME TO {oldq};\n\
             ALTER DATABASE {oldq} ALLOW_CONNECTIONS true;\n\
             ALTER DATABASE {tq} RENAME TO {dbq};\n"))
            .map_err(|e|format!("swap failed; restored data is in {target}. If {db} now refuses connections run: ALTER DATABASE {dbq} ALLOW_CONNECTIONS true; ({e})"))?;
        ui::note(&format!("Previous database kept as {old}. Drop it after verification: DROP DATABASE {oldq};"));
    }
    ui::success(&format!("Restore completed into {db} with owner {owner}")); Ok(())
}
