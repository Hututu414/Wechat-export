use anyhow::Result;
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use wechat_export::core::wechat::{
    database::{PrivateDir, WechatClient},
    discovery::{Account, message_table},
};
pub fn client(rows: u32, cancel: Arc<AtomicBool>) -> Result<WechatClient> {
    let work = PrivateDir::new(&PathBuf::from(".local/tests"))?;
    let session = Connection::open(work.path.join("0.db"))?;
    session.execute_batch("CREATE TABLE SessionTable(username TEXT);INSERT INTO SessionTable VALUES ('peer'),('room@chatroom'),('empty');")?;
    drop(session);
    let contacts = Connection::open(work.path.join("1.db"))?;
    contacts.execute_batch("CREATE TABLE contact(username TEXT,nick_name TEXT,remark TEXT); CREATE TABLE stranger(username TEXT,nick_name TEXT,remark TEXT); INSERT INTO contact VALUES ('self','自己',''),('peer','联系人','好友'),('member','群成员',''),('room@chatroom','测试群','');")?;
    drop(contacts);
    let resource = Connection::open(work.path.join("2.db"))?;
    resource.execute_batch("CREATE TABLE SenderName2Id(user_name TEXT); INSERT INTO SenderName2Id VALUES ('self'),('peer'),('member'); CREATE TABLE ChatName2Id(user_name TEXT); INSERT INTO ChatName2Id VALUES ('peer'),('room@chatroom');")?;
    drop(resource);
    for shard in 0..2 {
        let mut db = Connection::open(work.path.join(format!("{}.db", shard + 3)))?;
        db.execute_batch(&format!("CREATE TABLE Name2Id(user_name TEXT);INSERT INTO Name2Id(rowid,user_name) VALUES ({},'self'),({},'peer'),({},'member'),({},'room@chatroom');",5+shard*3,6+shard*3,7+shard*3,8+shard*3))?;
        for id in ["peer", "room@chatroom"] {
            let table = message_table(id)?;
            db.execute_batch(&format!("CREATE TABLE {table}(local_id INTEGER PRIMARY KEY,server_id INTEGER,local_type INTEGER,real_sender_id INTEGER,create_time INTEGER,sort_seq INTEGER,message_content BLOB,compress_content BLOB,source BLOB,packed_info_data BLOB); CREATE INDEX {table}_time ON {table}(create_time,sort_seq);"))?;
            let tx = db.transaction()?;
            {
                let mut insert = tx.prepare(&format!(
                    "INSERT INTO {table} VALUES (?1,?2,1,?3,?4,?5,?6,NULL,NULL,NULL)"
                ))?;
                for i in 0..rows {
                    // Deliberately overlap one server id across shards; local_id repeats on both.
                    let server = if i == 0 {
                        1
                    } else {
                        i as i64 + shard as i64 * rows as i64 + 1
                    };
                    let time = 1767225600 + i as i64;
                    let sender = (if id == "room@chatroom" {
                        3
                    } else if i % 2 == 0 {
                        1
                    } else {
                        2
                    }) + 4
                        + shard * 3;
                    insert.execute(rusqlite::params![
                        i + 1,
                        server,
                        sender,
                        time,
                        i,
                        if id == "room@chatroom" {
                            "member:\n中文 😀\n第二行"
                        } else {
                            "中文 😀\n第二行"
                        }
                    ])?;
                }
            }
            tx.commit()?;
        }
        drop(db);
    }
    WechatClient::from_snapshot(
        work,
        &Account {
            directory: PathBuf::from("self_suffix"),
            label: "test".into(),
        },
        "fixture".into(),
        cancel,
    )
}
