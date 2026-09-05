//! Executable local processing; no network clients or public object URLs.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Private local spool. Files use opaque identifiers, restrictive modes and atomic
/// no-clobber publication; a replay must match the complete original bytes.
#[derive(Clone)]
pub struct LocalRoot(PathBuf);
impl std::fmt::Debug for LocalRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalRoot([path omitted])")
    }
}
impl LocalRoot {
    pub fn new(path: &Path, environment: Environment) -> Result<Self, AdapterError> {
        if environment.is_production_like() || path.as_os_str().is_empty() {
            return Err(AdapterError::InvalidInput);
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|_| AdapterError::Unavailable)?
                .join(path)
        };
        let mut current = PathBuf::new();
        for component in path.components() {
            match component {
                Component::ParentDir => return Err(AdapterError::InvalidInput),
                Component::Normal(name)
                    if matches!(
                        name.to_str(),
                        Some("static" | "templates" | "dist" | "public")
                    ) =>
                {
                    return Err(AdapterError::InvalidInput);
                }
                Component::CurDir => continue,
                _ => current.push(component),
            }
            match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                    return Err(AdapterError::InvalidInput);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&current).map_err(|_| AdapterError::Unavailable)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o700))
                            .map_err(|_| AdapterError::Unavailable)?;
                    }
                }
                Err(_) => return Err(AdapterError::Unavailable),
            }
        }
        Ok(Self(path))
    }

    fn location(&self, category: &str, id: &str, extension: &str) -> Result<PathBuf, AdapterError> {
        if !matches!(
            category,
            "quarantine" | "processed" | "scan" | "errors" | "analytics" | "mail"
        ) || !crate::transaction::validate_operation_id(id)
            || !matches!(extension, "bin" | "json" | "eml")
        {
            return Err(AdapterError::InvalidInput);
        }
        // Recheck the whole chain on each operation: checking only the leaf would
        // follow an ancestor replaced with a symlink after initial construction.
        let mut ancestor = PathBuf::new();
        for component in self.0.components() {
            ancestor.push(component);
            let meta =
                std::fs::symlink_metadata(&ancestor).map_err(|_| AdapterError::Unavailable)?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(AdapterError::InvalidInput);
            }
        }
        let dir = self.0.join(category);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(AdapterError::Unavailable),
        }
        let meta = std::fs::symlink_metadata(&dir).map_err(|_| AdapterError::Unavailable)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(AdapterError::InvalidInput);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| AdapterError::Unavailable)?;
        }
        Ok(dir.join(format!("{id}.{extension}")))
    }
    pub async fn write(
        &self,
        category: &str,
        id: &str,
        extension: &str,
        bytes: Vec<u8>,
    ) -> Result<(), AdapterError> {
        let root = self.clone();
        let category = category.to_owned();
        let id = id.to_owned();
        let extension = extension.to_owned();
        tokio::task::spawn_blocking(move || {
            let output = root.location(&category, &id, &extension)?;
            let temporary =
                output.with_extension(format!("{}.tmp", crate::transaction::new_operation_id()));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let result = (|| {
                let mut file = options
                    .open(&temporary)
                    .map_err(|_| AdapterError::Unavailable)?;
                file.write_all(&bytes)
                    .map_err(|_| AdapterError::Unavailable)?;
                file.sync_all().map_err(|_| AdapterError::Unavailable)?;
                match std::fs::hard_link(&temporary, &output) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = std::fs::symlink_metadata(&output)
                            .map_err(|_| AdapterError::Unavailable)?;
                        if !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() != bytes.len() as u64
                        {
                            return Err(AdapterError::Conflict);
                        }
                        let existing =
                            std::fs::read(&output).map_err(|_| AdapterError::Unavailable)?;
                        if existing == bytes {
                            Ok(())
                        } else {
                            Err(AdapterError::Conflict)
                        }
                    }
                    Err(_) => Err(AdapterError::Unavailable),
                }?;
                #[cfg(unix)]
                for directory in [
                    output.parent().ok_or(AdapterError::Unavailable)?,
                    root.0.as_path(),
                ] {
                    std::fs::File::open(directory)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| AdapterError::Unavailable)?;
                }
                Ok(())
            })();
            let _ = std::fs::remove_file(temporary);
            result
        })
        .await
        .map_err(|_| AdapterError::Unavailable)?
    }
    pub async fn read(
        &self,
        category: &str,
        id: &str,
        extension: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        if max_bytes == 0 || max_bytes == u64::MAX {
            return Err(AdapterError::InvalidInput);
        }
        let root = self.clone();
        let category = category.to_owned();
        let id = id.to_owned();
        let extension = extension.to_owned();
        tokio::task::spawn_blocking(move || {
            let path = root.location(&category, &id, &extension)?;
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AdapterError::NotFound
                } else {
                    AdapterError::Unavailable
                }
            })?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > max_bytes
            {
                return Err(AdapterError::InvalidInput);
            }
            let mut data = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| AdapterError::Unavailable)?
                .take(max_bytes + 1)
                .read_to_end(&mut data)
                .map_err(|_| AdapterError::Unavailable)?;
            if data.len() as u64 > max_bytes {
                return Err(AdapterError::InvalidInput);
            }
            Ok(data)
        })
        .await
        .map_err(|_| AdapterError::Unavailable)?
    }
}

