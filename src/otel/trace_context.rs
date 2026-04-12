use crate::otel::span::OtelSpan;
use crate::otel::types::SpanStatus;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Generates random hex IDs for trace and span identifiers
fn random_hex(len: usize) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(len * 2);
    for _ in 0..len {
        let byte: u8 = rand::random();
        write!(buf, "{byte:02x}").unwrap();
    }
    buf
}

fn new_trace_id() -> String {
    random_hex(16) // 32 hex chars = 16 bytes
}

fn new_span_id() -> String {
    random_hex(8) // 16 hex chars = 8 bytes
}

/// Shared state for collecting spans across concurrent task execution.
///
/// The nesting model:
/// - Tasks with a monorepo config_root are grouped under a parent span for
///   that config_root
/// - All other tasks are direct children of the trace's root span
///
/// Emission model: every span is sent to the backend exactly once, when
/// it ends. Task spans are emitted from `end_task_span`. The root span
/// and monorepo group spans are emitted from `finish()` with their full
/// aggregated duration (min of child starts / max of child ends) and
/// error state. This matches standard OTLP semantics and works with any
/// backend without relying on merge-on-duplicate behavior.
#[derive(Clone)]
pub struct TraceContext {
    inner: Arc<Mutex<TraceContextInner>>,
}

struct TraceContextInner {
    trace_id: String,
    root_span: RootSpan,
    /// Monorepo parent spans keyed by config_root path
    monorepo_parents: HashMap<PathBuf, MonorepoParent>,
    /// Collected completed task / root / monorepo spans, returned by
    /// `finish()` for any callers that want to inspect them.
    spans: Vec<OtelSpan>,
    span_collector: crate::otel::OtelSpanCollector,
    span_collector_handle: Option<tokio::task::JoinHandle<()>>,
}

struct RootSpan {
    parent_span_id: Option<String>,
    span_id: String,
    name: String,
    /// When the trace was created. Used as a fallback start/end if no
    /// task ever ran so `finish()` still emits a reasonable span.
    init_time: SystemTime,
    /// Min start_time across all completed tasks.
    min_start: Option<SystemTime>,
    /// Max end_time across all completed tasks.
    max_end: Option<SystemTime>,
    has_error: bool,
}

struct MonorepoParent {
    span_id: String,
    init_time: SystemTime,
    min_start: Option<SystemTime>,
    max_end: Option<SystemTime>,
    has_error: bool,
}

/// Info returned when a task span is started, needed for the end phase and log correlation.
pub struct StartedSpan {
    pub trace_id: String,
    pub span_id: String,
    /// None means the task is a top-level span (no parent).
    pub parent_span_id: Option<String>,
    pub start_time: SystemTime,
}

impl TraceContext {
    pub fn new(root_span_name: &str) -> Self {
        Self::new_with_parent(root_span_name, new_trace_id(), None)
    }

    pub fn from_parent(root_span_name: &str, trace_id: String, parent_span_id: String) -> Self {
        Self::new_with_parent(root_span_name, trace_id, Some(parent_span_id))
    }

    fn new_with_parent(
        root_span_name: &str,
        trace_id: String,
        parent_span_id: Option<String>,
    ) -> Self {
        let root_span_id = new_span_id();
        let root_init_time = SystemTime::now();
        let (span_collector, span_collector_handle) = crate::otel::OtelSpanCollector::new();

        Self {
            inner: Arc::new(Mutex::new(TraceContextInner {
                trace_id,
                root_span: RootSpan {
                    parent_span_id,
                    span_id: root_span_id,
                    name: root_span_name.to_string(),
                    init_time: root_init_time,
                    min_start: None,
                    max_end: None,
                    has_error: false,
                },
                monorepo_parents: HashMap::new(),
                spans: Vec::new(),
                span_collector,
                span_collector_handle: Some(span_collector_handle),
            })),
        }
    }

    /// Get or create a monorepo parent span for a given config_root.
    /// Only allocates state; the group's span is emitted once at
    /// `finish()` with aggregated start/end times.
    fn get_or_create_monorepo_parent(&self, config_root: &PathBuf) -> String {
        let mut inner = self.inner.lock().unwrap();
        if let Some(parent) = inner.monorepo_parents.get(config_root) {
            return parent.span_id.clone();
        }
        let span_id = new_span_id();
        inner.monorepo_parents.insert(
            config_root.clone(),
            MonorepoParent {
                span_id: span_id.clone(),
                init_time: SystemTime::now(),
                min_start: None,
                max_end: None,
                has_error: false,
            },
        );
        span_id
    }

