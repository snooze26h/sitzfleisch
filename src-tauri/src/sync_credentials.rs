//! 云端登录会话只进系统凭据库，不写入普通存档，也不交给 WebView。
//!
//! 三个入口都须在后台线程调用：Android 原生插件会回到主线程执行，
//! 从 Tauri 同步命令的主线程等待插件会死锁。隔离测试应注入内存凭据库。

use std::path::Path;

const MAX_SESSION_BYTES: usize = 32 * 1024;
const INVALID_SCOPE: &str = "登录会话的存储标识无效。";
const INVALID_SESSION: &str = "登录会话格式无效，请重新登录。";
const READ_FAILED: &str = "无法安全读取登录会话，请解锁系统凭据库或重新登录。";
const WRITE_FAILED: &str = "无法安全保存登录会话，原有会话仍保留。";
const DELETE_FAILED: &str = "无法清除本机登录会话，请稍后重试。";

fn validate_scope(scope: &str) -> Result<(), &'static str> {
    if scope.len() != 32 || !scope.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return Err(INVALID_SCOPE);
    }
    Ok(())
}

fn validate_session(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > MAX_SESSION_BYTES {
        return Err(INVALID_SESSION);
    }
    // 具体字段与有效期由账号层检查；凭据边界拒绝非对象、超长和损坏的 JSON。
    match serde_json::from_str::<serde_json::Value>(value) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        _ => Err(INVALID_SESSION),
    }
}

fn decode_session(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_SESSION_BYTES { return Err(READ_FAILED.into()); }
    let value = String::from_utf8(bytes).map_err(|_| READ_FAILED.to_owned())?;
    validate_session(&value).map_err(|_| READ_FAILED.to_owned())?;
    Ok(value)
}

pub fn load(app: &tauri::AppHandle, data_dir: &Path, scope: &str) -> Result<Option<String>, String> {
    validate_scope(scope).map_err(str::to_owned)?;
    implementation::load(app, data_dir, scope)?.map(decode_session).transpose()
}

pub fn store(app: &tauri::AppHandle, data_dir: &Path, scope: &str, value: &str) -> Result<(), String> {
    validate_scope(scope).map_err(str::to_owned)?;
    validate_session(value).map_err(str::to_owned)?;
    implementation::store(app, data_dir, scope, value)
}

pub fn delete(app: &tauri::AppHandle, data_dir: &Path, scope: &str) -> Result<(), String> {
    validate_scope(scope).map_err(str::to_owned)?;
    implementation::delete(app, data_dir, scope)
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    use security_framework::passwords::{delete_generic_password, generic_password, set_generic_password, PasswordOptions};

    const SERVICE: &str = "com.snooze26h.sitzfleisch.x.sync-session.v1";
    const ITEM_NOT_FOUND: i32 = -25300;

    pub(super) fn load(_app: &tauri::AppHandle, _data_dir: &Path, scope: &str) -> Result<Option<Vec<u8>>, String> {
        match generic_password(PasswordOptions::new_generic_password(SERVICE, scope)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
            Err(_) => Err(READ_FAILED.into()),
        }
    }

    pub(super) fn store(_app: &tauri::AppHandle, _data_dir: &Path, scope: &str, value: &str) -> Result<(), String> {
        // 原生 SecItemAdd / SecItemUpdate 在失败时保留旧条目，不先删后建。
        set_generic_password(SERVICE, scope, value.as_bytes()).map_err(|_| WRITE_FAILED.into())
    }

    pub(super) fn delete(_app: &tauri::AppHandle, _data_dir: &Path, scope: &str) -> Result<(), String> {
        match delete_generic_password(SERVICE, scope) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
            Err(_) => Err(DELETE_FAILED.into()),
        }
    }
}

#[cfg(target_os = "android")]
mod implementation {
    use super::*;
    use serde::Deserialize;
    use tauri_plugin_sitzfleisch_android::SitzfleischAndroidExt;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SessionReply { value: Option<String> }

    pub(super) fn load(app: &tauri::AppHandle, _data_dir: &Path, scope: &str) -> Result<Option<Vec<u8>>, String> {
        let reply: SessionReply = app.sitzfleisch_android()
            .load_sync_session(serde_json::json!({ "scope": scope }))
            .map_err(|_| READ_FAILED.to_owned())?;
        Ok(reply.value.map(String::into_bytes))
    }

    pub(super) fn store(app: &tauri::AppHandle, _data_dir: &Path, scope: &str, value: &str) -> Result<(), String> {
        app.sitzfleisch_android()
            .store_sync_session(serde_json::json!({ "scope": scope, "value": value }))
            .map_err(|_| WRITE_FAILED.into())
    }

    pub(super) fn delete(app: &tauri::AppHandle, _data_dir: &Path, scope: &str) -> Result<(), String> {
        app.sitzfleisch_android()
            .delete_sync_session(serde_json::json!({ "scope": scope }))
            .map_err(|_| DELETE_FAILED.into())
    }
}

#[cfg(target_os = "windows")]
mod implementation {
    use super::*;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::PathBuf;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    const MAX_CIPHERTEXT_BYTES: usize = 64 * 1024;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;

    fn directory(data_dir: &Path, create: bool) -> io::Result<PathBuf> {
        let path = data_dir.join("sync-credentials");
        if create {
            match fs::create_dir(&path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error),
            }
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(path)
    }

