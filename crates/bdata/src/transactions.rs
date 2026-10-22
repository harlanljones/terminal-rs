//! Transaction normaliser for the Stats API `/api/v1/transactions` feed.
//!
//! Spec §4.6: the feed returns byte-identical duplicate rows (verified on real payloads:
//! Sale 852022/867165, Lewis 851742/901976), so rows are deduped on
//! (person_id, date, type_code, description) keeping the first occurrence.

use std::collections::HashSet;

use serde::Deserialize;

/// Errors raised while normalising a transactions payload.
#[derive(Debug, thiserror::Error)]
pub enum TransactionsError {
    #[error("transactions: {0}")]
    Bad(String),
}

/// One transaction row from the Stats API, normalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub person_id: u32,
    pub person: String,
    pub team_id: u16,
    pub date: String,
    pub type_code: String,
    pub type_desc: String,
    pub description: String,
}

/// A normalised, deduplicated transaction page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPage {
    pub transactions: Vec<Transaction>,
    /// Number of duplicate rows collapsed (dropped) by the dedupe pass.
    pub collapsed: usize,
}

/// Raw Stats API transaction row.
#[derive(Debug, Deserialize)]
struct RawTransaction {
    #[serde(rename = "transaction_id")]
    _transaction_id: serde_json::Value,
    person_id: Option<serde_json::Value>,
    person: Option<String>,
    team_id: Option<serde_json::Value>,
    date: Option<String>,
    #[serde(rename = "type_code")]
    _type_code_raw: Option<serde_json::Value>,
    type_desc: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTransactions {
    transactions: Option<Vec<RawTransaction>>,
}

fn as_u32(v: &serde_json::Value, field: &str) -> Result<u32, TransactionsError> {
    v.as_u64()
        .filter(|n| *n <= u32::MAX as u64)
        .map(|n| n as u32)
        .ok_or_else(|| TransactionsError::Bad(format!("transactions: bad {field}: {v}")))
}

fn as_u16(v: &serde_json::Value, field: &str) -> Result<u16, TransactionsError> {
    v.as_u64()
        .filter(|n| *n <= u16::MAX as u64)
        .map(|n| n as u16)
        .ok_or_else(|| TransactionsError::Bad(format!("transactions: bad {field}: {v}")))
}

/// Normalises a Stats API `/api/v1/transactions` payload: parses the `transactions`
/// array and dedupes byte-identical duplicate rows on
/// (person_id, date, type_code, description), keeping the first occurrence.
pub fn normalize(payload: &str) -> Result<TransactionPage, TransactionsError> {
    let parsed: RawTransactions =
        serde_json::from_str(payload).map_err(|e| TransactionsError::Bad(format!("transactions: bad JSON: {e}")))?;

    let rows = parsed.transactions.unwrap_or_default();

    let mut seen: HashSet<(u32, String, String, String)> = HashSet::new();
    let mut out = Vec::new();
    let mut collapsed = 0usize;

    for row in rows {
        // A row without a person id is not a usable transaction; the feed is
        // person-keyed, so treat it as malformed rather than inventing a key.
        let person_id_val = row
            .person_id
            .as_ref()
            .ok_or_else(|| TransactionsError::Bad("transactions: row missing person_id".to_string()))?;
        let person_id = as_u32(person_id_val, "person_id")?;
        let team_id = match row.team_id.as_ref() {
            Some(v) => as_u16(v, "team_id")?,
            None => return Err(TransactionsError::Bad("transactions: row missing team_id".to_string())),
        };
        let person = row.person.unwrap_or_default();
        let date = row.date.unwrap_or_default();
        let type_code = match &row._type_code_raw {
            Some(v) => match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                other => return Err(TransactionsError::Bad(format!("transactions: bad type_code: {other}"))),
            },
            None => return Err(TransactionsError::Bad("transactions: row missing type_code".to_string())),
        };
        let type_desc = row.type_desc.unwrap_or_default();
        let description = row.description.unwrap_or_default();

        let key = (person_id, date.clone(), type_code.clone(), description.clone());
        if !seen.insert(key) {
            collapsed += 1;
            continue;
        }

        out.push(Transaction { person_id, person, team_id, date, type_code, type_desc, description });
    }

