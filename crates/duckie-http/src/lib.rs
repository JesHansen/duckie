//! One-shot HTTP transport with pooled clients and independently bounded decoding.
use anyhow::{Context, Result, bail};
use duckie_model::*;
use futures_util::TryStreamExt;
use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant, SystemTime},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, ReadBuf};
use tokio_util::{
    io::{ReaderStream, StreamReader},
    sync::CancellationToken,
};

#[derive(Default)]
pub struct HttpEngine {
    clients: Mutex<VecDeque<(ProxyMode, reqwest::Client)>>,
}
impl HttpEngine {
    fn client(&self, proxy: &ProxyMode) -> Result<reqwest::Client> {
        let mut clients = self.clients.lock().unwrap();
        if let Some((_, client)) = clients.iter().find(|(key, _)| key == proxy) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .tls_backend_native()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .pool_idle_timeout(Duration::from_secs(60));
        match proxy {
            ProxyMode::System => {}
            ProxyMode::Direct => builder = builder.no_proxy(),
            ProxyMode::Explicit { url, bypass } => {
                let parsed = reqwest::Url::parse(url).context("Invalid proxy URL")?;
                if !parsed.username().is_empty() || parsed.password().is_some() {
                    bail!("Proxy credentials in URLs are unsupported in this preview");
                }
                builder = builder.proxy(
                    reqwest::Proxy::all(url)?.no_proxy(reqwest::NoProxy::from_string(bypass)),
                );
            }
        }
        let client = builder
            .build()
            .context("Cannot initialize Windows TLS transport")?;
        if clients.len() >= 4 {
            clients.pop_front();
        }
        clients.push_back((proxy.clone(), client.clone()));
        Ok(client)
    }
    pub async fn execute(
        &self,
        request: PreparedRequest,
        cancel: CancellationToken,
    ) -> Result<ExecutionResult> {
        self.execute_watched(request, cancel, Progress::default())
            .await
    }
    /// As `execute`, but reporting bytes as they arrive so a long download can be shown moving.
    pub async fn execute_watched(
        &self,
        request: PreparedRequest,
        cancel: CancellationToken,
        progress: Progress,
    ) -> Result<ExecutionResult> {
        let client = self.client(&request.proxy)?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .context("Invalid HTTP method")?;
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &request.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("Invalid header name"))?;
            let value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
                anyhow::anyhow!("Invalid header value; line breaks are not allowed")
            })?;
            headers.append(name, value);
        }
        // Compression is decoded below, never opaquely by reqwest.
        if !headers.contains_key("accept-encoding") {
            headers.insert(
                "accept-encoding",
                reqwest::header::HeaderValue::from_static("gzip, deflate, br, zstd"),
            );
        }
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(request.timeout_ms);
        let mut result = ExecutionResult {
            request_id: request.id,
            run_id: request.run_id,
            summary: request.summary,
            outcome: Outcome::Complete,
            status: None,
            status_text: String::new(),
            headers: vec![],
            body: BodyHandle::default(),
            encoded_bytes: 0,
            duration_ms: 0,
        };
        let build = async {
            let mut builder = client.request(method, &request.url).headers(headers);
            builder = match request.body {
                Body::None => builder,
                Body::Json { text } | Body::Text { text } => builder.body(text),
                Body::Form { rows } => builder.form(
                    &rows
                        .into_iter()
                        .filter(|r| r.enabled)
                        .map(|r| (r.name, r.value))
                        .collect::<Vec<_>>(),
                ),
                Body::File { path, .. } => {
                    let file = tokio::fs::File::open(path)
                        .await
                        .context("Cannot open selected body file")?;
                    let size = file.metadata().await?.len();
                    builder
                        .header("content-length", size)
                        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
                }
                Body::Multipart { parts } => {
                    let mut form = reqwest::multipart::Form::new();
                    for part in parts.into_iter().filter(|p| p.enabled) {
                        if part.file {
                            form = form
                                .file(part.name, part.value)
                                .await
                                .context("Cannot open multipart file")?;
                        } else {
                            form = form.text(part.name, part.value);
                        }
                    }
                    builder.multipart(form)
                }
            };
            builder.build().context("Cannot construct HTTP request")
        };
        let built = tokio::select! { _=cancel.cancelled()=>{result.outcome=Outcome::Cancelled;None}, _=tokio::time::sleep_until(deadline)=>{result.outcome=Outcome::Timeout;None}, v=build=>Some(v?) };
        let response = if let Some(built) = built {
            // Include generated protocol headers in the redacted effective summary.
            for (name, value) in built.headers() {
                if !result
                    .summary
                    .headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(name.as_str()))
                {
                    result.summary.headers.push((
                        name.to_string(),
                        value.to_str().unwrap_or("[binary]").into(),
                    ));
                }
            }
            tokio::select! {
                _=cancel.cancelled()=>{result.outcome=Outcome::Cancelled;None},
                _=tokio::time::sleep_until(deadline)=>{result.outcome=Outcome::Timeout;None},
                value=client.execute(built)=>match value {Ok(r)=>Some(r),Err(e)=> {result.outcome=classify(&e);None}}
            }
        } else {
            None
        };
        if let Some(response) = response {
            result.status = Some(response.status().as_u16());
            result.status_text = response.status().canonical_reason().unwrap_or("").into();
            result.headers = response
                .headers()
                .iter()
                .map(|(n, v)| {
                    (
                        n.to_string(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect();
            let encoding = response
                .headers()
                .get("content-encoding")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            // A declared length is of the encoded body, so it pairs with the encoded counter.
            progress
                .0
                .total
                .store(response.content_length().unwrap_or(0), Ordering::Relaxed);
            let limit_hit = Arc::new(AtomicBool::new(false));
            let stream = response.bytes_stream().map_err(io::Error::other);
            let reader = LimitedReader {
                inner: StreamReader::new(stream),
                count: progress.clone(),
                hit: limit_hit.clone(),
                limit: request.encoded_limit,
            };
            let buffered = BufReader::new(reader);
            use async_compression::tokio::bufread::{
                BrotliDecoder, GzipDecoder, ZlibDecoder, ZstdDecoder,
            };
            let mut reader: Pin<Box<dyn AsyncRead + Send>> = match encoding.as_str() {
                "" | "identity" => Box::pin(buffered),
                "gzip" => {
                    let mut d = GzipDecoder::new(buffered);
                    d.multiple_members(true);
                    Box::pin(d)
                }
                "deflate" => Box::pin(ZlibDecoder::new(buffered)),
                "br" => Box::pin(BrotliDecoder::new(buffered)),
                "zstd" => Box::pin(ZstdDecoder::new(buffered)),
                _ => {
                    result.outcome = Outcome::BodyError;
                    Box::pin(buffered)
                }
            };
            let mut capture = Capture::new();
            let mut chunk = vec![0u8; 64 * 1024];
            while result.outcome == Outcome::Complete {
                let remaining = request.decoded_limit.saturating_sub(capture.len);
                let take = (remaining.saturating_add(1)).min(chunk.len() as u64) as usize;
                let next = tokio::select! { _=cancel.cancelled()=>{result.outcome=Outcome::Cancelled;None}, _=tokio::time::sleep_until(deadline)=>{result.outcome=Outcome::Timeout;None}, read=reader.read(&mut chunk[..take])=>Some(read) };
                match next {
                    Some(Ok(0)) => break,
                    Some(Ok(n)) => {
                        let keep = (n as u64).min(remaining) as usize;
                        capture.write(&chunk[..keep]).await?;
                        progress.0.decoded.store(capture.len, Ordering::Relaxed);
                        if n > keep {
                            result.outcome = Outcome::DecodedLimit;
                        }
                    }
                    Some(Err(_)) => {
                        result.outcome = if limit_hit.load(Ordering::Relaxed) {
                            Outcome::EncodedLimit
                        } else {
                            Outcome::BodyError
                        }
                    }
                    None => break,
                }
            }
            result.encoded_bytes = progress.encoded();
            result.body = capture.finish().await?;
        }
        result.duration_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }
}
fn classify(error: &reqwest::Error) -> Outcome {
    if error.is_timeout() {
        return Outcome::Timeout;
    }
    // Inspect locally, but never expose error URLs (which may contain credentials).
    let description = format!("{error:?}").to_ascii_lowercase();
    if description.contains("certificate")
        || description.contains("tls")
        || description.contains("ssl")
    {
        Outcome::TlsError
    } else if description.contains("dns") || description.contains("resolve") {
        Outcome::DnsError
    } else {
        Outcome::ConnectionError
    }
}
struct LimitedReader<R> {
    inner: R,
    /// Counts encoded bytes, and is the same handle the caller watches for progress.
    count: Progress,
    hit: Arc<AtomicBool>,
    limit: u64,
}
impl<R: AsyncRead + Unpin> AsyncRead for LimitedReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let remaining = this.limit.saturating_sub(this.count.encoded());
        if remaining == 0 {
            let mut probe = [0u8; 1];
            let mut probe_buf = ReadBuf::new(&mut probe);
            match Pin::new(&mut this.inner).poll_read(cx, &mut probe_buf) {
                Poll::Ready(Ok(())) if !probe_buf.filled().is_empty() => {
                    this.hit.store(true, Ordering::Relaxed);
                    Poll::Ready(Err(io::Error::other("Encoded body limit reached")))
                }
                other => other,
            }
        } else {
            let take = buf.remaining().min(remaining as usize);
            let mut limited = ReadBuf::new(buf.initialize_unfilled_to(take));
            match Pin::new(&mut this.inner).poll_read(cx, &mut limited) {
                Poll::Ready(Ok(())) => {
                    let n = limited.filled().len();
                    buf.advance(n);
                    this.count.0.encoded.fetch_add(n as u64, Ordering::Relaxed);
                    Poll::Ready(Ok(()))
                }
                other => other,
            }
        }
    }
}
// LOCALAPPDATA inherits the current user's Windows profile ACL.
fn spool_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Duckie")
        .join("responses")
}
/// Removes response spool files left behind by a crash and returns bytes reclaimed.
///
/// Only files untouched for over 24 hours are removed. This is a safety property, not
/// just cleanup hygiene: Windows opens files with FILE_SHARE_DELETE, so deleting a spool
/// still open in another running Duckie instance would succeed rather than fail — we
/// cannot rely on the OS to protect a live file. A download in progress keeps writing its
/// spool, so it is never a day stale; anything older is safe to assume abandoned.
pub fn sweep_stale_spool() -> io::Result<u64> {
    sweep_dir(&spool_dir())
}
fn sweep_dir(root: &std::path::Path) -> io::Result<u64> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    let mut reclaimed = 0u64;
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("duckie-body-")
        {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if meta.modified().is_ok_and(|m| m < cutoff) && std::fs::remove_file(entry.path()).is_ok() {
            reclaimed += meta.len();
        }
    }
    Ok(reclaimed)
}
/// Bytes seen so far on a response body, shared with whoever started the request.
///
/// Cloning shares one set of counters; the reader updates them as chunks arrive and the caller
/// reads them from another thread, so both sides use relaxed atomics and neither blocks.
#[derive(Clone, Default)]
pub struct Progress(Arc<Counters>);
#[derive(Default)]
struct Counters {
    encoded: AtomicU64,
    decoded: AtomicU64,
    total: AtomicU64,
}
impl Progress {
    /// Bytes received on the wire, before any content decoding.
    pub fn encoded(&self) -> u64 {
        self.0.encoded.load(Ordering::Relaxed)
    }
    /// Bytes after decoding, which is what the response will hold.
    pub fn decoded(&self) -> u64 {
        self.0.decoded.load(Ordering::Relaxed)
    }
    /// The declared encoded length, or zero when the server did not give one.
    pub fn total(&self) -> u64 {
        self.0.total.load(Ordering::Relaxed)
    }
}
struct Capture {
    memory: Vec<u8>,
    file: Option<(tokio::fs::File, Arc<tempfile::TempPath>)>,
    len: u64,
}
impl Capture {
    fn new() -> Self {
        Self {
            memory: Vec::new(),
            file: None,
            len: 0,
        }
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if self.file.is_none() && self.len + bytes.len() as u64 > 2 * MIB {
            let root = spool_dir();
            tokio::fs::create_dir_all(&root).await?;
            let temp = tempfile::Builder::new()
                .prefix("duckie-body-")
                .tempfile_in(root)?;
            let (file, path) = temp.into_parts();
            let mut file = tokio::fs::File::from_std(file);
            file.write_all(&self.memory).await?;
            self.memory = Vec::new();
            self.file = Some((file, Arc::new(path)));
        }
        if let Some((file, _)) = &mut self.file {
            file.write_all(bytes).await?;
        } else {
            self.memory.extend_from_slice(bytes);
        }
        self.len += bytes.len() as u64;
        Ok(())
    }
    async fn finish(mut self) -> Result<BodyHandle> {
        if let Some((mut file, path)) = self.file.take() {
            file.flush().await?;
            drop(file);
            Ok(BodyHandle::File {
                path,
                len: self.len,
            })
        } else {
            Ok(BodyHandle::Memory(Arc::new(self.memory)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn server(response: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(&response).await;
        });
        format!("http://{address}")
    }
    fn prepared(url: String) -> PreparedRequest {
        prepare(
            &RequestDefinition {
                url,
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &EnvironmentSnapshot::default(),
            &RunBindings::default(),
            1,
        )
        .unwrap()
    }
    #[tokio::test]
    async fn progress_counts_bytes_and_records_a_declared_length() {
        const SIZE: usize = 300_000;
        let mut response =
            format!("HTTP/1.1 200 OK\r\ncontent-length: {SIZE}\r\n\r\n").into_bytes();
        response.extend(std::iter::repeat_n(b'x', SIZE));
        let url = server(response).await;
        let progress = Progress::default();
        // Before anything arrives the counters read zero rather than something invented.
        assert_eq!(
            (progress.encoded(), progress.decoded(), progress.total()),
            (0, 0, 0)
        );
        let result = HttpEngine::default()
            .execute_watched(prepared(url), CancellationToken::new(), progress.clone())
            .await
            .unwrap();
        assert_eq!(result.outcome, Outcome::Complete);
        assert_eq!(progress.encoded(), SIZE as u64);
        assert_eq!(progress.decoded(), SIZE as u64);
        assert_eq!(
            progress.total(),
            SIZE as u64,
            "the declared length is reported"
        );
        assert_eq!(progress.encoded(), result.encoded_bytes);
    }
    #[tokio::test]
    async fn error_status_is_complete_and_duplicate_headers_survive() {
        let url=server(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\nX-Value: a\r\nX-Value: b\r\n\r\n{}".to_vec()).await;
        let r = HttpEngine::default()
            .execute(prepared(url), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.status, Some(500));
        assert_eq!(r.outcome, Outcome::Complete);
        assert_eq!(r.headers.iter().filter(|(n, _)| n == "x-value").count(), 2);
    }
    #[tokio::test]
    async fn redirects_are_never_followed() {
        let url=server(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/must-not-call\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
        let r = HttpEngine::default()
            .execute(prepared(url), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.status, Some(302));
    }
    #[tokio::test]
    async fn body_limits_keep_partial_bytes() {
        let url = server(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n12345678".to_vec()).await;
        let mut req = prepared(url);
        req.decoded_limit = 4;
        let r = HttpEngine::default()
            .execute(req, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::DecodedLimit);
        assert_eq!(r.body.read(0, 100).unwrap(), b"1234");
        let url = server(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n12345678".to_vec()).await;
        let mut req = prepared(url);
        req.encoded_limit = 4;
        let r = HttpEngine::default()
            .execute(req, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::EncodedLimit);
        assert_eq!(r.encoded_bytes, 4);
    }
    #[tokio::test]
    async fn cancellation_and_timeout_have_no_invented_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut req = prepared(format!("http://{}", listener.local_addr().unwrap()));
        req.timeout_ms = 20;
        let r = HttpEngine::default()
            .execute(req, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::Timeout);
        assert_eq!(r.status, None);
        let token = CancellationToken::new();
        token.cancel();
        let r = HttpEngine::default()
            .execute(prepared("http://127.0.0.1:1".into()), token)
            .await
            .unwrap();
        assert_eq!(r.outcome, Outcome::Cancelled);
    }
    #[tokio::test]
    async fn large_bodies_spill_and_delete_on_release() {
        let mut capture = Capture::new();
        capture
            .write(&vec![42; 2 * MIB as usize + 1])
            .await
            .unwrap();
        let body = capture.finish().await.unwrap();
        let path = match &body {
            BodyHandle::File { path, .. } => path.to_path_buf(),
            _ => panic!("must spill"),
        };
        assert!(path.exists());
        assert_eq!(body.read(2 * MIB, 1).unwrap(), vec![42]);
        drop(body);
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn compressed_expansion_is_bounded_before_full_allocation() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&vec![b'a'; 4 * MIB as usize]).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            compressed.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&compressed);
        let mut request = prepared(server(bytes).await);
        request.decoded_limit = 128 * 1024;
        let result = HttpEngine::default()
            .execute(request, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.outcome, Outcome::DecodedLimit);
        assert_eq!(result.body.len(), 128 * 1024);
        assert!(result.encoded_bytes < 10 * 1024);
    }
    #[tokio::test]
    async fn sends_auth_duplicate_query_and_authored_body_exactly_once() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 2048];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.ends_with(b"{ \"ok\": true }\n") || n == 0 {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let mut req = RequestDefinition {
            url: format!("http://{address}"),
            method: "POST".into(),
            proxy: ProxyMode::Direct,
            body: Body::Json {
                text: "{ \"ok\": true }\n".into(),
            },
            ..Default::default()
        };
        req.query = vec![Row::new("x", "a b"), Row::new("x", "c")];
        req.auth.bearer = Some(SecretBinding {
            secret: "token".into(),
        });
        let env = EnvironmentSnapshot {
            secrets: Values::from([("token".into(), "Bearer test-token".into())]),
            ..Default::default()
        };
        let result = HttpEngine::default()
            .execute(
                prepare(&req, &env, &RunBindings::default(), 7).unwrap(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.status, Some(201));
        let captured = server.await.unwrap();
        assert!(captured.starts_with("POST /?x=a+b&x=c HTTP/1.1"));
        assert!(captured.contains("authorization: Bearer test-token\r\n"));
        assert!(captured.ends_with("{ \"ok\": true }\n"));
    }
    #[test]
    fn sweep_removes_only_stale_spool_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("duckie-body-old");
        let fresh = dir.path().join("duckie-body-fresh");
        let ignored = dir.path().join("not-a-spool-file");
        std::fs::write(&old, [0u8; 10]).unwrap();
        std::fs::write(&fresh, [0u8; 3]).unwrap();
        std::fs::write(&ignored, [0u8; 7]).unwrap();
        let day_ago = SystemTime::now() - Duration::from_secs(25 * 60 * 60);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(day_ago)
            .unwrap();
        let reclaimed = sweep_dir(dir.path()).unwrap();
        assert_eq!(reclaimed, 10);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(ignored.exists());
    }
    #[test]
    fn sweep_of_missing_dir_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sweep_dir(&dir.path().join("gone")).unwrap(), 0);
    }
}
