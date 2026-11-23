use crate::{config::Config,json,pg,ui,util};
use crate::util::{arg, has_flag};
use std::collections::HashSet;
use std::fs;
use std::process::Command;

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

// ---------------------------------------------------------------------------
// Patroni / etcd state (parsed from JSON, never from human-oriented tables)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Member { pub name:String, pub host:String, pub role:String, pub state:String, pub tl:String, pub lag:String }

impl Member {
    pub fn is_leader(&self)->bool { self.role.eq_ignore_ascii_case("leader") }
    pub fn is_replica(&self)->bool { let r=self.role.to_ascii_lowercase(); !self.is_leader() && (r.contains("replica") || (r.contains("standby") && !r.contains("leader"))) }
    pub fn is_healthy(&self)->bool { let s=self.state.to_ascii_lowercase(); s=="running" || s=="streaming" }
}

/// Parse `patronictl list -f json`.
pub fn parse_members(s:&str)->Result<Vec<Member>,String>{
    let v=json::parse(s).map_err(|e|format!("patronictl JSON: {e}"))?;
    let arr=v.as_array().ok_or("patronictl JSON: expected an array")?;
    let field=|m:&json::Value,keys:&[&str]|keys.iter().find_map(|k|m.get(k)).map(json::Value::display).unwrap_or_default();
    Ok(arr.iter().map(|m| Member{
        name:field(m,&["Member"]), host:field(m,&["Host"]), role:field(m,&["Role"]), state:field(m,&["State"]),
        tl:field(m,&["TL"]), lag:field(m,&["Lag in MB","Lag","Replay Lag","Receive Lag"]),
    }).filter(|m|!m.name.is_empty()).collect())
}

pub fn patroni_members(cfg:&Config)->Result<Vec<Member>,String>{
    let out=util::output("patronictl", &["-c",&cfg.patroni_config,"list",&cfg.cluster_name,"-f","json"])?;
    parse_members(&out)
}

/// Problems with a member list that make the cluster unsafe/degraded.
pub fn topology_failures(members:&[Member])->Vec<String>{
    let mut f=Vec::new();
    let leaders=members.iter().filter(|m|m.is_leader()).count();
    if members.len()<3 { f.push(format!("expected at least 3 Patroni members, observed {}",members.len())); }
    if leaders!=1 { f.push(format!("expected exactly one leader, observed {leaders}")); }
    let unhealthy:Vec<&str>=members.iter().filter(|m|!m.is_healthy()).map(|m|m.name.as_str()).collect();
    if !unhealthy.is_empty() { f.push(format!("unhealthy Patroni member(s): {}",unhealthy.join(","))); }
    f
}

#[derive(Debug, Default, PartialEq)]
pub struct EtcdQuorum { pub members:usize, pub healthy:usize, pub learners:usize, pub duplicates:usize }

impl EtcdQuorum {
    pub fn failures(&self)->Vec<String>{
        let mut f=Vec::new();
        if self.members<3 || self.members.is_multiple_of(2) { f.push(format!("etcd must have an odd quorum of at least 3 voters (observed {})",self.members)); }
        if self.learners>0 { f.push("etcd learner present; promote/remove learner before certification".into()); }
        if self.duplicates>0 { f.push("duplicate etcd member identity detected".into()); }
        if self.healthy < self.members/2+1 { f.push(format!("etcd healthy quorum not observed: {}/{}",self.healthy,self.members)); }
        f
    }
    pub fn ok(&self)->bool { self.failures().is_empty() }
}

