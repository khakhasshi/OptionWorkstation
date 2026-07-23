use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Deserialize)]
pub struct AuditCaptureRequest {
    pub kind: String,
    pub mode: String,
    pub symbol: String,
    pub snapshot_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: String,
    pub created_at: String,
    pub kind: String,
    pub mode: String,
    pub symbol: String,
    pub snapshot_id: Option<String>,
    pub previous_hash: Option<String>,
    pub record_hash: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditSummary {
    pub id: String,
    pub created_at: String,
    pub kind: String,
    pub mode: String,
    pub symbol: String,
    pub snapshot_id: Option<String>,
    pub record_hash: String,
}

pub struct AuditStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn append(&self, request: AuditCaptureRequest) -> anyhow::Result<AuditRecord> {
        validate_request(&request)?;
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("create audit directory")?;
        }
        let previous_hash = read_records(&self.path)?
            .last()
            .map(|record| record.record_hash.clone());
        let created_at = Utc::now().to_rfc3339();
        let symbol = request.symbol.to_uppercase();
        let record_hash = calculate_hash(
            &created_at,
            &request.kind,
            &request.mode,
            &symbol,
            request.snapshot_id.as_deref(),
            previous_hash.as_deref(),
            &request.payload,
        )?;
        let record = AuditRecord {
            id: record_hash[..20].into(),
            created_at,
            kind: request.kind,
            mode: request.mode,
            symbol,
            snapshot_id: request.snapshot_id,
            previous_hash,
            record_hash,
            payload: request.payload,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .context("open audit ledger")?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(record)
    }

    pub async fn list(&self, limit: usize) -> anyhow::Result<Vec<AuditSummary>> {
        let _guard = self.lock.lock().await;
        Ok(read_records(&self.path)?
            .into_iter()
            .rev()
            .take(limit.clamp(1, 200))
            .map(|record| AuditSummary {
                id: record.id,
                created_at: record.created_at,
                kind: record.kind,
                mode: record.mode,
                symbol: record.symbol,
                snapshot_id: record.snapshot_id,
                record_hash: record.record_hash,
            })
            .collect())
    }

    pub async fn get(&self, id: &str) -> anyhow::Result<AuditRecord> {
        let _guard = self.lock.lock().await;
        read_records(&self.path)?
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| anyhow!("audit record not found"))
    }
}

fn read_records(path: &Path) -> anyhow::Result<Vec<AuditRecord>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let mut previous_hash: Option<String> = None;
    for (index, line) in fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let record: AuditRecord = serde_json::from_str(line)
            .with_context(|| format!("decode audit record {}", index + 1))?;
        anyhow::ensure!(
            record.previous_hash.as_deref() == previous_hash.as_deref(),
            "audit hash chain is broken at record {}",
            index + 1
        );
        let expected = calculate_hash(
            &record.created_at,
            &record.kind,
            &record.mode,
            &record.symbol,
            record.snapshot_id.as_deref(),
            record.previous_hash.as_deref(),
            &record.payload,
        )?;
        anyhow::ensure!(
            record.record_hash == expected && record.id == expected[..20],
            "audit record integrity check failed at record {}",
            index + 1
        );
        previous_hash = Some(record.record_hash.clone());
        records.push(record);
    }
    Ok(records)
}

fn calculate_hash(
    created_at: &str,
    kind: &str,
    mode: &str,
    symbol: &str,
    snapshot_id: Option<&str>,
    previous_hash: Option<&str>,
    payload: &Value,
) -> anyhow::Result<String> {
    let material = serde_json::to_vec(&serde_json::json!({
        "created_at": created_at,
        "kind": kind,
        "mode": mode,
        "symbol": symbol,
        "snapshot_id": snapshot_id,
        "previous_hash": previous_hash,
        "payload": payload,
    }))?;
    Ok(hex::encode(Sha256::digest(material)))
}

fn validate_request(request: &AuditCaptureRequest) -> anyhow::Result<()> {
    anyhow::ensure!(
        !request.kind.trim().is_empty()
            && request
                .kind
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character)),
        "invalid audit event kind"
    );
    anyhow::ensure!(
        matches!(request.mode.as_str(), "live" | "replay" | "system"),
        "invalid audit mode"
    );
    anyhow::ensure!(
        !request.symbol.trim().is_empty() && request.symbol.len() <= 20,
        "invalid symbol"
    );
    anyhow::ensure!(
        !contains_secret(&request.payload),
        "credential-like fields are forbidden in audit payloads"
    );
    anyhow::ensure!(
        serde_json::to_vec(&request.payload)?.len() <= 6_000_000,
        "audit payload is too large"
    );
    Ok(())
}

fn contains_secret(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "app_key" | "app_secret" | "access_token" | "token" | "password"
            ) || contains_secret(value)
        }),
        Value::Array(values) => values.iter().any(contains_secret),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ledger_is_append_only_and_hash_chained() {
        let path = std::env::temp_dir().join(format!(
            "option-workstation-audit-{}.jsonl",
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let store = AuditStore::new(path.clone());
        let first = store
            .append(AuditCaptureRequest {
                kind: "snapshot".into(),
                mode: "replay".into(),
                symbol: "SPY".into(),
                snapshot_id: Some("a".into()),
                payload: serde_json::json!({"spot": 100}),
            })
            .await
            .unwrap();
        let second = store
            .append(AuditCaptureRequest {
                kind: "snapshot".into(),
                mode: "live".into(),
                symbol: "QQQ".into(),
                snapshot_id: Some("b".into()),
                payload: serde_json::json!({"spot": 200}),
            })
            .await
            .unwrap();
        assert_eq!(
            second.previous_hash.as_deref(),
            Some(first.record_hash.as_str())
        );
        assert_eq!(store.list(10).await.unwrap().len(), 2);
        assert_eq!(store.get(&first.id).await.unwrap().symbol, "SPY");
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn tampered_ledger_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "option-workstation-audit-tamper-{}.jsonl",
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let store = AuditStore::new(path.clone());
        store
            .append(AuditCaptureRequest {
                kind: "snapshot".into(),
                mode: "replay".into(),
                symbol: "SPY".into(),
                snapshot_id: Some("a".into()),
                payload: serde_json::json!({"spot": 100}),
            })
            .await
            .unwrap();
        let tampered = fs::read_to_string(&path)
            .unwrap()
            .replace("\"spot\":100", "\"spot\":101");
        fs::write(&path, tampered).unwrap();
        assert!(store.list(10).await.is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn credentials_are_rejected() {
        let request = AuditCaptureRequest {
            kind: "snapshot".into(),
            mode: "live".into(),
            symbol: "SPY".into(),
            snapshot_id: None,
            payload: serde_json::json!({"access_token": "secret"}),
        };
        assert!(validate_request(&request).is_err());
    }
}
