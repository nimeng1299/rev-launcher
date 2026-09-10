use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Formatter;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Deserialize, Serialize)]
pub enum AccountType {
    Online,
    Offline,
    Other,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Account {
    pub account_type: AccountType,
    pub name: String,
    pub uuid: String,
    pub token: String,
}

impl Account {
    pub fn new_offline(name: &str) -> Self {
        let input = format!("OfflinePlayer:{name}");
        let digest = md5::compute(input);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..]);

        bytes[6] = (bytes[6] & 0x0f) | 0x30;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        let uuid = Uuid::from_bytes(bytes).to_string();
        let token = Uuid::from_bytes([0u8; 16]).to_string();

        Self {
            account_type: AccountType::Offline,
            name: name.to_string(),
            uuid,
            token,
        }
    }
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "name: {}, type: {:?}, uuid:{}",
            self.name, self.account_type, self.uuid
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::account::Account;

    #[test]
    fn new_offline_test() {
        let account = Account::new_offline("bot_xiao");
        assert_eq!(account.uuid, "a3b83191-c9ac-3b20-9f61-782908edb07a");
    }
}