/// Build quorum facts from `member list -w json` and `endpoint health --cluster -w json`.
pub fn parse_etcd(member_json:&str,health_json:&str)->Result<EtcdQuorum,String>{
    let m=json::parse(member_json).map_err(|e|format!("etcd member list JSON: {e}"))?;
    let members=m.get("members").and_then(json::Value::as_array).ok_or("etcd member list JSON: no members")?;
    let mut names=HashSet::new(); let mut dup=0;
    for x in members {
        let id=x.get("name").and_then(json::Value::as_str).filter(|s|!s.is_empty()).map(str::to_string)
            .unwrap_or_else(||x.get("peerURLs").map(|p|format!("{p:?}")).unwrap_or_default());
        if !names.insert(id){dup+=1;}
    }
    let learners=members.iter().filter(|x|x.get("isLearner").and_then(json::Value::as_bool)==Some(true)).count();
    let h=json::parse(health_json).map_err(|e|format!("etcd endpoint health JSON: {e}"))?;
    let healthy=h.as_array().unwrap_or(&[]).iter().filter(|e|e.get("health").and_then(json::Value::as_bool)==Some(true)).count();
    Ok(EtcdQuorum{members:members.len(),healthy,learners,duplicates:dup})
}

pub fn etcd_quorum(cfg:&Config)->Result<EtcdQuorum,String>{
    if !util::command_exists("etcdctl") { return Err("etcdctl missing".into()); }
    let m=etcd_output(cfg,&["member","list","-w","json"])?;
    // endpoint health exits non-zero when any endpoint is down; the JSON still
    // lists every endpoint, so parse it either way.
    let h=etcd_raw(cfg,&["endpoint","health","--cluster","-w","json"]).map(|(o,_)|o)?;
    parse_etcd(&m,&h)
}

pub fn etcd_output(cfg:&Config,args:&[&str])->Result<String,String>{
    let (out,ok)=etcd_raw(cfg,args)?;
    if !ok { return Err(format!("etcdctl {} failed",args.join(" "))); }
    Ok(out)
}

fn etcd_raw(cfg:&Config,args:&[&str])->Result<(String,bool),String>{
    let out=Command::new("etcdctl")
        .env("ETCDCTL_API","3")
        .env("ETCDCTL_ENDPOINTS",&cfg.etcd_endpoints)
        .env("ETCDCTL_CACERT",&cfg.etcd_ca)
        .env("ETCDCTL_CERT",&cfg.etcd_cert)
        .env("ETCDCTL_KEY",&cfg.etcd_key)
        .env("ETCDCTL_DIAL_TIMEOUT","5s")
        .env("ETCDCTL_COMMAND_TIMEOUT","10s")
        .args(args)
        .output().map_err(|e|format!("etcdctl: {e}"))?;
    let stdout=String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() && stdout.is_empty() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    Ok((stdout,out.status.success()))
}

/// Patroni member name of this node (`name:` in the local patroni.yml).
pub fn local_member_name(cfg:&Config)->Result<String,String>{
    let y=fs::read_to_string(&cfg.patroni_config).map_err(|e|format!("{}: {e}",cfg.patroni_config))?;
    y.lines().find_map(|l|l.strip_prefix("name:")).map(|v|v.trim().trim_matches(|c|c=='\''||c=='"').to_string()).filter(|v|!v.is_empty())
        .ok_or_else(||format!("no top-level name: in {}",cfg.patroni_config))
}

fn postgres_can_write_watchdog()->bool{
    Command::new("runuser").args(["-u","postgres","--","test","-w","/dev/watchdog"]).status().map(|s|s.success()).unwrap_or(false)
}

const WATCHDOG_UDEV_RULE:&str="/etc/udev/rules.d/60-pgaegis-watchdog.rules";

/// Patroni runs as `postgres`; with `watchdog.mode: required` it refuses to
/// become leader unless it can open /dev/watchdog.
fn ensure_watchdog_access()->Result<(),String>{
    if postgres_can_write_watchdog() { return Ok(()); }
    util::atomic_write(WATCHDOG_UDEV_RULE,"# Managed by pgaegis: Patroni (user postgres) must open the watchdog device.\nKERNEL==\"watchdog\", OWNER=\"postgres\", GROUP=\"postgres\", MODE=\"0600\"\n",0o644)?;
    for dev in ["/dev/watchdog","/dev/watchdog0"] {
        if std::path::Path::new(dev).exists() { util::status("chown",&["postgres:postgres",dev])?; util::status("chmod",&["0600",dev])?; }
    }
    if !postgres_can_write_watchdog() { return Err("postgres cannot open /dev/watchdog even after applying the udev rule; check the watchdog driver".into()); }
    ui::note(&format!("Granted postgres access to /dev/watchdog ({WATCHDOG_UDEV_RULE})"));
    Ok(())
}

