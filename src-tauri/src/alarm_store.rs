#![cfg_attr(not(mobile), allow(dead_code))]

use std::{fs::{self, OpenOptions}, io::{self, Read, Write}, path::Path};
use serde::{Deserialize, Serialize};
use crate::alerts::{valid_applied_alarm, AppliedAlarm, MAX_APPLIED_ALARMS};

const MAX_BYTES: u64 = 256 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlarmFile {
    schema: u32,
    applied: Vec<AppliedAlarm>,
}

pub fn load(path: &Path) -> io::Result<Vec<AppliedAlarm>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW); }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES { return invalid(); }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES { return invalid(); }
    let saved: AlarmFile = serde_json::from_slice(&bytes).map_err(|_| io::ErrorKind::InvalidData)?;
    if saved.schema != 1 || saved.applied.len() > MAX_APPLIED_ALARMS
        || saved.applied.iter().any(|item| !valid_applied_alarm(item)) { return invalid(); }
    let mut identities = std::collections::BTreeSet::new();
    if saved.applied.iter().any(|item| !identities.insert(item.id)) { return invalid(); }
    Ok(saved.applied)
}

fn invalid<T>() -> io::Result<T> { Err(io::ErrorKind::InvalidData.into()) }

pub fn save(path: &Path, applied: &[AppliedAlarm]) -> io::Result<()> {
    if applied.len() > MAX_APPLIED_ALARMS || applied.iter().any(|item| !valid_applied_alarm(item)) { return invalid(); }
    let bytes = serde_json::to_vec(&AlarmFile { schema: 1, applied: applied.to_vec() })
        .map_err(|_| io::ErrorKind::InvalidData)?;
    if bytes.len() as u64 > MAX_BYTES { return invalid(); }
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = parent.join(format!(".alarms.{name}.tmp"));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        { fs::File::open(parent)?.sync_all()?; }
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sitzfleisch_core as core;

    fn directory() -> std::path::PathBuf {
        let mut random = [0;8];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!("sitzfleisch-alarms-{:x}",u64::from_ne_bytes(random)));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn store_roundtrips_and_rejects_corruption_without_replacing_the_original() {
        let directory = directory();
        let path = directory.join("alarms.json");
        assert!(load(&path).unwrap().is_empty());
        let kind=core::AlertKind::Water;
        let at=1_700_000_000;
        let item=AppliedAlarm { alert: core::PlannedAlert {kind,at,pause_started_at:None},
            id: crate::alerts::notification_id(kind,at), title: "喝点水吧".into(),
            body: "忙了一阵，喝几口水再继续。".into(), channel: "water".into() };
        save(&path,std::slice::from_ref(&item)).unwrap();
        assert_eq!(load(&path).unwrap(),vec![item]);
        fs::write(&path,b"{invalid").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(),b"{invalid");
        fs::write(&path,b"{\"schema\":2,\"applied\":[]}").unwrap();
        assert!(load(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn linked_store_is_not_followed_or_used_to_overwrite_another_file() {
        use std::os::unix::fs::symlink;
        let directory=directory();
        let target=directory.join("other");
        fs::write(&target,b"original").unwrap();
        let path=directory.join("alarms.json");
        symlink(&target,&path).unwrap();
        assert!(load(&path).is_err());
        save(&path,&[]).unwrap();
        assert_eq!(fs::read(&target).unwrap(),b"original");
        fs::remove_dir_all(directory).unwrap();
    }
}
