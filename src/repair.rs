use crate::{ui,util};
use std::fs;
use std::path::Path;

pub fn dispatch(args:&[String])->Result<(),String>{
    let fix=args.iter().any(|x|x=="--fix");
    let mut issues=Vec::new();
    if !Path::new("/etc/pgaegis/pgaegis.conf").exists(){issues.push("missing /etc/pgaegis/pgaegis.conf");}
    if !Path::new("/var/lib/pgaegis/certification").exists(){issues.push("missing certification state directory");}
    if fix {
        util::require_root()?;
        fs::create_dir_all("/etc/pgaegis").map_err(|e|e.to_string())?;
        fs::create_dir_all("/var/lib/pgaegis/certification").map_err(|e|e.to_string())?;
        if !Path::new("/etc/pgaegis/pgaegis.conf").exists() {
            let default="/usr/share/pgaegis/config/pgaegis.conf.example";
            if Path::new(default).exists(){fs::copy(default,"/etc/pgaegis/pgaegis.conf").map_err(|e|e.to_string())?; let _=util::status("chmod", &["640","/etc/pgaegis/pgaegis.conf"]);}
        }
        if util::command_exists("systemctl") { let _=util::status("systemctl", &["daemon-reload"]); }
        ui::success("Repair actions applied. PostgreSQL data directories were not modified.");
        return Ok(());
    }
    if issues.is_empty(){ui::success("REPAIR CHECK: PASS");Ok(())}else{for x in issues{ui::warning(x);}Err("repairable pgaegis installation issues found; run pgaegis repair --fix".into())}
}
