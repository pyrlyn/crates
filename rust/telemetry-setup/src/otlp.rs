// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTLP/HTTP trace export, built only with the `otlp` feature.

use std::borrow::Cow;
use std::time::Duration;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    BatchSpanProcessor, SdkTracerProvider, Span, SpanData, SpanProcessor,
};

use crate::redact::{REDACTED, Redactor, is_secret_name};
use crate::{BoxedLayer, Error};

/// Flushes and shuts the exporter down on drop so the last spans are not lost at exit.
#[derive(Debug)]
pub(crate) struct Provider(SdkTracerProvider);

impl Drop for Provider {
    fn drop(&mut self) {
        // Nowhere to report an export failure at shutdown; the process is exiting anyway.
        let _ = self.0.shutdown();
    }
}

pub(crate) fn layer(
    endpoint: &str,
    service: &str,
    redactor: Redactor,
) -> Result<(BoxedLayer, Provider), Error> {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| Error::Otlp(e.to_string()))?;
    let processor = Scrubbing {
        inner: BatchSpanProcessor::builder(exporter).build(),
        redactor,
    };
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(
            Resource::builder()
                .with_service_name(service.to_owned())
                .build(),
        )
        .build();
    let layer: BoxedLayer =
        Box::new(tracing_opentelemetry::layer().with_tracer(provider.tracer(service.to_owned())));
    Ok((layer, Provider(provider)))
}

/// The OTel layer records span fields itself, bypassing the log-line scrubbing, so spans are
/// masked here, just before they leave the process.
#[derive(Debug)]
struct Scrubbing<P> {
    inner: P,
    redactor: Redactor,
}

impl<P: SpanProcessor> SpanProcessor for Scrubbing<P> {
    fn on_start(&self, span: &mut Span, cx: &opentelemetry::Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, mut span: SpanData) {
        scrub_attributes(&self.redactor, &mut span.attributes);
        for event in &mut span.events.events {
            scrub_attributes(&self.redactor, &mut event.attributes);
            if let Cow::Owned(name) = self.redactor.scrub_text(&event.name) {
                event.name = name.into();
            }
        }
        self.inner.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

fn scrub_attributes(redactor: &Redactor, attributes: &mut [opentelemetry::KeyValue]) {
    for attribute in attributes {
        if is_secret_name(attribute.key.as_str()) {
            attribute.value = REDACTED.into();
        } else if let opentelemetry::Value::String(text) = &attribute.value
            && let Cow::Owned(masked) = redactor.scrub_text(text.as_str())
        {
            attribute.value = masked.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::KeyValue;

    use super::scrub_attributes;
    use crate::redact::Redactor;

    #[test]
    fn otlp_attributes_are_masked() {
        let mut attributes = vec![
            KeyValue::new("api_key", "k"),
            KeyValue::new("note", "sent Bearer abcdef0123456789"),
            KeyValue::new("turn", 3_i64),
        ];
        scrub_attributes(&Redactor::new::<&str>(&[]).unwrap(), &mut attributes);
        assert_eq!(attributes[0].value.as_str(), "[REDACTED]");
        assert_eq!(attributes[1].value.as_str(), "sent [REDACTED]");
        assert_eq!(attributes[2].value, 3_i64.into());
    }
}
