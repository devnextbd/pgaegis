//! Shared PostgreSQL helpers. All local administrative SQL runs as the
//! `postgres` OS user over the Unix socket (peer auth) and is fed through
//! stdin, so statements never appear in world-readable process arguments.

use crate::util;

const PSQL_BASE: [&str; 9] = ["-u", "postgres", "--", "psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-At"];

/// Run SQL in `db` and return trimmed unaligned output.
pub fn query(db: &str, sql: &str) -> Result<String, String> {
    let mut a: Vec<&str> = PSQL_BASE.to_vec();
    a.extend(["-d", db, "-f", "-"]);
    util::output_with("runuser", &a, &[], Some(sql))
}

/// Run SQL in `db`, streaming psql output to the terminal.
pub fn exec(db: &str, sql: &str) -> Result<(), String> {
    util::status_with("runuser", &["-u", "postgres", "--", "psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", db, "-f", "-"], &[], Some(sql))
}

/// Run a read-only report query with aligned/expanded output for humans.
pub fn report(db: &str, sql: &str, expanded: bool) -> Result<String, String> {
    let mut a = vec!["-u", "postgres", "--", "psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", db];
    if expanded {
        a.push("-x");
    }
    a.extend(["-f", "-"]);
    util::output_with("runuser", &a, &[], Some(sql))
}

pub fn ensure_writable_primary(what: &str) -> Result<(), String> {
    let out = query("postgres", "SELECT NOT pg_is_in_recovery() AND current_setting('transaction_read_only')='off';")?;
    if out.trim() != "t" {
        return Err(format!("{what} is allowed only on the current writable Primary"));
    }
    Ok(())
}

/// Conservative identifier rule used for every name pgaegis creates or
/// interpolates: ASCII letter/underscore first, then letters, digits, `_`.
pub fn validate_ident(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 63 {
        return Err("PostgreSQL identifier must be 1..63 bytes".into());
    }
    let mut it = s.chars();
    let first = it.next().unwrap_or('0');
    if !(first.is_ascii_alphabetic() || first == '_') || !it.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("invalid PostgreSQL identifier: {s}"));
    }
    Ok(())
}

pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `table` or `schema.table`, each part validated and quoted.
pub fn quote_qualified(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() || parts.len() > 2 {
        return Err("table must be table or schema.table".into());
    }
    for p in &parts {
        validate_ident(p)?;
    }
    Ok(parts.into_iter().map(quote_ident).collect::<Vec<_>>().join("."))
}

pub fn role_exists(name: &str) -> Result<bool, String> {
    Ok(query("postgres", &format!("SELECT 1 FROM pg_roles WHERE rolname={};", quote_literal(name)))?.trim() == "1")
}

pub fn db_exists(name: &str) -> Result<bool, String> {
    Ok(query("postgres", &format!("SELECT 1 FROM pg_database WHERE datname={};", quote_literal(name)))?.trim() == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        for ok in ["app", "_x", "App_1", &"a".repeat(63)] {
            assert!(validate_ident(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1abc", "a-b", "a b", "a;drop", "é", &"a".repeat(64), "a\"b"] {
            assert!(validate_ident(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_literal("o'neil"), "'o''neil'");
        assert_eq!(quote_qualified("public.t1").unwrap(), "\"public\".\"t1\"");
        assert!(quote_qualified("a.b.c").is_err());
        assert!(quote_qualified("a.1b").is_err());
    }
}