// ---------------------------------------------------------------------------

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

const REQUIRED_TLS:[&str;13]=[
    "/etc/patroni/tls/ca.crt","/etc/patroni/tls/node.crt","/etc/patroni/tls/node.key","/etc/patroni/tls/admin.crt","/etc/patroni/tls/admin.key",
    "/etc/etcd/tls/ca.crt","/etc/etcd/tls/patroni.crt","/etc/etcd/tls/patroni.key",
    "/etc/postgresql/tls/ca.crt","/etc/postgresql/tls/server.crt","/etc/postgresql/tls/server.key",
    // Client certificate Patroni presents for replication/pg_rewind (pg_hba uses clientcert=verify-ca).
    "/etc/postgresql/tls/client.crt","/etc/postgresql/tls/client.key",
];

fn setup(args:&[String])->Result<(),String>{
    util::require_root()?;
    let node=arg(args,"--node-name").ok_or("--node-name required")?;
    let node_ip=arg(args,"--node-ip").ok_or("--node-ip required")?;
    let cluster=arg(args,"--cluster").ok_or("--cluster required")?;
    let etcd_raw=arg(args,"--etcd").ok_or("--etcd etcd1:2379,etcd2:2379,etcd3:2379 required")?;
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
    util::validate_plain_path("--pgdata",pgdata)?;
    util::validate_safe_name("--api-user",api_user)?;
    let etcd_hosts=etcd_raw.split(',').map(|x|x.trim().trim_start_matches("https://").trim_start_matches("http://")).collect::<Vec<_>>().join(",");
    let etcd_urls=etcd_hosts.split(',').map(|x|format!("https://{x}")).collect::<Vec<_>>().join(",");
    let force=has_flag(args,"--force");
    let n=etcd_hosts.split(',').count();
    if n<3 || n.is_multiple_of(2){return Err("--etcd must contain an odd set of at least 3 DCS endpoints".into());}
    if std::path::Path::new("/etc/patroni/patroni.yml").exists() && !force {
        return Err("/etc/patroni/patroni.yml already exists; refusing overwrite. Back it up and rerun with --force only after review".into());
    }
    if std::path::Path::new(pgdata).join("PG_VERSION").exists() {
        return Err("PGDATA already contains a PostgreSQL cluster. Direct adoption is intentionally blocked; migrate it with a backup/restore or documented Patroni migration runbook".into());
    }
    for p in REQUIRED_TLS { if !std::path::Path::new(p).exists(){return Err(format!("required TLS file missing: {p}"));} }
    if !std::path::Path::new("/dev/watchdog").exists(){return Err("/dev/watchdog missing; production Patroni setup refuses to downgrade watchdog safety".into());}
    let template=util::asset_path("templates","patroni.production.yml")?;
    let y=fs::read_to_string(&template).map_err(|e|format!("template: {e}"))?;
    let pgpass=util::prompt_secret_confirm("PostgreSQL superuser password")?;
    let reppass=util::prompt_secret_confirm("Replication password")?;
    let rewind=util::prompt_secret_confirm("Rewind password")?;
    let apipass=util::prompt_secret_confirm("Patroni REST API password")?;
    let (pgdata_yaml,pgpass_yaml,reppass_yaml,rewind_yaml,apipass_yaml)=(util::yaml_single_quote(pgdata),util::yaml_single_quote(&pgpass),util::yaml_single_quote(&reppass),util::yaml_single_quote(&rewind),util::yaml_single_quote(&apipass));
    let y=util::render_template(&y,&[
        ("CLUSTER_NAME",cluster),("NODE_NAME",node),("NODE_IP",node_ip),("ETCD_ENDPOINTS",etcd_hosts.as_str()),("PGDATA",pgdata_yaml.as_str()),("PGBACKREST_STANZA",stanza),("CLIENT_CIDR",client_cidr),("REPLICATION_CIDR",replication_cidr),("PATRONI_API_USER",api_user),
        ("POSTGRES_PASSWORD",pgpass_yaml.as_str()),("REPLICATION_PASSWORD",reppass_yaml.as_str()),("REWIND_PASSWORD",rewind_yaml.as_str()),("PATRONI_API_PASSWORD",apipass_yaml.as_str())
    ])?;
    fs::create_dir_all(pgdata).map_err(|e|format!("create PGDATA: {e}"))?;
    util::status("chown",&["postgres:postgres",pgdata])?;
    util::status("chmod",&["0700",pgdata])?;
    for key in ["/etc/postgresql/tls/client.key","/etc/postgresql/tls/server.key"] {
        util::status("chown",&["postgres:postgres",key])?; util::status("chmod",&["0600",key])?;
    }
    ensure_watchdog_access()?;
    if force { let _=util::backup_file("/etc/patroni/patroni.yml")?; }
    util::write_secret_file("/etc/patroni/patroni.yml",&y)?;
    util::status("chown",&["postgres:postgres","/etc/patroni/patroni.yml"])?;
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
    if util::systemd_active("postgresql") {
        return Err("standalone postgresql.service is active; stop/disable the competing service before Patroni activation".into());
    }
    if !util::command_exists("patroni"){return Err("patroni is not installed".into());}
    let validate=Command::new("patroni").args(["--validate-config",&cfg.patroni_config]).status().map_err(|e|e.to_string())?;
    if !validate.success(){return Err("Patroni configuration validation failed".into());}
    ensure_watchdog_access()?;
    util::status("systemctl", &["enable","--now","patroni"])?;
    std::thread::sleep(std::time::Duration::from_secs(3));
    status()
}

