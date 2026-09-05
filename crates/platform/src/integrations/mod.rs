//! Replaceable external boundaries. Only isolated, explicitly labelled local adapters ship.
//! No provider URL, credential, public download or business attachment state lives here.
use crate::config::Environment;
use crate::outbox::{BoxFuture, OutboxEvent, Sink, SinkError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod local;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    #[error("适配器已关闭")]
    Disabled,
    #[error("适配器不可用（底层信息已脱敏）")]
    Unavailable,
    #[error("适配器输入或处理预算非法")]
    InvalidInput,
    #[error("适配器幂等标识已绑定其他输入")]
    Conflict,
    #[error("对象或受信处理任务不存在")]
    NotFound,
}
impl From<AdapterError> for SinkError {
    fn from(value: AdapterError) -> Self {
        match value {
            AdapterError::Disabled => Self::Disabled,
            AdapterError::Unavailable => Self::Retryable,
            AdapterError::InvalidInput | AdapterError::Conflict | AdapterError::NotFound => {
                Self::Permanent
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterMode {
    #[default]
    Disabled,
    LocalTest,
}

/// All defaults close the adapters. Limits below are explicit local processing budgets,
/// never production quotas. The root must remain outside served/static directories.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdapterConfig {
    pub mode: AdapterMode,
    pub local_root: Option<String>,
}
impl std::fmt::Debug for AdapterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterConfig")
            .field("mode", &self.mode)
            .field("local_root", &"[local path omitted]")
            .finish()
    }
}
impl AdapterConfig {
    pub fn validate(&self, environment: Environment) -> Result<(), AdapterError> {
        match self.mode {
            AdapterMode::Disabled => Ok(()),
            AdapterMode::LocalTest if environment.is_production_like() => {
                Err(AdapterError::InvalidInput)
            }
            AdapterMode::LocalTest if self.local_root.as_ref().is_none_or(|p| p.is_empty()) => {
                Err(AdapterError::InvalidInput)
            }
            AdapterMode::LocalTest => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectArea {
    Quarantine,
    Processed,
}
impl ObjectArea {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quarantine => "quarantine",
            Self::Processed => "processed",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKey {
    area: ObjectArea,
    id: String,
}
impl ObjectKey {
    pub fn new(area: ObjectArea, id: &str) -> Result<Self, AdapterError> {
        if !crate::transaction::validate_operation_id(id) {
            return Err(AdapterError::InvalidInput);
        }
        Ok(Self {
            area,
            id: id.into(),
        })
    }
    pub fn area(&self) -> ObjectArea {
        self.area
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectReceipt {
    pub sha256: String,
    pub bytes: u64,
    pub local_only: bool,
}
pub trait ObjectStore: Send + Sync {
    fn put<'a>(
        &'a self,
        key: &'a ObjectKey,
        bytes: &'a [u8],
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<ObjectReceipt, AdapterError>>;
    /// Internal-only read; this contract does not issue public URLs or authorize downloads.
    fn read<'a>(
        &'a self,
        key: &'a ObjectKey,
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<Vec<u8>, AdapterError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageMime {
    Png,
    Jpeg,
}
#[derive(Clone)]
pub struct ImageInput {
    pub bytes: Vec<u8>,
    pub mime: ImageMime,
    pub max_bytes: u64,
    pub max_pixels: u64,
    pub max_width: u32,
    pub max_height: u32,
}
impl std::fmt::Debug for ImageInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImageInput([content omitted])")
    }
}
#[derive(Clone)]
pub struct ProcessedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
pub trait ImageProcessor: Send + Sync {
    fn process<'a>(
        &'a self,
        input: ImageInput,
    ) -> BoxFuture<'a, Result<ProcessedImage, AdapterError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanVerdict {
    SimulatedClean,
    SimulatedRejected,
}
/// A simulated clean result never certifies safety or permits publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub sha256: String,
    pub bytes: u64,
    pub verdict: ScanVerdict,
    pub simulated: bool,
}
pub trait MalwareScanner: Send + Sync {
    fn scan<'a>(
        &'a self,
        event_id: &'a str,
        bytes: &'a [u8],
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<ScanReport, AdapterError>>;
}

/// Closed vocabulary excludes error messages, URLs, headers, user IDs and provider text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Mail,
    Media,
    Search,
    Analytics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    Unavailable,
    InvalidInput,
    HardBounce,
    Complaint,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafeDiagnostic {
    pub dependency: DependencyKind,
    pub code: DiagnosticCode,
}
pub trait ErrorTracker: Send + Sync {
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        diagnostic: &'a SafeDiagnostic,
    ) -> BoxFuture<'a, Result<(), AdapterError>>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateKind {
    Reads,
    Contributions,
    Topics,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafeAggregate {
    pub kind: AggregateKind,
    pub count: u64,
}
pub trait BasicAnalytics: Send + Sync {
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        value: &'a SafeAggregate,
    ) -> BoxFuture<'a, Result<(), AdapterError>>;
}

pub enum Effect {
    Store {
        key: ObjectKey,
        bytes: Vec<u8>,
        max_bytes: u64,
    },
    Image {
        output: ObjectKey,
        input: ImageInput,
    },
    Scan {
        bytes: Vec<u8>,
        max_bytes: u64,
    },
    Error(SafeDiagnostic),
    Analytics(SafeAggregate),
}
/// Implementations resolve committed references under their original domain permissions.
/// No Deserialize/public event-ingestion constructor is exposed for effect bodies.
pub trait EffectSource: Send + Sync {
    fn load<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<Effect, AdapterError>>;
}
pub struct UnwiredEffectSource;
impl EffectSource for UnwiredEffectSource {
    fn load<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Effect, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}

pub struct EffectsSink {
    pub source: Arc<dyn EffectSource>,
    pub objects: Arc<dyn ObjectStore>,
    pub images: Arc<dyn ImageProcessor>,
    pub scanner: Arc<dyn MalwareScanner>,
    pub errors: Arc<dyn ErrorTracker>,
    pub analytics: Arc<dyn BasicAnalytics>,
}
impl Sink for EffectsSink {
    fn name(&self) -> &str {
        "integrations.effects"
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            event.validate().map_err(|_| SinkError::Permanent)?;
            if event.event_type != "integrations.requested" || event.version != 1 {
                return Err(SinkError::Permanent);
            }
            let reference = event
                .payload
                .reference("resource_id")
                .ok_or(SinkError::Permanent)?;
            match self.source.load(reference).await? {
                Effect::Store {
                    key,
                    bytes,
                    max_bytes,
                } => {
                    self.objects.put(&key, &bytes, max_bytes).await?;
                }
                Effect::Image { output, input } => {
                    if output.area() != ObjectArea::Processed {
                        return Err(SinkError::Permanent);
                    }
                    let max_bytes = input.max_bytes;
                    let result = self.images.process(input).await?;
                    self.objects.put(&output, &result.png, max_bytes).await?;
                }
                Effect::Scan { bytes, max_bytes } => {
                    let report = self
                        .scanner
                        .scan(&event.event_id, &bytes, max_bytes)
                        .await?;
                    if report.verdict == ScanVerdict::SimulatedRejected {
                        return Err(SinkError::Permanent);
                    }
                }
                Effect::Error(record) => self.errors.record(&event.event_id, &record).await?,
                Effect::Analytics(record) => {
                    self.analytics.record(&event.event_id, &record).await?
                }
            }
            Ok(())
        })
    }
}

/// Disabled adapter implements every boundary without performing I/O or returning success.
pub struct DisabledAdapter;

pub struct AdapterRuntime {
    pub objects: Arc<dyn ObjectStore>,
    pub images: Arc<dyn ImageProcessor>,
    pub scanner: Arc<dyn MalwareScanner>,
    pub errors: Arc<dyn ErrorTracker>,
    pub analytics: Arc<dyn BasicAnalytics>,
    pub mail: Arc<dyn crate::mail::MailProvider>,
}
impl AdapterRuntime {
    pub fn assemble(
        config: &AdapterConfig,
        environment: Environment,
    ) -> Result<Self, AdapterError> {
        config.validate(environment)?;
        match config.mode {
            AdapterMode::Disabled => {
                let disabled = Arc::new(DisabledAdapter);
                Ok(Self {
                    objects: disabled.clone(),
                    images: disabled.clone(),
                    scanner: disabled.clone(),
                    errors: disabled.clone(),
                    analytics: disabled,
                    mail: Arc::new(crate::mail::DisabledMailProvider),
                })
            }
            AdapterMode::LocalTest => {
                let root = local::LocalRoot::new(
                    std::path::Path::new(
                        config
                            .local_root
                            .as_deref()
                            .ok_or(AdapterError::InvalidInput)?,
                    ),
                    environment,
                )?;
                let adapters = Arc::new(local::LocalAdapters { root: root.clone() });
                Ok(Self {
                    objects: adapters.clone(),
                    images: adapters.clone(),
                    scanner: adapters.clone(),
                    errors: adapters.clone(),
                    analytics: adapters,
                    mail: Arc::new(crate::mail::local::LocalMailProvider { root }),
                })
            }
        }
    }
    pub fn effects_sink(&self, source: Arc<dyn EffectSource>) -> EffectsSink {
        EffectsSink {
            source,
            objects: self.objects.clone(),
            images: self.images.clone(),
            scanner: self.scanner.clone(),
            errors: self.errors.clone(),
            analytics: self.analytics.clone(),
        }
    }
}
impl ObjectStore for DisabledAdapter {
    fn put<'a>(
        &'a self,
        _: &'a ObjectKey,
        _: &'a [u8],
        _: u64,
    ) -> BoxFuture<'a, Result<ObjectReceipt, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
    fn read<'a>(
        &'a self,
        _: &'a ObjectKey,
        _: u64,
    ) -> BoxFuture<'a, Result<Vec<u8>, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
impl ImageProcessor for DisabledAdapter {
    fn process<'a>(&'a self, _: ImageInput) -> BoxFuture<'a, Result<ProcessedImage, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
impl MalwareScanner for DisabledAdapter {
    fn scan<'a>(
        &'a self,
        _: &'a str,
        _: &'a [u8],
        _: u64,
    ) -> BoxFuture<'a, Result<ScanReport, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
impl ErrorTracker for DisabledAdapter {
    fn record<'a>(
        &'a self,
        _: &'a str,
        _: &'a SafeDiagnostic,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
impl BasicAnalytics for DisabledAdapter {
    fn record<'a>(
        &'a self,
        _: &'a str,
        _: &'a SafeAggregate,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
