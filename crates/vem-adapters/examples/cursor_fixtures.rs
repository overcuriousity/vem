//! Builds the binary Cursor fixtures (SQLite dbs) from JSON recipes.
//!
//! `cargo run -p vem-adapters --example cursor_fixtures -- fixtures/cursor/recipes fixtures/cursor`
//!
//! A recipe is `{ "out": "<path under the output dir>", "kind": "kv" | "chat-store" | "raw",
//! "tables": { "<table>": { "<key>": <value>, … } }, "wal_rows": { same shape }? }`, or for `raw`
//! `{ "out", "kind": "raw", "text" | "hex": … }`.
//! - `kv` tables are `(key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)` (Cursor's `ItemTable` and
//!   `cursorDiskKV`). `chat-store` has `blobs (id TEXT PRIMARY KEY, data BLOB)` and
//!   `meta (key TEXT PRIMARY KEY, value TEXT)`; its `tables` must name only those two.
//! - A value that is a string is stored as text; `{"hex": "…"}` as a blob of those bytes;
//!   `{"pb": [[field, "varint"|"bytes"|"str"|"msg", value], …]}` as a protobuf-encoded blob (`bytes` is
//!   hex, `msg` is a nested field list); any other JSON as its serialized text.
//! - `wal_rows` are written after a checkpoint with autocheckpoint off, and the db and its `-wal` are
//!   copied out while the writer is still open, so those rows exist only in `<out>-wal`.

use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::Path;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn pb(fields: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields.as_array().expect("pb field list") {
        let num = f[0].as_u64().expect("field number");
        let payload = match f[1].as_str().expect("wire type") {
            "varint" => {
                varint(num << 3, &mut out);
                varint(f[2].as_u64().expect("varint value"), &mut out);
                continue;
            }
            "bytes" => hex(f[2].as_str().expect("hex")),
            "str" => f[2].as_str().expect("str").as_bytes().to_vec(),
            "msg" => pb(&f[2]),
            t => panic!("unknown wire type {t}"),
        };
        varint((num << 3) | 2, &mut out);
        varint(payload.len() as u64, &mut out);
        out.extend(payload);
    }
    out
}

fn sql_value(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::String(s) => S::Text(s.clone()),
        Value::Object(o) if o.contains_key("hex") => S::Blob(hex(o["hex"].as_str().expect("hex"))),
        Value::Object(o) if o.contains_key("pb") => S::Blob(pb(&o["pb"])),
        other => S::Text(other.to_string()),
    }
}

fn insert_rows(conn: &Connection, kind: &str, tables: &Value) {
    for (table, rows) in tables.as_object().expect("tables object") {
        let sql = match (kind, table.as_str()) {
            ("kv", t) => format!("INSERT INTO \"{t}\" (key, value) VALUES (?1, ?2)"),
            ("chat-store", "blobs") => "INSERT INTO blobs (id, data) VALUES (?1, ?2)".into(),
            ("chat-store", "meta") => "INSERT INTO meta (key, value) VALUES (?1, ?2)".into(),
            (k, t) => panic!("table {t} not allowed in a {k} recipe"),
        };
        for (key, value) in rows.as_object().expect("rows object") {
            conn.execute(&sql, params![key, sql_value(value)])
                .expect("insert");
        }
    }
}

fn build(recipe: &Value, out_dir: &Path) {
    let out = out_dir.join(recipe["out"].as_str().expect("out"));
    std::fs::create_dir_all(out.parent().unwrap()).expect("mkdir");
    let kind = recipe["kind"].as_str().expect("kind");
    if kind == "raw" {
        let bytes = match (recipe.get("text"), recipe.get("hex")) {
            (Some(t), _) => t.as_str().expect("text").as_bytes().to_vec(),
            (_, Some(h)) => hex(h.as_str().expect("hex")),
            _ => panic!("raw recipe needs text or hex"),
        };
        std::fs::write(&out, bytes).expect("write");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let db = tmp.path().join("db");
    let conn = Connection::open(&db).expect("open");
    conn.execute_batch("PRAGMA journal_mode = WAL;")
        .expect("wal");
    let empty = Value::Object(Default::default());
    let tables = recipe.get("tables").unwrap_or(&empty);
    match kind {
        "kv" => {
            let wal = recipe.get("wal_rows").unwrap_or(&empty);
            let mut names: Vec<&String> = tables.as_object().unwrap().keys().collect();
            names.extend(wal.as_object().expect("wal_rows object").keys());
            names.sort();
            names.dedup();
            for t in names {
                conn.execute_batch(&format!(
                    "CREATE TABLE \"{t}\" (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);"
                ))
                .expect("create");
            }
        }
        "chat-store" => conn
            .execute_batch(
                "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
            )
            .expect("create"),
        k => panic!("unknown recipe kind {k}"),
    }
    insert_rows(&conn, kind, tables);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA wal_autocheckpoint = 0;")
        .expect("checkpoint");
    let wal_out = {
        let mut p = out.clone().into_os_string();
        p.push("-wal");
        std::path::PathBuf::from(p)
    };
    let _ = std::fs::remove_file(&wal_out);
    if let Some(wal) = recipe.get("wal_rows") {
        insert_rows(&conn, kind, wal);
        std::fs::copy(tmp.path().join("db-wal"), &wal_out).expect("copy wal");
    }
    std::fs::copy(&db, &out).expect("copy db");
    drop(conn);
    println!("{}", out.display());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, recipes, out_dir] = args.as_slice() else {
        eprintln!("usage: cursor_fixtures <recipes dir> <output dir>");
        std::process::exit(2);
    };
    let mut paths: Vec<_> = std::fs::read_dir(recipes)
        .expect("recipes dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p).expect("read recipe");
        let recipe: Value = serde_json::from_str(&text).expect("recipe json");
        for r in recipe.as_array().cloned().unwrap_or_else(|| vec![recipe]) {
            build(&r, Path::new(out_dir));
        }
    }
}
