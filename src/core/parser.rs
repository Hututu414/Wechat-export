use super::model::{Conversation, Message};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Local, TimeZone};
use serde_json::json;
use std::io::Read;

#[derive(Default)]
struct Xml {
    name: String,
    text: String,
    attributes: std::collections::BTreeMap<String, String>,
    children: Vec<Xml>,
}
impl Xml {
    fn find(&self, name: &str) -> Option<&Xml> {
        if self.name == name {
            Some(self)
        } else {
            self.children.iter().find_map(|n| n.find(name))
        }
    }
    fn value(&self, name: &str) -> String {
        self.find(name).map(|n| n.text.clone()).unwrap_or_default()
    }
    fn attr(&self, name: &str) -> String {
        self.attributes.get(name).cloned().unwrap_or_default()
    }
}
fn xml(text: &str) -> Option<Xml> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_str(text);
    let mut stack = vec![Xml::default()];
    let mut count = 0;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) | Event::Empty(e) => {
                // Reader position distinguishes self-closing elements without retaining borrowed events.
                let empty =
                    text.as_bytes().get(reader.buffer_position() as usize - 2) == Some(&b'/');
                count += 1;
                if count > 65536 || stack.len() > 64 {
                    return None;
                }
                let mut node = Xml {
                    name: std::str::from_utf8(e.name().as_ref()).ok()?.into(),
                    ..Default::default()
                };
                for attribute in e.attributes() {
                    let a = attribute.ok()?;
                    node.attributes.insert(
                        std::str::from_utf8(a.key.as_ref()).ok()?.into(),
                        a.decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .ok()?
                        .into_owned(),
                    );
                }
                if empty {
                    stack.last_mut()?.children.push(node);
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return None;
                }
                let node = stack.pop()?;
                stack.last_mut()?.children.push(node);
            }
            Event::Text(e) => stack.last_mut()?.text.push_str(&e.decode().ok()?),
            Event::CData(e) => stack.last_mut()?.text.push_str(&e.decode().ok()?),
            Event::GeneralRef(e) => {
                let encoded = format!("&{};", e.decode().ok()?);
                stack
                    .last_mut()?
                    .text
                    .push_str(&quick_xml::escape::unescape(&encoded).ok()?);
            }
            Event::DocType(_) => return None,
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return None;
    }
    let root = stack.pop()?;
    if root.children.len() != 1 || !root.text.trim().is_empty() {
        return None;
    }
    root.children.into_iter().next()
}
fn system_text(root: &Xml) -> String {
    for name in ["replacemsg", "plain"] {
        let text = root.value(name);
        if !text.is_empty() {
            return text;
        }
    }
    let mut template = root.value("template");
    if !template.is_empty() {
        if let Some(links) = root.find("link_list") {
            for link in &links.children {
                let text = if link.attr("hidden") == "1" {
                    String::new()
                } else {
                    let mut text = link.value("title");
                    if text.is_empty() {
                        text = link
                            .find("memberlist")
                            .map(|m| {
                                m.children
                                    .iter()
                                    .map(|n| n.value("nickname"))
                                    .collect::<Vec<_>>()
                                    .join("、")
                            })
                            .unwrap_or_default();
                    }
                    text
                };
                template = template.replace(&format!("${}$", link.attr("name")), &text);
            }
        }
        return template;
    }
    root.value("content")
}

