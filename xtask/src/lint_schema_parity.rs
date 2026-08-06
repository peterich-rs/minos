//! Schema parity gate: SQLite vs Postgres canonical migrations must share one
//! logical model (tables, columns+nullability, PK/UNIQUE column sets, FK graph,
//! critical CHECKs). Physical type encoding and partition children may differ.
//!
//! See docs/superpowers/specs/backend-storage-parity-design.md.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};

const SQLITE_PATH: &str = "crates/minos-backend/migrations/sqlite/0001_initial.sql";
const POSTGRES_PATH: &str = "crates/minos-backend/migrations/postgres/0001_initial.sql";

/// Partition children exist only on Postgres (physical layout).
const PG_ONLY_TABLES: &[&str] = &[
    "durable_event_log_account",
    "durable_event_log_conversation",
    "durable_event_log_project",
    "durable_event_log_agent_session",
    "durable_event_log_host",
];

/// Critical CHECK bodies (normalized) that both dialects must contain.
const CRITICAL_CHECK_SNIPPETS: &[&str] = &[
    // installation strict rule (substring after normalize)
    "account_id is not null and public_key is null",
    "account_id is null and public_key is not null",
    // direct pair order
    "direct_account_low < direct_account_high",
];

pub fn run(workspace_root: &Path) -> Result<()> {
    let sqlite_sql = std::fs::read_to_string(workspace_root.join(SQLITE_PATH))
        .with_context(|| format!("read {SQLITE_PATH}"))?;
    let postgres_sql = std::fs::read_to_string(workspace_root.join(POSTGRES_PATH))
        .with_context(|| format!("read {POSTGRES_PATH}"))?;

    let sqlite = parse_schema(&sqlite_sql, Dialect::Sqlite)?;
    let mut postgres = parse_schema(&postgres_sql, Dialect::Postgres)?;

    for t in PG_ONLY_TABLES {
        postgres.tables.remove(*t);
    }

    let mut errors = Vec::new();

    let sqlite_tables: BTreeSet<_> = sqlite.tables.keys().cloned().collect();
    let postgres_tables: BTreeSet<_> = postgres.tables.keys().cloned().collect();
    for t in sqlite_tables.difference(&postgres_tables) {
        errors.push(format!("table only in sqlite: {t}"));
    }
    for t in postgres_tables.difference(&sqlite_tables) {
        errors.push(format!("table only in postgres: {t}"));
    }

    for table in sqlite_tables.intersection(&postgres_tables) {
        let s = &sqlite.tables[table];
        let p = &postgres.tables[table];

        let s_cols: BTreeSet<_> = s.columns.keys().cloned().collect();
        let p_cols: BTreeSet<_> = p.columns.keys().cloned().collect();
        for c in s_cols.difference(&p_cols) {
            errors.push(format!("{table}: column only in sqlite: {c}"));
        }
        for c in p_cols.difference(&s_cols) {
            errors.push(format!("{table}: column only in postgres: {c}"));
        }
        for c in s_cols.intersection(&p_cols) {
            if s.columns[c].not_null != p.columns[c].not_null {
                errors.push(format!(
                    "{table}.{c}: nullability differs (sqlite not_null={}, postgres not_null={})",
                    s.columns[c].not_null, p.columns[c].not_null
                ));
            }
        }

        if s.primary_key != p.primary_key {
            errors.push(format!(
                "{table}: primary key columns differ sqlite={:?} postgres={:?}",
                s.primary_key, p.primary_key
            ));
        }

        let s_u: BTreeSet<_> = s.uniques.iter().cloned().collect();
        let p_u: BTreeSet<_> = p.uniques.iter().cloned().collect();
        if s_u != p_u {
            errors.push(format!(
                "{table}: unique column-sets differ sqlite={s_u:?} postgres={p_u:?}"
            ));
        }
    }

    let s_fks: BTreeSet<_> = sqlite.fks.iter().cloned().collect();
    let p_fks: BTreeSet<_> = postgres.fks.iter().cloned().collect();
    for fk in s_fks.difference(&p_fks) {
        errors.push(format!("FK only in sqlite: {fk}"));
    }
    for fk in p_fks.difference(&s_fks) {
        errors.push(format!("FK only in postgres: {fk}"));
    }

    let s_norm = normalize_sql(&sqlite_sql);
    let p_norm = normalize_sql(&postgres_sql);
    for snippet in CRITICAL_CHECK_SNIPPETS {
        let sn = snippet.to_ascii_lowercase();
        if !s_norm.contains(&sn) {
            errors.push(format!("sqlite missing critical CHECK snippet: {snippet}"));
        }
        if !p_norm.contains(&sn) {
            errors.push(format!("postgres missing critical CHECK snippet: {snippet}"));
        }
    }

    if !errors.is_empty() {
        eprintln!("schema parity failed ({} issues):", errors.len());
        for e in &errors {
            eprintln!("  - {e}");
        }
        bail!("schema parity: {} issue(s)", errors.len());
    }

    println!(
        "schema parity ok: {} tables (sqlite/postgres logical match)",
        sqlite_tables.len()
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum Dialect {
    Sqlite,
    Postgres,
}

#[derive(Debug, Default)]
struct Column {
    not_null: bool,
}

#[derive(Debug, Default)]
struct Table {
    columns: BTreeMap<String, Column>,
    primary_key: Vec<String>,
    uniques: Vec<Vec<String>>,
}

#[derive(Debug, Default)]
struct Schema {
    tables: BTreeMap<String, Table>,
    fks: Vec<String>,
}

fn normalize_sql(sql: &str) -> String {
    let mut out = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        let mut cleaned = String::new();
        let mut in_squote = false;
        let mut chars = trimmed.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                in_squote = !in_squote;
                cleaned.push(c);
                continue;
            }
            if !in_squote && c == '-' && chars.peek() == Some(&'-') {
                break;
            }
            cleaned.push(c);
        }
        out.push(' ');
        out.push_str(&cleaned);
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn parse_schema(sql: &str, dialect: Dialect) -> Result<Schema> {
    let norm = normalize_sql(sql);
    let mut schema = Schema::default();

    // Split on create table
    let lower = norm.as_str();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("create table ") {
        let start = search_from + rel + "create table ".len();
        let rest = &lower[start..];
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '(')
            .unwrap_or(rest.len());
        let table_name = rest[..name_end].trim().to_string();
        if table_name.is_empty() {
            search_from = start;
            continue;
        }
        // Skip PARTITION OF children bodies handled separately
        if table_name.contains("partition") {
            search_from = start;
            continue;
        }
        let after_name = rest[name_end..].trim_start();
        if !after_name.starts_with('(') {
            search_from = start + name_end;
            continue;
        }
        let body = match extract_paren_body(after_name) {
            Some(b) => b,
            None => {
                search_from = start + name_end;
                continue;
            }
        };
        let mut table = Table::default();
        parse_table_body(body, &mut table, &table_name, &mut schema.fks, dialect);
        // Also pick up UNIQUE indexes outside CREATE TABLE
        schema.tables.insert(table_name, table);
        search_from = start + name_end + body.len();
    }

    // CREATE UNIQUE INDEX ... ON table (cols)
    let mut idx_from = 0;
    while let Some(rel) = lower[idx_from..].find("create unique index ") {
        let start = idx_from + rel;
        let slice = &lower[start..];
        if let Some(on_pos) = slice.find(" on ") {
            let after_on = &slice[on_pos + 4..];
            let table_end = after_on
                .find(|c: char| c.is_whitespace() || c == '(')
                .unwrap_or(after_on.len());
            let table_name = after_on[..table_end].trim().to_string();
            if let Some(paren) = after_on[table_end..].find('(') {
                let cols_src = &after_on[table_end + paren..];
                if let Some(body) = extract_paren_body(cols_src) {
                    let cols = split_ident_list(body);
                    if let Some(t) = schema.tables.get_mut(&table_name) {
                        if !cols.is_empty() {
                            t.uniques.push(cols);
                        }
                    }
                }
            }
        }
        idx_from = start + 20;
    }

    Ok(schema)
}

