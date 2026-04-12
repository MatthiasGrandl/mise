use crate::otel::types::{LogBody, OtlpLogRecord, SEVERITY_ERROR, SEVERITY_INFO, StringKeyValue};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;

/// A log line captured from task output
struct LogLine {
    body: String,
    is_stderr: bool,
    timestamp: SystemTime,
    task_name: String,
    trace_id: String,
    span_id: String,
}

/// Collects task output lines and streams them as OTLP log records.
///
/// Uses a channel-based approach: sync closures (from CmdLineRunner callbacks)
/// send lines into an unbounded channel. A background tokio task drains the
/// channel in batches and POSTs to `/v1/logs`.
#[derive(Clone)]
pub struct OtelLogCollector {
    tx: Arc<Mutex<Option<mpsc::UnboundedSender<LogLine>>>>,
}

impl OtelLogCollector {
    /// Create a new log collector and spawn a background task to drain and export logs.
    /// Returns the collector (for sending lines) and a JoinHandle for the background task.
    pub fn new() -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(Self::drain_loop(rx));
        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
            },
            handle,
        )
    }

    /// Shut down the collector by dropping the sender, which causes the
    /// drain loop to finish after flushing remaining messages.
    pub fn shutdown(&self) {
        *self.tx.lock().unwrap() = None;
    }

    /// Create a sender bound to a specific task's trace/span context.
    /// The returned closure can be called from sync code (CmdLineRunner callbacks).
    pub fn sender(
        &self,
        task_name: String,
        trace_id: String,
        span_id: String,
        is_stderr: bool,
    ) -> Box<dyn Fn(String) + Send> {
        let tx = self.tx.clone();
        Box::new(move |line: String| {
            if let Some(ref sender) = *tx.lock().unwrap() {
                let _ = sender.send(LogLine {
                    body: line,
                    is_stderr,
                    timestamp: SystemTime::now(),
                    task_name: task_name.clone(),
                    trace_id: trace_id.clone(),
                    span_id: span_id.clone(),
                });
            }
        })
    }

    /// Background loop: drains the channel, batches log records, and exports them.
    async fn drain_loop(mut rx: mpsc::UnboundedReceiver<LogLine>) {
        let mut batch: Vec<OtlpLogRecord> = Vec::new();
        let flush_interval = Duration::from_secs(2);

        loop {
            // Wait for the first message or channel close
            let line = tokio::select! {
                msg = rx.recv() => msg,
                () = tokio::time::sleep(flush_interval) => {
                    // Timeout — flush whatever we have
                    if !batch.is_empty() {
                        Self::flush(&mut batch).await;
                    }
                    continue;
                }
            };

            let Some(line) = line else {
                // Channel closed — flush remaining and exit
                if !batch.is_empty() {
                    Self::flush(&mut batch).await;
                }
                break;
            };

            batch.push(Self::to_log_record(line));

            // Drain any additional immediately available messages
            while let Ok(line) = rx.try_recv() {
                batch.push(Self::to_log_record(line));
            }

            // Flush if batch is large enough, otherwise let the timer handle it
            if batch.len() >= 100 {
                Self::flush(&mut batch).await;
            }
        }
    }

    fn to_log_record(line: LogLine) -> OtlpLogRecord {
        let (severity_number, severity_text) = if line.is_stderr {
            (SEVERITY_ERROR, "ERROR")
        } else {
            (SEVERITY_INFO, "INFO")
        };

        let nanos = system_time_to_nanos(&line.timestamp);
        OtlpLogRecord {
            time_unix_nano: nanos.clone(),
            observed_time_unix_nano: nanos,
            severity_number,
            severity_text: severity_text.to_string(),
            body: LogBody {
                string_value: line.body,
            },
            attributes: vec![
                StringKeyValue::new("mise.task.name", &line.task_name),
                StringKeyValue::new(
                    "output.stream",
                    if line.is_stderr { "stderr" } else { "stdout" },
                ),
            ],
            // TraceFlags: 0x01 = sampled. Required for SigNoz to link logs to traces.
            flags: 1,
            trace_id: line.trace_id,
            span_id: line.span_id,
        }
    }

    async fn flush(batch: &mut Vec<OtlpLogRecord>) {
        let logs = std::mem::take(batch);
        if let Err(err) = crate::otel::export_logs(logs).await {
            debug!("failed to export otel logs: {err}");
        }
    }
}

fn system_time_to_nanos(time: &SystemTime) -> String {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}
