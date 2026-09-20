use super::{
    model::{ExportFields, Filter, Format, Message},
    wechat::{
        database::{WechatClient, check_cancel},
        discovery::wide,
    },
};
use anyhow::{Context, Result, bail};
use serde::{Serializer, ser::SerializeMap};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub struct ExportConfig {
    pub filter: Filter,
    pub format: Format,
    pub fields: ExportFields,
    pub directory: PathBuf,
}
#[derive(Clone)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub conversation: String,
    pub elapsed: f64,
}
pub struct ExportResult {
    pub path: PathBuf,
    pub messages: u64,
    pub elapsed: f64,
}

struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub struct StreamWriter<W: Write> {
    writer: W,
    format: Format,
    fields: ExportFields,
    count: u64,
}
impl<W: Write> StreamWriter<W> {
    pub fn new(mut writer: W, format: Format, fields: ExportFields) -> Result<Self> {
        fields.validate()?;
        if format == Format::Json {
            writer.write_all(b"[")?;
        }
        Ok(Self {
            writer,
            format,
            fields,
            count: 0,
        })
    }
    pub fn message(&mut self, message: &Message) -> Result<()> {
        if self.format == Format::Json && self.count > 0 {
            self.writer.write_all(b",")?;
        }
        // Serialize selected values by reference, without cloning raw payloads or buffering rows.
        let mut serializer = serde_json::Serializer::new(&mut self.writer);
        let mut map = serializer.serialize_map(None)?;
        macro_rules! field {
            ($key:literal, $value:expr) => {
                if self.fields.selected.contains($key) {
                    map.serialize_entry($key, $value)?;
                }
            };
        }
        field!("id", &message.id);
        field!("conversation_id", &message.conversation_id);
        field!("conversation_name", &message.conversation_name);
        field!("sender_id", &message.sender_id);
        field!("sender_name", &message.sender_name);
        field!("is_self", &message.is_self);
        field!("timestamp", &message.timestamp);
        field!("datetime", &message.datetime);
        field!("type", &message.kind);
        field!("content", &message.content);
        field!("raw_type", &message.raw_type);
        field!("media", &message.media);
        field!("quoted", &message.quoted);
        if let Some(value) = &message.raw_xml {
            field!("raw_xml", value);
        }
        if let Some(value) = &message.raw_payload {
            field!("raw_payload", value);
        }
        if !message.warnings.is_empty() {
            field!("warnings", &message.warnings);
        }
        map.end()?;
        self.writer.write_all(b"\n")?;
        self.count += 1;
        Ok(())
    }
    pub fn finish(mut self) -> Result<W> {
        if self.format == Format::Json {
            self.writer.write_all(b"]\n")?;
        }
        self.writer.flush()?;
        Ok(self.writer)
    }
}
pub fn export(
    client: &WechatClient,
    config: &ExportConfig,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<ExportResult> {
    let start = Instant::now();
    config.fields.validate()?;
    if !config.directory.is_dir() {
        bail!("输出目录不存在");
    }
    let total = client.count_messages(&config.filter, cancel)?.messages;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let extension = if config.format == Format::Jsonl {
        "jsonl"
    } else {
        "json"
    };
    let path = config
        .directory
        .join(format!("wechat-{}-{stamp}.{extension}", std::process::id()));
    let partial_path = path.with_extension(format!("{extension}.partial"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial_path)
        .context("输出目录不可写")?;
    let partial = Partial(partial_path);
    let mut writer = StreamWriter::new(
        BufWriter::with_capacity(256 * 1024, file),
        config.format,
        config.fields.clone(),
    )?;
    let mut count = 0;
    let mut last = Instant::now();
    client.for_each_message(&config.filter, cancel, |message| {
        check_cancel(cancel)?;
        writer
            .message(&message)
            .context("输出写入失败，请检查磁盘空间和目录权限")?;
        count += 1;
        if last.elapsed().as_millis() >= 100 || count == total {
            progress(Progress {
                done: count,
                total,
                conversation: message.conversation_name.clone(),
                elapsed: start.elapsed().as_secs_f64(),
            });
            last = Instant::now();
        }
        Ok(())
    })?;
    check_cancel(cancel)?;
    if count != total {
        bail!("导出数量与预览不一致，未生成最终文件");
    }
    let writer = writer.finish()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    // MoveFileEx without REPLACE_EXISTING: never overwrite a user's existing export.
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    if unsafe {
        MoveFileExW(
            wide(&partial.0).as_ptr(),
            wide(&path).as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("无法完成导出文件，请检查磁盘和同名文件");
    }
    Ok(ExportResult {
        path,
        messages: count,
        elapsed: start.elapsed().as_secs_f64(),
    })
}
pub fn write_log(directory: &Path, error: &anyhow::Error) -> Result<()> {
    fs::create_dir_all(directory)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("wechat-export.log"))?;
    // Error chains contain only our fixed diagnostics and OS/SQLite errors, never data rows or keys.
    writeln!(log, "{} {error:#}", chrono::Local::now().to_rfc3339())?;
    Ok(())
}
