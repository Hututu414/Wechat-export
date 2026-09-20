//! Developer-only local acceptance. Prints aggregates only; exports stay in the specified workspace folder.
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use wechat_export::core::{
    export::{self, ExportConfig},
    model::{ExportFields, Filter, Format, Message},
    wechat::{database::WechatClient, discovery},
};
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).context("需要微信目录")?);
    let accounts = discovery::accounts(&root)?;
    ensure!(accounts.len() == 1, "请选择一个账号目录");
    let cancel = Arc::new(AtomicBool::new(false));
    let client = WechatClient::open(
        &accounts[0],
        &PathBuf::from(".local/validation"),
        cancel.clone(),
        |s| eprintln!("{s}"),
    )?;
    let count = client.count_messages(&Filter::default(), &cancel)?;
    println!(
        "version={} conversations={} nonempty={} messages={}",
        client.version,
        client.conversations.len(),
        count.conversations,
        count.messages
    );
    let mut ids = HashSet::new();
    let mut seen = 0;
    let mut groups = 0;
    let mut self_count = 0;
    let mut unknown_sender = 0;
    let mut text = 0;
    let mut kinds = std::collections::BTreeMap::new();
    let mut unresolved = std::collections::BTreeMap::new();
    let mut warnings = std::collections::BTreeMap::new();
    let mut prior = String::new();
    let mut time = i64::MIN;
    client.for_each_message(&Filter::default(), &cancel, |m| {
        ensure!(ids.insert(m.id.clone()), "duplicate canonical id");
        if m.conversation_id == prior {
            ensure!(m.timestamp >= time, "timestamp order");
        } else {
            prior = m.conversation_id.clone();
        }
        time = m.timestamp;
        seen += 1;
        groups += usize::from(m.conversation_id.ends_with("@chatroom"));
        self_count += usize::from(m.is_self == Some(true));
        unknown_sender += usize::from(m.sender_id.is_none());
        text += usize::from(m.kind == "text");
        *kinds.entry(m.kind.clone()).or_insert(0u64) += 1;
        if m.sender_id.is_none() {
            *unresolved.entry(m.kind.clone()).or_insert(0u64) += 1;
        }
        for warning in m.warnings {
            *warnings.entry(warning).or_insert(0u64) += 1;
        }
        Ok(())
    })?;
    ensure!(seen == count.messages, "COUNT mismatch");
    println!(
        "stream_verified={seen} group_messages={groups} self_messages={self_count} unknown_sender={unknown_sender} text={text}"
    );
    println!("types={kinds:?} unresolved_senders={unresolved:?} warnings={warnings:?}");
    fs::create_dir_all(".local/exports")?;
    let chosen = client
        .conversations
        .iter()
        .find(|c| c.is_group)
        .context("无群聊样本")?;
    let filter = Filter {
        conversations: Some(BTreeSet::from([chosen.id.clone()])),
        ..Default::default()
    };
    let expected = client.count_messages(&filter, &cancel)?.messages;
    for format in [Format::Jsonl, Format::Json] {
        let result = export::export(
            &client,
            &ExportConfig {
                filter: filter.clone(),
                format,
                fields: ExportFields::all(),
                directory: PathBuf::from(".local/exports"),
            },
            &cancel,
            |_| {},
        )?;
        let actual = if format == Format::Jsonl {
            let mut n = 0;
            for line in BufReader::new(File::open(&result.path)?).lines() {
                let _: Message = serde_json::from_str(&line?)?;
                n += 1;
            }
            n
        } else {
            serde_json::from_reader::<_, Vec<Message>>(File::open(&result.path)?)?.len() as u64
        };
        ensure!(actual == expected, "serialized count mismatch");
        println!(
            "export_valid format={} messages={actual}",
            if format == Format::Jsonl {
                "jsonl"
            } else {
                "json"
            }
        );
        fs::remove_file(&result.path)?;
    }
    Ok(())
}