    fn check_file(path: &Path) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
            Ok(metadata) if metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 => Ok(()),
            Ok(_) => Err(io::ErrorKind::PermissionDenied.into()),
        }
    }

    fn protect(bytes: &[u8], scope: &str, encrypt: bool) -> io::Result<Vec<u8>> {
        if bytes.is_empty() || bytes.len() > MAX_CIPHERTEXT_BYTES { return Err(io::ErrorKind::InvalidData.into()); }
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr().cast_mut() };
        // 绑定应用和项目范围：同一用户把密文复制到另一个项目的文件名下也解不开。
        let entropy = format!("sitzfleisch-sync-session-v1:{scope}");
        let entropy = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_ptr().cast_mut() };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // 不使用 LOCAL_MACHINE；解密权限绑定当前 Windows 登录用户。
        let ok = unsafe {
            if encrypt {
                CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            }
        };
        if ok == 0 { return Err(io::Error::last_os_error()); }
        let limit = if encrypt { MAX_CIPHERTEXT_BYTES } else { MAX_SESSION_BYTES };
        let result = if output.pbData.is_null() || output.cbData == 0 || output.cbData as usize > limit {
            Err(io::ErrorKind::InvalidData.into())
        } else {
            Ok(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec())
        };
        unsafe { windows_sys::Win32::Foundation::LocalFree(output.pbData.cast()); }
        result
    }

    fn read_file(path: &Path) -> io::Result<Vec<u8>> {
        let file = OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_CIPHERTEXT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_CIPHERTEXT_BYTES { return Err(io::ErrorKind::InvalidData.into()); }
        Ok(bytes)
    }

    pub(super) fn load(_app: &tauri::AppHandle, data_dir: &Path, scope: &str) -> Result<Option<Vec<u8>>, String> {
        let result = (|| -> io::Result<Vec<u8>> {
            let path = directory(data_dir, false)?.join(format!("{scope}.bin"));
            protect(&read_file(&path)?, scope, false)
        })();
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(READ_FAILED.into()),
        }
    }

    pub(super) fn store(_app: &tauri::AppHandle, data_dir: &Path, scope: &str, value: &str) -> Result<(), String> {
        let result = (|| -> io::Result<()> {
            let encrypted = protect(value.as_bytes(), scope, true)?;
            let directory = directory(data_dir, true)?;
            let destination = directory.join(format!("{scope}.bin"));
            check_file(&destination)?;
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).map_err(io::Error::other)?;
            let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
            let temporary = directory.join(format!("{scope}.{suffix}.tmp"));
            let mut file: File = OpenOptions::new().write(true).create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&temporary)?;
            let written = file.write_all(&encrypted).and_then(|_| file.sync_all());
            drop(file);
            // 临时文件只含 DPAPI 密文；写全并落盘后才替换，失败不先删除原会话。
            let saved = written.and_then(|_| fs::rename(&temporary, &destination));
            if saved.is_err() { let _ = fs::remove_file(&temporary); }
            saved
        })();
        result.map_err(|_| WRITE_FAILED.into())
    }

    pub(super) fn delete(_app: &tauri::AppHandle, data_dir: &Path, scope: &str) -> Result<(), String> {
        let result = (|| -> io::Result<()> {
            let path = directory(data_dir, false)?.join(format!("{scope}.bin"));
            check_file(&path)?;
            fs::remove_file(path)
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(DELETE_FAILED.into()),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "android")))]
mod implementation {
    use super::*;
    pub(super) fn load(_app: &tauri::AppHandle, _data_dir: &Path, _scope: &str) -> Result<Option<Vec<u8>>, String> {
        Err(READ_FAILED.into())
    }
    pub(super) fn store(_app: &tauri::AppHandle, _data_dir: &Path, _scope: &str, _value: &str) -> Result<(), String> {
        Err(WRITE_FAILED.into())
    }
    pub(super) fn delete(_app: &tauri::AppHandle, _data_dir: &Path, _scope: &str) -> Result<(), String> {
        Err(DELETE_FAILED.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_bounded_and_cannot_become_a_path_or_another_keychain_query() {
        assert!(validate_scope("0123456789abcdef0123456789abcdef").is_ok());
        for invalid in ["", "../account", "0123456789ABCDEF0123456789ABCDEF", "0000000000000000000000000000000/", "0000000000000000000000000000000\0"] {
            assert_eq!(validate_scope(invalid), Err(INVALID_SCOPE));
        }
        assert!(validate_scope(&"0".repeat(31)).is_err());
        assert!(validate_scope(&"0".repeat(33)).is_err());
    }

    #[test]
    fn session_boundary_rejects_bad_json_and_measures_utf8_bytes() {
        assert!(validate_session(r#"{"refresh_token":"synthetic-test-value"}"#).is_ok());
        for invalid in ["", "[]", "null", "42", "\"token\"", "{", "{}{}"] {
            assert_eq!(validate_session(invalid), Err(INVALID_SESSION));
        }
        let prefix = r#"{"value":""#;
        let suffix = "\"}";
        let exact = format!("{prefix}{}{suffix}", "x".repeat(MAX_SESSION_BYTES - prefix.len() - suffix.len()));
        assert_eq!(exact.len(), MAX_SESSION_BYTES);
        assert!(validate_session(&exact).is_ok());
        let over = format!("{prefix}{}{suffix}", "字".repeat(MAX_SESSION_BYTES / 3));
        assert!(over.chars().count() < MAX_SESSION_BYTES);
        assert_eq!(validate_session(&over), Err(INVALID_SESSION));
    }

    #[test]
    fn corrupted_storage_errors_never_echo_the_credential() {
        let secret = "synthetic-sensitive-marker";
        for bytes in [secret.as_bytes().to_vec(), vec![0xff], vec![b'x'; MAX_SESSION_BYTES + 1]] {
            let error = decode_session(bytes).unwrap_err();
            assert_eq!(error, READ_FAILED);
            assert!(!error.contains(secret));
        }
        assert_eq!(decode_session(br#"{"value":"test"}"#.to_vec()).unwrap(), r#"{"value":"test"}"#);
    }
}
