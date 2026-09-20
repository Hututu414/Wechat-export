use super::key::{self, Handle};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Storage::FileSystem::*, System::Threading::*};

#[derive(Clone)]
pub struct Account {
    pub directory: PathBuf,
    pub label: String,
}
pub fn accounts(root: &Path) -> Result<Vec<Account>> {
    if !root.is_dir() {
        bail!("数据目录不存在");
    }
    let mut dirs = if root.join("db_storage").is_dir() {
        vec![root.to_path_buf()]
    } else if root.file_name().is_some_and(|n| n == "db_storage") {
        vec![root.parent().context("账号目录无效")?.to_path_buf()]
    } else {
        fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.join("db_storage").is_dir())
            .collect()
    };
    dirs.sort();
    Ok(dirs
        .into_iter()
        .map(|directory| Account {
            label: directory
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            directory,
        })
        .collect())
}
pub fn default_root() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Some(p) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(p).join("Documents/xwechat_files"));
    }
    if let Some(p) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(p).join("Tencent/xwechat_files"));
    }
    // Check only named drive roots, never recursively search unrelated personal folders.
    roots.extend(('C'..='Z').map(|d| PathBuf::from(format!("{d}:\\xwechat_files"))));
    roots.into_iter().find(|r| r.is_dir())
}
pub fn detect() -> Result<String> {
    let ids = key::process_ids()?;
    if ids.is_empty() {
        bail!("未检测到当前用户的微信，请启动并登录微信 4.x");
    }
    for pid in ids {
        // SAFETY: fixed-size path buffer and version structure are validated before use.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                continue;
            }
            let handle = Handle(handle);
            let mut path = vec![0u16; 32768];
            let mut len = path.len() as u32;
            if QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut len) == 0 {
                continue;
            }
            path[len as usize] = 0;
            let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
            if size == 0 {
                continue;
            }
            let mut bytes = vec![0u64; (size as usize).div_ceil(8)];
            if GetFileVersionInfoW(path.as_ptr(), 0, size, bytes.as_mut_ptr().cast()) == 0 {
                continue;
            }
            let mut info = std::ptr::null_mut();
            let mut n = 0;
            if VerQueryValueW(
                bytes.as_ptr().cast(),
                [92u16, 0].as_ptr(),
                &mut info,
                &mut n,
            ) == 0
                || n < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
            {
                continue;
            }
            let v = &*info.cast::<VS_FIXEDFILEINFO>();
            let major = v.dwFileVersionMS >> 16;
            if major != 4 {
                bail!("不支持的微信版本：仅支持微信 4.x");
            }
            return Ok(format!(
                "{}.{}.{}.{}",
                major,
                v.dwFileVersionMS & 0xffff,
                v.dwFileVersionLS >> 16,
                v.dwFileVersionLS & 0xffff
            ));
        }
    }
    bail!("无法读取微信版本信息")
}
pub fn message_table(username: &str) -> Result<String> {
    use windows_sys::Win32::Security::Cryptography::{BCRYPT_MD5_ALG_HANDLE, BCryptHash};
    let mut digest = [0; 16];
    // MD5 here names an existing WeChat table; it is not a security primitive.
    let status = unsafe {
        BCryptHash(
            BCRYPT_MD5_ALG_HANDLE,
            std::ptr::null(),
            0,
            username.as_ptr(),
            u32::try_from(username.len())?,
            digest.as_mut_ptr(),
            16,
        )
    };
    if status < 0 {
        bail!("Windows 表名散列失败");
    }
    Ok(format!(
        "Msg_{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}
pub fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
