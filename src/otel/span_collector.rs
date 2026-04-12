use crate::otel::span::OtelSpan;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Serializes span exports so OTLP updates for a single run keep their order.
#[derive(Clone)]
pub struct OtelSpanCollector {
    tx: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<OtelSpan>>>>>,
}

impl OtelSpanCollector {
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

    pub fn push(&self, span: OtelSpan) {
        self.push_batch(vec![span]);
    }

    pub fn push_batch(&self, spans: Vec<OtelSpan>) {
        if let Some(ref sender) = *self.tx.lock().unwrap() {
            let _ = sender.send(spans);
        }
    }

    pub fn shutdown(&self) {
        *self.tx.lock().unwrap() = None;
    }

    async fn drain_loop(mut rx: mpsc::UnboundedReceiver<Vec<OtelSpan>>) {
        while let Some(spans) = rx.recv().await {
            if let Err(err) = crate::otel::export_spans(spans).await {
                debug!("failed to export otel spans: {err}");
            }
        }
    }
}
