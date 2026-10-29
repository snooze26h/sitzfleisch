//! 每个账号独立保存计时、同步基线与未确认请求；原子替换保证重启不会丢失待上传操作。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sitzfleisch_core::{self as core, sync::SyncDocument};

use crate::sync_http::{valid_account_id, AuthUser};

const MAX_ACCOUNT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingWrite {
    pub operation_id: String,
    pub expected_revision: i64,
    pub document: SyncDocument,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteVersion {
    pub revision: i64,
    pub document: SyncDocument,
    pub server_time: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountFile {
    pub schema: u32,
    pub user: AuthUser,
    pub state: core::State,
    pub revision: i64,
    #[serde(default)]
    pub clock_offset: i64,
    pub base: SyncDocument,
    pub draft: SyncDocument,
    pub pending: Option<PendingWrite>,
    pub conflict: Option<RemoteVersion>,
}

impl AccountFile {
    fn validate(&mut self) -> Result<(), String> {
        if self.schema != 1 || !valid_account_id(&self.user.id) || crate::sync_http::validate_email(&self.user.email).is_err()
            || !(0..=9_007_199_254_740_991).contains(&self.revision)
            || self.clock_offset.unsigned_abs() > core::sync::MAX_UNIX_SECONDS as u64
        { return Err("账号存档版本或身份无效，原文件已保留。".into()); }
        self.base.validate()?;
        self.draft.validate()?;
        self.state = core::from_json(&core::to_json(&self.state)).map_err(|_| "账号计时存档无效，原文件已保留。")?;
        if let Some(pending) = &self.pending {
            if !valid_id(&pending.operation_id) || pending.expected_revision != self.revision {
                return Err("待同步请求的版本无效，原文件已保留。".into());
            }
            pending.document.validate()?;
        }
        if let Some(remote) = &self.conflict {
            if remote.revision < self.revision || remote.server_time <= 0 || remote.server_time > 32_503_680_000 {
                return Err("冲突记录无效，原文件已保留。".into());
            }
            remote.document.validate()?;
        }
        Ok(())
    }
}

pub fn valid_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成安全的同步标识。")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub struct Store {
    pub directory: PathBuf,
    pub scope: String,
    pub device_id: String,
}

impl Store {
    pub fn open(data_dir: &Path, service_url: &str) -> Result<Self, String> {
        let scope = format!("{:x}", Sha256::digest(format!("{service_url}\n{}", data_dir.display())))[..32].to_owned();
        let root = data_dir.join("sync");
        private_directory(&root)?;
        let directory = root.join(&scope);
        private_directory(&directory)?;
        private_directory(&directory.join("accounts"))?;
        private_directory(&directory.join("recovery"))?;
        let device_path = directory.join("device.json");
        let device_id = match read_optional(&device_path, 256)? {
            Some(raw) => serde_json::from_slice::<String>(&raw).map_err(|_| "设备同步标识损坏，原文件已保留。")?,
            None => {
                let id = new_id()?;
                atomic_write(&device_path, &serde_json::to_vec(&id).map_err(|_| "设备标识编码失败。")?)?;
                id
            }
        };
        if !valid_id(&device_id) { return Err("设备同步标识无效，原文件已保留。".into()); }
        Ok(Self { directory, scope, device_id })
    }

    pub fn account_path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_account_id(id) { return Err("账号标识无效。".into()); }
        Ok(self.directory.join("accounts").join(format!("{id}.json")))
    }

    pub fn active_user(&self) -> Result<Option<AuthUser>, String> {
        let Some(bytes) = read_optional(&self.directory.join("active.json"), 2048)? else { return Ok(None); };
        let user: AuthUser = serde_json::from_slice(&bytes).map_err(|_| "账号选择记录损坏，请先恢复存档。")?;
        if !valid_account_id(&user.id) || crate::sync_http::validate_email(&user.email).is_err() {
            return Err("账号选择记录无效，请先恢复存档。".into());
        }
        Ok(Some(user))
    }

    pub fn select(&self, user: Option<&AuthUser>) -> Result<(), String> {
        let path = self.directory.join("active.json");
        if let Some(user) = user {
            if !valid_account_id(&user.id) { return Err("账号标识无效。".into()); }
            atomic_write(&path, &serde_json::to_vec(user).map_err(|_| "账号信息编码失败。")?)
        } else {
            if path.exists() { check_file(&path)?; fs::remove_file(&path).map_err(|_| "无法保存退出登录状态，请重试。")?; }
            Ok(())
        }
    }

    pub fn load(&self, user: &AuthUser) -> Result<Option<AccountFile>, String> {
        let Some(bytes) = read_optional(&self.account_path(&user.id)?, MAX_ACCOUNT_BYTES)? else { return Ok(None); };
        let mut file: AccountFile = serde_json::from_slice(&bytes).map_err(|_| "账号存档无法解析，原文件已保留。")?;
        file.validate()?;
        if file.user.id != user.id { return Err("账号存档与登录身份不匹配，已停止加载。".into()); }
        Ok(Some(file))
    }

    pub fn save(&self, account: &AccountFile, state: &core::State) -> Result<(), String> {
        let mut file = account.clone();
        file.state = state.clone();
        let bytes = serde_json::to_vec(&file).map_err(|_| "账号存档编码失败。")?;
        if bytes.len() as u64 > MAX_ACCOUNT_BYTES { return Err("账号存档超过容量限制，待同步数据已保留在内存中，请先导出备份。".into()); }
        atomic_write(&self.account_path(&account.user.id)?, &bytes)
    }

    pub fn backup(&self, label: &str, value: &impl Serialize) -> Result<PathBuf, String> {
        if !matches!(label, "migration" | "conflict") { return Err("恢复副本名称无效。".into()); }
        let bytes = serde_json::to_vec_pretty(value).map_err(|_| "恢复副本编码失败。")?;
        if bytes.len() as u64 > MAX_ACCOUNT_BYTES * 2 { return Err("恢复副本超过容量限制，未修改现有数据。".into()); }
        let path = self.directory.join("recovery").join(format!("{label}-{}.json", new_id()?));
        atomic_write(&path, &bytes)?;
        Ok(path)
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
    match builder.create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("无法创建私有同步目录。".into()),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查同步目录。")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err("同步目录不是普通目录，已停止读写。".into()); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err("同步目录必须仅允许当前用户访问。".into());
        }
    }
    #[cfg(target_os = "windows")]
    { use std::os::windows::fs::MetadataExt; if metadata.file_attributes() & 0x400 != 0 { return Err("同步目录不能是重解析点。".into()); } }
    Ok(())
}