fn extract_paren_body(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if !s.starts_with('(') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_squote = false;
    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        if c == '\'' {
            in_squote = !in_squote;
            continue;
        }
        if in_squote {
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[1..i]);
            }
        }
    }
    None
}

fn parse_table_body(
    body: &str,
    table: &mut Table,
    table_name: &str,
    fks: &mut Vec<String>,
    _dialect: Dialect,
) {
    for raw_item in split_top_level_commas(body) {
        let item = raw_item.trim();
        if item.is_empty() {
            continue;
        }
        let lower = item.to_ascii_lowercase();

        if lower.starts_with("constraint ")
            || lower.starts_with("check ")
            || lower.starts_with("unique ")
            || lower.starts_with("unique(")
            || lower.starts_with("primary key ")
            || lower.starts_with("primary key(")
            || lower.starts_with("foreign key ")
            || lower.starts_with("foreign key(")
        {
            if lower.contains("primary key") {
                if let Some(cols) = extract_cols_after_keyword(item, "primary key") {
                    table.primary_key = cols;
                } else if let Some(body) = extract_paren_body(
                    item
                        .to_ascii_lowercase()
                        .find("primary key")
                        .map(|p| item[p + "primary key".len()..].trim_start())
                        .unwrap_or(""),
                ) {
                    table.primary_key = split_ident_list(body);
                }
            }
            if lower.contains("unique") && !lower.contains("unique index") {
                if let Some(cols) = extract_cols_after_keyword(item, "unique") {
                    table.uniques.push(cols);
                } else if let Some(paren_at) = lower.find('(') {
                    if let Some(body) = extract_paren_body(&item[paren_at..]) {
                        let cols = split_ident_list(body);
                        if !cols.is_empty() {
                            table.uniques.push(cols);
                        }
                    }
                }
            }
            if let Some(fk) = parse_fk_clause(table_name, item) {
                fks.push(fk);
            }
            continue;
        }

        // Column definition: name type ... 
        let mut parts = item.split_whitespace();
        let Some(col_name) = parts.next() else {
            continue;
        };
        if col_name.eq_ignore_ascii_case("like") {
            continue;
        }
        let not_null = lower.contains(" not null")
            || lower.contains(" primary key")
            || (lower.starts_with(col_name) && lower.contains("primary key"));
        let mut col = Column {
            not_null: not_null || lower.contains(" primary key"),
        };
        // PRIMARY KEY on column
        if lower.contains("primary key") {
            table.primary_key = vec![col_name.to_string()];
            col.not_null = true;
        }
        if lower.contains(" unique") || lower.ends_with(" unique") {
            table.uniques.push(vec![col_name.to_string()]);
        }
        // Inline REFERENCES
        if let Some(fk) = parse_inline_references(table_name, col_name, item) {
            fks.push(fk);
        }
        table.columns.insert(col_name.to_string(), col);
    }
}

