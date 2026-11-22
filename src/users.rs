use crate::{database, ui, util};

pub fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("check") => check(args),
        Some("create") => create(args),
        _ => Err("usage: pgaegis user <list|check|create> [options]".into()),
    }
}

fn list() -> Result<(), String> {
    let sql = "SELECT rolname||'|'||rolcanlogin||'|'||rolsuper||'|'||rolcreatedb||'|'||rolcreaterole||'|'||rolreplication||'|'||COALESCE(rolconnlimit::text,'-1') FROM pg_roles ORDER BY rolname;";
    let out = util::output("runuser", &["-u","postgres","--","psql","-Atqc",sql])?;
    let rows = out.lines().filter(|l| !l.trim().is_empty()).map(|line| {
        let p: Vec<&str> = line.split('|').collect();
        vec![
            p.first().unwrap_or(&"").to_string(),
            yesno(p.get(1).copied().unwrap_or("f")),
            yesno(p.get(2).copied().unwrap_or("f")),
            yesno(p.get(3).copied().unwrap_or("f")),
            yesno(p.get(4).copied().unwrap_or("f")),
            yesno(p.get(5).copied().unwrap_or("f")),
            p.get(6).unwrap_or(&"-1").to_string(),
        ]
    }).collect::<Vec<_>>();
    ui::table("PostgreSQL Users & Roles", &["Role","Login","Superuser","Create DB","Create Role","Replication","Conn limit"], &rows);
    ui::success(&format!("{} role(s) found", rows.len()));
    Ok(())
}

fn check(args: &[String]) -> Result<(), String> {
    let name = arg(args,"--name").ok_or("--name required")?;
    database::validate_ident_public(name)?;
    let lit = database::quote_literal_public(name);
    let sql = format!("SELECT rolname||'|'||rolcanlogin||'|'||rolsuper||'|'||rolcreatedb||'|'||rolcreaterole||'|'||rolreplication||'|'||COALESCE(rolvaliduntil::text,'never') FROM pg_roles WHERE rolname={lit};");
    let out = util::output("runuser", &["-u","postgres","--","psql","-Atqc",&sql])?;
    if out.trim().is_empty() { return Err(format!("role {name} does not exist")); }
    let p: Vec<&str> = out.split('|').collect();
    ui::card("PostgreSQL Role", &[
        ("Role", p.first().unwrap_or(&"").to_string()),
        ("Login", yesno(p.get(1).copied().unwrap_or("f"))),
        ("Superuser", yesno(p.get(2).copied().unwrap_or("f"))),
        ("Create DB", yesno(p.get(3).copied().unwrap_or("f"))),
        ("Create Role", yesno(p.get(4).copied().unwrap_or("f"))),
        ("Replication", yesno(p.get(5).copied().unwrap_or("f"))),
        ("Valid until", p.get(6).unwrap_or(&"never").to_string()),
        ("Password", "hidden by design".into()),
    ]);
    Ok(())
}

fn create(args: &[String]) -> Result<(), String> {
    database::create_role_public(args)
}

fn yesno(v:&str)->String { if v=="t" { ui::badge_ok("Yes") } else { ui::dim("No") } }
fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str)}
