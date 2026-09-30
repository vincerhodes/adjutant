//! OS keyring access (secret-service, blocking). Service "adjutant",
//! key "account/<uuid>". Passwords NEVER touch the database, logs, toasts,
//! or sync log. Used on the sync thread only (no dbus on the UI thread).

use std::collections::HashMap;

use uuid::Uuid;

use crate::email::EmailError;

const SERVICE: &str = "adjutant";
const CONTENT_TYPE: &str = "text/plain";

fn connect() -> Result<secret_service::blocking::SecretService<'static>, EmailError> {
    secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
        .map_err(|e| EmailError::Keyring(format!("connect: {e:?}")))
}

fn key(account_id: &Uuid) -> String {
    format!("account/{account_id}")
}

/// Build the attribute map borrowing from `key_str` for the duration of `f`.
fn with_attrs<R>(account_id: &Uuid, f: impl FnOnce(HashMap<&str, &str>) -> R) -> R {
    let key_str = key(account_id);
    let mut m = HashMap::new();
    m.insert("service", SERVICE);
    m.insert("account-key", key_str.as_str());
    f(m)
}

pub fn set_password(account_id: Uuid, password: &str) -> Result<(), EmailError> {
    let ss = connect()?;
    let collection = ss
        .get_default_collection()
        .map_err(|e| EmailError::Keyring(format!("collection: {e:?}")))?;
    let label = format!("adjutant {}", key(&account_id));
    with_attrs(&account_id, |attrs| {
        collection.create_item(&label, attrs, password.as_bytes(), true, CONTENT_TYPE)
    })
    .map_err(|e| EmailError::Keyring(format!("store: {e:?}")))?;
    Ok(())
}

pub fn get_password(account_id: Uuid) -> Result<String, EmailError> {
    let ss = connect()?;
    let items = with_attrs(&account_id, |attrs| ss.search_items(attrs))
        .map_err(|e| EmailError::Keyring(format!("search: {e:?}")))?;
    let item = items.unlocked.first().ok_or_else(|| {
        EmailError::Keyring(format!("no password stored for {}", key(&account_id)))
    })?;
    let secret = item
        .get_secret()
        .map_err(|e| EmailError::Keyring(format!("read: {e:?}")))?;
    String::from_utf8(secret).map_err(|_| EmailError::Keyring("password not utf-8".to_string()))
}

pub fn delete_password(account_id: Uuid) -> Result<(), EmailError> {
    let ss = connect()?;
    let items = with_attrs(&account_id, |attrs| ss.search_items(attrs))
        .map_err(|e| EmailError::Keyring(format!("search: {e:?}")))?;
    for item in items.unlocked.into_iter().chain(items.locked) {
        item.delete()
            .map_err(|e| EmailError::Keyring(format!("delete: {e:?}")))?;
    }
    Ok(())
}
