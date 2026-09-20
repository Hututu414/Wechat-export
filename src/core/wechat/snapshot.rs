use super::cipher::{PAGE, PageKey};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("操作已取消");
    }
    Ok(())
}

fn hash_copy(source: &Path, destination: Option<&Path>, cancel: &AtomicBool) -> Result<[u8; 32]> {
    let mut input = File::open(source)?;
    let mut output = destination.map(File::create).transpose()?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut hash = Sha256::new();
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        if let Some(file) = &mut output {
            file.write_all(&buffer[..n])?;
        }
    }
    Ok(hash.finalize().into())
}

pub fn prepare(
    source: &Path,
    destination: &Path,
    key: &PageKey,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let raw = destination.with_extension("encrypted");
    let raw_wal = destination.with_extension("encrypted-wal");
    let wal = PathBuf::from(format!("{}-wal", source.display()));
    let mut stable = false;
    for _ in 0..3 {
        let first = hash_copy(source, Some(&raw), cancel)?;
        let had_wal = wal.is_file();
        if !had_wal && raw_wal.exists() {
            fs::remove_file(&raw_wal)?;
        }
        let first_wal = if had_wal {
            Some(hash_copy(&wal, Some(&raw_wal), cancel)?)
        } else {
            None
        };
        if first == hash_copy(source, None, cancel)?
            && had_wal == wal.is_file()
            && (!had_wal || first_wal == Some(hash_copy(&wal, None, cancel)?))
        {
            stable = true;
            break;
        }
    }
    if !stable {
        bail!("微信数据库正在频繁变化，未取得一致副本，请稍后重试");
    }
    let size = raw.metadata()?.len();
    if size == 0 || size % PAGE as u64 != 0 {
        bail!("数据库长度不符合支持的加密页格式");
    }
    let mut input = std::io::BufReader::new(File::open(&raw)?);
    let mut output = std::io::BufWriter::new(File::create(destination)?);
    let mut page = [0; PAGE];
    for number in 1..=size / PAGE as u64 {
        cancelled(cancel)?;
        input.read_exact(&mut page)?;
        let plain = key.decrypt(&page, u32::try_from(number)?)?;
        output.write_all(&plain)?;
    }
    output.flush()?;
    drop(output);
    if raw_wal.exists() {
        apply_wal(destination, &raw_wal, key, cancel)?;
    }
    // Only our private materialized snapshot changes journal header mode.
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .open(destination)?;
    output.seek(SeekFrom::Start(18))?;
    output.write_all(&[1, 1])?;
    output.sync_all()?;
    drop(output);
    let conn = open_readonly(destination)?;
    let interrupt = cancel.clone();
    conn.progress_handler(2000, Some(move || interrupt.load(Ordering::Relaxed)))?;
    let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        bail!("数据库副本完整性检查失败");
    }
    fs::remove_file(raw)?;
    if raw_wal.exists() {
        fs::remove_file(raw_wal)?;
    }
    Ok(())
}

pub fn open_readonly(path: &Path) -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

fn be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn checksum(bytes: &[u8], big_endian: bool, state: &mut [u32; 2]) {
    for pair in bytes.chunks_exact(8) {
        let a = if big_endian {
            be(&pair[..4])
        } else {
            u32::from_le_bytes(pair[..4].try_into().unwrap_or([0; 4]))
        };
        let b = if big_endian {
            be(&pair[4..8])
        } else {
            u32::from_le_bytes(pair[4..8].try_into().unwrap_or([0; 4]))
        };
        state[0] = state[0].wrapping_add(a).wrapping_add(state[1]);
        state[1] = state[1].wrapping_add(b).wrapping_add(state[0]);
    }
}

