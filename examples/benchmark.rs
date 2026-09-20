#[path = "../tests/support/mod.rs"]
mod support;
use anyhow::Result;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use wechat_export::core::{
    export::{self, ExportConfig},
    model::{ExportFields, Filter, Format},
};
fn main() -> Result<()> {
    let rows = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "250001".into())
        .parse::<u32>()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let client = support::client(rows, cancel.clone())?;
    fs::create_dir_all(".local/benchmark")?;
    let result = export::export(
        &client,
        &ExportConfig {
            filter: Filter::default(),
            format: Format::Jsonl,
            fields: ExportFields::all(),
            directory: PathBuf::from(".local/benchmark"),
        },
        &cancel,
        |_| {},
    )?;
    let bytes = fs::metadata(&result.path)?.len();
    fs::remove_file(&result.path)?;
    println!(
        "messages={} bytes={} export_seconds={:.3} total_seconds={:.3}",
        result.messages,
        bytes,
        result.elapsed,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
