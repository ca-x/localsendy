use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 4;
pub const SINGLE_USER_SUBJECT: &str = "single";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceKey {
    pub subject: String,
}

impl InstanceKey {
    pub fn single() -> Self {
        Self {
            subject: SINGLE_USER_SUBJECT.to_owned(),
        }
    }

    pub fn instance_id(&self) -> String {
        format!("single:{}", self.subject)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingScope<'a> {
    Global,
    Instance(&'a InstanceKey),
}

impl SettingScope<'_> {
    fn key(&self) -> String {
        match self {
            Self::Global => "global".to_owned(),
            Self::Instance(instance) => instance.instance_id(),
        }
    }
}

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceDefaults<'a> {
    pub key: &'a InstanceKey,
    pub alias: &'a str,
    pub device_type: &'a str,
    pub device_model: Option<&'a str>,
    pub preferred_port: u16,
    pub identity_path: &'a str,
    pub download_path: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceRecord {
    pub instance_id: String,
    pub key: InstanceKey,
    pub alias: String,
    pub device_type: String,
    pub device_model: Option<String>,
    pub port: u16,
    pub identity_path: String,
    pub download_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferRecord {
    pub id: String,
    pub batch_id: String,
    pub instance_id: String,
    pub direction: String,
    pub peer_alias: String,
    pub file_name: String,
    pub size: u64,
    pub status: String,
    pub created_at_ms: i64,
    pub error: Option<String>,
    pub content_type: Option<String>,
    pub is_clipboard: bool,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut connection = Connection::open(path)
            .with_context(|| format!("failed to open SQLite database {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;",
        )?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn ensure_instance(&self, defaults: InstanceDefaults<'_>) -> Result<InstanceRecord> {
        validate_subject(&defaults.key.subject)?;
        let instance_id = defaults.key.instance_id();
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = query_instance(&transaction, &instance_id)? {
            transaction.commit()?;
            return Ok(existing);
        }

        let port = allocate_port(&transaction, defaults.preferred_port)?;
        transaction.execute(
            r#"
            INSERT INTO instances (
                instance_id, auth_mode, subject, alias, device_type, device_model,
                port, identity_path, download_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                instance_id,
                "single",
                defaults.key.subject,
                defaults.alias,
                defaults.device_type,
                defaults.device_model,
                port,
                defaults.identity_path,
                defaults.download_path,
            ],
        )?;
        transaction.commit()?;
        Ok(InstanceRecord {
            instance_id,
            key: defaults.key.clone(),
            alias: defaults.alias.to_owned(),
            device_type: defaults.device_type.to_owned(),
            device_model: defaults.device_model.map(ToOwned::to_owned),
            port,
            identity_path: defaults.identity_path.to_owned(),
            download_path: defaults.download_path.to_owned(),
        })
    }

    pub fn update_instance_alias(&self, key: &InstanceKey, alias: &str) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE instances SET alias = ?2, updated_at_ms = unixepoch('subsec') * 1000 WHERE instance_id = ?1",
            params![key.instance_id(), alias],
        )?;
        if changed == 0 {
            bail!("unknown Localsendy instance")
        }
        Ok(())
    }

    pub fn load_setting<T: DeserializeOwned>(
        &self,
        scope: SettingScope<'_>,
        key: &str,
    ) -> Result<Option<T>> {
        validate_setting_key(key)?;
        let value = self
            .lock()?
            .query_row(
                "SELECT value_json FROM settings WHERE scope = ?1 AND key = ?2",
                params![scope.key(), key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).context("failed to decode stored setting"))
            .transpose()
    }

    pub fn store_setting<T: Serialize>(
        &self,
        scope: SettingScope<'_>,
        key: &str,
        value: &T,
    ) -> Result<()> {
        validate_setting_key(key)?;
        let value = serde_json::to_string(value)?;
        self.lock()?.execute(
            r#"
            INSERT INTO settings (scope, key, value_json, updated_at_ms)
            VALUES (?1, ?2, ?3, unixepoch('subsec') * 1000)
            ON CONFLICT(scope, key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![scope.key(), key, value],
        )?;
        Ok(())
    }

    pub fn record_transfer(&self, transfer: &TransferRecord) -> Result<()> {
        let connection = self.lock()?;
        upsert_transfer(&connection, transfer)
    }

    pub fn record_transfers(&self, transfers: &[TransferRecord]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for transfer in transfers {
            upsert_transfer(&transaction, transfer)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_transfer_batches(
        &self,
        key: &InstanceKey,
        direction: &str,
        limit: usize,
    ) -> Result<Vec<TransferRecord>> {
        let limit = i64::try_from(limit.min(1000)).expect("transfer batch limit is bounded");
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            r#"
            WITH recent_batches AS (
                SELECT batch_id, MAX(created_at_ms) AS batch_created_at
                FROM transfers
                WHERE instance_id = ?1 AND direction = ?2
                GROUP BY batch_id
                ORDER BY batch_created_at DESC, batch_id DESC
                LIMIT ?3
            )
            SELECT transfers.id, transfers.batch_id, transfers.instance_id,
                   transfers.direction, transfers.peer_alias, transfers.file_name,
                   transfers.size, transfers.status, transfers.created_at_ms,
                   transfers.error, transfers.content_type, transfers.is_clipboard
            FROM transfers
            INNER JOIN recent_batches USING (batch_id)
            WHERE transfers.instance_id = ?1 AND transfers.direction = ?2
            ORDER BY recent_batches.batch_created_at DESC, transfers.id ASC
            "#,
        )?;
        let rows =
            statement.query_map(params![key.instance_id(), direction, limit], transfer_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_transfers(&self, key: &InstanceKey, limit: usize) -> Result<Vec<TransferRecord>> {
        let limit = i64::try_from(limit.min(1000)).expect("transfer history limit is bounded");
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, batch_id, instance_id, direction, peer_alias, file_name, size,
                   status, created_at_ms, error, content_type, is_clipboard
            FROM transfers
            WHERE instance_id = ?1
            ORDER BY created_at_ms DESC, id DESC
            LIMIT ?2
            "#,
        )?;
        let rows = statement.query_map(params![key.instance_id(), limit], transfer_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite connection lock is poisoned"))
    }
}

fn upsert_transfer(connection: &Connection, transfer: &TransferRecord) -> Result<()> {
    let size = i64::try_from(transfer.size).context("transfer size exceeds SQLite range")?;
    connection.execute(
        r#"
        INSERT INTO transfers (
            id, batch_id, instance_id, direction, peer_alias, file_name, size,
            status, created_at_ms, error, content_type, is_clipboard
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(id) DO UPDATE SET
            batch_id = excluded.batch_id,
            status = excluded.status,
            error = excluded.error,
            content_type = excluded.content_type,
            is_clipboard = excluded.is_clipboard
        "#,
        params![
            transfer.id,
            transfer.batch_id,
            transfer.instance_id,
            transfer.direction,
            transfer.peer_alias,
            transfer.file_name,
            size,
            transfer.status,
            transfer.created_at_ms,
            transfer.error,
            transfer.content_type,
            transfer.is_clipboard,
        ],
    )?;
    Ok(())
}

fn transfer_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRecord> {
    let size = row.get::<_, i64>(6)?;
    let size = u64::try_from(size).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(TransferRecord {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        instance_id: row.get(2)?,
        direction: row.get(3)?,
        peer_alias: row.get(4)?,
        file_name: row.get(5)?,
        size,
        status: row.get(7)?,
        created_at_ms: row.get(8)?,
        error: row.get(9)?,
        content_type: row.get(10)?,
        is_clipboard: row.get(11)?,
    })
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        bail!("database schema version {current} is newer than supported version {SCHEMA_VERSION}");
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if current == 0 {
        transaction.execute_batch(
            r#"
            CREATE TABLE instances (
                instance_id TEXT PRIMARY KEY,
                auth_mode TEXT NOT NULL CHECK (auth_mode = 'single'),
                subject TEXT NOT NULL,
                alias TEXT NOT NULL,
                device_type TEXT NOT NULL,
                device_model TEXT,
                port INTEGER NOT NULL UNIQUE CHECK (port BETWEEN 1 AND 65535),
                identity_path TEXT NOT NULL,
                download_path TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000),
                updated_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000),
                UNIQUE (auth_mode, subject)
            );

            CREATE TABLE settings (
                scope TEXT NOT NULL,
                key TEXT NOT NULL,
                value_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000),
                PRIMARY KEY (scope, key)
            );

            CREATE TABLE transfers (
                id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL,
                instance_id TEXT NOT NULL REFERENCES instances(instance_id) ON DELETE CASCADE,
                direction TEXT NOT NULL CHECK (direction IN ('incoming', 'outgoing')),
                peer_alias TEXT NOT NULL,
                file_name TEXT NOT NULL,
                size INTEGER NOT NULL CHECK (size >= 0),
                status TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                error TEXT,
                content_type TEXT,
                is_clipboard INTEGER NOT NULL DEFAULT 0 CHECK (is_clipboard IN (0, 1))
            );

            CREATE INDEX idx_transfers_instance_created
                ON transfers(instance_id, created_at_ms DESC);
            CREATE INDEX idx_transfers_instance_direction_batch
                ON transfers(instance_id, direction, created_at_ms DESC, batch_id);

            PRAGMA user_version = 4;
            "#,
        )?;
    } else if current == 1 {
        transaction.execute_batch(
            r#"
            ALTER TABLE transfers ADD COLUMN content_type TEXT;
            ALTER TABLE transfers ADD COLUMN is_clipboard INTEGER NOT NULL DEFAULT 0 CHECK (is_clipboard IN (0, 1));
            ALTER TABLE transfers ADD COLUMN batch_id TEXT;
            UPDATE transfers
            SET batch_id = CASE
                WHEN direction = 'outgoing' AND instr(id, ':') > 0
                    THEN substr(id, 1, instr(id, ':') - 1)
                ELSE id
            END;
            CREATE INDEX idx_transfers_instance_direction_batch
                ON transfers(instance_id, direction, created_at_ms DESC, batch_id);
            PRAGMA user_version = 4;
            "#,
        )?;
    } else if current == 2 {
        transaction.execute_batch(
            r#"
            ALTER TABLE transfers ADD COLUMN is_clipboard INTEGER NOT NULL DEFAULT 0 CHECK (is_clipboard IN (0, 1));
            UPDATE transfers SET is_clipboard = 1 WHERE preview IS NOT NULL;
            UPDATE transfers SET preview = NULL WHERE preview IS NOT NULL;
            ALTER TABLE transfers DROP COLUMN preview;
            ALTER TABLE transfers ADD COLUMN batch_id TEXT;
            UPDATE transfers
            SET batch_id = CASE
                WHEN direction = 'outgoing' AND instr(id, ':') > 0
                    THEN substr(id, 1, instr(id, ':') - 1)
                ELSE id
            END;
            CREATE INDEX idx_transfers_instance_direction_batch
                ON transfers(instance_id, direction, created_at_ms DESC, batch_id);
            PRAGMA user_version = 4;
            "#,
        )?;
    } else if current == 3 {
        transaction.execute_batch(
            r#"
            ALTER TABLE transfers ADD COLUMN batch_id TEXT;
            UPDATE transfers
            SET batch_id = CASE
                WHEN direction = 'outgoing' AND instr(id, ':') > 0
                    THEN substr(id, 1, instr(id, ':') - 1)
                ELSE id
            END;
            CREATE INDEX idx_transfers_instance_direction_batch
                ON transfers(instance_id, direction, created_at_ms DESC, batch_id);
            PRAGMA user_version = 4;
            "#,
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn query_instance(connection: &Connection, instance_id: &str) -> Result<Option<InstanceRecord>> {
    connection
        .query_row(
            r#"
            SELECT instance_id, auth_mode, subject, alias, device_type, device_model,
                   port, identity_path, download_path
            FROM instances WHERE instance_id = ?1
            "#,
            params![instance_id],
            |row| {
                match row.get::<_, String>(1)?.as_str() {
                    "single" => {}
                    value => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            format!("unknown auth mode {value}").into(),
                        ));
                    }
                }
                Ok(InstanceRecord {
                    instance_id: row.get(0)?,
                    key: InstanceKey {
                        subject: row.get(2)?,
                    },
                    alias: row.get(3)?,
                    device_type: row.get(4)?,
                    device_model: row.get(5)?,
                    port: row.get(6)?,
                    identity_path: row.get(7)?,
                    download_path: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn allocate_port(connection: &Connection, preferred: u16) -> Result<u16> {
    for port in preferred..=u16::MAX {
        let used = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM instances WHERE port = ?1)",
            [port],
            |row| row.get::<_, bool>(0),
        )?;
        if !used {
            return Ok(port);
        }
    }
    bail!("no LocalSend TCP port is available")
}

fn validate_subject(subject: &str) -> Result<()> {
    if subject != SINGLE_USER_SUBJECT {
        bail!("single mode must use the fixed single-user subject")
    }
    if subject.is_empty()
        || subject.len() > 128
        || subject.chars().any(char::is_control)
        || subject.contains(['/', '\\'])
    {
        bail!("invalid Localsendy identity subject")
    }
    Ok(())
}

fn validate_setting_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("invalid Localsendy setting key")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults<'a>(key: &'a InstanceKey, alias: &'a str, port: u16) -> InstanceDefaults<'a> {
        InstanceDefaults {
            key,
            alias,
            device_type: "server",
            device_model: Some("Linux"),
            preferred_port: port,
            identity_path: "/data/identity.pem",
            download_path: "/data/downloads",
        }
    }

    #[test]
    fn instances_keep_stable_ports() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("localsendy.sqlite3")).unwrap();
        let key = InstanceKey::single();
        let first = database
            .ensure_instance(defaults(&key, "One", 53317))
            .unwrap();
        let again = database
            .ensure_instance(defaults(&key, "Changed", 54000))
            .unwrap();
        assert_eq!(again, first);
    }

    #[test]
    fn global_and_instance_settings_do_not_collide() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("localsendy.sqlite3")).unwrap();
        let key = InstanceKey::single();
        database
            .store_setting(SettingScope::Global, "interfaces", &vec!["enp2s0"])
            .unwrap();
        database
            .store_setting(SettingScope::Instance(&key), "interfaces", &vec!["lo"])
            .unwrap();
        let global: Vec<String> = database
            .load_setting(SettingScope::Global, "interfaces")
            .unwrap()
            .unwrap();
        let instance: Vec<String> = database
            .load_setting(SettingScope::Instance(&key), "interfaces")
            .unwrap()
            .unwrap();
        assert_eq!(global, vec!["enp2s0"]);
        assert_eq!(instance, vec!["lo"]);
    }

    #[test]
    fn transfer_history_preserves_content_type() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("localsendy.sqlite3")).unwrap();
        let key = InstanceKey::single();
        database
            .ensure_instance(defaults(&key, "One", 53317))
            .unwrap();
        database
            .record_transfer(&TransferRecord {
                id: "transfer:file".to_owned(),
                batch_id: "transfer".to_owned(),
                instance_id: key.instance_id(),
                direction: "outgoing".to_owned(),
                peer_alias: "Phone".to_owned(),
                file_name: "message.txt".to_owned(),
                size: 5,
                status: "completed".to_owned(),
                created_at_ms: 42,
                error: None,
                content_type: Some("text/plain".to_owned()),
                is_clipboard: true,
            })
            .unwrap();

