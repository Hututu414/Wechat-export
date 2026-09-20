use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub name: String,
    pub is_group: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub conversation_name: String,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub is_self: Option<bool>,
    pub timestamp: i64,
    pub datetime: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub content: String,
    pub raw_type: i64,
    pub media: Option<serde_json::Value>,
    pub quoted: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_xml: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// None = all conversations; Some(empty) = no conversations.
    pub conversations: Option<BTreeSet<String>>,
    pub start: Option<i64>,
    pub end: Option<i64>,
}
impl Filter {
    pub fn dates(start: NaiveDate, end: NaiveDate) -> Result<Self> {
        if end < start {
            bail!("结束日期不能早于开始日期");
        }
        fn midnight(date: NaiveDate) -> Result<i64> {
            let time = date.and_hms_opt(0, 0, 0).context("日期无效")?;
            Local
                .from_local_datetime(&time)
                .single()
                .map(|v| v.timestamp())
                .context("该日期在系统时区中没有唯一的午夜时间")
        }
        Ok(Self {
            start: Some(midnight(start)?),
            end: Some(midnight(end.succ_opt().context("日期超出范围")?)?),
            ..Default::default()
        })
    }
    pub fn includes(&self, id: &str) -> bool {
        self.conversations
            .as_ref()
            .is_none_or(|set| set.contains(id))
    }
    pub fn validate(&self) -> Result<()> {
        if self.start.zip(self.end).is_some_and(|(a, b)| a > b) {
            bail!("时间范围无效");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jsonl,
    Json,
}

#[derive(Clone, Debug)]
pub struct ExportFields {
    pub selected: BTreeSet<&'static str>,
}
impl ExportFields {
    pub const OPTIONS: [(&'static str, &'static str); 16] = [
        ("datetime", "发送时间"),
        ("sender_name", "发送者名称"),
        ("type", "消息类型"),
        ("content", "消息正文"),
        ("conversation_name", "会话名称"),
        ("conversation_id", "会话 ID"),
        ("sender_id", "发送者 ID"),
        ("is_self", "是否本人发送"),
        ("timestamp", "Unix 时间戳（秒）"),
        ("id", "消息 ID"),
        ("media", "媒体信息"),
        ("quoted", "引用消息"),
        ("raw_type", "原始类型编号"),
        ("raw_xml", "原始 XML"),
        ("raw_payload", "原始载荷"),
        ("warnings", "解析警告"),
    ];
    pub fn all() -> Self {
        Self {
            selected: Self::OPTIONS.iter().map(|(key, _)| *key).collect(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.selected.is_empty() {
            bail!("请至少勾选一个导出字段");
        }
        if self
            .selected
            .iter()
            .any(|key| !Self::OPTIONS.iter().any(|(known, _)| known == key))
        {
            bail!("包含不支持的导出字段");
        }
        Ok(())
    }
}
impl Default for ExportFields {
    fn default() -> Self {
        Self {
            selected: BTreeSet::from(["datetime", "sender_name", "type", "content"]),
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct Count {
    pub conversations: u64,
    pub messages: u64,
}