fn status()->Result<(),String>{
    let cfg=Config::load();
    let members=patroni_members(&cfg)?;
    let rows=members.iter().map(|m|{
        let health=if m.is_healthy(){ui::badge_ok("Healthy")}else{ui::badge_warn(&m.state)};
        vec![m.name.clone(),m.host.clone(),m.role.clone(),m.state.clone(),m.tl.clone(),if m.lag.is_empty(){"-".into()}else{m.lag.clone()},health]
    }).collect::<Vec<_>>();
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
    match etcd_quorum(&cfg) { Ok(q)=>failures.extend(q.failures()), Err(e)=>failures.push(format!("etcd quorum check failed: {e}")) }
    if !std::path::Path::new("/dev/watchdog").exists() { failures.push("/dev/watchdog missing".into()); }
    else if !postgres_can_write_watchdog() { failures.push("postgres (Patroni) cannot open /dev/watchdog".into()); }
    // Verify the rules PostgreSQL actually loaded, not just the YAML text.
    match pg::query("postgres","SELECT count(*) FILTER (WHERE type='hostssl' AND 'replication'=ANY(database))||'|'||count(*) FILTER (WHERE error IS NOT NULL) FROM pg_hba_file_rules;") {
        Ok(v) => { let p:Vec<&str>=v.trim().split('|').collect();
            if p.first().copied().unwrap_or("0")=="0" { failures.push("live pg_hba has no hostssl replication rule (replicas cannot connect)".into()); }
            if p.get(1).copied().unwrap_or("0")!="0" { failures.push("live pg_hba.conf contains invalid lines".into()); } },
        Err(e) => failures.push(format!("unable to read live pg_hba rules: {e}")),
    }
    match patroni_members(&cfg) { Ok(m)=>failures.extend(topology_failures(&m)), Err(e)=>failures.push(format!("patronictl list failed: {e}")) }
    if failures.is_empty() { ui::success("HA AUDIT: PASS"); Ok(()) } else { for f in &failures { ui::error(f); } Err(format!("HA audit failed: {} issue(s)",failures.len())) }
}

