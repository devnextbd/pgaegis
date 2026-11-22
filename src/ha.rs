use crate::{config::Config,ui,util};
use std::fs;
use std::process::Command;
use std::collections::HashSet;

pub fn dispatch(args:&[String])->Result<(),String>{
    if args.is_empty() || args.first().map(String::as_str)==Some("wizard") { return wizard(); }
    if args.first().map(String::as_str)!=Some("ha") { return Err("usage: pgaegis replica [wizard] | pgaegis replica ha <setup|activate|status|audit|switchover|failover|rejoin>".into()); }
    match args.get(1).map(String::as_str) {
        Some("setup") => setup(args),
        Some("activate") => activate(),
        Some("status") => status(),
        Some("audit") => audit(),
        Some("switchover") => switchover(args),
        Some("failover") => failover(args),
        Some("rejoin") => rejoin(args),
        _ => Err("usage: pgaegis replica ha <setup|activate|status|audit|switchover|failover|rejoin>".into())
    }
}


fn wizard()->Result<(),String>{
    ui::menu("HA / Replica Wizard", &[
        ("1","Configure a fresh Patroni HA member"),("2","Activate Patroni on this member"),("3","Show cluster topology"),
        ("4","Run HA safety audit"),("5","Planned switchover"),("6","Rejoin this former Primary as replica"),("0","Exit")
    ]);
    ui::note("Patroni roles are dynamic: a member can be Primary now and Standby later.");
    let choice=util::prompt_line("Select",Some("3"))?;
    match choice.as_str(){
        "0"=>Ok(()),
        "1"=>{
            let node=util::prompt_line("Node name",None)?;
            let node_ip=util::prompt_line("Node IP",None)?;
            let cluster=util::prompt_line("Cluster name",Some("production"))?;
            let etcd=util::prompt_line("etcd endpoints (host:2379,host:2379,host:2379)",None)?;
            let client=util::prompt_line("Application client CIDR",None)?;
            let repl=util::prompt_line("Replication CIDR",None)?;
            let a=vec!["ha".to_string(),"setup".to_string(),"--node-name".into(),node,"--node-ip".into(),node_ip,"--cluster".into(),cluster,"--etcd".into(),etcd,"--client-cidr".into(),client,"--replication-cidr".into(),repl];
            setup(&a)?;
            if util::prompt_yes_no("Activate Patroni now?",false)?{activate()?;}
            Ok(())
        },
        "2"=>activate(),
        "3"=>status(),
        "4"=>audit(),
        "5"=>{let c=util::prompt_line("Candidate member",None)?; let a=vec!["ha".into(),"switchover".into(),"--candidate".into(),c]; switchover(&a)},
        "6"=>{let allow=util::prompt_yes_no("Allow destructive replica rebuild if pg_rewind cannot rejoin?",false)?; let mut a=vec!["ha".into(),"rejoin".into()]; if allow{a.push("--allow-reinit".into());} rejoin(&a)},
        _=>Err("invalid selection".into())
    }
}