        let transfers = database.list_transfers(&key, 10).unwrap();
        assert_eq!(transfers[0].content_type.as_deref(), Some("text/plain"));
        assert!(transfers[0].is_clipboard);
    }

    #[test]
    fn transfer_batches_are_atomic_and_loaded_whole() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("localsendy.sqlite3")).unwrap();
        let key = InstanceKey::single();
        database
            .ensure_instance(defaults(&key, "One", 53317))
            .unwrap();
        let record = |id: &str, size: u64| TransferRecord {
            id: id.to_owned(),
            batch_id: "batch".to_owned(),
            instance_id: key.instance_id(),
            direction: "outgoing".to_owned(),
            peer_alias: "Phone".to_owned(),
            file_name: format!("{id}.bin"),
            size,
            status: "completed".to_owned(),
            created_at_ms: 42,
            error: None,
            content_type: Some("application/octet-stream".to_owned()),
            is_clipboard: false,
        };

        assert!(
            database
                .record_transfers(&[record("first", 1), record("invalid", u64::MAX)])
                .is_err()
        );
        assert!(
            database
                .list_transfer_batches(&key, "outgoing", 1)
                .unwrap()
                .is_empty()
        );

        database
            .record_transfers(&[record("first", 1), record("second", 2)])
            .unwrap();
        let restored = database.list_transfer_batches(&key, "outgoing", 1).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.iter().all(|record| record.batch_id == "batch"));
    }

    #[test]
    fn migrates_schema_v2_clipboard_history_without_payloads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("localsendy.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE transfers (
                    id TEXT PRIMARY KEY,
                    instance_id TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    peer_alias TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    error TEXT,
                    content_type TEXT,
                    preview TEXT
                );
                INSERT INTO transfers (
                    id, instance_id, direction, peer_alias, file_name, size,
                    status, created_at_ms, error, content_type, preview
                ) VALUES ('legacy:clip', 'single:single', 'outgoing', 'Phone', 'message.txt', 5,
                          'completed', 42, NULL, 'text/plain', 'secret clipboard text');
                PRAGMA user_version = 2;
                "#,
            )
            .unwrap();
        drop(connection);

        drop(Database::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        let clipboard_column: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('transfers') WHERE name = 'is_clipboard'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_preview_column: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('transfers') WHERE name = 'preview'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_clipboard: i64 = connection
            .query_row(
                "SELECT is_clipboard FROM transfers WHERE id = 'legacy:clip'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();

        assert_eq!(clipboard_column, 1);
        assert_eq!(legacy_preview_column, 0);
        assert_eq!(legacy_clipboard, 1);
        assert_eq!(version, SCHEMA_VERSION);
        drop(connection);
        let secret = b"secret clipboard text";
        assert!(
            !std::fs::read(&path)
                .unwrap()
                .windows(secret.len())
                .any(|window| window == secret)
        );
        let wal_path = path.with_extension("sqlite3-wal");
        if let Ok(wal) = std::fs::read(wal_path) {
            assert!(!wal.windows(secret.len()).any(|window| window == secret));
        }
    }

    #[test]
    fn rejects_future_database_versions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("localsendy.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        assert!(Database::open(&path).is_err());
    }
}
