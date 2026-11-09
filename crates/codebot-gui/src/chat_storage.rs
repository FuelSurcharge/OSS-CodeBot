use crate::Session;
use codebot_core::ChatRole;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    version: u32,
    pub active: usize,
    pub sessions: Vec<Session>,
}

pub fn load(path: &Path) -> Result<Option<Archive>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err("保存されたチャットを読み込めません。元ファイルは保持しています".into())
        }
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "チャットの読み込みに失敗しました")?;
    if bytes.len() > MAX_BYTES {
        return Err("チャット保存ファイルが8 MiBを超えています".into());
    }
    let mut archive: Archive = serde_json::from_slice(&bytes).map_err(|_| {
        "チャットの保存形式が不正です。元ファイルを退避してから再読み込みしてください"
    })?;
    validate(&archive)?;
    // Restart never grants consent to send a restored conversation externally.
    for session in &mut archive.sessions {
        session.history_enabled = false;
    }
    Ok(Some(archive))
}

fn validate(archive: &Archive) -> Result<(), String> {
    if archive.version != 1 {
        return Err("未対応のチャット保存バージョンです。元ファイルは保持しています".into());
    }
    if archive.sessions.is_empty()
        || archive.sessions.len() > 1000
        || archive.active >= archive.sessions.len()
    {
        return Err("チャットの件数または選択位置が不正です".into());
    }
    for session in &archive.sessions {
        if session.messages.len() > 10000
            || session.history.len() % 2 != 0
            || session.history.len() > 10000
            || session.history.iter().enumerate().any(|(index, m)| {
                m.content.trim().is_empty()
                    || m.role
                        != if index % 2 == 0 {
                            ChatRole::User
                        } else {
                            ChatRole::Assistant
                        }
            })
        {
            return Err("保存された会話履歴の形式が不正です".into());
        }
    }
    Ok(())
}

pub fn save(path: &Path, sessions: &[Session], active: usize) -> Result<(), String> {
    let archive = Archive {
        version: 1,
        active,
        sessions: sessions.to_vec(),
    };
    validate(&archive)?;
    let bytes = serde_json::to_vec(&archive).map_err(|_| "チャットを保存形式へ変換できません")?;
    write_atomic(path, &bytes, true)
}

pub fn export(
    path: &Path,
    session: &Session,
    markdown: bool,
    overwrite: bool,
) -> Result<(), String> {
    let bytes = if markdown {
        let mut text = format!("# {}\n\n", session.title());
        for (user, content) in &session.messages {
            text.push_str(if *user {
                "## ユーザー\n\n"
            } else {
                "## CodeBot\n\n"
            });
            text.push_str(content);
            text.push_str("\n\n");
        }
        text.into_bytes()
    } else {
        serde_json::to_vec_pretty(&Archive {
            version: 1,
            active: 0,
            sessions: vec![session.clone()],
        })
        .map_err(|_| "チャットを書き出せません")?
    };
    write_atomic(path, &bytes, overwrite)
}

fn write_atomic(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    if bytes.len() > MAX_BYTES {
        return Err("保存するチャットが8 MiBを超えています。古い会話を整理してください".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|_| "チャットの保存フォルダを作成できません")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "チャット保存用の一時ファイルを作成できません")?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| "チャットを書き込めません。空き容量・権限を確認してください")?;
    if overwrite {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    }
    .map_err(|_| "チャットを保存できません。既存ファイル・権限・空き容量を確認してください")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codebot_core::ChatMessage;
    #[test]
    fn saved_chats_restore_drafts_names_and_history_without_send_consent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chats.json");
        let session = Session {
            label: "調査".into(),
            draft: "下書き".into(),
            history_enabled: true,
            messages: vec![(true, "質問".into()), (false, "回答".into())],
            history: vec![ChatMessage::user("質問"), ChatMessage::assistant("回答")],
            in_flight_input: Some("保存しない内部状態".into()),
            ..Default::default()
        };
        save(&path, &[Session::default(), session], 1).unwrap();
        let archive = load(&path).unwrap().unwrap();
        assert_eq!(archive.active, 1);
        assert_eq!(archive.sessions[1].title(), "調査");
        assert_eq!(archive.sessions[1].draft, "下書き");
        assert_eq!(archive.sessions[1].history.len(), 2);
        assert!(!archive.sessions[1].history_enabled);
        assert!(archive.sessions[1].in_flight_input.is_none());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("保存しない内部状態"));
        assert!(!raw.contains("api_key"));
    }
    #[test]
    fn malformed_or_future_files_survive_and_export_does_not_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chats.json");
        for raw in ["invalid", r#"{"version":99,"sessions":[],"active":0}"#] {
            std::fs::write(&path, raw).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
        assert!(export(&path, &Session::default(), true, false).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"version":99,"sessions":[],"active":0}"#
        );
        let markdown = dir.path().join("chat.md");
        export(
            &markdown,
            &Session {
                messages: vec![(true, "質問".into()), (false, "**回答**".into())],
                ..Default::default()
            },
            true,
            false,
        )
        .unwrap();
        assert!(std::fs::read_to_string(markdown)
            .unwrap()
            .contains("**回答**"));
    }
    #[test]
    fn oversized_files_and_failed_replacements_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("too-large.json");
        File::create(&path)
            .unwrap()
            .set_len((MAX_BYTES + 1) as u64)
            .unwrap();
        assert!(load(&path).err().unwrap().contains("8 MiB"));
        let directory_target = dir.path().join("a-directory");
        std::fs::create_dir(&directory_target).unwrap();
        assert!(save(&directory_target, &[Session::default()], 0).is_err());
        assert!(directory_target.is_dir());
    }
}
