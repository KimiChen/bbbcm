//! `90` §6 吞吐门槛载体：按**固定速率**发压，经完整 HTTP 栈测量。
//!
//! 与 `crates/content/tests/postgres_performance.rs` 的区别必须说清楚，两者测的
//! 不是一件事：
//!
//! | | 延迟载体 | 本载体 |
//! |---|---|---|
//! | 发压方式 | 16 路背靠背（闭环） | 固定 200 req/s（开环） |
//! | 速率 | 由系统快慢决定，不受控 | 由载体决定，系统慢也不减速 |
//! | 被测层 | `ContentService` 领域读路径 | Axum handler + Askama 渲染 + HTTP 往返 |
//! | 时长 | 320 次请求，常不足 1 秒 | 固定秒数，跑到稳态 |
//!
//! 闭环测不出吞吐：系统一慢，请求自然变少，负载跟着塌下去，测出来的永远是
//! 「系统当前能做到的速率」而不是「能不能扛住目标速率」。
//!
//! **不引入 HTTP 客户端依赖**：手写 HTTP/1.1 GET + keep-alive 即可，被测端点都是
//! 简单 GET。多一个 registry 包要过供应链审查，不值当。
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 一次请求的观测。`scheduled` 是**计划发出**的时刻，不是实际发出的时刻。
struct Sample {
    /// 从计划发出到收到完整响应的耗时（毫秒）。
    latency_ms: f64,
    status: u16,
}

/// 建一条 keep-alive 连接。
async fn connect(addr: &str) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(addr).await?;
    // 关掉 Nagle：请求小且要求低延迟，攒包会给 p95 平白加上几十毫秒。
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// 在一条连接上发一次 GET 并读完响应，返回状态码。
///
/// 只解析到能判断响应边界为止：状态行取状态码，头里取 `Content-Length`
/// 或 `Transfer-Encoding: chunked`。不做完整 HTTP 实现——够判断「响应完整收到」
/// 即可，多写只会引入自己的 bug。
async fn get(stream: &mut TcpStream, host: &str, path: &str) -> std::io::Result<u16> {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\nAccept: text/html\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 8192];
    // 先读到头部结束。
    let header_end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "读头部时连接关闭",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let lower = head.to_ascii_lowercase();
    if let Some(at) = lower.find("content-length:") {
        let len: usize = lower[at + 15..]
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(0);
        while buf.len() < header_end + len {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    } else if lower.contains("transfer-encoding: chunked") {
        // chunked：读到结束块为止。
        while !buf.ends_with(b"0\r\n\r\n") {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }
    Ok(status)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = q * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        sorted[low]
    } else {
        sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64)
    }
}

