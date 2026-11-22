use crate::{ui,util};

pub fn run()->Result<(),String>{
    let required=["psql","pg_isready","patroni","patronictl","etcdctl","pgbackrest","haproxy","pgbouncer","curl","openssl"];
    let mut failed=0usize;
    let mut rows=Vec::new();
    for c in required {
        let ok=util::command_exists(c);
        if !ok { failed+=1; }
        rows.push(vec![c.to_string(), if ok {ui::badge_ok("Installed")} else {ui::badge_bad("Missing")}]);
    }
    rows.push(vec!["postgresql service".into(), service_status("postgresql")]);
    rows.push(vec!["patroni service".into(), service_status("patroni")]);
    rows.push(vec!["monitor timer".into(), timer_status("pgaegis-monitor.timer")]);
    rows.push(vec!["backup full timer".into(), timer_status("pgaegis-backup-full.timer")]);
    rows.push(vec!["backup diff timer".into(), timer_status("pgaegis-backup-diff.timer")]);
    ui::table("pgaegis Doctor", &["Check","Status"], &rows);
    if failed>0 {
        ui::warning(&format!("{failed} required production tool(s) are missing"));
        return Err(format!("{failed} required/production tools missing"));
    }
    ui::success("Core production dependencies are available");
    Ok(())
}

fn service_status(name:&str)->String {
    if !util::command_exists("systemctl") { return ui::badge_warn("N/A"); }
    match std::process::Command::new("systemctl").args(["is-active","--quiet",name]).status() {
        Ok(s) if s.success()=>ui::badge_ok("Running"), _=>ui::badge_warn("Not active")
    }
}
fn timer_status(name:&str)->String {
    if !util::command_exists("systemctl") { return ui::badge_warn("N/A"); }
    match std::process::Command::new("systemctl").args(["is-enabled","--quiet",name]).status() {
        Ok(s) if s.success()=>ui::badge_ok("Enabled"), _=>ui::badge_warn("Disabled")
    }
}