fn setup(args:&[String])->Result<(),String>{
    util::require_root()?;
    let node=arg(args,"--node-name").ok_or("--node-name required")?;
    let node_ip=arg(args,"--node-ip").ok_or("--node-ip required")?;
    let cluster=arg(args,"--cluster").ok_or("--cluster required")?;
    let etcd_raw=arg(args,"--etcd").ok_or("--etcd etcd1:2379,etcd2:2379,etcd3:2379 required")?;
    let etcd_hosts=etcd_raw.split(',').map(|x|x.trim().trim_start_matches("https://").trim_start_matches("http://")).collect::<Vec<_>>().join(",");
    let etcd_urls=etcd_hosts.split(',').map(|x|format!("https://{x}")).collect::<Vec<_>>().join(",");
    let pgdata=arg(args,"--pgdata").unwrap_or("/var/lib/postgresql/18/main");
    let client_cidr=arg(args,"--client-cidr").ok_or("--client-cidr is required; refusing 0.0.0.0/0 production access")?;
    let replication_cidr=arg(args,"--replication-cidr").ok_or("--replication-cidr is required")?;
    let stanza=arg(args,"--stanza").unwrap_or("main");
    let api_user=arg(args,"--api-user").unwrap_or("pgaegis");
    util::validate_safe_name("--node-name",node)?;
    util::validate_safe_name("--cluster",cluster)?;
    util::validate_safe_name("--stanza",stanza)?;
    util::validate_ip("--node-ip",node_ip)?;
    util::validate_cidr("--client-cidr",client_cidr)?;
    util::validate_cidr("--replication-cidr",replication_cidr)?;
    util::reject_control_chars("--etcd",etcd_raw)?;
    util::validate_host_port_list("--etcd",etcd_raw)?;
    util::validate_abs_path("--pgdata",pgdata)?;
    util::validate_safe_name("--api-user",api_user)?;
    let force=args.iter().any(|x|x=="--force");
    if etcd_hosts.split(',').count()<3 || etcd_hosts.split(',').count()%2==0{return Err("--etcd must contain an odd set of at least 3 DCS endpoints".into());}
    if std::path::Path::new("/etc/patroni/patroni.yml").exists() && !force {
        return Err("/etc/patroni/patroni.yml already exists; refusing overwrite. Back it up and rerun with --force only after review".into());
    }
    if std::path::Path::new(pgdata).join("PG_VERSION").exists() {
        return Err("PGDATA already contains a PostgreSQL cluster. Direct adoption is intentionally blocked; migrate it with a backup/restore or documented Patroni migration runbook".into());
    }
    std::fs::create_dir_all(pgdata).map_err(|e|format!("create PGDATA: {e}"))?;
    let _=Command::new("chown").args(["postgres:postgres",pgdata]).status();
    for p in ["/etc/patroni/tls/ca.crt","/etc/patroni/tls/node.crt","/etc/patroni/tls/node.key","/etc/patroni/tls/admin.crt","/etc/patroni/tls/admin.key","/etc/etcd/tls/ca.crt","/etc/etcd/tls/patroni.crt","/etc/etcd/tls/patroni.key","/etc/postgresql/tls/ca.crt","/etc/postgresql/tls/server.crt","/etc/postgresql/tls/server.key"] { if !std::path::Path::new(p).exists(){return Err(format!("required TLS file missing: {p}"));} }
    if !std::path::Path::new("/dev/watchdog").exists(){return Err("/dev/watchdog missing; production Patroni setup refuses to downgrade watchdog safety".into());}
    let template=if std::path::Path::new("/usr/share/pgaegis/templates/patroni.production.yml").exists(){"/usr/share/pgaegis/templates/patroni.production.yml"}else{"templates/patroni.production.yml"};
    let mut y=fs::read_to_string(template).map_err(|e|format!("template: {e}"))?;
    let pgpass=util::prompt_secret_confirm("PostgreSQL superuser password")?;
    let reppass=util::prompt_secret_confirm("Replication password")?;
    let rewind=util::prompt_secret_confirm("Rewind password")?;
    let apipass=util::prompt_secret_confirm("Patroni REST API password")?;
    let pgdata_yaml=util::yaml_single_quote(pgdata);
    let pgpass_yaml=util::yaml_single_quote(&pgpass);
    let reppass_yaml=util::yaml_single_quote(&reppass);
    let rewind_yaml=util::yaml_single_quote(&rewind);
    let apipass_yaml=util::yaml_single_quote(&apipass);
    let replacements=[
        ("${CLUSTER_NAME}",cluster),("${NODE_NAME}",node),("${NODE_IP}",node_ip),("${ETCD_ENDPOINTS}",etcd_hosts.as_str()),("${PGDATA}",pgdata_yaml.as_str()),("${PGBACKREST_STANZA}",stanza),("${CLIENT_CIDR}",client_cidr),("${REPLICATION_CIDR}",replication_cidr),("${PATRONI_API_USER}",api_user),
        ("${POSTGRES_PASSWORD}",pgpass_yaml.as_str()),("${REPLICATION_PASSWORD}",reppass_yaml.as_str()),("${REWIND_PASSWORD}",rewind_yaml.as_str()),("${PATRONI_API_PASSWORD}",apipass_yaml.as_str())
    ];
    for (k,v) in replacements { y=y.replace(k,v); }
    if y.contains("${"){return Err("unresolved Patroni template variables remain".into());}
    if force { let _=util::backup_file("/etc/patroni/patroni.yml")?; }
    util::write_secret_file("/etc/patroni/patroni.yml",&y)?;
    let _=Command::new("chown").args(["postgres:postgres","/etc/patroni/patroni.yml"]).status();
    util::upsert_kv_file("/etc/pgaegis/pgaegis.conf", &[
        ("CLUSTER_NAME",cluster),("PATRONI_CONFIG","/etc/patroni/patroni.yml"),("PGBACKREST_STANZA",stanza),("PGDATA",pgdata),("ETCD_ENDPOINTS",etcd_urls.as_str()),
        ("ETCD_CA","/etc/etcd/tls/ca.crt"),("ETCD_CERT","/etc/etcd/tls/patroni.crt"),("ETCD_KEY","/etc/etcd/tls/patroni.key")
    ])?;
    ui::success("Patroni configuration written to /etc/patroni/patroni.yml (0600)");
    ui::note("Cluster/DCS settings updated. Next: `pgaegis replica ha activate`");
    Ok(())
}