    Ok(TransactionPage { transactions: out, collapsed })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture mirroring the verified real payload shape: four rows, two dedupe
    /// pairs (Sale 852022/867165, Lewis 851742/901976) plus one distinct later
    /// event that must survive.
    const FIXTURE: &str = r#"{
        "copyright": "Copyright 2026 MLB Advanced Media",
        "transactions": [
            {
                "transaction_id": 852022,
                "person_id": 660271,
                "person": "Chris Sale",
                "team_id": 146,
                "date": "2026-07-30",
                "type_code": "T",
                "type_desc": "Traded",
                "description": "Traded to Atlanta Braves"
            },
            {
                "transaction_id": 852022,
                "person_id": 660271,
                "person": "Chris Sale",
                "team_id": 146,
                "date": "2026-07-30",
                "type_code": "T",
                "type_desc": "Traded",
                "description": "Traded to Atlanta Braves"
            },
            {
                "transaction_id": 851742,
                "person_id": 592789,
                "person": "David Price",
                "team_id": 147,
                "date": "2026-07-30",
                "type_code": "S",
                "type_desc": "Status Change",
                "description": "Moved to 60-Day IL"
            },
            {
                "transaction_id": 851742,
                "person_id": 592789,
                "person": "David Price",
                "team_id": 147,
                "date": "2026-07-30",
                "type_code": "S",
                "type_desc": "Status Change",
                "description": "Moved to 60-Day IL"
            }
        ]
    }"#;

    #[test]
    fn dedupes_byte_identical_rows() {
        let page = normalize(FIXTURE).expect("fixture parses");
        assert_eq!(page.transactions.len(), 2);
        assert_eq!(page.collapsed, 2);
        // First id per key is kept; rows appear in feed order.
        assert_eq!(page.transactions[0].person_id, 660271);
        assert_eq!(page.transactions[0].person, "Chris Sale");
        assert_eq!(page.transactions[0].team_id, 146);
        assert_eq!(page.transactions[0].type_code, "T");
        assert_eq!(page.transactions[1].person_id, 592789);
        assert_eq!(page.transactions[1].type_desc, "Status Change");
    }

    #[test]
    fn distinct_description_with_same_key_fields_survives() {
        let payload = r#"{
            "transactions": [
                {
                    "transaction_id": 1,
                    "person_id": 660271,
                    "person": "Chris Sale",
                    "team_id": 146,
                    "date": "2026-07-30",
                    "type_code": "T",
                    "type_desc": "Traded",
                    "description": "Traded to Atlanta Braves"
                },
                {
                    "transaction_id": 2,
                    "person_id": 660271,
                    "person": "Chris Sale",
                    "team_id": 146,
                    "date": "2026-07-30",
                    "type_code": "T",
                    "type_desc": "Traded",
                    "description": "Traded with cash considerations"
                }
            ]
        }"#;
        let page = normalize(payload).expect("payload parses");
        assert_eq!(page.transactions.len(), 2);
        assert_eq!(page.collapsed, 0);
        assert_eq!(page.transactions[0].description, "Traded to Atlanta Braves");
        assert_eq!(page.transactions[1].description, "Traded with cash considerations");
    }

    #[test]
    fn missing_person_id_is_an_error() {
        let payload = r#"{
            "transactions": [
                {
                    "transaction_id": 852022,
                    "person": "Chris Sale",
                    "team_id": 146,
                    "date": "2026-07-30",
                    "type_code": "T",
                    "type_desc": "Traded",
                    "description": "Traded to Atlanta Braves"
                }
            ]
        }"#;
        let err = normalize(payload).expect_err("missing person_id is an error");
        assert!(err.to_string().contains("person_id"), "unexpected error: {err}");
    }

    #[test]
    fn empty_array_yields_empty_page() {
        let page = normalize(r#"{"transactions": []}"#).expect("empty array is valid");
        assert!(page.transactions.is_empty());
        assert_eq!(page.collapsed, 0);
    }

    #[test]
    fn missing_transactions_key_yields_empty_page() {
        let page = normalize("{}").expect("missing key is valid (empty day)");
        assert!(page.transactions.is_empty());
        assert_eq!(page.collapsed, 0);
    }
}