    /// Fold a completed task into the root span's aggregate state.
    fn fold_into_root(inner: &mut TraceContextInner, task_start: SystemTime, task_end: SystemTime, status: SpanStatus) {
        inner.root_span.min_start = Some(match inner.root_span.min_start {
            Some(existing) if existing <= task_start => existing,
            _ => task_start,
        });
        inner.root_span.max_end = Some(match inner.root_span.max_end {
            Some(existing) if existing >= task_end => existing,
            _ => task_end,
        });
        inner.root_span.has_error |= status == SpanStatus::Error;
    }

    /// Fold a completed task into the matching monorepo group's aggregate state.
    fn fold_into_group(
        inner: &mut TraceContextInner,
        parent_span_id: &str,
        task_start: SystemTime,
        task_end: SystemTime,
        status: SpanStatus,
    ) {
        for parent in inner.monorepo_parents.values_mut() {
            if parent.span_id != parent_span_id {
                continue;
            }
            parent.min_start = Some(match parent.min_start {
                Some(existing) if existing <= task_start => existing,
                _ => task_start,
            });
            parent.max_end = Some(match parent.max_end {
                Some(existing) if existing >= task_end => existing,
                _ => task_end,
            });
            parent.has_error |= status == SpanStatus::Error;
            return;
        }
    }

    /// Determine the parent span ID for a task.
    /// If the task has a monorepo config_root that differs from the project root,
    /// return the monorepo parent span. Otherwise the task hangs directly off the
    /// run's root span.
    pub fn parent_span_for_task(
        &self,
        config_root: Option<&PathBuf>,
        project_root: Option<&PathBuf>,
    ) -> Option<String> {
        let root_span_id = self.inner.lock().unwrap().root_span.span_id.clone();
        if let Some(cr) = config_root {
            let is_monorepo = project_root.is_none_or(|pr| cr != pr);
            if is_monorepo {
                return Some(self.get_or_create_monorepo_parent(cr));
            }
        }
        Some(root_span_id)
    }

    /// Start a task span: allocates IDs and returns the context needed by
    /// `end_task_span` and log correlation. The span itself is not emitted
    /// until `end_task_span` is called — we send each span exactly once.
    pub fn start_task_span(
        &self,
        _task_name: &str,
        parent_span_id: Option<String>,
        _attributes: Vec<(String, String)>,
    ) -> StartedSpan {
        let trace_id = self.inner.lock().unwrap().trace_id.clone();
        let span_id = new_span_id();
        let start_time = SystemTime::now();
        StartedSpan {
            trace_id,
            span_id,
            parent_span_id,
            start_time,
        }
    }

    /// End a task span: emits the completed span with real timing/status,
    /// and folds its timing into the parent root/group aggregate state so
    /// those get correct durations when `finish()` emits them.
    pub async fn end_task_span(
        &self,
        started: StartedSpan,
        task_name: &str,
        end_time: SystemTime,
        status: SpanStatus,
        error_message: Option<String>,
        attributes: Vec<(String, String)>,
    ) {
        let span = OtelSpan {
            trace_id: started.trace_id.clone(),
            span_id: started.span_id.clone(),
            parent_span_id: started.parent_span_id.clone(),
            name: task_name.to_string(),
            start_time: started.start_time,
            end_time,
            status,
            error_message,
            attributes,
            links: vec![],
        };
        let task_start = started.start_time;
        let mut inner = self.inner.lock().unwrap();
        if let Some(parent_span_id) = &started.parent_span_id {
            Self::fold_into_group(&mut inner, parent_span_id, task_start, end_time, status);
        }
        Self::fold_into_root(&mut inner, task_start, end_time, status);
        inner.span_collector.push(span.clone());
        inner.spans.push(span);
    }

    /// Finalize the trace: emit the monorepo group spans and the root
    /// span (each exactly once) with their full aggregated duration /
    /// error state, then drain the span collector.
    pub async fn finish(self, has_failures: bool) -> Vec<OtelSpan> {
        let mut inner = self.inner.lock().unwrap();
        let now = SystemTime::now();
        let trace_id = inner.trace_id.clone();
        let root_span_id = inner.root_span.span_id.clone();
        let span_collector = inner.span_collector.clone();

        // Emit one monorepo group span per tracked config_root.
        let groups: Vec<(PathBuf, MonorepoParent)> = inner
            .monorepo_parents
            .drain()
            .collect();
        for (config_root, parent) in groups {
            let name = config_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| config_root.display().to_string());
            let start = parent.min_start.unwrap_or(parent.init_time);
            let end = parent.max_end.unwrap_or(now);
            let group_span = OtelSpan {
                trace_id: trace_id.clone(),
                span_id: parent.span_id,
                parent_span_id: Some(root_span_id.clone()),
                name,
                start_time: start,
                end_time: end,
                status: if parent.has_error {
                    SpanStatus::Error
                } else {
                    SpanStatus::Ok
                },
                error_message: None,
                attributes: vec![
                    ("mise.span_type".to_string(), "monorepo_group".to_string()),
                    (
                        "mise.config_root".to_string(),
                        config_root.display().to_string(),
                    ),
                ],
                links: vec![],
            };
            span_collector.push(group_span.clone());
            inner.spans.push(group_span);
        }