fn activate()->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    if std::process::Command::new("systemctl").args(["is-active","--quiet","postgresql"]).status().map(|s|s.success()).unwrap_or(false) {
        return Err("standalone postgresql.service is active; stop/disable the competing service before Patroni activation".into());
    }
    if !util::command_exists("patroni"){return Err("patroni is not installed".into());}
    let validate=Command::new("patroni").args(["--validate-config",&cfg.patroni_config]).status().map_err(|e|e.to_string())?;
    if !validate.success(){return Err("Patroni configuration validation failed".into());}
    util::status("systemctl", &["enable","--now","patroni"])?;
    std::thread::sleep(std::time::Duration::from_secs(3));
    status()
}


fn status()->Result<(),String>{
    let cfg=Config::load();
    let out=util::output("patronictl", &["-c",&cfg.patroni_config,"list",&cfg.cluster_name,"-f","tsv"])?;
    let mut rows=Vec::new();
    for line in out.lines().filter(|l|!l.trim().is_empty()) {
        let p:Vec<&str>=line.split('\t').collect();
        if p.len()<4 { continue; }
        let role=p.get(2).copied().unwrap_or("-");
        let state=p.get(3).copied().unwrap_or("-");
        let health=if state.eq_ignore_ascii_case("running") || state.eq_ignore_ascii_case("streaming") {ui::badge_ok("Healthy")} else {ui::badge_warn(state)};
        rows.push(vec![p.first().unwrap_or(&"-").to_string(),p.get(1).unwrap_or(&"-").to_string(),role.to_string(),state.to_string(),p.get(4).unwrap_or(&"-").to_string(),p.get(5).unwrap_or(&"-").to_string(),health]);
    }
    ui::table(&format!("PostgreSQL HA Cluster / {}",cfg.cluster_name), &["Member","Host","Role","State","TL","Lag","Health"], &rows);
    if rows.is_empty(){return Err("Patroni returned no cluster members".into());}
    Ok(())
}