fn split_top_level_commas(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_squote = false;
    for c in body.chars() {
        if c == '\'' {
            in_squote = !in_squote;
            cur.push(c);
            continue;
        }
        if !in_squote {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            } else if c == ',' && depth == 0 {
                out.push(std::mem::take(&mut cur));
                continue;
            }
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn extract_cols_after_keyword(item: &str, keyword: &str) -> Option<Vec<String>> {
    let lower = item.to_ascii_lowercase();
    let kw = keyword.to_ascii_lowercase();
    let pos = lower.find(&kw)?;
    let after = item[pos + keyword.len()..].trim_start();
    let body = extract_paren_body(after)?;
    Some(split_ident_list(body))
}

fn split_ident_list(body: &str) -> Vec<String> {
    body.split(',')
        .map(|s| {
            s.split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('"')
                .to_ascii_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_inline_references(from_table: &str, from_col: &str, item: &str) -> Option<String> {
    let lower = item.to_ascii_lowercase();
    let pos = lower.find(" references ")?;
    let after = item[pos + " references ".len()..].trim_start();
    let to_table_end = after
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(after.len());
    let to_table = after[..to_table_end].trim().to_ascii_lowercase();
    let rest = after[to_table_end..].trim_start();
    let to_cols = if rest.starts_with('(') {
        extract_paren_body(rest)
            .map(split_ident_list)
            .unwrap_or_default()
    } else {
        vec![]
    };
    let on_delete = parse_on_delete(&lower);
    Some(format!(
        "{from_table}({}) -> {to_table}({}) on delete {on_delete}",
        from_col.to_ascii_lowercase(),
        to_cols.join(",")
    ))
}

fn parse_fk_clause(from_table: &str, item: &str) -> Option<String> {
    let lower = item.to_ascii_lowercase();
    if !lower.contains("foreign key") && !lower.contains("references") {
        return None;
    }
    if lower.contains("foreign key") {
        let fk_pos = lower.find("foreign key")?;
        let after_fk = item[fk_pos + "foreign key".len()..].trim_start();
        let from_cols = extract_paren_body(after_fk).map(split_ident_list)?;
        let refs_pos = lower.find(" references ")?;
        let after_refs = item[refs_pos + " references ".len()..].trim_start();
        let to_table_end = after_refs
            .find(|c: char| c.is_whitespace() || c == '(')
            .unwrap_or(after_refs.len());
        let to_table = after_refs[..to_table_end].trim().to_ascii_lowercase();
        let to_cols = extract_paren_body(after_refs[to_table_end..].trim_start())
            .map(split_ident_list)
            .unwrap_or_default();
        let on_delete = parse_on_delete(&lower);
        return Some(format!(
            "{from_table}({}) -> {to_table}({}) on delete {on_delete}",
            from_cols.join(","),
            to_cols.join(",")
        ));
    }
    None
}

fn parse_on_delete(lower: &str) -> &'static str {
    if lower.contains("on delete cascade") {
        "cascade"
    } else if lower.contains("on delete set null") {
        "set null"
    } else if lower.contains("on delete restrict") {
        "restrict"
    } else if lower.contains("on delete no action") {
        "no action"
    } else {
        "no action"
    }
}
