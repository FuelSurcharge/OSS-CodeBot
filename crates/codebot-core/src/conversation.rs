use crate::Config;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}
impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// Validate before creating a client or starting a model. Character limits are
/// an application budget, not an estimate of a model's tokenization.
pub fn validate_messages(config: &Config, messages: &[ChatMessage]) -> Result<(), String> {
    let last = messages.last().ok_or("入力が空です")?;
    if last.content.trim().is_empty() {
        return Err("入力が空です".into());
    }
    if messages.len() % 2 == 0
        || messages.iter().enumerate().any(|(index, m)| {
            m.role
                != if index % 2 == 0 {
                    ChatRole::User
                } else {
                    ChatRole::Assistant
                }
                || m.content.trim().is_empty()
        })
    {
        return Err(
            "会話は空でないユーザー・回答の順とし、最後をユーザー入力にしてください".into(),
        );
    }
    if last.content.chars().count() > config.max_input_chars {
        return Err("入力が文字数の上限を超えています".into());
    }
    let total = messages
        .iter()
        .try_fold(0usize, |n, m| n.checked_add(m.content.chars().count()))
        .ok_or("会話のサイズが上限を超えています")?;
    if total > config.max_context_chars {
        return Err("会話が文字数予算を超えています。履歴を消去するか、CLIで/clearまたは/trimを実行してください。モデル固有のトークン上限も別途適用されます".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_roles_and_checks_unicode_context_before_network() {
        let config = Config {
            max_input_chars: 3,
            max_context_chars: 6,
            ..Default::default()
        };
        let messages = vec![
            ChatMessage::user("名前"),
            ChatMessage::assistant("たろう"),
            ChatMessage::user("誰"),
        ];
        assert!(validate_messages(&config, &messages).is_ok());
        let mut too_long = messages.clone();
        too_long[1].content.push('！');
        assert!(validate_messages(&config, &too_long).is_err());
        assert!(validate_messages(&config, &[ChatMessage::assistant("回答")]).is_err());
        assert!(validate_messages(&config, &messages[..2]).is_err());
        assert!(validate_messages(&config, &[ChatMessage::user(" ")]).is_err());
    }
}
