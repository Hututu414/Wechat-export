use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    sync::atomic::AtomicBool,
};
use wechat_export::core::wechat::{cipher::PAGE, key, snapshot};

fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).context("需要数据目录路径")?);
    let accounts: Vec<_> = fs::read_dir(&root)?
        .filter_map(|v| v.ok())
        .map(|v| v.path())
        .filter(|p| p.join("db_storage").is_dir())
        .collect();
    if accounts.len() != 1 {
        bail!("测试入口要求根目录下恰好一个账号");
    }
    let files = [
        "session/session.db",
        "contact/contact.db",
        "message/message_0.db",
        "message/message_resource.db",
    ];
    let pages = files
        .iter()
        .map(|name| {
            let mut page = vec![0; PAGE];
            File::open(accounts[0].join("db_storage").join(name))?.read_exact(&mut page)?;
            Ok(page)
        })
        .collect::<Result<Vec<_>>>()?;
    let keys = key::acquire(&pages, &AtomicBool::new(false), |s| eprintln!("{s}"))?;
    fs::create_dir_all(".local/probe")?;
    for ((name, page), key) in files.iter().zip(&pages).zip(keys) {
        let plain = key.decrypt(page, 1)?;
        let schema_header_valid = &plain[..16] == b"SQLite format 3\0";
        eprintln!("{name}: key_valid=true, header_valid={schema_header_valid}");
        let dest = PathBuf::from(".local/probe").join(name.replace('/', "_"));
        snapshot::prepare(
            &accounts[0].join("db_storage").join(name),
            &dest,
            &key,
            &std::sync::Arc::new(AtomicBool::new(false)),
        )?;
        let conn = snapshot::open_readonly(&dest)?;
        let mut stmt =
            conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
        let tables = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen_message = false;
        for table in &tables {
            if table.starts_with("Msg_") && seen_message {
                continue;
            }
            if table.starts_with("Msg_") {
                seen_message = true;
            }
            let mut columns = conn.prepare(&format!(
                "PRAGMA table_info(\"{}\")",
                table.replace('"', "\"\"")
            ))?;
            let columns = columns
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            println!(
                "table={} columns={columns:?}",
                if table.starts_with("Msg_") {
                    "Msg_[redacted]"
                } else {
                    table
                }
            );
        }
        println!("snapshot_valid=true tables={}", tables.len());
    }
    // Only workspace snapshots are written. No key or message content is printed.
    let conn = snapshot::open_readonly(&PathBuf::from(".local/probe/message_message_0.db"))?;
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name GLOB 'Msg_*'")?;
    let tables = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut count = 0;
    let mut chinese = 0;
    for table in tables {
        let mut query = conn.prepare(&format!(
            "SELECT message_content FROM \"{table}\" WHERE (local_type & 4294967295)=1 LIMIT 10"
        ))?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let data = match row.get_ref(0)? {
                rusqlite::types::ValueRef::Text(v) | rusqlite::types::ValueRef::Blob(v) => v,
                _ => continue,
            };
            let bytes = if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
                zstd::stream::decode_all(data)?
            } else {
                data.to_vec()
            };
            let text = std::str::from_utf8(&bytes)?;
            count += 1;
            if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
                chinese += 1;
            }
        }
    }
    println!(
        "RUNTIME-VERIFIED real_text_decoded={count} contains_chinese={chinese}; no text logged"
    );
    Ok(())
}
