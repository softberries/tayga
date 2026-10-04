use crate::model::*;
use crate::params::{AlertFilter, GroupFilter, SeriesQuery, TemplateFilter, TraceFilter};
use crate::repo::Repo;
use std::sync::Mutex;
use tayga_store::metrics_store::MetricPointRow;

#[derive(Default)]
pub struct FakeRepo {
    pub groups: Vec<GroupView>,
    pub detail: Option<GroupDetail>,
    pub story: Option<StoryView>,
    pub trace: Option<TraceView>,
    pub edges: Vec<EdgeView>,
    pub alerts: Vec<LogAlertView>,
    pub templates: Vec<LogTemplateView>,
    pub template_detail: Option<LogTemplateDetail>,
    pub trace_templates: Vec<TraceLogTemplate>,
    pub overview: OverviewView,
    pub stories_series: StoriesSeries,
    pub trace_hits: Vec<TraceHitView>,
    pub services: Vec<String>,
    pub service: Option<ServiceView>,
    pub nodes: Vec<NodeView>,
    pub search: SearchView,
    pub metric_points: Vec<MetricPointRow>,
    pub fail: bool,
    pub last_since: Mutex<Option<u32>>,
    pub last_trace_filter: Mutex<Option<TraceFilter>>,
    pub last_series_query: Mutex<Option<(SeriesQuery, u32)>>,
    pub last_q: Mutex<Option<String>>,
    pub last_filter: Mutex<Option<GroupFilter>>,
    pub last_alert_filter: Mutex<Option<AlertFilter>>,
    pub last_template_filter: Mutex<Option<TemplateFilter>>,
}

impl FakeRepo {
    fn check(&self) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("clickhouse down")
        } else {
            Ok(())
        }
    }
}

impl Repo for FakeRepo {
    async fn story_groups(&self, f: &GroupFilter) -> anyhow::Result<Vec<GroupView>> {
        self.check()?;
        *self.last_filter.lock().unwrap() = Some(f.clone());
        Ok(self.groups.clone())
    }
    async fn story_group(&self, _fp: &str, _since: u32) -> anyhow::Result<Option<GroupDetail>> {
        self.check()?;
        Ok(self.detail.clone())
    }
    async fn story(&self, _id: &str) -> anyhow::Result<Option<StoryView>> {
        self.check()?;
        Ok(self.story.clone())
    }
    async fn trace(&self, id: &str) -> anyhow::Result<TraceView> {
        self.check()?;
        Ok(self.trace.clone().unwrap_or(TraceView {
            trace_id: id.into(),
            spans: vec![],
            logs: vec![],
            story_id: None,
        }))
    }
    async fn service_map(&self, _since: u32) -> anyhow::Result<Vec<EdgeView>> {
        self.check()?;
        Ok(self.edges.clone())
    }
    async fn log_alerts(&self, f: &AlertFilter) -> anyhow::Result<Vec<LogAlertView>> {
        self.check()?;
        *self.last_alert_filter.lock().unwrap() = Some(f.clone());
        Ok(self.alerts.clone())
    }
    async fn log_templates(&self, f: &TemplateFilter) -> anyhow::Result<Vec<LogTemplateView>> {
        self.check()?;
        *self.last_template_filter.lock().unwrap() = Some(f.clone());
        Ok(self.templates.clone())
    }
    async fn log_template(
        &self,
        _id: &str,
        _since: u32,
    ) -> anyhow::Result<Option<LogTemplateDetail>> {
        self.check()?;
        Ok(self.template_detail.clone())
    }
    async fn trace_log_templates(&self, _trace_id: &str) -> anyhow::Result<Vec<TraceLogTemplate>> {
        self.check()?;
        Ok(self.trace_templates.clone())
    }
    async fn overview(&self, since: u32) -> anyhow::Result<OverviewView> {
        self.check()?;
        *self.last_since.lock().unwrap() = Some(since);
        Ok(self.overview.clone())
    }
    async fn stories_series(&self, f: &GroupFilter) -> anyhow::Result<StoriesSeries> {
        self.check()?;
        *self.last_filter.lock().unwrap() = Some(f.clone());
        Ok(self.stories_series.clone())
    }
    async fn traces_search(&self, f: &TraceFilter) -> anyhow::Result<Vec<TraceHitView>> {
        self.check()?;
        *self.last_trace_filter.lock().unwrap() = Some(f.clone());
        Ok(self.trace_hits.clone())
    }
    async fn services(&self) -> anyhow::Result<Vec<String>> {
        self.check()?;
        Ok(self.services.clone())
    }
    async fn service(&self, _name: &str, since: u32) -> anyhow::Result<Option<ServiceView>> {
        self.check()?;
        *self.last_since.lock().unwrap() = Some(since);
        Ok(self.service.clone())
    }
    async fn service_graph(&self, since: u32) -> anyhow::Result<ServiceMapView> {
        self.check()?;
        *self.last_since.lock().unwrap() = Some(since);
        Ok(ServiceMapView {
            edges: self.edges.clone(),
            nodes: self.nodes.clone(),
        })
    }
    async fn search(&self, q: &str) -> anyhow::Result<SearchView> {
        self.check()?;
        *self.last_q.lock().unwrap() = Some(q.to_string());
        Ok(self.search.clone())
    }
    async fn metric_buckets(
        &self,
        q: &SeriesQuery,
        step: u32,
    ) -> anyhow::Result<Vec<MetricPointRow>> {
        self.check()?;
        *self.last_series_query.lock().unwrap() = Some((q.clone(), step));
        Ok(self.metric_points.clone())
    }
}
