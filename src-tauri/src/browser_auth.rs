//! 配对密钥只经桌面 IPC 展示；回环网络传送一次性证明，不传送密钥。

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WINDOW: Duration = Duration::from_secs(30);
const MAX_NONCES: usize = 4096;
type HmacSha256 = Hmac<Sha256>;

pub(crate) struct Auth {
    key: [u8; 32],
    session: String,
    nonces: VecDeque<(String, Instant)>,
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode(value: &str) -> Result<[u8; 32], ()> {
    if value.len() != 64 || !value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) { return Err(()); }
    let mut bytes = [0; 32];
    for (byte, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| ())?, 16).map_err(|_| ())?;
    }
    Ok(bytes)
}

pub(crate) fn request_message(protocol: u8, applied: Option<&str>, time: &str, nonce: &str, session: &str) -> String {
    format!("sitzfleisch-request-v1\nGET /v1/rules\n{protocol}\n{}\n{time}\n{nonce}\n{session}", applied.unwrap_or(""))
}

impl Auth {
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let result = (|| -> std::io::Result<_> {
            let directory = data_dir.join("browser-auth");
            private_directory(&directory)?;
            let path = directory.join("key");
            let key = match open_private(&path, false) {
                Ok(file) => {
                    check_private(&file)?;
                    let mut stored = Vec::new();
                    file.take(4097).read_to_end(&mut stored)?;
                    if stored.len() > 4096 { return Err(std::io::ErrorKind::InvalidData.into()); }
                    let bytes = unprotect(&stored)?;
                    bytes.try_into().map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let key = random_key()?;
                    let mut file = open_private(&path, true)?;
                    file.write_all(&protect(&key)?)?;
                    file.sync_all()?;
                    key
                }
                Err(error) => return Err(error),
            };
            Ok(Self { key, session: hex(&random_key()?), nonces: VecDeque::new() })
        })();
        result.map_err(|_| "无法安全读取浏览器配对凭据，请检查应用数据目录的权限。".into())
    }

    pub fn pairing_code(&self) -> String { hex(&self.key) }
    pub fn session(&self) -> &str { &self.session }

    pub fn rotate(&mut self, data_dir: &Path) -> Result<(), String> {
        let result = (|| -> std::io::Result<_> {
            let directory = data_dir.join("browser-auth");
            private_directory(&directory)?;
            let key = random_key()?;
            let session = hex(&random_key()?);
            let path = directory.join(format!("key.{}", hex(&random_key()?)));
            let mut file = open_private(&path, true)?;
            let saved = file.write_all(&protect(&key)?).and_then(|_| file.sync_all());
            drop(file);
            let saved = saved.and_then(|_| fs::rename(&path, directory.join("key")));
            if saved.is_err() { let _ = fs::remove_file(&path); }
            saved?;
            Ok((key, session))
        })();
        let (key, session) = result.map_err(|_| "配对凭据更新失败，原配对仍有效。")?;
        self.key = key;
        self.session = session;
        self.nonces.clear();
        Ok(())
    }

    pub fn proof(&self, message: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("固定长度 HMAC 密钥");
        mac.update(message);
        hex(&mac.finalize().into_bytes())
    }

    pub fn authorize(&mut self, protocol: u8, applied: Option<&str>, time: &str, nonce: &str, proof: &str, session: &str) -> Result<(), ()> {
        if session != self.session { return Err(()); }
        decode(nonce)?;
        let signature = decode(proof)?;
        if time.len() > 20 { return Err(()); }
        let timestamp: u64 = time.parse().map_err(|_| ())?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| ())?.as_secs();
        if timestamp.to_string() != time || now.abs_diff(timestamp) > WINDOW.as_secs() { return Err(()); }
        let mut mac = HmacSha256::new_from_slice(&self.key).map_err(|_| ())?;
        mac.update(request_message(protocol, applied, time, nonce, session).as_bytes());
        mac.verify_slice(&signature).map_err(|_| ())?;
        while self.nonces.front().is_some_and(|(_, at)| at.elapsed() > WINDOW * 2) { self.nonces.pop_front(); }
        // 窗口内不淘汰已使用 nonce，否则高频请求能把旧请求重新变成有效请求。
        if self.nonces.len() >= MAX_NONCES || self.nonces.iter().any(|(used, _)| used == nonce) { return Err(()); }
        self.nonces.push_back((nonce.into(), Instant::now()));
        Ok(())
    }

    pub fn response_proof(&self, nonce: &str, status: &str, body: &[u8]) -> String {
        let mut message = format!("sitzfleisch-response-v1\n{nonce}\n{status}\n").into_bytes();
        message.extend_from_slice(body);
        self.proof(&message)
    }

    #[cfg(test)]
    pub fn testing() -> Self { Self { key: [7; 32], session: hex(&[9; 32]), nonces: VecDeque::new() } }
}

fn random_key() -> std::io::Result<[u8; 32]> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(std::io::Error::other)?;
    Ok(key)
}

