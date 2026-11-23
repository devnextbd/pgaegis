mod backup;
mod certify;
mod config;
mod doctor;
mod database;
mod extensions;
mod ha;
mod json;
mod monitor;
mod optimize;
mod pg;
mod repair;
mod routing;
mod scram;
mod system;
mod tables;
mod ui;
mod updater;
mod users;
mod util;

use std::env;

const FULL_FORM: &str = "PostgreSQL Availability, Governance, Engineering, Integrity & Safety";
const TAGLINE: &str = "Production PostgreSQL Operations, HA & Protection";

fn usage() {
    ui::header(TAGLINE);
    println!("{}", ui::brand(&format!("pgaegis {}", env!("CARGO_PKG_VERSION"))));
    println!("{}\n", ui::dim(FULL_FORM));
    println!(r#"USAGE
  pgaegis                              Interactive management console
  pgaegis info                         Platform and PostgreSQL summary
  pgaegis about                        Product name, full form and paths
  pgaegis doctor                       Dependency/service health checks

DATA
  pgaegis database list
  pgaegis database check --db DB
  pgaegis database create --db DB --owner USER
  pgaegis database backup --db DB --file /absolute/path.dump
  pgaegis database restore --db DB --owner USER --file /absolute/path.dump
  pgaegis user list
  pgaegis user check --name USER
  pgaegis user create --name USER
  pgaegis table list --db DB [--schema public]
  pgaegis table check --db DB --table TABLE [--schema public]
  pgaegis extension <list|installed|enable|preload> [args]

PRODUCTION
  pgaegis system <install|enable-monitoring|resources|swap> [args]
  pgaegis replica                         Interactive HA wizard
  pgaegis replica ha <setup|activate|status|audit|switchover|failover|rejoin> [args]
  pgaegis routing <check|render> [args]
  pgaegis backup <setup-s3|schedule-enable|check|full|diff|restore-drill> [args]
  pgaegis monitor <check [--alert]|test-alert>
  pgaegis optimize <report|recommend|analyze|autovacuum|queries|locks> [args]
  pgaegis repair [--fix]
  pgaegis update [--check|--force]
  pgaegis certify <check|report|lab>

OUTPUT
  Colors are automatically disabled for non-interactive terminals.
  Set NO_COLOR=1 for plain output.

The HA controller is Patroni. pgaegis does not implement a competing leader election.
"#);
}

fn about() -> Result<(), String> {
    ui::card("pgaegis", &[
        ("Full form", FULL_FORM.into()),
        ("Meaning", "Aegis = shield, protection, safeguard".into()),
        ("Purpose", TAGLINE.into()),
        ("CLI", "pgaegis".into()),
        ("Config", "/etc/pgaegis/".into()),
        ("State", "/var/lib/pgaegis/".into()),
        ("Logs", "/var/log/pgaegis/".into()),
        ("Version", env!("CARGO_PKG_VERSION").into()),
    ]);
    Ok(())
}

fn info() -> Result<(), String> {
    let cfg = config::Config::load();
    let pg_ready = if util::command_exists("pg_isready") && std::process::Command::new("pg_isready").arg("-q").status().map(|s|s.success()).unwrap_or(false) { ui::badge_ok("Ready") } else { ui::badge_warn("Not ready") };
    let role = pg::query("postgres","SELECT CASE WHEN pg_is_in_recovery() THEN 'Standby' ELSE 'Primary' END;").unwrap_or_else(|_| "Unavailable".into());
    ui::card("pgaegis / System", &[
        ("Version", env!("CARGO_PKG_VERSION").into()),
        ("OS", util::os_pretty_name()),
        ("Architecture", std::env::consts::ARCH.into()),
        ("PostgreSQL", util::command_version("psql", &["--version"])),
        ("PostgreSQL state", pg_ready),
        ("Node role", role),
        ("Patroni", util::command_version("patroni", &["--version"])),
        ("etcdctl", util::command_version("etcdctl", &["version"])),
        ("pgBackRest", util::command_version("pgbackrest", &["version"])),
        ("HAProxy", util::command_version("haproxy", &["-v"])),
        ("PgBouncer", util::command_version("pgbouncer", &["--version"])),
        ("Cluster", cfg.cluster_name),
        ("Config", "/etc/pgaegis/pgaegis.conf".into()),
    ]);
    Ok(())
}


fn database_menu()->Result<(),String>{
    ui::menu("Database Management", &[("1","List databases"),("2","Check database"),("3","Create database + owner"),("4","Logical backup"),("5","Restore database"),("0","Back")]);
    match util::prompt_line("Select",Some("1"))?.as_str(){
        "0"=>Ok(()),
        "1"=>database::dispatch(&["list".into()]),
        "2"=>{let db=util::prompt_line("Database",None)?;database::dispatch(&["check".into(),"--db".into(),db])},
        "3"=>{let db=util::prompt_line("Database",None)?;let owner=util::prompt_line("Owner role",Some(db.as_str()))?;database::dispatch(&["create".into(),"--db".into(),db,"--owner".into(),owner])},
        "4"=>{let db=util::prompt_line("Database",None)?;let file=util::prompt_line("Absolute backup file",None)?;database::dispatch(&["backup".into(),"--db".into(),db,"--file".into(),file])},
        "5"=>{let db=util::prompt_line("Target database",None)?;let owner=util::prompt_line("Owner role",Some(db.as_str()))?;let file=util::prompt_line("Absolute dump/SQL file",None)?;let mut a=vec!["restore".into(),"--db".into(),db,"--owner".into(),owner,"--file".into(),file];if util::prompt_yes_no("Replace target if it already exists?",false)?{a.push("--replace".into());}database::dispatch(&a)},
        _=>Err("invalid selection".into())
    }
}

fn user_menu()->Result<(),String>{
    ui::menu("Users & Roles", &[("1","List roles"),("2","Check role"),("3","Create login role"),("0","Back")]);
    match util::prompt_line("Select",Some("1"))?.as_str(){
        "0"=>Ok(()),"1"=>users::dispatch(&["list".into()]),
        "2"=>{let n=util::prompt_line("Role name",None)?;users::dispatch(&["check".into(),"--name".into(),n])},
        "3"=>{let n=util::prompt_line("Role name",None)?;users::dispatch(&["create".into(),"--name".into(),n])},
        _=>Err("invalid selection".into())
    }
}

fn table_menu()->Result<(),String>{
    ui::menu("Table Health", &[("1","List tables"),("2","Check one table"),("0","Back")]);
    match util::prompt_line("Select",Some("1"))?.as_str(){
        "0"=>Ok(()),
        "1"=>{let db=util::prompt_line("Database",None)?;let schema=util::prompt_line("Schema",Some("public"))?;tables::dispatch(&["list".into(),"--db".into(),db,"--schema".into(),schema])},
        "2"=>{let db=util::prompt_line("Database",None)?;let schema=util::prompt_line("Schema",Some("public"))?;let table=util::prompt_line("Table",None)?;tables::dispatch(&["check".into(),"--db".into(),db,"--schema".into(),schema,"--table".into(),table])},
        _=>Err("invalid selection".into())
    }
}

fn backup_menu()->Result<(),String>{
    ui::menu("Backup & Recovery", &[("1","Backup health"),("2","Run full pgBackRest backup"),("3","Run differential backup"),("4","Enable backup schedule"),("5","Restore drill"),("0","Back")]);
    match util::prompt_line("Select",Some("1"))?.as_str(){
        "0"=>Ok(()),"1"=>backup::dispatch(&["check".into()]),"2"=>backup::dispatch(&["full".into()]),"3"=>backup::dispatch(&["diff".into()]),"4"=>backup::dispatch(&["schedule-enable".into()]),
        "5"=>{let path=util::prompt_line("Restore drill target",Some("/var/lib/pgaegis/restore-drill/test-01"))?;backup::dispatch(&["restore-drill".into(),"--target-dir".into(),path])},
        _=>Err("invalid selection".into())
    }
}

fn extension_menu()->Result<(),String>{
    ui::menu("PostgreSQL Extensions", &[("1","Show available extensions"),("2","Show enabled extensions"),("3","Enable extension in database"),("4","Add shared preload library"),("0","Back")]);
    match util::prompt_line("Select",Some("1"))?.as_str(){
        "0"=>Ok(()),
        "1"=>{let db=util::prompt_line("Database",Some("postgres"))?;extensions::dispatch(&["list".into(),"--db".into(),db])},
        "2"=>{let db=util::prompt_line("Database",Some("postgres"))?;extensions::dispatch(&["installed".into(),"--db".into(),db])},
        "3"=>{let name=util::prompt_line("Extension",None)?;let db=util::prompt_line("Database",Some("postgres"))?;extensions::dispatch(&["enable".into(),name,"--db".into(),db])},
        "4"=>{let name=util::prompt_line("Library/extension",None)?;extensions::dispatch(&["preload".into(),name])},
        _=>Err("invalid selection".into())
    }
}

fn interactive_main()->Result<(),String>{
    loop {
        ui::menu("Production PostgreSQL Operations, HA & Protection", &[
            ("1", "Overview / info"),
            ("2", "Doctor / health checks"),
            ("3", "Databases"),
            ("4", "Users & roles"),
            ("5", "Tables"),
            ("6", "HA / replication"),
            ("7", "Backup health"),
            ("8", "Extensions"),
            ("9", "Optimization report"),
            ("10", "Production certification"),
            ("11", "System resources"),
            ("0", "Exit"),
        ]);
        let choice=util::prompt_line("Select",Some("1"))?;
        let run = match choice.as_str(){
            "0"=>return Ok(()),
            "1"=>info(),
            "2"=>doctor::run(),
            "3"=>database_menu(),
            "4"=>user_menu(),
            "5"=>table_menu(),
            "6"=>ha::dispatch(&[]),
            "7"=>backup_menu(),
            "8"=>extension_menu(),
            "9"=>optimize::dispatch(&["report".into(),"--db".into(),"postgres".into()]),
            "10"=>certify::dispatch(&["check".into()]),
            "11"=>system::dispatch(&["resources".into()]),
            _=>Err("invalid selection".into()),
        };
        if let Err(e)=run { ui::error(&e); }
        println!();
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => interactive_main(),
        Some("--help") | Some("-h") | Some("help") => { usage(); Ok(()) },
        Some("--version") | Some("version") => { println!("pgaegis {}", env!("CARGO_PKG_VERSION")); Ok(()) },
        Some("about") => about(),
        Some("info") | Some("status") => info(),
        Some("doctor") => doctor::run(),
        Some("database") => database::dispatch(&args[1..]),
        Some("user") | Some("role") => users::dispatch(&args[1..]),
        Some("table") => tables::dispatch(&args[1..]),
        Some("extension") => extensions::dispatch(&args[1..]),
        Some("system") => system::dispatch(&args[1..]),
        Some("replica") => ha::dispatch(&args[1..]),
        Some("routing") => routing::dispatch(&args[1..]),
        Some("backup") => backup::dispatch(&args[1..]),
        Some("monitor") => monitor::dispatch(&args[1..]),
        Some("optimize") => optimize::dispatch(&args[1..]),
        Some("repair") => repair::dispatch(&args[1..]),
        Some("update") => updater::dispatch(&args[1..]),
        Some("certify") => certify::dispatch(&args[1..]),
        Some(x) => Err(format!("unknown command: {x}. Run `pgaegis help`.")),
    };
    if let Err(e) = result {
        ui::error(&e);
        std::process::exit(1);
    }
}
