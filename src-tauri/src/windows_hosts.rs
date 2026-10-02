//! Windows 提权边界：系统路径来自 Win32，待安装字节在写入前绑定 SHA-256。

use base64::Engine;
use std::path::Path;

fn quoted(raw: &str) -> String { format!("'{}'", raw.replace('\'', "''")) }

fn encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn write_script(staged: &Path, digest: &str) -> Result<String, String> {
    let path = staged.to_str().ok_or("暂存路径不是有效的 UTF-8。")?;
    if path.len() > 4096 || path.chars().any(char::is_control)
        || digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("暂存路径或内容摘要无效，未调用 UAC。".into());
    }
    // 先读入内存并核验，再写固定的系统目标；不存在校验后重新读源文件的窗口。
    // PowerShell 字符串不会展开 %NAME%，整个命令再用 UTF-16 Base64 传递，完全不经过 cmd。
    Ok(format!(r#"$ErrorActionPreference = 'Stop'
try {{
  $source = [System.IO.File]::Open({}, 'Open', 'Read', 'Read')
  try {{
    if ($source.Length -lt 1 -or $source.Length -gt 65536) {{ throw 'invalid size' }}
    $memory = [System.IO.MemoryStream]::new()
    $source.CopyTo($memory)
    $bytes = $memory.ToArray()
    $memory.Dispose()
  }} finally {{ $source.Dispose() }}
  $hash = [System.Security.Cryptography.SHA256]::Create()
  try {{ $actual = [System.BitConverter]::ToString($hash.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant() }} finally {{ $hash.Dispose() }}
  if ($actual -cne '{digest}') {{ throw 'content rejected' }}
  $target = [System.IO.Path]::Combine([System.Environment]::SystemDirectory, 'drivers', 'etc', 'hosts')
  [System.IO.File]::WriteAllBytes($target, $bytes)
  try {{ & ([System.IO.Path]::Combine([System.Environment]::SystemDirectory, 'ipconfig.exe')) /flushdns | Out-Null }} catch {{}}
  exit 0
}} catch {{ exit 1 }}"#, quoted(path)))
}

fn elevation_script(powershell: &Path, script: &str) -> Result<String, String> {
    let path = powershell.to_str().ok_or("系统 PowerShell 路径无效。")?;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; try {{ $p = Start-Process -FilePath {} -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {}' -Verb RunAs -Wait -PassThru; exit $p.ExitCode }} catch {{ exit 1 }}",
        quoted(path), encoded(script),
    );
    let command = encoded(&script);
    if command.len() > 30000 { return Err("提权命令超过系统长度上限，未调用 UAC。".into()); }
    Ok(command)
}

#[cfg(target_os = "windows")]
pub(crate) fn system_directory() -> Result<std::path::PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = [0u16; 32768];
    // 固定大小缓冲区覆盖 Win32 路径上限；失败时关闭此功能，不接受环境变量兜底。
    let count = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if count == 0 || count >= buffer.len() { return Err("无法从系统读取可信的 Windows 目录。".into()); }
    Ok(std::ffi::OsString::from_wide(&buffer[..count]).into())
}

#[cfg(target_os = "windows")]
pub(crate) fn powershell_process() -> Result<std::process::Command, String> {
    let system = system_directory()?;
    let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
    let root = system.parent().ok_or("Windows 系统目录无效。")?;
    let mut command = std::process::Command::new(powershell);
    command.env_clear().env("SystemRoot", root).env("WINDIR", root).env("PATH", &system)
        .env("PSModulePath", system.join("WindowsPowerShell/v1.0/Modules")).current_dir(&system);
    Ok(command)
}

#[cfg(target_os = "windows")]
pub(crate) fn install(content: &str, data_dir: &Path) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    if content.is_empty() || content.len() > 65536 { return Err("hosts 内容为空或超过 65536 字节，未调用 UAC。".into()); }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法生成安全暂存文件名。")?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = data_dir.join(format!("hosts.{name}.staged"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| "无法创建 hosts 暂存文件。")?;
    let result = (|| {
        file.write_all(content.as_bytes()).and_then(|_| file.sync_all()).map_err(|_| "hosts 暂存写入失败。")?;
        drop(file);
        // 直到提权进程退出才放开共享锁，拒绝其他进程写入或删除。即使之前抢先替换，
        // 提权侧也会核验应用内存里计算的摘要，恶意内容绝不会进入系统 hosts。
        let guard = OpenOptions::new().read(true).share_mode(1).open(&path).map_err(|_| "无法锁定 hosts 暂存文件。")?;
        let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
        let powershell = system_directory()?.join("WindowsPowerShell/v1.0/powershell.exe");
        let script = elevation_script(&powershell, &write_script(&path, &digest)?)?;
        let status = powershell_process()?.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &script]).status().map_err(|_| "无法调用系统 UAC 提权。")?;
        drop(guard);
        if status.success() { Ok(()) } else { Err("提权被取消或内容核验失败，屏蔽规则未写入。".into()) }
    })();
    let _ = fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_binds_content_and_never_uses_cmd_or_environment_paths() {
        let path = Path::new("C:/test/%PAYLOAD%/$('bad');quote'/hosts.staged");
        let script = write_script(path, &"a".repeat(64)).unwrap();
        assert!(script.find("$actual -cne").unwrap() < script.find("WriteAllBytes").unwrap());
        assert!(script.contains("quote''/hosts.staged"));
        assert!(script.contains("[System.Environment]::SystemDirectory"));
        assert!(!script.contains("cmd.exe"));
        assert!(!script.contains("$env:"));
        let command = elevation_script(Path::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"), &script).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(command).unwrap();
        let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let outer = String::from_utf16(&words).unwrap();
        assert!(outer.contains("-Verb RunAs -Wait -PassThru"));
        assert!(outer.contains("exit $p.ExitCode"));
        assert!(!outer.contains("%PAYLOAD%"));
        assert!(write_script(Path::new("bad\npath"), &"a".repeat(64)).is_err());
        assert!(write_script(path, "not-a-digest").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_rejects_replaced_bytes_before_writing_and_accepts_approved_content() {
        use sha2::{Digest, Sha256};
        let directory = std::env::temp_dir().join(format!("sitzfleisch-hosts-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let staged = directory.join("%PAYLOAD%' hosts");
        let target = directory.join("test.hosts");
        let approved = b"127.0.0.1 localhost\n";
        std::fs::write(&target, b"original").unwrap();
        let script = write_script(&staged, &format!("{:x}", Sha256::digest(approved))).unwrap()
            .replace("[System.IO.Path]::Combine([System.Environment]::SystemDirectory, 'drivers', 'etc', 'hosts')", &quoted(target.to_str().unwrap()))
            .replace("try { & ([System.IO.Path]::Combine([System.Environment]::SystemDirectory, 'ipconfig.exe')) /flushdns | Out-Null } catch {}", "");
        assert!(!script.contains("'drivers'"), "测试只能写临时目标");
        assert!(!script.contains("ipconfig.exe"), "测试不能刷新系统 DNS");
        let run = || powershell_process().unwrap().args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded(&script)]).status().unwrap();
        std::fs::write(&staged, b"192.0.2.1 victim.example\n").unwrap();
        assert!(!run().success());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        std::fs::write(&staged, approved).unwrap();
        assert!(run().success());
        assert_eq!(std::fs::read(&target).unwrap(), approved);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