#[derive(Clone)]
pub struct LocalAdapters {
    pub root: LocalRoot,
}
impl ObjectStore for LocalAdapters {
    fn put<'a>(
        &'a self,
        key: &'a ObjectKey,
        bytes: &'a [u8],
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<ObjectReceipt, AdapterError>> {
        Box::pin(async move {
            if bytes.is_empty() || max_bytes == 0 || bytes.len() as u64 > max_bytes {
                return Err(AdapterError::InvalidInput);
            }
            self.root
                .write(key.area().as_str(), key.id(), "bin", bytes.to_vec())
                .await?;
            Ok(ObjectReceipt {
                sha256: digest(bytes),
                bytes: bytes.len() as u64,
                local_only: true,
            })
        })
    }
    fn read<'a>(
        &'a self,
        key: &'a ObjectKey,
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<Vec<u8>, AdapterError>> {
        Box::pin(async move {
            self.root
                .read(key.area().as_str(), key.id(), "bin", max_bytes)
                .await
        })
    }
}
impl ImageProcessor for LocalAdapters {
    fn process<'a>(
        &'a self,
        input: ImageInput,
    ) -> BoxFuture<'a, Result<ProcessedImage, AdapterError>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                if input.bytes.is_empty()
                    || input.bytes.len() as u64 > input.max_bytes
                    || input.max_pixels == 0
                    || input.max_width == 0
                    || input.max_height == 0
                {
                    return Err(AdapterError::InvalidInput);
                }
                let format =
                    image::guess_format(&input.bytes).map_err(|_| AdapterError::InvalidInput)?;
                let expected = match input.mime {
                    ImageMime::Png => image::ImageFormat::Png,
                    ImageMime::Jpeg => image::ImageFormat::Jpeg,
                };
                if format != expected {
                    return Err(AdapterError::InvalidInput);
                }
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(u32::try_from(input.max_pixels).unwrap_or(u32::MAX));
                limits.max_image_height = limits.max_image_width;
                limits.max_alloc = Some(
                    input
                        .max_pixels
                        .checked_mul(8)
                        .ok_or(AdapterError::InvalidInput)?,
                );
                let mut reader = image::ImageReader::with_format(Cursor::new(&input.bytes), format);
                // into_dimensions constructs a decoder and may parse ancillary metadata.
                // Apply allocation limits before even this first header inspection.
                reader.limits(limits.clone());
                let (width, height) = reader
                    .into_dimensions()
                    .map_err(|_| AdapterError::InvalidInput)?;
                if width == 0
                    || height == 0
                    || u64::from(width) * u64::from(height) > input.max_pixels
                {
                    return Err(AdapterError::InvalidInput);
                }
                let mut reader = image::ImageReader::with_format(Cursor::new(&input.bytes), format);
                limits.max_image_width = Some(width);
                limits.max_image_height = Some(height);
                reader.limits(limits);
                // A fresh pixel buffer and encoder deliberately discard EXIF/ICC/text metadata.
                let decoded = reader.decode().map_err(|_| AdapterError::InvalidInput)?;
                // image::thumbnail may upscale: clamp its target before it allocates
                // any new pixels, preserving the validated source pixel budget.
                let target_width = input.max_width.min(width);
                let target_height = input.max_height.min(height);
                if u64::from(target_width) * u64::from(target_height) > input.max_pixels {
                    return Err(AdapterError::InvalidInput);
                }
                let clean = image::DynamicImage::ImageRgba8(
                    decoded.thumbnail(target_width, target_height).to_rgba8(),
                );
                let mut output = Cursor::new(Vec::new());
                clean
                    .write_to(&mut output, image::ImageFormat::Png)
                    .map_err(|_| AdapterError::InvalidInput)?;
                let png = output.into_inner();
                if png.len() as u64 > input.max_bytes {
                    return Err(AdapterError::InvalidInput);
                }
                Ok(ProcessedImage {
                    png,
                    width: clean.width(),
                    height: clean.height(),
                })
            })
            .await
            .map_err(|_| AdapterError::Unavailable)?
        })
    }
}
impl MalwareScanner for LocalAdapters {
    fn scan<'a>(
        &'a self,
        event_id: &'a str,
        bytes: &'a [u8],
        max_bytes: u64,
    ) -> BoxFuture<'a, Result<ScanReport, AdapterError>> {
        Box::pin(async move {
            if bytes.is_empty() || bytes.len() as u64 > max_bytes {
                return Err(AdapterError::InvalidInput);
            }
            let marker = b"BBBCM-SIMULATED-MALWARE";
            let report = ScanReport {
                sha256: digest(bytes),
                bytes: bytes.len() as u64,
                verdict: if bytes.windows(marker.len()).any(|w| w == marker) {
                    ScanVerdict::SimulatedRejected
                } else {
                    ScanVerdict::SimulatedClean
                },
                simulated: true,
            };
            self.root
                .write(
                    "scan",
                    event_id,
                    "json",
                    serde_json::to_vec(&report).map_err(|_| AdapterError::InvalidInput)?,
                )
                .await?;
            Ok(report)
        })
    }
}
impl ErrorTracker for LocalAdapters {
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        diagnostic: &'a SafeDiagnostic,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            self.root
                .write(
                    "errors",
                    event_id,
                    "json",
                    serde_json::to_vec(diagnostic).map_err(|_| AdapterError::InvalidInput)?,
                )
                .await
        })
    }
}
impl BasicAnalytics for LocalAdapters {
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        value: &'a SafeAggregate,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            self.root
                .write(
                    "analytics",
                    event_id,
                    "json",
                    serde_json::to_vec(value).map_err(|_| AdapterError::InvalidInput)?,
                )
                .await
        })
    }
}

#[cfg(test)]
mod tests;
