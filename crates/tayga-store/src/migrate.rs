//! Ordered, idempotent schema migrations. Run from exactly one place
//! (`tayga-writer migrate`) because ClickHouse has no DDL locking.

use clickhouse::Client;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct ClickHouseSettings {
    pub url: String,
    #[serde(default = "default_database")]
    pub database: String,
}

fn default_database() -> String {
    "tayga".to_string()
}

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_raw_tables.sql")),
    (2, include_str!("../migrations/0002_analysis_tables.sql")),
    (
        3,
        include_str!("../migrations/0003_replay_safe_analysis.sql"),
    ),
    (4, include_str!("../migrations/0004_log_templates.sql")),
];

pub fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub async fn migrate(s: &ClickHouseSettings) -> anyhow::Result<Vec<u32>> {
    let server = Client::default().with_url(&s.url);
    server
        .query(&format!("CREATE DATABASE IF NOT EXISTS `{}`", s.database))
        .execute()
        .await?;
    let db = server.clone().with_database(&s.database);
    db.query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version UInt32, applied_at DateTime DEFAULT now()) \
         ENGINE = MergeTree ORDER BY version",
    )
    .execute()
    .await?;
    let applied: Vec<u32> = db
        .query("SELECT version FROM schema_migrations")
        .fetch_all()
        .await?;

    let mut newly = Vec::new();
    for (version, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        for stmt in split_statements(sql) {
            db.query(&stmt).execute().await?;
        }
        db.query("INSERT INTO schema_migrations (version) VALUES (?)")
            .bind(*version)
            .execute()
            .await?;
        tracing::info!(version, "applied migration");
        newly.push(*version);
    }
    Ok(newly)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_semicolons_and_drops_empty() {
        let v = split_statements("CREATE A;\n\n  CREATE B ;\n;  ");
        assert_eq!(v, vec!["CREATE A".to_string(), "CREATE B".to_string()]);
    }

    #[test]
    fn migrations_are_strictly_increasing() {
        assert!(MIGRATIONS.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(split_statements(MIGRATIONS[0].1).len(), 2);
        assert_eq!(split_statements(MIGRATIONS[1].1).len(), 3);
        assert_eq!(split_statements(MIGRATIONS[2].1).len(), 3);
        assert_eq!(split_statements(MIGRATIONS[3].1).len(), 3);
    }
}
