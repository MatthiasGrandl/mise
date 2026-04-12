use crate::otel::types::{
    OtlpSpan, OtlpStatus, SPAN_KIND_INTERNAL, SpanLink, SpanStatus, StringKeyValue,
};
use std::time::SystemTime;

/// A collected span representing a task execution or grouping
#[derive(Debug, Clone)]
pub struct OtelSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time: SystemTime,
    pub end_time: SystemTime,
    pub status: SpanStatus,
    pub error_message: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub links: Vec<(String, String)>, // (trace_id, span_id) pairs
}

impl OtelSpan {
    pub fn into_otlp(self) -> OtlpSpan {
        let attributes: Vec<StringKeyValue> = self
            .attributes
            .into_iter()
            .map(|(k, v)| StringKeyValue::new(&k, &v))
            .collect();

        let links: Vec<SpanLink> = self
            .links
            .into_iter()
            .map(|(trace_id, span_id)| SpanLink {
                trace_id,
                span_id,
                attributes: vec![],
            })
            .collect();

        OtlpSpan {
            trace_id: self.trace_id,
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            name: self.name,
            kind: SPAN_KIND_INTERNAL,
            start_time_unix_nano: system_time_to_nanos(&self.start_time),
            end_time_unix_nano: system_time_to_nanos(&self.end_time),
            attributes,
            status: OtlpStatus {
                code: self.status.code(),
                message: self.error_message,
            },
            links,
        }
    }
}

fn system_time_to_nanos(time: &SystemTime) -> String {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}
