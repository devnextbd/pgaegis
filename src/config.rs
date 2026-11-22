use std::collections::HashMap;
use std::fs;

#[derive(Clone, Debug)]
pub struct Config {
    pub cluster_name: String,
    pub patroni_config: String,
    pub pgbackrest_stanza: String,
    pub max_replication_lag_bytes: u64,
    pub backup_max_age_hours: u64,
    pub writer_host: String,
    pub writer_port: u16,
    pub alert_webhook: String,
    pub fence_public_key: String,
    pub certification_dir: String,
    pub etcd_endpoints: String,
    pub etcd_ca: String,
    pub etcd_cert: String,
    pub etcd_key: String,
    pub monitor_dsn: String,
    pub rto_target_seconds: u64,
    pub rpo_max_lost_rows: u64,
    pub certification_evidence_max_age_hours: u64,
    pub pgdata: String,
    pub disk_usage_max_percent: u32,
    pub max_slot_retained_wal_bytes: u64,
    pub certificate_min_valid_days: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cluster_name: "pgaegis".into(),
            patroni_config: "/etc/patroni/patroni.yml".into(),
            pgbackrest_stanza: "main".into(),
            max_replication_lag_bytes: 64 * 1024 * 1024,
            backup_max_age_hours: 26,
            writer_host: "127.0.0.1".into(),
            writer_port: 6432,
            alert_webhook: String::new(),
            fence_public_key: "/etc/pgaegis/fencing.pub".into(),
            certification_dir: "/var/lib/pgaegis/certification".into(),
            etcd_endpoints: "https://127.0.0.1:2379".into(),
            etcd_ca: "/etc/etcd/tls/ca.crt".into(),
            etcd_cert: "/etc/etcd/tls/patroni.crt".into(),
            etcd_key: "/etc/etcd/tls/patroni.key".into(),
            monitor_dsn: String::new(),
            rto_target_seconds: 60,
            rpo_max_lost_rows: 0,
            certification_evidence_max_age_hours: 168,
            pgdata: "/var/lib/postgresql/18/main".into(),
            disk_usage_max_percent: 85,
            max_slot_retained_wal_bytes: 2 * 1024 * 1024 * 1024,
            certificate_min_valid_days: 14,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let mut c = Self::default();
        let path = std::env::var("PGAEGIS_CONFIG").unwrap_or_else(|_| "/etc/pgaegis/pgaegis.conf".into());
        let Ok(data) = fs::read_to_string(path) else { return c; };
        let mut m = HashMap::new();
        for raw in data.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some((k,v)) = line.split_once('=') { m.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string()); }
        }
        if let Some(v)=m.get("CLUSTER_NAME") { c.cluster_name=v.clone(); }
        if let Some(v)=m.get("PATRONI_CONFIG") { c.patroni_config=v.clone(); }
        if let Some(v)=m.get("PGBACKREST_STANZA") { c.pgbackrest_stanza=v.clone(); }
        if let Some(v)=m.get("MAX_REPLICATION_LAG_BYTES").and_then(|x|x.parse().ok()) { c.max_replication_lag_bytes=v; }
        if let Some(v)=m.get("BACKUP_MAX_AGE_HOURS").and_then(|x|x.parse().ok()) { c.backup_max_age_hours=v; }
        if let Some(v)=m.get("WRITER_HOST") { c.writer_host=v.clone(); }
        if let Some(v)=m.get("WRITER_PORT").and_then(|x|x.parse().ok()) { c.writer_port=v; }
        if let Some(v)=m.get("ALERT_WEBHOOK") { c.alert_webhook=v.clone(); }
        if let Some(v)=m.get("FENCE_PUBLIC_KEY") { c.fence_public_key=v.clone(); }
        if let Some(v)=m.get("CERTIFICATION_DIR") { c.certification_dir=v.clone(); }
        if let Some(v)=m.get("ETCD_ENDPOINTS") { c.etcd_endpoints=v.clone(); }
        if let Some(v)=m.get("ETCD_CA") { c.etcd_ca=v.clone(); }
        if let Some(v)=m.get("ETCD_CERT") { c.etcd_cert=v.clone(); }
        if let Some(v)=m.get("ETCD_KEY") { c.etcd_key=v.clone(); }
        if let Some(v)=m.get("MONITOR_DSN") { c.monitor_dsn=v.clone(); }
        if let Some(v)=m.get("RTO_TARGET_SECONDS").and_then(|x|x.parse().ok()) { c.rto_target_seconds=v; }
        if let Some(v)=m.get("RPO_MAX_LOST_ROWS").and_then(|x|x.parse().ok()) { c.rpo_max_lost_rows=v; }
        if let Some(v)=m.get("CERT_EVIDENCE_MAX_AGE_HOURS").and_then(|x|x.parse().ok()) { c.certification_evidence_max_age_hours=v; }
        if let Some(v)=m.get("PGDATA") { c.pgdata=v.clone(); }
        if let Some(v)=m.get("DISK_USAGE_MAX_PERCENT").and_then(|x|x.parse().ok()) { c.disk_usage_max_percent=v; }
        if let Some(v)=m.get("MAX_SLOT_RETAINED_WAL_BYTES").and_then(|x|x.parse().ok()) { c.max_slot_retained_wal_bytes=v; }
        if let Some(v)=m.get("CERTIFICATE_MIN_VALID_DAYS").and_then(|x|x.parse().ok()) { c.certificate_min_valid_days=v; }
        c
    }
}
