// Portions adapted from wechatauto-replica/wechatauto/db.py, commit
// 492a8fb70b95865613d6d8d9740323233dbfa197, Apache-2.0.
// Modified 2026-09-19: Rust, bounded buffers, cancellation, no disk key cache.
// See THIRD_PARTY_NOTICES.md and licenses/wechatauto-Apache-2.0.txt.
use super::cipher::PageKey;
use anyhow::{Result, bail};
use std::{
    collections::HashSet,
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::{Debug::ReadProcessMemory, ToolHelp::*},
        Memory::*,
        RemoteDesktop::ProcessIdToSessionId,
        Threading::*,
    },
};
use zeroize::Zeroizing;

const NAME: &[u8] = b"com.Tencent.WCDB.Config.Cipher";
const MASK: [u8; 32] = [
    0xd2, 0xc7, 0x44, 0x24, 0x58, 0x02, 0, 0, 0, 0x48, 0x89, 0x44, 0x24, 0x50, 0x48, 0x8b, 0x45, 0,
    0x48, 0x84, 0x4c, 0x24, 0x48, 0x48, 0x89, 0x44, 0x25, 0x40, 0x48, 0x58, 0x4c, 0x24,
];
pub(super) struct Handle(pub(super) HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn same_user(process: HANDLE) -> bool {
    // Aligned storage retains the SID pointers for the duration of EqualSid.
    unsafe fn user(process: HANDLE) -> Option<Vec<usize>> {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let token = Handle(token);
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return None;
        }
        Some(storage)
    }
    // SAFETY: buffers own both TOKEN_USER structures and their SIDs.
    unsafe {
        let (Some(a), Some(b)) = (user(GetCurrentProcess()), user(process)) else {
            return false;
        };
        let target = (*(b.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        if EqualSid((*(a.as_ptr().cast::<TOKEN_USER>())).User.Sid, target) != 0 {
            return true;
        }
        // A restricted launcher may run under a sandbox account. Still require the target
        // to belong to the actual shell owner on this same desktop. Compare local token
        // SIDs directly: no account-name/domain lookup that could contact a network.
        let mut shell_pid = 0;
        if GetWindowThreadProcessId(GetShellWindow(), &mut shell_pid) == 0 {
            return false;
        }
        let shell = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, shell_pid);
        if shell.is_null() {
            return false;
        }
        let shell = Handle(shell);
        let Some(owner) = user(shell.0) else {
            return false;
        };
        EqualSid((*(owner.as_ptr().cast::<TOKEN_USER>())).User.Sid, target) != 0
    }
}

pub fn process_ids() -> Result<Vec<u32>> {
    // SAFETY: Toolhelp structures and handles are initialized and kept alive.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        let snapshot = Handle(snapshot);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ids = Vec::new();
        let mut found = Process32FirstW(snapshot.0, &mut entry);
        let mut own_session = 0;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut own_session) == 0 {
            bail!("无法确认当前 Windows 会话");
        }
        while found != 0 {
            let end = entry
                .szExeFile
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            let mut session = 0;
            if name.eq_ignore_ascii_case("Weixin.exe")
                && ProcessIdToSessionId(entry.th32ProcessID, &mut session) != 0
                && session == own_session
            {
                let process =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !process.is_null() {
                    let process = Handle(process);
                    if same_user(process.0) {
                        ids.push(entry.th32ProcessID);
                    }
                }
            }
            found = Process32NextW(snapshot.0, &mut entry);
        }
        Ok(ids)
    }
}

fn read(handle: &Handle, address: usize, size: usize) -> Option<Zeroizing<Vec<u8>>> {
    if !(0x10000..0x800000000000).contains(&address) || size > 1024 * 1024 + 128 {
        return None;
    }
    let mut bytes = Zeroizing::new(vec![0; size]);
    let mut count = 0;
    // SAFETY: destination points to size allocated bytes; source access is checked by Windows.
    unsafe {
        ReadProcessMemory(
            handle.0,
            address as _,
            bytes.as_mut_ptr().cast(),
            size,
            &mut count,
        );
    }
    if count == 0 {
        return None;
    }
    bytes.truncate(count);
    Some(bytes)
}

fn scan(handle: &Handle, cancel: &AtomicBool, mut visit: impl FnMut(usize, &[u8])) -> Result<()> {
    let mut address = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("操作已取消");
        }
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: writable correctly-sized descriptor; no changes to the target process.
        if unsafe {
            VirtualQueryEx(
                handle.0,
                address as _,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        } == 0
        {
            break;
        }
        let base = info.BaseAddress as usize;
        let Some(next) = base.checked_add(info.RegionSize).filter(|n| *n > address) else {
            break;
        };
        if info.State == MEM_COMMIT && info.Protect & PAGE_GUARD == 0 && info.Protect & 0xe6 != 0 {
            let mut cursor = base;
            while cursor < next {
                if cancel.load(Ordering::Relaxed) {
                    bail!("操作已取消");
                }
                let n = (next - cursor).min(1024 * 1024);
                if let Some(bytes) = read(handle, cursor, (next - cursor).min(n + 128)) {
                    visit(cursor, &bytes);
                }
                cursor += n;
            }
        }
        address = next;
    }
    Ok(())
}

