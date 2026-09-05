use super::*;

fn fixture() -> (PathBuf, LocalAdapters) {
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "bbbcm-adapters-{}",
        crate::transaction::new_operation_id()
    ));
    let root = LocalRoot::new(&dir, Environment::Test).unwrap();
    (dir, LocalAdapters { root })
}
fn input() -> ImageInput {
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        8,
        image::Rgba([200, 20, 30, 255]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .unwrap();
    ImageInput {
        bytes: encoded.into_inner(),
        mime: ImageMime::Png,
        max_bytes: 100_000,
        max_pixels: 128,
        max_width: 8,
        max_height: 8,
    }
}
fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut crc = !0_u32;
    for byte in &chunk[4..] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb88320
            } else {
                crc >> 1
            };
        }
    }
    chunk.extend_from_slice(&(!crc).to_be_bytes());
    chunk
}
#[tokio::test]
async fn objects_are_private_bounded_durable_and_idempotent() {
    let (dir, local) = fixture();
    let id = crate::transaction::new_operation_id();
    let key = ObjectKey::new(ObjectArea::Quarantine, &id).unwrap();
    let receipt = local.put(&key, b"synthetic bytes", 100).await.unwrap();
    assert!(receipt.local_only);
    assert_eq!(receipt.sha256, digest(b"synthetic bytes"));
    assert_eq!(local.read(&key, 100).await.unwrap(), b"synthetic bytes");
    assert_eq!(
        std::fs::read(dir.join("quarantine").join(format!("{id}.bin"))).unwrap(),
        b"synthetic bytes"
    );
    local.put(&key, b"synthetic bytes", 100).await.unwrap();
    assert_eq!(
        local.put(&key, b"changed bytes", 100).await,
        Err(AdapterError::Conflict)
    );
    assert_eq!(local.read(&key, 3).await, Err(AdapterError::InvalidInput));
    assert!(ObjectKey::new(ObjectArea::Quarantine, "../secret").is_err());
    let missing = ObjectKey::new(
        ObjectArea::Processed,
        &crate::transaction::new_operation_id(),
    )
    .unwrap();
    assert_eq!(local.read(&missing, 100).await, Err(AdapterError::NotFound));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.join("quarantine").join(format!("{id}.bin")))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn image_decodes_resizes_reencodes_and_rejects_bad_mime_budget() {
    let (dir, local) = fixture();
    let result = local.process(input()).await.unwrap();
    assert_eq!((result.width, result.height), (8, 4));
    assert_eq!(image::load_from_memory(&result.png).unwrap().width(), 8);
    let mut large_target = input();
    large_target.max_width = 160;
    large_target.max_height = 80;
    let bounded = local.process(large_target).await.unwrap();
    assert_eq!(
        (bounded.width, bounded.height),
        (16, 8),
        "large output limits must never upscale beyond the input pixel budget"
    );
    let mut oversized = input();
    oversized.max_pixels = 127;
    assert!(matches!(
        local.process(oversized).await,
        Err(AdapterError::InvalidInput)
    ));
    let mut mismatch = input();
    mismatch.mime = ImageMime::Jpeg;
    assert!(matches!(
        local.process(mismatch).await,
        Err(AdapterError::InvalidInput)
    ));
    let mut corrupt = input();
    corrupt.bytes = b"not a PNG".to_vec();
    assert!(matches!(
        local.process(corrupt).await,
        Err(AdapterError::InvalidInput)
    ));
    let mut jpeg = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(16, 8, image::Rgb([10, 20, 30])))
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    let jpeg = jpeg.into_inner();
    let metadata = b"Exif\0\0SYNTHETIC-GPS-METADATA";
    let mut with_metadata = jpeg[..2].to_vec();
    with_metadata.extend_from_slice(&[0xff, 0xe1]);
    with_metadata.extend_from_slice(&u16::try_from(metadata.len() + 2).unwrap().to_be_bytes());
    with_metadata.extend_from_slice(metadata);
    with_metadata.extend_from_slice(&jpeg[2..]);
    let mut jpeg_input = input();
    jpeg_input.mime = ImageMime::Jpeg;
    jpeg_input.bytes = with_metadata;
    let clean = local.process(jpeg_input).await.unwrap();
    assert!(
        !clean
            .png
            .windows(metadata.len())
            .any(|part| part == metadata)
    );
    assert_eq!(
        image::guess_format(&clean.png).unwrap(),
        image::ImageFormat::Png
    );
    let mut metadata_input = input();
    // A valid PNG with tiny pixels and large ancillary text must not get the
    // decoder's default metadata allocation budget during dimension probing.
    let mut payload = b"Comment\0".to_vec();
    payload.extend(vec![b'a'; 8192]);
    let metadata = png_chunk(b"tEXt", &payload);
    let mut png = metadata_input.bytes[..33].to_vec();
    png.extend(metadata);
    png.extend_from_slice(&metadata_input.bytes[33..]);
    metadata_input.bytes = png;
    assert!(matches!(
        local.process(metadata_input).await,
        Err(AdapterError::InvalidInput)
    ));
    std::fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn scanner_and_telemetry_have_actual_inspectable_results() {
    let (dir, local) = fixture();
    let clean_id = crate::transaction::new_operation_id();
    let bad_id = crate::transaction::new_operation_id();
    let clean = local
        .scan(&clean_id, b"safe synthetic fixture", 100)
        .await
        .unwrap();
    let rejected = local
        .scan(&bad_id, b"BBBCM-SIMULATED-MALWARE", 100)
        .await
        .unwrap();
    assert_eq!(clean.verdict, ScanVerdict::SimulatedClean);
    assert!(clean.simulated);
    assert_eq!(rejected.verdict, ScanVerdict::SimulatedRejected);
    assert!(rejected.simulated);
    let saved = local
        .root
        .read("scan", &bad_id, "json", 10_000)
        .await
        .unwrap();
    assert!(
        String::from_utf8(saved)
            .unwrap()
            .contains("simulated_rejected")
    );
    let error_id = crate::transaction::new_operation_id();
    ErrorTracker::record(
        &local,
        &error_id,
        &SafeDiagnostic {
            dependency: DependencyKind::Mail,
            code: DiagnosticCode::Unavailable,
        },
    )
    .await
    .unwrap();
    let raw = local
        .root
        .read("errors", &error_id, "json", 1000)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["code"],
        "unavailable"
    );
    let analytic_id = crate::transaction::new_operation_id();
    BasicAnalytics::record(
        &local,
        &analytic_id,
        &SafeAggregate {
            kind: AggregateKind::Reads,
            count: 7,
        },
    )
    .await
    .unwrap();
    let raw = local
        .root
        .read("analytics", &analytic_id, "json", 1000)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["count"],
        7
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn disabled_unavailable_and_symlink_boundaries_do_not_succeed() {
    let id = crate::transaction::new_operation_id();
    let key = ObjectKey::new(ObjectArea::Quarantine, &id).unwrap();
    let runtime =
        AdapterRuntime::assemble(&AdapterConfig::default(), Environment::Production).unwrap();
    assert_eq!(
        runtime.objects.put(&key, b"data", 10).await,
        Err(AdapterError::Disabled)
    );
    assert!(matches!(
        runtime.images.process(input()).await,
        Err(AdapterError::Disabled)
    ));
    assert_eq!(
        runtime.scanner.scan(&id, b"data", 10).await,
        Err(AdapterError::Disabled)
    );
    assert_eq!(
        runtime
            .errors
            .record(
                &id,
                &SafeDiagnostic {
                    dependency: DependencyKind::Media,
                    code: DiagnosticCode::Unavailable
                }
            )
            .await,
        Err(AdapterError::Disabled)
    );
    assert_eq!(
        runtime
            .analytics
            .record(
                &id,
                &SafeAggregate {
                    kind: AggregateKind::Reads,
                    count: 0
                }
            )
            .await,
        Err(AdapterError::Disabled)
    );
    assert!(
        AdapterRuntime::assemble(
            &AdapterConfig {
                mode: AdapterMode::LocalTest,
                local_root: Some("data/adapters".into())
            },
            Environment::Production
        )
        .is_err()
    );
    let (dir, local) = fixture();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        local.put(&key, b"data", 10).await,
        Err(AdapterError::Unavailable)
    );
    #[cfg(unix)]
    {
        let (dir, local) = fixture();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("quarantine")).unwrap();
        assert_eq!(
            local.put(&key, b"data", 10).await,
            Err(AdapterError::InvalidInput)
        );
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    {
        let parent = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "bbbcm-ancestor-{}",
            crate::transaction::new_operation_id()
        ));
        let nested = parent.join("original/root");
        let root = LocalRoot::new(&nested, Environment::Test).unwrap();
        let elsewhere = parent.join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("root")).unwrap();
        std::fs::rename(parent.join("original"), parent.join("saved")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, parent.join("original")).unwrap();
        assert_eq!(
            root.write("quarantine", &id, "bin", b"data".to_vec()).await,
            Err(AdapterError::InvalidInput)
        );
        assert!(!elsewhere.join("root/quarantine").exists());
        std::fs::remove_dir_all(parent).unwrap();
    }
}