/// 按固定速率对一个端点发压，返回（样本, 实际发出数, 错误数）。
async fn drive(
    addr: String,
    host: String,
    path: String,
    rate: usize,
    seconds: u64,
    connections: usize,
) -> (Vec<Sample>, u64, u64) {
    let total: u64 = rate as u64 * seconds;
    let interval = Duration::from_secs_f64(1.0 / rate as f64);
    let errors = Arc::new(AtomicU64::new(0));
    let issued = Arc::new(AtomicU64::new(0));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Sample>();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(connections));

    let start = Instant::now();
    let mut handles = Vec::new();
    for i in 0..total {
        // 计划发出的时刻由**序号**决定，不由上一次请求何时结束决定。
        // 这是开环的核心：系统变慢不会让发压变慢。
        let scheduled = start + interval.mul_f64(i as f64);
        let now = Instant::now();
        if scheduled > now {
            tokio::time::sleep(scheduled - now).await;
        }
        let (addr, host, path) = (addr.clone(), host.clone(), path.clone());
        let (tx, errors, issued) = (tx.clone(), errors.clone(), issued.clone());
        let semaphore = semaphore.clone();
        handles.push(tokio::spawn(async move {
            // 连接数上限即「客户端并发上限」。取不到许可说明系统已经慢到
            // 连接全被占住——这本身就是过载信号，不能悄悄排队掩盖。
            let Ok(_permit) = semaphore.acquire().await else {
                errors.fetch_add(1, Ordering::Relaxed);
                return;
            };
            issued.fetch_add(1, Ordering::Relaxed);
            let result = async {
                let mut stream = connect(&addr).await?;
                get(&mut stream, &host, &path).await
            }
            .await;
            match result {
                Ok(status) => {
                    // **从计划发出的时刻起算**，不是从实际发出起算。
                    // 后者会漏掉排队时间，正是 coordinated omission：系统越慢，
                    // 漏掉的越多，报出来的 p95 越好看。
                    let latency_ms = scheduled.elapsed().as_secs_f64() * 1000.0;
                    let _ = tx.send(Sample { latency_ms, status });
                }
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    drop(tx);
    for h in handles {
        let _ = h.await;
    }
    let mut rx = rx;
    let mut samples = Vec::new();
    while let Some(s) = rx.recv().await {
        samples.push(s);
    }
    (
        samples,
        issued.load(Ordering::Relaxed),
        errors.load(Ordering::Relaxed),
    )
}

/// `90` §6：公开读 100 read/s 起点（`DEC-071`）、公开读 p95 ≤400ms、SSR 首屏 p95 ≤800ms。
///
/// 需要一个已经跑起来、已接真实 PostgreSQL 的 `bbbcm-web`。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要运行中的 bbbcm-web 与真实 PostgreSQL；见 docs/development/postgres.md"]
async fn public_read_throughput_baseline() {
    let base = std::env::var("BBBCM_THROUGHPUT_BASE")
        .unwrap_or_else(|_| panic!("需要 BBBCM_THROUGHPUT_BASE，例如 127.0.0.1:18099"));
    // 默认取 `90` §6 的公开读起点。`DEC-071` 把它由 200 下调为 100——该目标落在
    // 实测饱和点（约 128 req/s）之内，因此**必须实测确认而不能由 128 > 100 推定
    // 通过**：饱和点附近的排队行为是非线性的。
    let rate = env_usize("BBBCM_THROUGHPUT_RATE", 100);
    let seconds = env_usize("BBBCM_THROUGHPUT_SECONDS", 60) as u64;
    let connections = env_usize("BBBCM_THROUGHPUT_CONNECTIONS", 200);
    let paths: Vec<String> = std::env::var("BBBCM_THROUGHPUT_PATHS")
        .unwrap_or_else(|_| "/,/new,/daily".to_owned())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    println!("\n===== WP-MVP-E02 公开读吞吐基线（开环）=====");
    println!("目标：{rate} req/s × {seconds}s；客户端连接上限 {connections}");
    println!("被测：完整 HTTP 栈（Axum handler + Askama 渲染），非领域读路径");
    println!("计时：自**计划发出**时刻起算，含排队时间（避免 coordinated omission）");

    let mut failed = Vec::new();
    for path in &paths {
        let started = Instant::now();
        let (samples, issued, errors) = drive(
            base.clone(),
            base.clone(),
            path.clone(),
            rate,
            seconds,
            connections,
        )
        .await;
        let elapsed = started.elapsed().as_secs_f64();
        let mut latencies: Vec<f64> = samples.iter().map(|s| s.latency_ms).collect();
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let ok = samples.iter().filter(|s| s.status == 200).count();
        let achieved = issued as f64 / elapsed;
        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let p99 = percentile(&latencies, 0.99);

        println!(
            "{path}: 发出 {issued} 实际 {achieved:.1} req/s（目标 {rate}），200 响应 {ok}，错误 {errors}\n  \
             p50={p50:.1}ms p95={p95:.1}ms p99={p99:.1}ms max={:.1}ms",
            latencies.last().copied().unwrap_or(f64::NAN)
        );

        // 防空：请求没真正发出去、或响应不是 200，延迟数字毫无意义。
        assert!(
            issued >= (rate as u64 * seconds) * 9 / 10,
            "{path} 只发出 {issued} 次，不足目标 {} 的九成——载体自身受限，数字不可用",
            rate as u64 * seconds
        );
        assert!(
            ok as f64 >= samples.len() as f64 * 0.99,
            "{path} 仅 {ok}/{} 为 200；非 200 响应通常更快，混进来会让 p95 虚低",
            samples.len()
        );
        if p95 > 400.0 {
            failed.push(format!("{path} p95 {p95:.1}ms"));
        }
    }
    assert!(
        failed.is_empty(),
        "以下路径超出 90 §6 的公开读 400ms p95 门槛：{}",
        failed.join("；")
    );
}