fn check_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查同步文件。")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() { return Err("同步文件不能是目录或符号链接。".into()); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err("同步文件权限或链接状态不安全。".into());
        }
    }
    #[cfg(target_os = "windows")]
    { use std::os::windows::fs::MetadataExt; if metadata.file_attributes() & 0x400 != 0 { return Err("同步文件不能是重解析点。".into()); } }
    Ok(())
}

fn read_optional(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取同步文件信息。".into()),
        Ok(_) => check_file(path)?,
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW); }
    let file = options.open(path).map_err(|_| "无法读取同步文件。")?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| "同步文件没有完整读出。")?;
    if bytes.len() as u64 > limit { return Err("同步文件过大，原文件已保留。".into()); }
    Ok(Some(bytes))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => check_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err("无法检查待保存的同步文件。".into()),
    }
    let tmp = path.with_extension(format!("{}.tmp", new_id()?));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW); }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() { File::open(parent)?.sync_all()?; }
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&tmp); }
    result.map_err(|_| "同步存档保存失败，尚未上传本次修改，请重试保存。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_have_separate_namespaces_and_reject_unsafe_paths() {
        let root = std::env::temp_dir().join(format!("sitzfleisch-sync-store-{}", new_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let a = Store::open(&root, "https://a.example.com").unwrap();
        let b = Store::open(&root, "https://b.example.com").unwrap();
        assert_ne!(a.scope, b.scope);
        assert_eq!(a.device_id, Store::open(&root, "https://a.example.com").unwrap().device_id);
        assert!(a.account_path("../../state").is_err());
        assert_eq!(a.active_user().unwrap().map(|user| user.id), None);
        let user = AuthUser { id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(), email: "a@example.com".into() };
        a.select(Some(&user)).unwrap();
        assert_eq!(a.active_user().unwrap().unwrap().id, user.id);
        a.select(None).unwrap();
        assert!(a.active_user().unwrap().is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = root.join("untouched");
            fs::write(&target, "private").unwrap();
            let link = a.directory.join("linked.json");
            symlink(&target, &link).unwrap();
            assert!(atomic_write(&link, b"bad").is_err());
            assert_eq!(fs::read_to_string(target).unwrap(), "private");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
