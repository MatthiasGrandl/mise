use serde::Serialize;

// ============================================================================
// Trace types
// ============================================================================

/// OTLP JSON trace export payload
/// See: https://opentelemetry.io/docs/specs/otlp/#otlphttp-request
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpTraceExport {
    pub resource_spans: Vec<ResourceSpan>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpan {
    pub resource: SpanResource,
    pub scope_spans: Vec<ScopeSpan>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpanResource {
    pub attributes: Vec<StringKeyValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeSpan {
    pub scope: InstrumentationScope,
    pub spans: Vec<OtlpSpan>,
}

/// An individual span in the OTLP JSON format
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpSpan {
    pub trace_id: String,
    pub span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: u32,
    /// Start time in nanoseconds since epoch
    pub start_time_unix_nano: String,
    /// End time in nanoseconds since epoch
    pub end_time_unix_nano: String,
    pub attributes: Vec<StringKeyValue>,
    pub status: OtlpStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<SpanLink>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpStatus {
    pub code: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A link to another span (used for dependency relationships)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanLink {
    pub trace_id: String,
    pub span_id: String,
    pub attributes: Vec<StringKeyValue>,
}

// ============================================================================
// Log types
// ============================================================================

/// OTLP JSON log export payload
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpLogExport {
    pub resource_logs: Vec<ResourceLogs>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLogs {
    pub resource: SpanResource,
    pub scope_logs: Vec<ScopeLogs>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeLogs {
    pub scope: InstrumentationScope,
    pub log_records: Vec<OtlpLogRecord>,
}

/// An individual log record in the OTLP JSON format
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpLogRecord {
    pub time_unix_nano: String,
    pub observed_time_unix_nano: String,
    pub severity_number: u32,
    pub severity_text: String,
    pub body: LogBody,
    pub attributes: Vec<StringKeyValue>,
    /// W3C TraceFlags — bit 0x01 is "sampled". Must be set for SigNoz to
    /// correlate the log to its trace.
    pub flags: u32,
    pub trace_id: String,
    pub span_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBody {
    pub string_value: String,
}

// ============================================================================
// Shared types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct InstrumentationScope {
    pub name: String,
    pub version: String,
}

/// String-typed key-value attribute
#[derive(Debug, Clone, Serialize)]
pub struct StringKeyValue {
    pub key: String,
    pub value: AttributeValue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeValue {
    pub string_value: String,
}

impl StringKeyValue {
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: key.to_string(),
            value: AttributeValue {
                string_value: value.to_string(),
            },
        }
    }
}

/// Span status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

impl SpanStatus {
    pub fn code(&self) -> u32 {
        match self {
            SpanStatus::Unset => 0,
            SpanStatus::Ok => 1,
            SpanStatus::Error => 2,
        }
    }
}

/// OTLP span kind values
pub const SPAN_KIND_INTERNAL: u32 = 1;

/// OTLP log severity numbers
pub const SEVERITY_INFO: u32 = 9;
pub const SEVERITY_ERROR: u32 = 17;
