use super::{
    cipher::PAGE,
    discovery::{self, Account},
    key, snapshot,
};
use crate::core::{
    model::{Conversation, Count, Filter, Message},
    parser::{RawMessage, normalize},
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params, types::ValueRef};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub struct PrivateDir {
    pub path: PathBuf,
    base: PathBuf,
}
impl PrivateDir {
    pub fn new(base: &Path) -> Result<Self> {
        fs::create_dir_all(base).context("无法创建工具工作目录，请放在可写文件夹中运行")?;
        let base = base.canonicalize()?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = base.join(format!("snapshot-{}-{stamp}", std::process::id()));
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{Authorization::*, SECURITY_ATTRIBUTES},
            Storage::FileSystem::CreateDirectoryW,
        };
        let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: the descriptor lives through CreateDirectory; the new folder grants only its owner access.
        unsafe {
            let mut descriptor = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let ok = CreateDirectoryW(discovery::wide(&path).as_ptr(), &attributes);
            let error = std::io::Error::last_os_error();
            LocalFree(descriptor);
            if ok == 0 {
                return Err(error.into());
            }
        }
        Ok(Self { path, base })
    }
}
impl Drop for PrivateDir {
    fn drop(&mut self) {
        // Delete only this unique, newly-created direct child, never an input/data directory.
        if self.path.parent() == Some(self.base.as_path())
            && self
                .path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("snapshot-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("操作已取消");
    }
    Ok(())
}

fn strings(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(sql)?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn uri(path: &Path) -> String {
    let s = path
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/");
    let encoded: String = s
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/:._-".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("file:{encoded}?mode=ro&immutable=1")
}

pub struct WechatClient {
    conn: Connection,
    pub conversations: Vec<Conversation>,
    tables: HashMap<String, Vec<(usize, String)>>,
    names: HashMap<String, String>,
    senders: HashMap<usize, HashMap<i64, String>>,
    pub self_id: String,
    pub self_name: String,
    pub version: String,
    _work: PrivateDir,
}
impl WechatClient {
    pub fn open(
        account: &Account,
        work_root: &Path,
        cancel: Arc<AtomicBool>,
        mut progress: impl FnMut(&str),
    ) -> Result<Self> {
        let version = discovery::detect()?;
        let source = account.directory.join("db_storage");
        let mut files = vec![
            PathBuf::from("session/session.db"),
            PathBuf::from("contact/contact.db"),
            PathBuf::from("message/message_resource.db"),
        ];
        let mut shards = Vec::new();
        for entry in fs::read_dir(source.join("message")).context("未找到消息数据库目录")?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if ["message_", "biz_message_"].iter().any(|prefix| {
                name.strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix(".db"))
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            }) {
                shards.push(PathBuf::from("message").join(name));
            }
        }
        shards.sort();
        if shards.is_empty() {
            bail!("没有找到消息分片，请确认账号目录");
        }
        if shards.len() > 125 {
            bail!("消息分片超过当前 SQLite 支持的 125 个上限");
        }
        files.extend(shards);
        let pages = files
            .iter()
            .map(|p| {
                let mut page = vec![0; PAGE];
                File::open(source.join(p))
                    .context("必要数据库不存在或不可读")?
                    .read_exact(&mut page)?;
                Ok(page)
            })
            .collect::<Result<Vec<_>>>()?;
        let keys = key::acquire(&pages, &cancel, &mut progress)?;
        let work = PrivateDir::new(work_root)?;
        for (i, (file, key)) in files.iter().zip(&keys).enumerate() {
            check_cancel(&cancel)?;
            progress(&format!("准备只读副本 {}/{}", i + 1, files.len()));
            snapshot::prepare(
                &source.join(file),
                &work.path.join(format!("{i}.db")),
                key,
                &cancel,
            )?;
        }
        drop(keys);
        Self::from_snapshot(work, account, version, cancel)
    }
    pub fn from_snapshot(
        work: PrivateDir,
        account: &Account,
        version: String,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        let contact = snapshot::open_readonly(&work.path.join("1.db"))?;
        let mut names = HashMap::new();
        for table in ["stranger", "contact"] {
            let sql = format!(
                "SELECT username,COALESCE(NULLIF(remark,''),NULLIF(nick_name,''),username) FROM {table}"
            );
            let mut stmt = contact
                .prepare(&sql)
                .context("联系人数据库 schema 不兼容")?;
            for row in
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            {
                let (id, name) = row?;
                names.insert(id, name);
            }
        }
        let resource = snapshot::open_readonly(&work.path.join("2.db"))?;
        let resource_senders: HashMap<i64, String> = resource
            .prepare("SELECT rowid,user_name FROM SenderName2Id")
            .context("发送者映射 schema 不兼容")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let folder = account
            .directory
            .file_name()
            .context("账号目录无效")?
            .to_string_lossy();
        let identity: Vec<_> = resource_senders
            .values()
            .filter(|id| {
                folder == id.as_str()
                    || folder
                        .strip_prefix(id.as_str())
                        .is_some_and(|s| s.starts_with('_'))
            })
            .collect();
        if identity.len() != 1 {
            bail!("无法唯一确认当前账号，目录名称与发送者映射不匹配");
        }
        let self_id = identity[0].clone();
        let self_name = names.get(&self_id).unwrap_or(&self_id).clone();
        let mut ids = strings(&resource, "SELECT user_name FROM ChatName2Id")?;
        let session = snapshot::open_readonly(&work.path.join("0.db"))?;
        ids.extend(strings(&session, "SELECT username FROM SessionTable")?);
        drop((contact, resource, session));
        let conn = Connection::open_in_memory()?;
        // SQLite external sorts spill to this private folder, never to the WeChat directory.
        conn.pragma_update(
            None,
            "temp_store_directory",
            work.path.to_string_lossy().as_ref(),
        )?;
        conn.pragma_update(None, "temp_store", "FILE")?;
        let interrupted = cancel.clone();
        conn.progress_handler(2000, Some(move || interrupted.load(Ordering::Relaxed)))?;
        let mut tables: HashMap<String, Vec<(usize, String)>> = HashMap::new();
        let mut senders = HashMap::new();
        let mut conversations = BTreeMap::new();
        for id in &ids {
            if !id.is_empty() {
                conversations.insert(
                    id.clone(),
                    Conversation {
                        id: id.clone(),
                        name: names.get(id).unwrap_or(id).clone(),
                        is_group: id.ends_with("@chatroom"),
                    },
                );
            }
        }
        for shard in 0..125 {
            check_cancel(&cancel)?;
            let path = work.path.join(format!("{}.db", shard + 3));
            if !path.exists() {
                break;
            }
            let alias = format!("s{shard}");
            conn.execute(&format!("ATTACH DATABASE ? AS {alias}"), [uri(&path)])?;
            conn.execute_batch(&format!("PRAGMA {alias}.cache_size=-512;"))?;
            let local_senders: HashMap<i64, String> = conn
                .prepare(&format!("SELECT rowid,user_name FROM {alias}.Name2Id"))
                .context("消息索引 schema 不兼容")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut mapping = HashMap::new();
            for id in ids.iter().chain(local_senders.values()) {
                mapping.insert(discovery::message_table(id)?, id.clone());
            }
            senders.insert(shard, local_senders);
            let found = strings(
                &conn,
                &format!(
                    "SELECT name FROM {alias}.sqlite_master WHERE type='table' AND name GLOB 'Msg_*'"
                ),
            )?;
            for table in found {
                if table.len() != 36 || !table[4..].bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("消息表名称 schema 不兼容");
                }
                let id = mapping
                    .get(&table)
                    .cloned()
                    .unwrap_or_else(|| format!("unresolved:{}", &table[4..]));
                // Unknown conversation mapping is retained instead of silently dropping a table.
                conversations
                    .entry(id.clone())
                    .or_insert_with(|| Conversation {
                        id: id.clone(),
                        name: names.get(&id).unwrap_or(&id).clone(),
                        is_group: id.ends_with("@chatroom"),
                    });
                conn.prepare(&format!("SELECT local_id,server_id,local_type,real_sender_id,create_time,sort_seq,message_content,compress_content,source,packed_info_data FROM {alias}.{} LIMIT 0",quote(&table))).context("消息数据库 schema 不兼容")?;
                tables.entry(id).or_default().push((shard, table));
            }
        }
        conn.pragma_update(None, "query_only", true)?;
        Ok(Self {
            conn,
            conversations: conversations.into_values().collect(),
            tables,
            names,
            senders,
            self_id,
            self_name,
            version,
            _work: work,
        })
    }
    fn query(&self, id: &str, filter: &Filter, count: bool) -> Option<String> {
        let tables = self.tables.get(id)?;
        let union=tables.iter().map(|(db,table)|format!("SELECT {db} AS db,local_id,server_id,create_time,sort_seq FROM s{db}.{} WHERE (?1 IS NULL OR create_time>=?1) AND (?2 IS NULL OR create_time<?2)",quote(table))).collect::<Vec<_>>().join(" UNION ALL ");
        let cte = format!(
            "WITH refs AS ({union}), chosen AS (SELECT *,ROW_NUMBER() OVER (PARTITION BY CASE WHEN server_id<>0 THEN 's'||server_id ELSE 'l'||db||':'||local_id END ORDER BY db,local_id) AS n FROM refs)"
        );
        let _ = filter;
        Some(if count {
            format!("{cte} SELECT COUNT(*) FROM chosen WHERE n=1")
        } else {
            format!(
                "{cte} SELECT db,local_id FROM chosen WHERE n=1 ORDER BY create_time,sort_seq,server_id,db,local_id"
            )
        })
    }
    pub fn count_messages(&self, filter: &Filter, cancel: &AtomicBool) -> Result<Count> {
        filter.validate()?;
        let mut count = Count::default();
        for conversation in &self.conversations {
            check_cancel(cancel)?;
            if !filter.includes(&conversation.id) {
                continue;
            }
            if let Some(sql) = self.query(&conversation.id, filter, true) {
                let n = u64::try_from(self.conn.query_row(
                    &sql,
                    params![filter.start, filter.end],
                    |r| r.get::<_, i64>(0),
                )?)?;
                if n > 0 {
                    count.conversations += 1;
                    count.messages += n;
                }
            }
        }
        Ok(count)
    }
    pub fn for_each_message(
        &self,
        filter: &Filter,
        cancel: &AtomicBool,
        mut emit: impl FnMut(Message) -> Result<()>,
    ) -> Result<()> {
        filter.validate()?;
        for conversation in &self.conversations {
            check_cancel(cancel)?;
            if !filter.includes(&conversation.id) {
                continue;
            }
            let Some(sql) = self.query(&conversation.id, filter, false) else {
                continue;
            };
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![filter.start, filter.end])?;
            while let Some(row) = rows.next()? {
                check_cancel(cancel)?;
                let db = usize::try_from(row.get::<_, i64>(0)?)?;
                let local_id: i64 = row.get(1)?;
                let table = self.tables[&conversation.id]
                    .iter()
                    .find(|(i, _)| *i == db)
                    .context("消息分片索引无效")?
                    .1
                    .as_str();
                let sql = format!(
                    "SELECT local_id,server_id,local_type,real_sender_id,create_time,message_content,compress_content,source,packed_info_data FROM s{db}.{} WHERE local_id=?",
                    quote(table)
                );
                let raw = self.conn.prepare_cached(&sql)?.query_row([local_id], |r| {
                    fn bytes(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Vec<u8>> {
                        Ok(match r.get_ref(i)? {
                            ValueRef::Text(v) | ValueRef::Blob(v) => v.to_vec(),
                            ValueRef::Null => Vec::new(),
                            v => format!("{v:?}").into_bytes(),
                        })
                    }
                    Ok(RawMessage {
                        local_id: r.get(0)?,
                        server_id: r.get(1)?,
                        raw_type: r.get(2)?,
                        sender: r.get(3)?,
                        timestamp: r.get(4)?,
                        body: bytes(r, 5)?,
                        compressed: bytes(r, 6)?,
                        source: bytes(r, 7)?,
                        packed: bytes(r, 8)?,
                    })
                })?;
                let sender = self
                    .senders
                    .get(&db)
                    .and_then(|names| names.get(&raw.sender))
                    .filter(|s| !s.is_empty())
                    .map(String::as_str);
                let name = sender.map(|s| self.names.get(s).map(String::as_str).unwrap_or(s));
                emit(normalize(
                    raw,
                    conversation,
                    sender,
                    name,
                    &self.self_id,
                    db,
                ))?;
            }
        }
        Ok(())
    }
}
