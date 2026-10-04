use crate::model::*;
use crate::params::{AlertFilter, GroupFilter, TemplateFilter};
use crate::repo::Repo;
use std::sync::Mutex;

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
    pub fail: bool,
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
}
