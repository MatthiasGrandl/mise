mod log_collector;
mod span;
mod span_collector;
mod trace_context;
mod types;

pub use log_collector::OtelLogCollector;
pub use span::OtelSpan;
pub use span_collector::OtelSpanCollector;
pub use trace_context::TraceContext;
pub use types::SpanStatus;

use crate::config::Settings;
use crate::http::HTTP;
use eyre::Result;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::str::FromStr;
use types::{
    InstrumentationScope, OtlpLogExport, OtlpTraceExport, ResourceLogs, ResourceSpan, ScopeLogs,
    ScopeSpan, SpanResource, StringKeyValue,
};

/// Check if OpenTelemetry export is enabled
pub fn is_enabled() -> bool {
    let settings = Settings::get();
    settings.otel.endpoint.is_some()
}

/// Build common OTLP headers from settings
fn otel_headers() -> HeaderMap {
    let settings = Settings::get();
    let mut headers = HeaderMap::new();
    if let Some(ref otel_headers) = settings.otel.headers {
        for (key, value) in otel_headers {
            if let (Ok(name), Ok(val)) = (HeaderName::from_str(key), HeaderValue::from_str(value)) {
                headers.insert(name, val);
            }
        }
    }
    headers
}

/// Build the OTLP resource with service.name
fn otel_resource() -> SpanResource {
    let settings = Settings::get();
    let service_name = settings.otel.service_name.clone();
    SpanResource {
        attributes: vec![
            StringKeyValue::new("service.name", &service_name),
            StringKeyValue::new("telemetry.sdk.name", "mise"),
            StringKeyValue::new("telemetry.sdk.language", "rust"),
        ],
    }
}

/// Get the OTLP endpoint base URL from settings
fn otel_endpoint() -> Option<String> {
    Settings::get()
        .otel
        .endpoint
        .as_ref()
        .map(|e| e.trim_end_matches('/').to_string())
}

/// Export collected spans to the OTLP /v1/traces endpoint.
pub async fn export_spans(spans: Vec<OtelSpan>) -> Result<()> {
    if spans.is_empty() {
        return Ok(());
    }
    let Some(endpoint) = otel_endpoint() else {
        return Ok(());
    };
    let url = format!("{endpoint}/v1/traces");
    let otlp_spans: Vec<types::OtlpSpan> = spans.into_iter().map(|s| s.into_otlp()).collect();
    let span_count = otlp_spans.len();
    let payload = OtlpTraceExport {
        resource_spans: vec![ResourceSpan {
            resource: otel_resource(),
            scope_spans: vec![ScopeSpan {
                scope: InstrumentationScope {
                    name: "mise.tasks".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                spans: otlp_spans,
            }],
        }],
    };
    match HTTP
        .post_json_with_headers(&url, &payload, &otel_headers())
        .await
    {
        Ok(true) => debug!("exported {span_count} spans to {url}"),
        Ok(false) => debug!("collector rejected {span_count} spans at {url}"),
        Err(err) => debug!("failed to export otel spans: {err}"),
    }
    Ok(())
}

/// Export log records to the OTLP /v1/logs endpoint.
pub async fn export_logs(logs: Vec<types::OtlpLogRecord>) -> Result<()> {
    if logs.is_empty() {
        return Ok(());
    }
    let Some(endpoint) = otel_endpoint() else {
        return Ok(());
    };
    let url = format!("{endpoint}/v1/logs");
    let log_count = logs.len();
    let payload = OtlpLogExport {
        resource_logs: vec![ResourceLogs {
            resource: otel_resource(),
            scope_logs: vec![ScopeLogs {
                scope: InstrumentationScope {
                    name: "mise.tasks".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                log_records: logs,
            }],
        }],
    };
    match HTTP
        .post_json_with_headers(&url, &payload, &otel_headers())
        .await
    {
        Ok(true) => debug!("exported {log_count} log records to {url}"),
        Ok(false) => debug!("collector rejected {log_count} log records at {url}"),
        Err(err) => debug!("failed to export otel logs: {err}"),
    }
    Ok(())
}
