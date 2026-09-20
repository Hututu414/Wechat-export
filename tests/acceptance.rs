mod support;
use anyhow::Result;
use chrono::{Days, Local, NaiveDate, TimeZone};
use std::{
    collections::{BTreeSet, HashSet},
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use wechat_export::core::{
    export::StreamWriter,
    model::{Conversation, ExportFields, Filter, Format, Message},
    parser::{RawMessage, decode, normalize},
};

#[test]
fn shards_filters_senders_streams_and_cancel() -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let client = support::client(8, cancel.clone())?;
    assert_eq!(client.self_id, "self"); // Sender row id 2 is deliberately the OTHER user.
    let mut cases = vec![
        (Filter::default(), 30),
        (
            Filter {
                conversations: Some(BTreeSet::from(["peer".into()])),
                ..Default::default()
            },
            15,
        ),
        (
            Filter {
                conversations: Some(BTreeSet::from(["peer".into(), "room@chatroom".into()])),
                ..Default::default()
            },
            30,
        ),
        (
            Filter {
                conversations: Some(BTreeSet::from(["empty".into()])),
                ..Default::default()
            },
            0,
        ),
        (
            Filter {
                conversations: Some(BTreeSet::new()),
                ..Default::default()
            },
            0,
        ),
        (
            Filter {
                start: Some(1767225601),
                end: Some(1767225604),
                ..Default::default()
            },
            12,
        ),
        (
            Filter {
                start: Some(42),
                end: Some(42),
                ..Default::default()
            },
            0,
        ),
    ];
    let today = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
    for days in [7, 30, 90] {
        cases.push((
            Filter::dates(today.checked_sub_days(Days::new(days - 1)).unwrap(), today)?,
            30,
        ));
    }
    for (filter, expected) in cases {
        assert_eq!(client.count_messages(&filter, &cancel)?.messages, expected);
        for format in [Format::Jsonl, Format::Json] {
            let mut out = StreamWriter::new(Vec::new(), format, ExportFields::all())?;
            let mut seen = HashSet::new();
            let mut prior = String::new();
            let mut time = i64::MIN;
            client.for_each_message(&filter, &cancel, |m| {
                assert!(seen.insert(m.id.clone()));
                if m.conversation_id == prior {
                    assert!(m.timestamp >= time);
                }
                prior = m.conversation_id.clone();
                time = m.timestamp;
                assert_eq!(m.content, "中文 😀\n第二行");
                assert_eq!(m.is_self, Some(m.sender_id.as_deref() == Some("self")));
                if m.conversation_id == "room@chatroom" {
                    assert_eq!(m.sender_id.as_deref(), Some("member"));
                    assert_eq!(m.sender_name.as_deref(), Some("群成员"));
                }
                out.message(&m)
            })?;
            let bytes = out.finish()?;
            let messages: Vec<Message> = if format == Format::Json {
                serde_json::from_slice(&bytes)?
            } else {
                String::from_utf8(bytes)?
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<serde_json::Result<_>>()?
            };
            assert_eq!(messages.len() as u64, expected);
        }
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(client.count_messages(&Filter::default(), &cancel).is_err());
    Ok(())
}
#[test]
fn inclusive_end_day_and_invalid_dates() -> Result<()> {
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap();
    let filter = Filter::dates(start, end)?;
    assert_eq!(
        filter.start,
        Some(
            Local
                .from_local_datetime(&start.and_hms_opt(0, 0, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        )
    );
    assert_eq!(
        filter.end,
        Some(
            Local
                .from_local_datetime(&end.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        )
    );
    assert!(Filter::dates(end, start).is_err());
    Ok(())
}
fn message(kind: i64, body: &str) -> Message {
    normalize(
        RawMessage {
            local_id: 1,
            server_id: 8,
            raw_type: kind,
            sender: 2,
            timestamp: 1767225600,
            body: body.as_bytes().to_vec(),
            compressed: vec![],
            source: vec![],
            packed: vec![],
        },
        &Conversation {
            id: "peer".into(),
            name: "好友".into(),
            is_group: false,
        },
        Some("peer"),
        Some("好友"),
        "self",
        0,
    )
}
#[test]
fn types_xml_entities_quotes_and_unknown_are_lossless() -> Result<()> {
    let quote = "<msg><appmsg><title><![CDATA[回复 <中文> 😀]]></title><type>57</type><refermsg><svrid>123</svrid><fromusr>member</fromusr><displayname>A &amp; B</displayname><content><![CDATA[原文\n第二行]]></content></refermsg></appmsg></msg>";
    let m = message((57i64 << 32) | 49, quote);
    assert_eq!(m.kind, "quote");
    assert_eq!(m.content, "回复 <中文> 😀");
    assert_eq!(m.quoted.as_ref().unwrap()["sender_name"], "A & B");
    assert_eq!(m.raw_xml.as_deref(), Some(quote));
    for (kind, body, expected) in [
        (3, "<msg><img md5='abc'/></msg>", "image"),
        (34, "<msg><voicemsg voicelength='1000'/></msg>", "voice"),
        (43, "<msg><videomsg playlength='2'/></msg>", "video"),
        (47, "<msg><emoji md5='xyz'/></msg>", "sticker"),
        (
            48,
            "<msg><location x='1' y='2' label='中文 &amp; 😀'/></msg>",
            "location",
        ),
        (
            49,
            "<msg><appmsg><title>文件.txt</title><type>6</type><appattach><totallen>8</totallen><fileext>txt</fileext></appattach></appmsg></msg>",
            "file",
        ),
        (
            49,
            "<msg><appmsg><title>链接</title><type>5</type><url>https://example.invalid</url></appmsg></msg>",
            "link",
        ),
        (10000, "系统消息", "system"),
        (999, "未知 😀", "unknown"),
    ] {
        let m = message(kind, body);
        assert_eq!(m.kind, expected);
        if expected == "unknown" {
            assert!(m.raw_payload.is_some());
        }
    }
    let template = "<sysmsg><sysmsgtemplate><content_template><plain></plain><template>$who$ 加入群聊$hidden$</template><link_list><link name='who'><memberlist><member><nickname>小明</nickname></member></memberlist></link><link name='hidden' hidden='1'><title>撤销</title></link></link_list></content_template></sysmsgtemplate></sysmsg>";
    assert_eq!(message(10000, template).content, "小明 加入群聊");
    for body in [
        "<msg><bad></msg>",
        "<!DOCTYPE msg SYSTEM 'file:///private'><msg>&private;</msg>",
        "<msg>&undefined;</msg>",
    ] {
        let m = message(49, body);
        assert_eq!(m.kind, "unknown");
        assert_eq!(m.raw_xml.as_deref(), Some(body));
        assert!(m.raw_payload.is_some());
    }
    let input = "保留 \t\r\n 中文😀";
    let zipped = zstd::stream::encode_all(input.as_bytes(), 1)?;
    assert_eq!(decode(&zipped).as_deref(), Some(input));
    assert!(decode(&[0xff, 0xfe]).is_none());
    Ok(())
}
#[test]
fn streaming_writer_propagates_disk_failure() {
    struct Full;
    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(112))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = StreamWriter::new(Full, Format::Jsonl, ExportFields::default()).unwrap();
    assert!(writer.message(&message(1, "text")).is_err());
}

#[test]
fn cancellation_removes_only_own_partial_export() -> Result<()> {
    use wechat_export::core::{
        export::{ExportConfig, export},
        wechat::database::PrivateDir,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let client = support::client(50, cancel.clone())?;
    let work = PrivateDir::new(std::path::Path::new(".local/tests"))?;
    let previous = work.path.join("previous.jsonl");
    std::fs::write(&previous, b"previous export")?;
    let result = export(
        &client,
        &ExportConfig {
            filter: Filter::default(),
            format: Format::Jsonl,
            fields: ExportFields::all(),
            directory: work.path.clone(),
        },
        &cancel,
        |_| cancel.store(true, Ordering::Relaxed),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&previous)?, b"previous export");
    assert_eq!(std::fs::read_dir(&work.path)?.count(), 1);
    Ok(())
}

#[test]
fn selected_fields_match_original_values_in_both_formats() -> Result<()> {
    use serde_json::Value;
    let mut rich = message(999, "未知内容 😀\n第二行");
    rich.media = Some(serde_json::json!({"filename": "图片.jpg"}));
    rich.quoted = Some(serde_json::json!({"content": "引用原文"}));
    rich.raw_xml = Some("<msg>原始内容</msg>".into());
    rich.warnings.push("测试警告".into());
    rich.sender_name = None;
    let messages = [message(1, "中文 😀\n第二行"), rich];
    let mut selections = vec![ExportFields::default(), ExportFields::all()];
    selections.extend(ExportFields::OPTIONS.iter().map(|(key, _)| ExportFields {
        selected: BTreeSet::from([*key]),
    }));
    assert_eq!(
        ExportFields::default().selected,
        BTreeSet::from(["datetime", "sender_name", "type", "content"])
    );
    assert_eq!(
        serde_json::to_value(&messages[1])?
            .as_object()
            .unwrap()
            .len(),
        ExportFields::OPTIONS.len()
    );
    for format in [Format::Jsonl, Format::Json] {
        for fields in &selections {
            let mut writer = StreamWriter::new(Vec::new(), format, fields.clone())?;
            for message in &messages {
                writer.message(message)?;
            }
            let bytes = writer.finish()?;
            let actual: Vec<Value> = if format == Format::Json {
                serde_json::from_slice(&bytes)?
            } else {
                std::str::from_utf8(&bytes)?
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<serde_json::Result<_>>()?
            };
            assert_eq!(actual.len(), messages.len());
            for (actual, message) in actual.iter().zip(&messages) {
                let mut expected = serde_json::to_value(message)?;
                expected
                    .as_object_mut()
                    .unwrap()
                    .retain(|key, _| fields.selected.contains(key.as_str()));
                assert_eq!(actual, &expected, "selected fields: {:?}", fields.selected);
            }
        }
        for selected in [BTreeSet::new(), BTreeSet::from(["unsupported"])] {
            let mut bytes = Vec::new();
            assert!(StreamWriter::new(&mut bytes, format, ExportFields { selected }).is_err());
            assert!(bytes.is_empty());
        }
    }
    Ok(())
}

#[test]
fn export_applies_field_selection_and_rejects_empty_before_writing() -> Result<()> {
    use wechat_export::core::{
        export::{ExportConfig, export},
        wechat::database::PrivateDir,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let client = support::client(3, cancel.clone())?;
    let work = PrivateDir::new(std::path::Path::new(".local/tests"))?;
    for format in [Format::Jsonl, Format::Json] {
        let mut config = ExportConfig {
            filter: Filter::default(),
            format,
            fields: ExportFields {
                selected: BTreeSet::from(["sender_name", "content"]),
            },
            directory: work.path.clone(),
        };
        let result = export(&client, &config, &cancel, |_| {})?;
        let bytes = std::fs::read(&result.path)?;
        let values: Vec<serde_json::Value> = if format == Format::Json {
            serde_json::from_slice(&bytes)?
        } else {
            std::str::from_utf8(&bytes)?
                .lines()
                .map(serde_json::from_str)
                .collect::<serde_json::Result<_>>()?
        };
        assert_eq!(values.len() as u64, result.messages);
        assert_eq!(result.messages, 10);
        for value in values {
            assert_eq!(value.as_object().unwrap().len(), 2);
            assert!(value["sender_name"].is_string());
            assert_eq!(value["content"], "中文 😀\n第二行");
        }
        std::fs::remove_file(result.path)?;
        config.fields.selected.clear();
        assert!(export(&client, &config, &cancel, |_| {}).is_err());
        assert_eq!(std::fs::read_dir(&work.path)?.count(), 0);
    }
    Ok(())
}
