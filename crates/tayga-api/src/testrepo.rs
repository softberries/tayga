use crate::model::*;
use crate::params::GroupFilter;
use crate::repo::Repo;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeRepo {
    pub groups: Vec<GroupView>,
    pub detail: Option<GroupDetail>,
    pub story: Option<StoryView>,
    pub trace: Option<TraceView>,
    pub edges: Vec<EdgeView>,
    pub fail: bool,
    pub last_filter: Mutex<Option<GroupFilter>>,
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
}