/// With synchronous_mode, Patroni only accepts the synchronous standby as a
/// switchover candidate (HTTP 412 otherwise); check up front with a clear hint.
pub fn check_switchover_candidate(members:&[Member],candidate:&str)->Result<(),String>{
    let m=members.iter().find(|m|m.name==candidate).ok_or_else(||format!("candidate {candidate} is not a member of the cluster"))?;
    if m.is_leader() { return Err(format!("{candidate} is already the leader")); }
    let syncs:Vec<&str>=members.iter().filter(|m|m.role.to_ascii_lowercase().contains("sync standby")).map(|m|m.name.as_str()).collect();
    if !syncs.is_empty() && !syncs.contains(&candidate) {
        return Err(format!("synchronous mode is enabled: only the sync standby can be a switchover candidate (use --candidate {})",syncs.join(" or ")));
    }
    Ok(())
}

fn switchover(args:&[String])->Result<(),String>{
    util::require_root()?; audit()?; let cfg=Config::load(); let candidate=arg(args,"--candidate").ok_or("--candidate required")?;
    util::validate_safe_name("--candidate",candidate)?;
    check_switchover_candidate(&patroni_members(&cfg)?,candidate)?;
    util::status("patronictl", &["-c",&cfg.patroni_config,"switchover",&cfg.cluster_name,"--candidate",candidate,"--force"])
}

fn failover(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load(); let candidate=arg(args,"--candidate").ok_or("--candidate required")?;
    util::validate_safe_name("--candidate",candidate)?;
    let fenced_node=arg(args,"--fenced-node").ok_or("--fenced-node required")?;
    let evidence=arg(args,"--fence-evidence").ok_or("--fence-evidence required")?;
    let signature=arg(args,"--fence-signature").ok_or("--fence-signature required")?;
    if !has_flag(args,"--acknowledge-possible-data-loss") { return Err("emergency failover may lose transactions; add --acknowledge-possible-data-loss after independent fencing is verified".into()); }
    verify_fence(&cfg,evidence,signature,fenced_node)?;
    let q=etcd_quorum(&cfg)?;
    if q.members<3 || q.healthy < q.members/2+1 {return Err("DCS quorum is not healthy; manual failover blocked".into());}
    util::status("patronictl", &["-c",&cfg.patroni_config,"failover",&cfg.cluster_name,"--candidate",candidate,"--force"])
}

/// Fence evidence must be explicitly issued for failover (`purpose=failover`);
/// certification drill evidence (`purpose=certification`) is not accepted.
pub fn check_fence_body(body:&str,fenced_node:&str,now:u64)->Result<(),String>{
    let has=|x:&str|body.lines().any(|l|l.trim()==x);
    if !has("purpose=failover") { return Err("fencing evidence is not issued for failover (purpose=failover required)".into()); }
    if !has("isolated=true") { return Err("fencing evidence does not confirm isolated=true".into()); }
    if !has(&format!("node={fenced_node}")) { return Err("fencing evidence node does not match --fenced-node".into()); }
    let exp=body.lines().find_map(|l|l.trim().strip_prefix("expires_epoch=")).and_then(|v|v.trim().parse::<u64>().ok()).ok_or("fencing evidence missing expires_epoch")?;
    if now>exp { return Err("fencing evidence expired".into()); }
    Ok(())
}

fn verify_fence(cfg:&Config,evidence:&str,signature:&str,fenced_node:&str)->Result<(),String>{
    // Verify the signature first, then parse exactly the bytes that were verified.
    let s=Command::new("openssl").args(["dgst","-sha256","-verify",&cfg.fence_public_key,"-signature",signature,evidence]).stdout(std::process::Stdio::null()).status().map_err(|e|format!("openssl: {e}"))?;
    if !s.success() { return Err("fencing signature verification failed".into()); }
    let body=fs::read_to_string(evidence).map_err(|e|format!("fence evidence: {e}"))?;
    check_fence_body(&body,fenced_node,util::unix_now())
}