        // Emit the root span.
        inner.root_span.has_error |= has_failures;
        let root_start = inner.root_span.min_start.unwrap_or(inner.root_span.init_time);
        let root_end = inner.root_span.max_end.unwrap_or(now);
        let root_span = OtelSpan {
            trace_id,
            span_id: root_span_id,
            parent_span_id: inner.root_span.parent_span_id.clone(),
            name: inner.root_span.name.clone(),
            start_time: root_start,
            end_time: root_end,
            status: if inner.root_span.has_error {
                SpanStatus::Error
            } else {
                SpanStatus::Ok
            },
            error_message: None,
            attributes: vec![("mise.span_type".to_string(), "run".to_string())],
            links: vec![],
        };
        span_collector.push(root_span.clone());
        inner.spans.push(root_span);

        inner.span_collector.shutdown();
        let handle = inner.span_collector_handle.take();
        let spans = std::mem::take(&mut inner.spans);
        drop(inner);
        if let Some(handle) = handle {
            let _ = handle.await;
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_by_name<'a>(spans: &'a [OtelSpan], name: &str) -> &'a OtelSpan {
        spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("missing span {name}"))
    }

    #[tokio::test]
    async fn finish_builds_root_and_monorepo_hierarchy() {
        let trace = TraceContext::new("mise run");
        let project_root = PathBuf::from("/workspace");
        let monorepo_root = PathBuf::from("/workspace/packages/frontend");

        let direct_parent = trace.parent_span_for_task(Some(&project_root), Some(&project_root));
        let direct_task = trace.start_task_span("lint", direct_parent, vec![]);
        trace
            .end_task_span(
                direct_task,
                "lint",
                SystemTime::now(),
                SpanStatus::Ok,
                None,
                vec![],
            )
            .await;

        let monorepo_parent = trace.parent_span_for_task(Some(&monorepo_root), Some(&project_root));
        let monorepo_task = trace.start_task_span("build", monorepo_parent, vec![]);
        trace
            .end_task_span(
                monorepo_task,
                "build",
                SystemTime::now(),
                SpanStatus::Ok,
                None,
                vec![],
            )
            .await;

        let spans = trace.finish(false).await;
        let root = span_by_name(&spans, "mise run");
        let group = span_by_name(&spans, "frontend");
        let lint = span_by_name(&spans, "lint");
        let build = span_by_name(&spans, "build");

        assert_eq!(root.parent_span_id, None);
        assert_eq!(root.status, SpanStatus::Ok);
        assert_eq!(group.parent_span_id.as_deref(), Some(root.span_id.as_str()));
        assert_eq!(group.status, SpanStatus::Ok);
        assert_eq!(lint.parent_span_id.as_deref(), Some(root.span_id.as_str()));
        assert_eq!(
            build.parent_span_id.as_deref(),
            Some(group.span_id.as_str())
        );
    }

    #[tokio::test]
    async fn finish_marks_failed_group_and_root_error() {
        let trace = TraceContext::new("mise run");
        let project_root = PathBuf::from("/workspace");
        let monorepo_root = PathBuf::from("/workspace/packages/frontend");

        let parent_span = trace.parent_span_for_task(Some(&monorepo_root), Some(&project_root));
        let task = trace.start_task_span("build", parent_span, vec![]);
        trace
            .end_task_span(
                task,
                "build",
                SystemTime::now(),
                SpanStatus::Error,
                Some("boom".to_string()),
                vec![],
            )
            .await;

        let spans = trace.finish(true).await;
        let root = span_by_name(&spans, "mise run");
        let group = span_by_name(&spans, "frontend");
        let build = span_by_name(&spans, "build");

        assert_eq!(build.status, SpanStatus::Error);
        assert_eq!(group.status, SpanStatus::Error);
        assert_eq!(root.status, SpanStatus::Error);
    }

    #[tokio::test]
    async fn finish_keeps_parent_span_for_nested_run() {
        let trace = TraceContext::from_parent(
            "mise run nested",
            "0123456789abcdef0123456789abcdef".to_string(),
            "0123456789abcdef".to_string(),
        );

        let spans = trace.finish(false).await;
        let root = span_by_name(&spans, "mise run nested");

        assert_eq!(root.trace_id, "0123456789abcdef0123456789abcdef");
        assert_eq!(root.parent_span_id.as_deref(), Some("0123456789abcdef"));
    }
}
