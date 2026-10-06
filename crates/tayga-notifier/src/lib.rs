//! Log alert delivery (spec 7b §4): settings, payload builders, the retrying delivery of one alert
//! to one target, the per-record route and commit decision, and metrics. The Kafka loop is in `main.rs`.

pub mod config;
pub mod deliver;
pub mod metrics;
pub mod payload;
pub mod route;