fn word(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?) as usize)
}

pub fn acquire(
    pages: &[Vec<u8>],
    cancel: &AtomicBool,
    mut progress: impl FnMut(&str),
) -> Result<Vec<PageKey>> {
    let ids = process_ids()?;
    if ids.is_empty() {
        bail!("未检测到微信 4.x，请启动并登录微信");
    }
    let mut keys: Vec<Option<PageKey>> = (0..pages.len()).map(|_| None).collect();
    let mut accessible = false;
    for pid in ids {
        progress("正在只读扫描微信数据库密钥");
        // SAFETY: only query/read permissions; no write, injection, suspend or debug APIs.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            continue;
        }
        accessible = true;
        let handle = Handle(handle);
        let mut names = HashSet::new();
        scan(&handle, cancel, |base, bytes| {
            for offset in memchr::memmem::find_iter(bytes, NAME) {
                names.insert(base + offset);
            }
        })?;
        if names.is_empty() {
            continue;
        }
        let mut configs = HashSet::new();
        scan(&handle, cancel, |base, bytes| {
            for offset in (0..bytes.len().saturating_sub(15)).step_by(8) {
                if word(bytes, offset + 8) == Some(NAME.len())
                    && word(bytes, offset).is_some_and(|v| names.contains(&v))
                {
                    if let Some(node) = (base + offset)
                        .checked_sub(16)
                        .and_then(|p| read(&handle, p, 80))
                    {
                        if let Some(config) = word(&node, 0x28) {
                            configs.insert(config);
                        }
                    }
                }
            }
        })?;
        for config in configs {
            let Some(obj) = config
                .checked_add(0x88)
                .and_then(|p| read(&handle, p, 0x28))
            else {
                continue;
            };
            let (Some(ptr), Some(len)) = (word(&obj, 8), word(&obj, 16)) else {
                continue;
            };
            if len == 0 || len > 1024 {
                continue;
            }
            let Some(mut blob) = read(&handle, ptr, len) else {
                continue;
            };
            if blob.len() != len {
                continue;
            }
            for (i, b) in blob.iter_mut().enumerate() {
                *b ^= MASK[i % MASK.len()];
            }
            for offset in 0..blob.len().saturating_sub(2) {
                if !matches!(blob[offset], b'x' | b'X') || blob[offset + 1] != b'\'' {
                    continue;
                }
                let hex = &blob[offset + 2..];
                let len = hex.iter().take_while(|b| b.is_ascii_hexdigit()).count();
                if !(64..=192).contains(&len) || hex.get(len) != Some(&b'\'') {
                    continue;
                }
                let mut starts = vec![0];
                if len > 96 {
                    starts.extend((0..=len - 64).step_by(32));
                    starts.push(len - 64);
                }
                for start in starts {
                    let mut candidate = Zeroizing::new([0u8; 48]);
                    let count = if start + 96 <= len { 48 } else { 32 };
                    for i in 0..count {
                        let hi = (hex[start + 2 * i] as char).to_digit(16).unwrap_or(0);
                        let lo = (hex[start + 2 * i + 1] as char).to_digit(16).unwrap_or(0);
                        candidate[i] = ((hi << 4) | lo) as u8;
                    }
                    let enc: &[u8; 32] = candidate[..32].try_into()?;
                    for (key, page) in keys.iter_mut().zip(pages) {
                        if key.is_some() {
                            continue;
                        }
                        *key = PageKey::verified(enc, None, page);
                        if key.is_none() && count == 48 {
                            *key =
                                PageKey::verified(enc, Some(candidate[32..48].try_into()?), page);
                        }
                    }
                }
            }
        }
        if keys.iter().all(Option::is_some) {
            return keys
                .into_iter()
                .map(|v| v.ok_or_else(|| anyhow::anyhow!("Key 验证失败")))
                .collect();
        }
    }
    if !accessible {
        bail!("无权只读访问微信进程，请确认微信与工具由同一用户运行");
    }
    bail!(
        "Key 获取不完整：已验证 {}/{} 个数据库；请确认目标账号已登录。当前微信版本可能不兼容",
        keys.iter().filter(|k| k.is_some()).count(),
        keys.len()
    )
}