pub struct RawMessage {
    pub local_id: i64,
    pub server_id: i64,
    pub raw_type: i64,
    pub sender: i64,
    pub timestamp: i64,
    pub body: Vec<u8>,
    pub compressed: Vec<u8>,
    pub source: Vec<u8>,
    pub packed: Vec<u8>,
}
pub fn decode(data: &[u8]) -> Option<String> {
    const LIMIT: u64 = 64 * 1024 * 1024;
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut bytes = Vec::new();
        let mut decoder = zstd::stream::read::Decoder::new(data).ok()?;
        decoder.window_log_max(26).ok()?;
        decoder.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > LIMIT {
            return None;
        }
        String::from_utf8(bytes).ok()
    } else {
        std::str::from_utf8(data).ok().map(str::to_owned)
    }
}
pub fn normalize(
    raw: RawMessage,
    conversation: &Conversation,
    sender: Option<&str>,
    sender_name: Option<&str>,
    self_id: &str,
    shard: usize,
) -> Message {
    let decoded = decode(&raw.body)
        .filter(|s| !s.is_empty())
        .or_else(|| decode(&raw.compressed).filter(|s| !s.is_empty()));
    let mut content = decoded.clone().unwrap_or_default();
    if conversation.is_group
        && let Some(sender) = sender
        && let Some(rest) = content.strip_prefix(&format!("{sender}:\n"))
    {
        content = rest.to_owned();
    }
    let base = raw.raw_type & 0xffffffff;
    let mut kind = match base {
        1 => "text",
        3 => "image",
        34 => "voice",
        43 | 62 => "video",
        47 => "sticker",
        48 => "location",
        10000 | 10002 => "system",
        _ => "unknown",
    };
    let datetime = Local
        .timestamp_opt(raw.timestamp, 0)
        .single()
        .map(|v| v.to_rfc3339())
        .unwrap_or_default();
    let mut warnings = Vec::new();
    if sender.is_none() {
        warnings.push("sender_unresolved".into());
    }
    if datetime.is_empty() {
        warnings.push("timestamp_out_of_range".into());
    }
    let mut media = None;
    let mut quoted = None;
    let mut raw_xml = None;
    if base != 1 && content.trim_start().starts_with('<') {
        raw_xml = Some(content.clone());
        if let Some(root) = xml(&content) {
            if base == 49
                && let Some(app) = root.find("appmsg")
            {
                let subtype = app
                    .children
                    .iter()
                    .find(|n| n.name == "type")
                    .map(|n| n.text.as_str())
                    .unwrap_or("");
                kind = match subtype {
                    "6" | "74" => "file",
                    "57" => "quote",
                    "5" => "link",
                    _ => "unknown",
                };
                if kind != "unknown" {
                    content = app.value("title");
                }
                if kind == "quote"
                    && let Some(q) = app.find("refermsg")
                {
                    quoted = Some(
                        json!({"id":q.value("svrid"),"sender_id":q.value("fromusr"),"sender_name":q.value("displayname"),"timestamp":q.value("createtime").parse::<i64>().ok(),"raw_type":q.value("type"),"content":q.value("content")}),
                    );
                }
                if kind == "file" || kind == "link" {
                    media = Some(
                        json!({"filename":app.value("title"),"original_path":app.value("filepath"),"hash":app.value("md5"),"metadata":{"url":app.value("url"),"file_extension":app.value("fileext"),"size":app.value("totallen"),"attach_id":app.value("attachid")}}),
                    );
                }
            }
            if kind == "system" {
                let text = system_text(&root);
                if !text.is_empty() {
                    content = text;
                } else {
                    warnings.push("system_xml_unresolved".into());
                }
            }
            let tag = match kind {
                "image" => "img",
                "voice" => "voicemsg",
                "video" => "videomsg",
                "sticker" => "emoji",
                "location" => "location",
                _ => "",
            };
            if !tag.is_empty()
                && let Some(node) = root.find(tag)
            {
                media = Some(
                    json!({"filename":node.attr("filename"),"original_path":node.attr("path"),"hash":node.attr("md5"),"metadata":node.attributes}),
                );
                content = if kind == "location" {
                    node.attr("label")
                } else {
                    String::new()
                };
            }
        } else {
            warnings.push("xml_unparsed".into());
            kind = "unknown";
        }
    }
    if decoded.is_none() && (!raw.body.is_empty() || !raw.compressed.is_empty()) {
        warnings.push("payload_decode_failed".into());
        kind = "unknown";
    }
    if base == 1 && decoded.is_none() && raw.body.is_empty() && raw.compressed.is_empty() {
        kind = "text";
    }
    if matches!(
        kind,
        "image" | "voice" | "video" | "file" | "sticker" | "location" | "link"
    ) {
        let value = media.get_or_insert_with(|| json!({"metadata":{}}));
        if !raw.packed.is_empty() {
            value["packed_info_base64"] = json!(STANDARD.encode(&raw.packed));
        }
    }
    let payload=(kind=="unknown" || !raw.source.is_empty() || (raw_xml.is_none()&&kind!="text")).then(||json!({"encoding":"base64","message_content":STANDARD.encode(&raw.body),"compress_content":STANDARD.encode(&raw.compressed),"source":STANDARD.encode(&raw.source),"packed_info_data":STANDARD.encode(&raw.packed),"real_sender_id":raw.sender}));
    Message {
        id: if raw.server_id != 0 {
            format!("{}:s:{}", conversation.id, raw.server_id)
        } else {
            format!("{}:l:{shard}:{}", conversation.id, raw.local_id)
        },
        conversation_id: conversation.id.clone(),
        conversation_name: conversation.name.clone(),
        sender_id: sender.map(str::to_owned),
        sender_name: sender_name.map(str::to_owned),
        is_self: sender.map(|v| v == self_id),
        timestamp: raw.timestamp,
        datetime,
        kind: kind.into(),
        content,
        raw_type: raw.raw_type,
        media,
        quoted,
        raw_xml,
        raw_payload: payload,
        warnings,
    }
}