// SQLite WAL format, https://sqlite.org/fileformat2.html#walformat.
// Two passes keep memory bounded: locate last valid commit, then materialize only its frames.
fn apply_wal(db: &Path, wal: &Path, key: &PageKey, cancel: &AtomicBool) -> Result<()> {
    let mut input = std::io::BufReader::new(File::open(wal)?);
    let size = input.get_ref().metadata()?.len();
    if size == 0 {
        return Ok(());
    }
    if size < 32 {
        bail!("WAL 头不完整");
    }
    let mut header = [0; 32];
    input.read_exact(&mut header)?;
    let magic = be(&header[..4]);
    if !matches!(magic, 0x377f0682 | 0x377f0683)
        || be(&header[4..8]) != 3007000
        || be(&header[8..12]) != PAGE as u32
    {
        bail!("WAL 格式不兼容");
    }
    let mut sum = [0; 2];
    let big = magic & 1 == 1;
    checksum(&header[..24], big, &mut sum);
    if sum != [be(&header[24..28]), be(&header[28..32])] {
        bail!("WAL 头校验失败");
    }
    let mut frame = [0u8; 24 + PAGE];
    let mut committed = 0;
    let mut pages = 0;
    let original_pages = fs::metadata(db)?.len() / PAGE as u64;
    for i in 0..(size - 32) / (24 + PAGE) as u64 {
        cancelled(cancel)?;
        input.read_exact(&mut frame)?;
        if frame[8..16] != header[16..24] {
            break;
        }
        checksum(&frame[..8], big, &mut sum);
        checksum(&frame[24..], big, &mut sum);
        if sum != [be(&frame[16..20]), be(&frame[20..24])] {
            bail!("WAL 帧校验失败，请重新连接");
        }
        if be(&frame[..4]) == 0 {
            bail!("WAL 页号无效");
        }
        let commit_size = be(&frame[4..8]);
        if commit_size != 0 {
            if commit_size as u64 > original_pages + i + 1 {
                bail!("WAL 提交页数超出数据库和帧范围");
            }
            committed = i + 1;
            pages = commit_size;
        }
    }
    if committed == 0 {
        return Ok(());
    }
    input.seek(SeekFrom::Start(32))?;
    let mut output = OpenOptions::new().read(true).write(true).open(db)?;
    for _ in 0..committed {
        cancelled(cancel)?;
        input.read_exact(&mut frame)?;
        let number = be(&frame[..4]);
        let plain = key
            .decrypt(&frame[24..], number)
            .context("WAL 数据页认证失败")?;
        if number <= pages {
            output.seek(SeekFrom::Start((number as u64 - 1) * PAGE as u64))?;
            output.write_all(&plain)?;
        }
    }
    output.set_len(pages as u64 * PAGE as u64)?;
    // SQLite WAL commit size is authoritative for the resulting main database.
    output.seek(SeekFrom::Start(28))?;
    output.write_all(&pages.to_be_bytes())?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::wechat::{cipher::fixture_page, database::PrivateDir};
    #[test]
    fn wal_commits_checksums_shrink_and_uncommitted_tail() -> Result<()> {
        let work = PrivateDir::new(Path::new(".local/tests"))?;
        let db = work.path.join("db");
        let wal = work.path.join("wal");
        let cancel = AtomicBool::new(false);
        let first = fixture_page(1, 0);
        let key = PageKey::verified(&[7; 32], None, &first).context("fixture")?;
        for big in [false, true] {
            let base = key.decrypt(&first, 1)?.repeat(3);
            fs::write(&db, &base)?;
            let mut header = [0u8; 32];
            header[..4].copy_from_slice(&(0x377f0682u32 + u32::from(big)).to_be_bytes());
            header[4..8].copy_from_slice(&3007000u32.to_be_bytes());
            header[8..12].copy_from_slice(&(PAGE as u32).to_be_bytes());
            header[16..24].fill(8);
            let mut sum = [0; 2];
            checksum(&header[..24], big, &mut sum);
            header[24..28].copy_from_slice(&sum[0].to_be_bytes());
            header[28..32].copy_from_slice(&sum[1].to_be_bytes());
            let mut bytes = header.to_vec();
            for (tag, commit) in [(17u8, 2u32), (99, 0)] {
                let mut frame = vec![0; 24 + PAGE];
                frame[..4].copy_from_slice(&2u32.to_be_bytes());
                frame[4..8].copy_from_slice(&commit.to_be_bytes());
                frame[8..16].copy_from_slice(&header[16..24]);
                frame[24..].copy_from_slice(&fixture_page(2, tag));
                checksum(&frame[..8], big, &mut sum);
                checksum(&frame[24..], big, &mut sum);
                frame[16..20].copy_from_slice(&sum[0].to_be_bytes());
                frame[20..24].copy_from_slice(&sum[1].to_be_bytes());
                bytes.extend(frame);
            }
            bytes.extend([0u8; 13]);
            fs::write(&wal, &bytes)?;
            apply_wal(&db, &wal, &key, &cancel)?;
            let plain = fs::read(&db)?;
            assert_eq!(plain.len(), 2 * PAGE);
            assert_eq!(plain[PAGE + 100], 17);
            assert_eq!(be(&plain[28..32]), 2);
            assert_eq!(fs::read(&wal)?, bytes);
            bytes[32 + 24 + 40] ^= 1;
            fs::write(&wal, &bytes)?;
            assert!(apply_wal(&db, &wal, &key, &cancel).is_err());
        }
        Ok(())
    }
}
