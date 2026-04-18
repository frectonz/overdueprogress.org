use std::collections::HashMap;

use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{trace::SdkTracerProvider, Resource};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

const AXIOM_ENDPOINT: &str = "https://api.axiom.co/v1/traces";
const SERVICE_NAME: &str = "overdueprogress";

pub struct OtelHandle {
    provider: SdkTracerProvider,
}

impl Drop for OtelHandle {
    fn drop(&mut self) {
        if let Err(err) = self.provider.shutdown() {
            eprintln!("otel shutdown: {err}");
        }
    }
}

pub fn init(
    axiom_token: Option<&str>,
    axiom_dataset: Option<&str>,
) -> color_eyre::Result<Option<OtelHandle>> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| include_str!("../RUST_LOG.txt").trim().into());
    let fmt_layer = tracing_subscriber::fmt::layer();

    let (otel_layer, handle, axiom_on) = match (axiom_token, axiom_dataset) {
        (Some(token), Some(dataset)) => {
            let provider = build_axiom_provider(token, dataset)?;
            let tracer = provider.tracer(SERVICE_NAME);
            let layer = tracing_opentelemetry::layer().with_tracer(tracer);
            global::set_tracer_provider(provider.clone());
            (Some(layer), Some(OtelHandle { provider }), true)
        }
        _ => (None, None, false),
    };

    Registry::default()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    if axiom_on {
        tracing::info!("tracing: stdout + axiom");
    } else {
        tracing::info!("tracing: stdout only (AXIOM_TOKEN/AXIOM_DATASET unset)");
    }

    Ok(handle)
}

fn build_axiom_provider(token: &str, dataset: &str) -> color_eyre::Result<SdkTracerProvider> {
    let mut headers = HashMap::new();
    headers.insert("Authorization".into(), format!("Bearer {token}"));
    headers.insert("X-Axiom-Dataset".into(), dataset.to_string());

    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(AXIOM_ENDPOINT)
        .with_protocol(Protocol::HttpBinary)
        .with_headers(headers)
        .build()?;

    Ok(SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_attribute(KeyValue::new("service.name", SERVICE_NAME))
                .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
                .build(),
        )
        .build())
}
