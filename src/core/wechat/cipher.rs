// Portions adapted from wechatauto-replica/wechatauto/db.py, commit
// 492a8fb70b95865613d6d8d9740323233dbfa197, Apache-2.0.
// Modified 2026-09-19: Rust implementation, authenticate every page, no key logging.
// See THIRD_PARTY_NOTICES.md and licenses/wechatauto-Apache-2.0.txt.
use aes::{
    Aes256,
    cipher::{BlockDecrypt, KeyInit},
};
use anyhow::{Result, bail};
use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

pub const PAGE: usize = 4096;
const DATA_END: usize = PAGE - 80;

pub struct PageKey {
    encryption: Zeroizing<[u8; 32]>,
    authentication: Zeroizing<[u8; 32]>,
}

impl PageKey {
    pub fn verified(
        candidate: &[u8; 32],
        explicit_salt: Option<&[u8; 16]>,
        page: &[u8],
    ) -> Option<Self> {
        if page.len() != PAGE {
            return None;
        }
        let salt = explicit_salt.map(|s| &s[..]).unwrap_or(&page[..16]);
        let salt: Vec<u8> = salt.iter().map(|v| v ^ 0x3a).collect();
        let mut authentication = Zeroizing::new([0; 32]);
        pbkdf2::pbkdf2_hmac::<Sha512>(candidate, &salt, 2, &mut *authentication);
        let key = Self {
            encryption: Zeroizing::new(*candidate),
            authentication,
        };
        key.authenticate(page, 1).ok().map(|_| key)
    }

    fn authenticate(&self, page: &[u8], number: u32) -> Result<()> {
        if page.len() != PAGE || number == 0 {
            bail!("数据库页大小或页号无效");
        }
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(&*self.authentication)?;
        mac.update(&page[if number == 1 { 16 } else { 0 }..PAGE - 64]);
        mac.update(&number.to_le_bytes());
        mac.verify_slice(&page[PAGE - 64..])
            .map_err(|_| anyhow::anyhow!("数据库页认证失败（页 {number}）"))
    }

    pub fn decrypt(&self, page: &[u8], number: u32) -> Result<[u8; PAGE]> {
        self.authenticate(page, number)?;
        let cipher = Aes256::new_from_slice(&*self.encryption)?;
        let mut result = [0; PAGE];
        let start = if number == 1 { 16 } else { 0 };
        let mut previous = [0; 16];
        previous.copy_from_slice(&page[DATA_END..DATA_END + 16]);
        for offset in (start..DATA_END).step_by(16) {
            let mut block = aes::cipher::Block::<Aes256>::default();
            block.copy_from_slice(&page[offset..offset + 16]);
            cipher.decrypt_block(&mut block);
            for i in 0..16 {
                result[offset + i] = block[i] ^ previous[i];
            }
            previous.copy_from_slice(&page[offset..offset + 16]);
        }
        if number == 1 {
            result[..16].copy_from_slice(b"SQLite format 3\0");
            if result[16..18] != [16, 0] || result[20] != 80 {
                bail!("数据库加密布局不兼容");
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
pub(crate) fn fixture_page(number: u32, tag: u8) -> [u8; PAGE] {
    use aes::cipher::BlockEncrypt;
    let enc = [7; 32];
    let salt = [9u8; 16];
    let mut auth = [0; 32];
    pbkdf2::pbkdf2_hmac::<Sha512>(&enc, &salt.map(|v| v ^ 0x3a), 2, &mut auth);
    let cipher = Aes256::new_from_slice(&enc).unwrap();
    let mut page = [0; PAGE];
    page[..16].copy_from_slice(&salt);
    page[DATA_END..DATA_END + 16].fill(3);
    let mut plain = [tag; PAGE];
    plain[16..18].copy_from_slice(&[16, 0]);
    plain[20] = 80;
    let mut previous = [3; 16];
    let start = if number == 1 { 16 } else { 0 };
    for offset in (start..DATA_END).step_by(16) {
        let mut block = aes::cipher::Block::<Aes256>::default();
        for i in 0..16 {
            block[i] = plain[offset + i] ^ previous[i];
        }
        cipher.encrypt_block(&mut block);
        page[offset..offset + 16].copy_from_slice(&block);
        previous.copy_from_slice(&block);
    }
    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(&auth).unwrap();
    mac.update(&page[start..PAGE - 64]);
    mac.update(&number.to_le_bytes());
    page[PAGE - 64..].copy_from_slice(&mac.finalize().into_bytes());
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;
    #[test]
    fn authenticates_decrypts_and_rejects_corruption() {
        let enc = [7; 32];
        let salt = [9; 16];
        let mut auth = [0; 32];
        pbkdf2::pbkdf2_hmac::<Sha512>(&enc, &salt.map(|v| v ^ 0x3a), 2, &mut auth);
        let cipher = Aes256::new_from_slice(&enc).unwrap();
        let mut page = [0; PAGE];
        page[..16].copy_from_slice(&salt);
        page[DATA_END..DATA_END + 16].fill(3);
        let mut previous = [3; 16];
        let mut plain = [0; PAGE];
        plain[16..18].copy_from_slice(&[16, 0]);
        plain[20] = 80;
        for offset in (16..DATA_END).step_by(16) {
            let mut block = aes::cipher::Block::<Aes256>::default();
            for i in 0..16 {
                block[i] = plain[offset + i] ^ previous[i];
            }
            cipher.encrypt_block(&mut block);
            page[offset..offset + 16].copy_from_slice(&block);
            previous.copy_from_slice(&block);
        }
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(&auth).unwrap();
        mac.update(&page[16..PAGE - 64]);
        mac.update(&1u32.to_le_bytes());
        page[PAGE - 64..].copy_from_slice(&mac.finalize().into_bytes());
        let key = PageKey::verified(&enc, None, &page).unwrap();
        assert_eq!(
            &key.decrypt(&page, 1).unwrap()[16..DATA_END],
            &plain[16..DATA_END]
        );
        page[40] ^= 1;
        assert!(key.decrypt(&page, 1).is_err());
        assert!(key.decrypt(&page, 2).is_err());
    }
}
