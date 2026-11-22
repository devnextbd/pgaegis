use std::io::{self, IsTerminal};

fn use_color() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map(|v| v != "dumb").unwrap_or(true)
}

fn paint(code: &str, text: &str) -> String {
    if use_color() { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
}

pub fn brand(text: &str) -> String { paint("1;36", text) }
pub fn good(text: &str) -> String { paint("1;32", text) }
pub fn warn_text(text: &str) -> String { paint("1;33", text) }
pub fn bad(text: &str) -> String { paint("1;31", text) }
pub fn dim(text: &str) -> String { paint("2", text) }
pub fn key(text: &str) -> String { paint("1", text) }

pub fn success(message: &str) { println!("{} {}", good("✓"), message); }
pub fn warning(message: &str) { println!("{} {}", warn_text("!"), message); }
pub fn error(message: &str) { eprintln!("{} {}", bad("✗"), message); }
pub fn note(message: &str) { println!("{} {}", brand("•"), message); }

fn visible_width(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut width = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i+1] == b'[' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'm' { i += 1; }
            if i < bytes.len() { i += 1; }
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        width += 1;
        i += ch.len_utf8();
    }
    width
}

fn pad(s: &str, width: usize) -> String {
    let current = visible_width(s);
    if current >= width { s.to_string() } else { format!("{}{}", s, " ".repeat(width-current)) }
}

pub fn header(subtitle: &str) {
    let title = format!("{}  {}", brand("pgaegis"), dim(subtitle));
    let width = 78usize;
    println!("╭{}╮", "─".repeat(width));
    println!("│ {}{}│", title, " ".repeat(width.saturating_sub(1 + visible_width(&title))));
    println!("╰{}╯", "─".repeat(width));
}

pub fn card(title: &str, rows: &[(&str, String)]) {
    let label_width = rows.iter().map(|(k,_)| visible_width(k)).max().unwrap_or(0).max(10);
    let value_width = rows.iter().map(|(_,v)| visible_width(v)).max().unwrap_or(0).max(12);
    let inner = (label_width + value_width + 7).max(visible_width(title)+4).min(112);
    let left = if inner > title.len()+4 { (inner-title.len()-2)/2 } else { 1 };
    let right = inner.saturating_sub(left + title.len() + 2);
    println!("╭{} {} {}╮", "─".repeat(left), brand(title), "─".repeat(right));
    for (k,v) in rows {
        println!("│  {} : {}  │", pad(&key(k), label_width), pad(v, value_width));
    }
    println!("╰{}╯", "─".repeat(inner));
}

pub fn table(title: &str, headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| visible_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i >= widths.len() { widths.push(visible_width(cell)); }
            else { widths[i] = widths[i].max(visible_width(cell)); }
        }
    }
    for w in &mut widths { *w = (*w).min(42); }
    let inner = widths.iter().sum::<usize>() + widths.len().saturating_sub(1)*3 + 4;
    let title_len = visible_width(title);
    let left = inner.saturating_sub(title_len+2)/2;
    let right = inner.saturating_sub(title_len+2+left);
    println!("╭{} {} {}╮", "─".repeat(left), brand(title), "─".repeat(right));
    let head = headers.iter().enumerate().map(|(i,h)| pad(&key(h), widths[i])).collect::<Vec<_>>().join(" │ ");
    println!("│  {}  │", head);
    println!("├{}┤", "─".repeat(inner));
    for row in rows {
        let line = row.iter().enumerate().map(|(i,c)| pad(c, widths.get(i).copied().unwrap_or(12))).collect::<Vec<_>>().join(" │ ");
        println!("│  {}  │", line);
    }
    if rows.is_empty() { println!("│  {}  │", pad(&dim("No rows found"), inner.saturating_sub(4))); }
    println!("╰{}╯", "─".repeat(inner));
}

pub fn badge_ok(text: &str) -> String { good(&format!("✓ {text}")) }
pub fn badge_warn(text: &str) -> String { warn_text(&format!("! {text}")) }
pub fn badge_bad(text: &str) -> String { bad(&format!("✗ {text}")) }

pub fn menu(title: &str, items: &[(&str,&str)]) {
    header(title);
    for (n, label) in items { println!("  {}  {}", brand(n), label); }
    println!();
}
