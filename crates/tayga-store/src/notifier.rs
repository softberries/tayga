//! Notifier delivery state: one row per `(alert_id, target)`, the latest write wins.

use crate::store::Store;
use serde::Deserialize;

pub const STATUS_PENDING: i8 = 1;
pub const STATUS_DELIVERED: i8 = 2;
pub const STATUS_FAILED: i8 = 3;

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Deserialize)]
pub struct DeliveryRow {
    pub alert_id: String,
    pub target: String,
    /// 1 pending, 2 delivered, 3 failed.
    pub status: i8,
    pub attempts: u32,
    pub last_error: String,
}

impl Store {
    /// Latest delivery state of `alert_id` for `target`, or `None` when none was recorded.
    pub async fn delivery_get(
        &self,
        alert_id: &str,
        target: &str,
    ) -> clickhouse::error::Result<Option<DeliveryRow>> {
        self.client()
            .query(
                "SELECT alert_id, target, status, attempts, last_error \
                 FROM notifier_deliveries FINAL WHERE alert_id = ? AND target = ?",
            )
            .bind(alert_id)
            .bind(target)
            .fetch_optional()
            .await
    }

    /// Records a delivery state; the latest write wins.
    pub async fn delivery_put(&self, r: &DeliveryRow) -> clickhouse::error::Result<()> {
        self.client()
            .query(
                "INSERT INTO notifier_deliveries \
                 (alert_id, target, status, attempts, last_error, updated) \
                 VALUES (?, ?, ?, ?, ?, now64(9))",
            )
            .bind(&r.alert_id)
            .bind(&r.target)
            .bind(r.status)
            .bind(r.attempts)
            .bind(&r.last_error)
            .execute()
            .await
    }
}