pub fn audit()->Result<(),String>{
    let cfg=Config::load(); let mut failures=Vec::new();
    if !util::command_exists("patronictl") { failures.push("patronictl missing".to_string()); }
    let required_local=["watchdog:","mode: required","use_pg_rewind: true","check_timeline: true","maximum_lag_on_failover","failsafe_mode: true","synchronous_mode: true","synchronous_mode_strict: true","primary_start_timeout:","primary_stop_timeout:","verify_client: required","protocol: https","hostssl all all","hostssl replication"] ;
    if !util::contains_all(&cfg.patroni_config,&required_local) { failures.push("Patroni local/bootstrap safeguards are incomplete".into()); }
    match util::output("patronictl", &["-c",&cfg.patroni_config,"show-config",&cfg.cluster_name]) {
        Ok(dynamic) => {
            for key in ["failsafe_mode: true","check_timeline: true","maximum_lag_on_failover","synchronous_mode: true","synchronous_mode_strict: true","primary_start_timeout:","primary_stop_timeout:"] {
                if !dynamic.contains(key) { failures.push(format!("Patroni DCS dynamic config missing: {key}")); }
            }
        },
        Err(e) => failures.push(format!("unable to read Patroni DCS dynamic config: {e}")),
    }
    if util::command_exists("etcdctl") {
        let mut etcd_member_count=0usize;
        match etcd_output(&cfg, &["member","list"]) {
            Ok(members) => {
                let lines: Vec<&str> = members.lines().filter(|l| !l.trim().is_empty()).collect();
                etcd_member_count=lines.len();
                let ids: HashSet<&str> = lines.iter().filter_map(|l| l.split(',').next()).collect();
                if lines.len() < 3 || lines.len().is_multiple_of(2) { failures.push("etcd must have an odd quorum of at least 3 voters".into()); }
                if ids.len() != lines.len() { failures.push("duplicate etcd member identity detected".into()); }
            },
            Err(e) => failures.push(format!("etcd member list failed: {e}")),
        }
        if let Ok(json)=etcd_output(&cfg, &["member","list","-w","json"]) {
            if json.contains("\"isLearner\":true") { failures.push("etcd learner present; promote/remove learner before certification".into()); }
        }
        match etcd_output(&cfg, &["endpoint","health","--cluster","-w","json"]) {
            Ok(h) => { let healthy=count_json_true(&h,"health"); let quorum=(etcd_member_count/2)+1; if etcd_member_count<3 || healthy < quorum { failures.push(format!("etcd healthy quorum not observed: {healthy}/{etcd_member_count}")); } },
            Err(e)=>failures.push(format!("etcd cluster health failed: {e}")),
        }
    } else { failures.push("etcdctl missing".into()); }
    if !std::path::Path::new("/dev/watchdog").exists() { failures.push("/dev/watchdog missing".into()); }
    let list=util::output("patronictl", &["-c",&cfg.patroni_config,"list",&cfg.cluster_name,"-f","tsv"]).unwrap_or_default();
    let visible_members=list.lines().filter(|l|!l.trim().is_empty()).count();
    let leaders=list.lines().filter(|l|l.to_ascii_lowercase().contains("leader") || l.to_ascii_lowercase().contains("primary")).count();
    if visible_members<3 { failures.push(format!("expected at least 3 Patroni members, observed {visible_members}")); }
    if leaders!=1 { failures.push(format!("expected exactly one leader, observed {leaders}")); }
    if failures.is_empty() { ui::success("HA AUDIT: PASS"); Ok(()) } else { for f in &failures { ui::error(f); } Err(format!("HA audit failed: {} issue(s)",failures.len())) }
}

fn switchover(args:&[String])->Result<(),String>{
    util::require_root()?; audit()?; let cfg=Config::load(); let candidate=arg(args,"--candidate").ok_or("--candidate required")?;
    util::status("patronictl", &["-c",&cfg.patroni_config,"switchover",&cfg.cluster_name,"--candidate",candidate,"--force"])
}

