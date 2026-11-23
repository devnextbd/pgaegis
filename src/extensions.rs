use crate::{config::Config,pg,ui,util};
use crate::util::arg;

pub fn dispatch(args:&[String])->Result<(),String>{
    match args.first().map(String::as_str) {
        None => list_available(args),
        Some("list") => list_available(args),
        Some("installed") => list_installed(args),
        Some("enable") => enable(args),
        Some("preload") => preload(args),
        _ => Err("usage: pgaegis extension <list|installed|enable NAME|preload NAME> [--db DB]".into())
    }
}

fn enable(args:&[String])->Result<(),String>{
    util::require_root()?;
    let name=args.get(1).ok_or("extension name required")?; let db=arg(args,"--db").unwrap_or("postgres");
    validate_ext(name)?; pg::validate_ident(db)?;
    pg::ensure_writable_primary("extension mutation")?;
    pg::exec(db,&format!("CREATE EXTENSION IF NOT EXISTS {};",pg::quote_ident(name)))?;
    ui::success(&format!("Extension {name} enabled in database {db}")); Ok(())
}

fn list_available(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").unwrap_or("postgres"); pg::validate_ident(db)?;
    let out=pg::query(db,"SELECT name||'|'||default_version||'|'||COALESCE(installed_version,'') FROM pg_available_extensions ORDER BY name;")?;
    let rows=out.lines().filter(|x|!x.trim().is_empty()).map(|l|{let p:Vec<&str>=l.split('|').collect(); vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"-").to_string(),if p.get(2).copied().unwrap_or("").is_empty(){ui::dim("Not enabled")}else{ui::badge_ok(p.get(2).copied().unwrap_or("enabled"))}]}).collect::<Vec<_>>();
    ui::table(&format!("{db} / Available Extensions"), &["Extension","Default","Installed"], &rows); Ok(())
}

fn list_installed(args:&[String])->Result<(),String>{
    let db=arg(args,"--db").unwrap_or("postgres"); pg::validate_ident(db)?;
    let out=pg::query(db,"SELECT extname||'|'||extversion FROM pg_extension ORDER BY extname;")?;
    let rows=out.lines().filter(|x|!x.trim().is_empty()).map(|l|{let p:Vec<&str>=l.split('|').collect(); vec![p.first().unwrap_or(&"").to_string(),p.get(1).unwrap_or(&"-").to_string(),ui::badge_ok("Enabled")]}).collect::<Vec<_>>();
    ui::table(&format!("{db} / Enabled Extensions"), &["Extension","Version","Status"], &rows); Ok(())
}

/// Extract `postgresql.parameters.shared_preload_libraries` from
/// `patronictl show-config` YAML. `Ok(None)` means the key is not set.
pub fn preload_from_show_config(yaml:&str)->Option<Vec<String>>{
    let line=yaml.lines().find(|l|l.trim_start().starts_with("shared_preload_libraries:"))?;
    let raw=line.trim_start().trim_start_matches("shared_preload_libraries:").trim();
    let raw=raw.trim_matches(|c|c=='\''||c=='"');
    Some(raw.split(',').map(str::trim).map(|x|x.trim_matches('"')).filter(|x|!x.is_empty()).map(str::to_string).collect())
}

fn preload(args:&[String])->Result<(),String>{
    util::require_root()?; let name=args.get(1).ok_or("extension/library name required")?; validate_ext(name)?;
    let cfg=Config::load();
    if !util::command_exists("patronictl"){return Err("Patroni is required for cluster-safe shared_preload_libraries changes".into());}
    // The DCS is the source of truth (it also contains changes still pending a
    // restart). A read failure aborts instead of overwriting the value.
    let dcs=util::output("patronictl", &["-c",&cfg.patroni_config,"show-config",&cfg.cluster_name])
        .map_err(|e|format!("cannot read current Patroni DCS config; refusing to change shared_preload_libraries: {e}"))?;
    let mut libs=preload_from_show_config(&dcs).unwrap_or_default();
    if libs.iter().any(|x|x==name){ ui::success(&format!("{name} is already in shared_preload_libraries")); return Ok(()); }
    libs.push(name.clone());
    let value=libs.join(",");
    util::status("patronictl", &["-c",&cfg.patroni_config,"edit-config",&cfg.cluster_name,"--set",&format!("postgresql.parameters.shared_preload_libraries={value}"),"--force"])?;
    ui::success(&format!("shared_preload_libraries updated to: {value}"));
    ui::warning("A controlled rolling restart is required before enabling preload-dependent extensions.");
    Ok(())
}

/// Extension/library names: letters, digits, `_` and `-` (e.g. "uuid-ossp").
fn validate_ext(s:&str)->Result<(),String>{
    if !s.is_empty() && s.len()<=63 && s.chars().next().is_some_and(|c|c.is_ascii_alphabetic()||c=='_') && s.chars().all(|c|c.is_ascii_alphanumeric()||c=='_'||c=='-') {Ok(())} else {Err(format!("invalid extension/library name: {s}"))}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_preload_from_dcs_yaml() {
        let y="loop_wait: 10\npostgresql:\n  parameters:\n    shared_preload_libraries: pg_stat_statements,auto_explain\n    wal_level: replica\n";
        assert_eq!(preload_from_show_config(y), Some(vec!["pg_stat_statements".to_string(),"auto_explain".to_string()]));
        assert_eq!(preload_from_show_config("    shared_preload_libraries: 'timescaledb'\n"), Some(vec!["timescaledb".to_string()]));
        assert_eq!(preload_from_show_config("ttl: 30\n"), None);
    }

    #[test]
    fn extension_names() {
        assert!(validate_ext("uuid-ossp").is_ok());
        assert!(validate_ext("pg_stat_statements").is_ok());
        assert!(validate_ext("x\";drop").is_err());
        assert!(validate_ext("-x").is_err());
    }
}
