use crate::{config::Config,ui,util};

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        None => list_available(args),
        Some("list") => list_available(args),
        Some("installed") => list_installed(args),
        Some("enable") => { util::require_root()?; let name=args.get(1).ok_or("extension name required")?; let db=arg(args,"--db").unwrap_or("postgres"); validate_ident(name)?; validate_ident(db)?; ensure_primary()?; util::status("runuser", &["-u","postgres","--","psql","-d",db,"-v","ON_ERROR_STOP=1","-c",&format!("CREATE EXTENSION IF NOT EXISTS \"{}\";",name)])?; ui::success(&format!("Extension {name} enabled in database {db}")); Ok(()) },
        Some("preload") => preload(args),
        _ => Err("usage: pgaegis extension <list|installed|enable NAME|preload NAME> [--db DB]".into())
    }
}

fn list_available(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").unwrap_or("postgres"); validate_ident(db)?;
    let out=util::output("runuser", &["-u","postgres","--","psql","-d",db,"-Atc","SELECT name||'|'||default_version||'|'||COALESCE(installed_version,'') FROM pg_available_extensions ORDER BY name;"])?;
    let rows=out.lines().filter(|x|!x.trim().is_empty()).map(|l|{let p:Vec<&str>=l.split('|').collect(); vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"-").to_string(),if p.get(2).copied().unwrap_or("").is_empty(){ui::dim("Not enabled")}else{ui::badge_ok(p.get(2).copied().unwrap_or("enabled"))}]}).collect::<Vec<_>>();
    ui::table(&format!("{db} / Available Extensions"), &["Extension","Default","Installed"], &rows); Ok(())
}

fn list_installed(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").unwrap_or("postgres"); validate_ident(db)?;
    let out=util::output("runuser", &["-u","postgres","--","psql","-d",db,"-Atc","SELECT extname||'|'||extversion FROM pg_extension ORDER BY extname;"])?;
    let rows=out.lines().filter(|x|!x.trim().is_empty()).map(|l|{let p:Vec<&str>=l.split('|').collect(); vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"-").to_string(),ui::badge_ok("Enabled")]}).collect::<Vec<_>>();
    ui::table(&format!("{db} / Enabled Extensions"), &["Extension","Version","Status"], &rows); Ok(())
}

fn preload(args:&[String])->Result<(),String>{
    util::require_root()?; let name=args.get(1).ok_or("extension/library name required")?; validate_ident(name)?;
    let cfg=Config::load();
    if !util::command_exists("patronictl"){return Err("Patroni is required for cluster-safe shared_preload_libraries changes".into());}
    let current=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SHOW shared_preload_libraries;"]).unwrap_or_default();
    let mut libs:Vec<String>=current.split(',').map(str::trim).filter(|x|!x.is_empty()).map(str::to_string).collect();
    if !libs.iter().any(|x|x==name){libs.push(name.clone());}
    libs.sort(); libs.dedup(); let value=libs.join(",");
    util::status("patronictl", &["-c",&cfg.patroni_config,"edit-config",&cfg.cluster_name,"--set",&format!("postgresql.parameters.shared_preload_libraries={value}"),"--force"])?;
    ui::success(&format!("shared_preload_libraries updated to: {value}"));
    ui::warning("A controlled rolling restart is required before enabling preload-dependent extensions.");
    Ok(())
}
fn ensure_primary()->Result<(),String>{let x=util::output("runuser", &["-u","postgres","--","psql","-Atqc","SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';"])?; if x.trim()=="t"{Ok(())}else{Err("extension mutation is allowed only on writable Primary".into())}}
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{ a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str) }
fn validate_ident(s:&str)->Result<(),String>{if !s.is_empty()&&s.len()<=63&&s.chars().all(|c|c.is_ascii_alphanumeric()||c=='_') {Ok(())} else {Err("invalid PostgreSQL identifier".into())}}