#[cfg(unix)]
fn private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_directory(path: &Path) -> std::io::Result<()> { fs::create_dir_all(path) }

fn open_private(path: &Path, create: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    if create { options.write(true).create_new(true); } else { options.read(true); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT：凭据文件不跟随链接。
    }
    options.open(path)
}

fn check_private(file: &File) -> std::io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() { return Err(std::io::ErrorKind::PermissionDenied.into()); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return Err(std::io::ErrorKind::PermissionDenied.into()); }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn protect(bytes: &[u8]) -> std::io::Result<Vec<u8>> { Ok(bytes.to_vec()) }
#[cfg(not(target_os = "windows"))]
fn unprotect(bytes: &[u8]) -> std::io::Result<Vec<u8>> { Ok(bytes.to_vec()) }

#[cfg(target_os = "windows")]
fn dpapi(bytes: &[u8], encrypt: bool) -> std::io::Result<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN};
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // 不使用 LOCAL_MACHINE：密文即使被其他登录用户读到，也不能解密成配对密钥。
    let ok = unsafe {
        if encrypt { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) }
        else { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) }
    };
    if ok == 0 { return Err(std::io::Error::last_os_error()); }
    if output.pbData.is_null() || output.cbData == 0 || output.cbData > 4096 {
        unsafe { windows_sys::Win32::Foundation::LocalFree(output.pbData.cast()); }
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let result = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { windows_sys::Win32::Foundation::LocalFree(output.pbData.cast()); }
    Ok(result)
}
#[cfg(target_os = "windows")]
fn protect(bytes: &[u8]) -> std::io::Result<Vec<u8>> { dpapi(bytes, true) }
#[cfg(target_os = "windows")]
fn unprotect(bytes: &[u8]) -> std::io::Result<Vec<u8>> { dpapi(bytes, false) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_proof_matches_the_independent_node_crypto_vector() {
        let message = request_message(2, Some("0123456789abcdef"), "1700000000", &"a".repeat(64), Auth::testing().session());
        assert_eq!(Auth::testing().proof(message.as_bytes()), "2ddc93f40e7c694acb60ac7aa3851589be95eb3a46539ae5c71dc4270a2f432d");
    }

    #[test]
    fn requests_and_responses_are_bound_to_identity_nonce_and_contents() {
        let mut auth = Auth::testing();
        let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
        let nonce = "a".repeat(64);
        let session = auth.session().to_string();
        let proof = auth.proof(request_message(2, Some("0123456789abcdef"), &time, &nonce, &session).as_bytes());
        assert!(auth.authorize(2, None, &time, &nonce, &proof, &session).is_err());
        assert!(auth.authorize(2, Some("0123456789abcdef"), &time, &nonce, &"0".repeat(64), &session).is_err());
        assert!(auth.authorize(2, Some("0123456789abcdef"), &time, &nonce, &proof, &"0".repeat(64)).is_err(), "旧启动会话的请求不能使用");
        assert!(auth.authorize(2, Some("0123456789abcdef"), &time, &nonce, &proof, &session).is_ok());
        assert!(auth.authorize(2, Some("0123456789abcdef"), &time, &nonce, &proof, &session).is_err(), "重复请求必须拒绝");
        assert!(auth.authorize(2, None, "1", &nonce, &proof, &session).is_err());
        assert_ne!(auth.response_proof(&nonce, "200 OK", b"{}"), auth.response_proof(&nonce, "200 OK", b"{ }"));
        assert_ne!(auth.response_proof(&nonce, "200 OK", b"{}"), auth.response_proof(&"b".repeat(64), "200 OK", b"{}"));
    }

    #[test]
    fn credentials_persist_rotate_and_reject_shared_or_linked_files() {
        let directory = std::env::temp_dir().join(format!("sitzfleisch-auth-test-{}", hex(&random_key().unwrap())));
        fs::create_dir(&directory).unwrap();
        let mut auth = Auth::load(&directory).unwrap();
        let before = auth.pairing_code();
        let mut restarted = Auth::load(&directory).unwrap();
        assert_eq!(restarted.pairing_code(), before);
        assert_ne!(restarted.session(), auth.session(), "启动会话不能在重启后复用");
        let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
        let nonce = "c".repeat(64);
        let proof = auth.proof(request_message(2, None, &time, &nonce, auth.session()).as_bytes());
        assert!(restarted.authorize(2, None, &time, &nonce, &proof, auth.session()).is_err(), "截获但尚未消费的旧会话请求不能跨重启读取规则");
        auth.rotate(&directory).unwrap();
        assert_ne!(auth.pairing_code(), before);
        assert_eq!(Auth::load(&directory).unwrap().pairing_code(), auth.pairing_code());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let path = directory.join("browser-auth/key");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(Auth::load(&directory).is_err());
            fs::remove_file(&path).unwrap();
            symlink(directory.join("other"), &path).unwrap();
            assert!(Auth::load(&directory).is_err());
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