fn rejoin(args:&[String])->Result<(),String>{
    util::require_root()?; let cfg=Config::load();
    if !util::contains_all(&cfg.patroni_config,&["use_pg_rewind: true"]) { return Err("use_pg_rewind must be enabled before rejoin".into()); }
    if arg(args,"--member").is_some() { return Err("rejoin must be run locally on the former primary; remote rejoin is intentionally not supported".into()); }
    let member=local_member_name(&cfg)?;
    util::status("systemctl", &["restart","patroni"])?;
    for _ in 0..24 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if patroni_members(&cfg).unwrap_or_default().iter().any(|m|m.name==member && m.is_replica() && m.is_healthy()) { ui::success(&format!("Former primary {member} rejoined as replica.")); return Ok(()); }
    }
    if has_flag(args,"--allow-reinit") {
        util::status("patronictl", &["-c",&cfg.patroni_config,"reinit",&cfg.cluster_name,&member,"--force"])
    } else { Err(format!("automatic pg_rewind rejoin of {member} did not complete; inspect Patroni logs or rerun with --allow-reinit for destructive replica rebuild")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST:&str=r#"[{"Cluster":"primary-prod","Member":"pg1","Host":"10.0.0.11","Role":"Leader","State":"running","TL":4},
        {"Cluster":"primary-prod","Member":"pg2","Host":"10.0.0.12","Role":"Sync Standby","State":"streaming","TL":4,"Lag in MB":0},
        {"Cluster":"primary-prod","Member":"pg10","Host":"10.0.0.20","Role":"Replica","State":"streaming","TL":4,"Lag in MB":1}]"#;

    #[test]
    fn members_and_topology() {
        let m=parse_members(LIST).unwrap();
        assert_eq!(m.len(),3);
        assert_eq!(m.iter().filter(|x|x.is_leader()).count(),1, "cluster name containing 'primary' must not count as leader");
        assert!(m[1].is_replica() && m[2].is_replica());
        assert!(topology_failures(&m).is_empty());
        let two=&m[..2];
        assert!(!topology_failures(two).is_empty(), "two members must fail the 3-member gate");
        let mut bad=m.clone(); bad[2].state="stopped".into();
        assert!(topology_failures(&bad).iter().any(|f|f.contains("pg10")));
        let standby_leader=Member{name:"x".into(),host:"".into(),role:"Standby Leader".into(),state:"running".into(),tl:"".into(),lag:"".into()};
        assert!(!standby_leader.is_leader() && !standby_leader.is_replica());
    }

    #[test]
    fn switchover_candidate_must_be_sync_standby() {
        let m=parse_members(LIST).unwrap();
        assert!(check_switchover_candidate(&m,"pg2").is_ok());
        let e=check_switchover_candidate(&m,"pg10").unwrap_err();
        assert!(e.contains("--candidate pg2"));
        assert!(check_switchover_candidate(&m,"pg1").is_err());
        assert!(check_switchover_candidate(&m,"nope").is_err());
    }

    #[test]
    fn etcd_quorum_parsing() {
        let members=r#"{"header":{"cluster_id":1},"members":[{"ID":1,"name":"e1"},{"ID":2,"name":"e2"},{"ID":3,"name":"e3","isLearner":true}]}"#;
        let health=r#"[{"endpoint":"https://a:2379","health":true},{"endpoint":"https://b:2379","health":true},{"endpoint":"https://c:2379","health":false,"error":"x"}]"#;
        let q=parse_etcd(members,health).unwrap();
        assert_eq!(q,EtcdQuorum{members:3,healthy:2,learners:1,duplicates:0});
        assert!(!q.ok());
        let ok=parse_etcd(&members.replace(r#","isLearner":true"#,""),health).unwrap();
        assert!(ok.ok());
        let dup=parse_etcd(r#"{"members":[{"name":"e1"},{"name":"e1"},{"name":"e3"}]}"#,health).unwrap();
        assert_eq!(dup.duplicates,1);
    }

    #[test]
    fn fence_evidence_purpose() {
        let body="status=PASS\nisolated=true\nnode=pg1\npurpose=failover\nexpires_epoch=200\n";
        assert!(check_fence_body(body,"pg1",100).is_ok());
        assert!(check_fence_body(body,"pg1",300).is_err());
        assert!(check_fence_body(body,"pg2",100).is_err());
        assert!(check_fence_body(&body.replace("purpose=failover","purpose=certification"),"pg1",100).is_err());
    }
}