fn failover(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load(); let candidate=arg(args,"--candidate").ok_or("--candidate required")?;
    let fenced_node=arg(args,"--fenced-node").ok_or("--fenced-node required")?;
    let evidence=arg(args,"--fence-evidence").ok_or("--fence-evidence required")?;
    let signature=arg(args,"--fence-signature").ok_or("--fence-signature required")?;
    if !args.iter().any(|x|x=="--acknowledge-possible-data-loss") { return Err("emergency failover may lose transactions; add --acknowledge-possible-data-loss after independent fencing is verified".into()); }
    verify_fence(&cfg,evidence,signature,fenced_node)?;
    let members=etcd_output(&cfg, &["member","list"])?;
    let member_count=members.lines().filter(|l|!l.trim().is_empty()).count();
    let health=etcd_output(&cfg, &["endpoint","health","--cluster","-w","json"])?;
    if member_count<3 || count_json_true(&health,"health") < (member_count/2)+1 {return Err("DCS quorum is not healthy; manual failover blocked".into());}
    util::status("patronictl", &["-c",&cfg.patroni_config,"failover",&cfg.cluster_name,"--candidate",candidate,"--force"])
}

fn verify_fence(cfg:&Config,evidence:&str,signature:&str,fenced_node:&str)->Result<(),String>{
    let body=fs::read_to_string(evidence).map_err(|e|format!("fence evidence: {e}"))?;
    if !body.lines().any(|l|l.trim()=="isolated=true") { return Err("fencing evidence does not confirm isolated=true".into()); }
    if !body.lines().any(|l| l.trim() == format!("node={fenced_node}")) { return Err("fencing evidence node does not match --fenced-node".into()); }
    let exp=body.lines().find_map(|l|l.strip_prefix("expires_epoch=")).and_then(|v|v.trim().parse::<u64>().ok()).ok_or("fencing evidence missing expires_epoch")?;
    if util::unix_now()>exp { return Err("fencing evidence expired".into()); }
    let s=Command::new("openssl").args(["dgst","-sha256","-verify",&cfg.fence_public_key,"-signature",signature,evidence]).status().map_err(|e|format!("openssl: {e}"))?;
    if !s.success() { return Err("fencing signature verification failed".into()); }
    Ok(())
}

fn rejoin(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    if !util::contains_all(&cfg.patroni_config,&["use_pg_rewind: true"]) { return Err("use_pg_rewind must be enabled before rejoin".into()); }
    if arg(args,"--member").is_some() { return Err("rejoin must be run locally on the former primary; remote rejoin is intentionally not supported".into()); }
    util::status("systemctl", &["restart","patroni"])?;
    let member=util::output("hostname", &["-s"])?;
    for _ in 0..24 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let list=util::output("patronictl", &["-c",&cfg.patroni_config,"list",&cfg.cluster_name,"-f","tsv"]).unwrap_or_default();
        if list.lines().any(|l|l.contains(&member) && (l.to_ascii_lowercase().contains("replica") || l.to_ascii_lowercase().contains("standby"))) { println!("Former primary rejoined as replica."); return Ok(()); }
    }
    if args.iter().any(|x|x=="--allow-reinit") {
        util::status("patronictl", &["-c",&cfg.patroni_config,"reinit",&cfg.cluster_name,&member,"--force"])
    } else { Err("automatic pg_rewind rejoin did not complete; inspect Patroni logs or rerun with --allow-reinit for destructive replica rebuild".into()) }
}

fn arg<'a>(a:&'a[String],k:&str)->Option<&'a str>{ a.iter().position(|x|x==k).and_then(|i|a.get(i+1)).map(String::as_str) }


fn count_json_true(json:&str,key:&str)->usize{
    let compact:String=json.chars().filter(|c|!c.is_whitespace()).collect();
    compact.matches(&format!("\"{key}\":true")).count()
}

pub fn etcd_output(cfg:&Config,args:&[&str])->Result<String,String>{
    let out=Command::new("etcdctl")
        .env("ETCDCTL_API","3")
        .env("ETCDCTL_ENDPOINTS",&cfg.etcd_endpoints)
        .env("ETCDCTL_CACERT",&cfg.etcd_ca)
        .env("ETCDCTL_CERT",&cfg.etcd_cert)
        .env("ETCDCTL_KEY",&cfg.etcd_key)
        .args(args)
        .output().map_err(|e|format!("etcdctl: {e}"))?;
    if !out.status.success(){return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());}
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
