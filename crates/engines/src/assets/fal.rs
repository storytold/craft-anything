//! Game assets from fal.ai over plain HTTPS: pictures (textures, skies, concept art)
//! and game-ready 3D models with Tripo P1 (a picture in, a low-poly GLB model out, its face count
//! capped). One key (`Authorization: Key …`), paid per generation; every call reports what it cost.
//! Which models and what they cost come from the asset table (`assets.toml`).

use super::{AssetProvider, AssetRequest, Generated, Shape, SizeBy};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::retry;
use serde_json::{Value, json};
use std::io::Read;
use std::time::Duration;

/// The provider id the asset table names.
pub const ID: &str = "fal";
const BASE: &str = "https://fal.run";
/// Largest download we keep (a textured GLB is a few MB).
const MAX_BYTES: u64 = 200 << 20;

/// How fal is told a picture's shape: the field and its value for the model.
fn fal_size(by: SizeBy, shape: Shape) -> (&'static str, &'static str) {
    match (by, shape) {
        (SizeBy::ImageSize, Shape::Square) => ("image_size", "square_hd"),
        (SizeBy::ImageSize, Shape::Wide) => ("image_size", "landscape_16_9"),
        (SizeBy::ImageSize, Shape::Tall) => ("image_size", "portrait_16_9"),
        (SizeBy::AspectRatio, Shape::Square) => ("aspect_ratio", "1:1"),
        (SizeBy::AspectRatio, Shape::Wide) => ("aspect_ratio", "16:9"),
        (SizeBy::AspectRatio, Shape::Tall) => ("aspect_ratio", "9:16"),
    }
}

/// fal.ai, as an [`AssetProvider`].
pub struct Fal {
    base: String,
}

impl Default for Fal {
    fn default() -> Fal {
        Fal { base: BASE.to_string() }
    }
}

impl AssetProvider for Fal {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "fal.ai"
    }

    fn keys_page(&self) -> Option<&'static str> {
        Some("https://fal.ai/dashboard/keys")
    }

    fn key_env(&self) -> Option<&'static str> {
        Some("FAL_KEY")
    }

    fn generate(&self, key: &str, req: &AssetRequest) -> Result<Generated, ToolError> {
        let key = key.trim();
        match *req {
            AssetRequest::Image { model, prompt, shape } => {
                let (field, size) = fal_size(model.size_by, shape);
                let body = json!({"prompt": prompt, field: size, "num_images": 1, "output_format": "png"});
                let v = self.post(key, &model.endpoint, &body)?;
                if v.pointer("/has_nsfw_concepts/0").and_then(Value::as_bool) == Some(true) {
                    return Err(
                        ToolError::new(ErrorCode::ProgramFailed, "fal.ai's safety check blocked that picture.").with_remediation("Describe it differently.")
                    );
                }
                let url = image_url(&v).ok_or_else(|| unexpected("picture", &v))?;
                Ok(Generated { bytes: download(&url)?, url, cost_usd: req.listed_cost() })
            }
            AssetRequest::Model3d { model, image_url, textured, faces } => {
                let range = model.faces();
                let faces = faces.clamp(*range.start(), *range.end());
                let v = self.post(key, &model.endpoint, &json!({"image_url": image_url, "texture": textured, "face_limit": faces}))?;
                let url = model_url(&v).ok_or_else(|| unexpected("3D model", &v))?;
                Ok(Generated { bytes: download(&url)?, url, cost_usd: req.listed_cost() })
            }
        }
    }
}

impl Fal {
    /// Send one paid generation and return fal's answer. Redirects are not followed: a redirected
    /// paid POST is ambiguous, so it's reported as possibly billed rather than sent again.
    fn post(&self, key: &str, model: &str, body: &Value) -> Result<Value, ToolError> {
        let url = format!("{}/{model}", self.base);
        // A 3D model takes a minute or two; fal answers once it's done.
        let resp = agent(Duration::from_secs(10 * 60))
            .post(&url)
            .config()
            .max_redirects(0)
            .build()
            .header("Authorization", &format!("Key {key}"))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(Fal::post_error)?;
        let status = resp.status().as_u16();
        let body = resp.into_body().read_to_string();
        if (200..=299).contains(&status) {
            let text = body.map_err(Fal::answer_error)?;
            return serde_json::from_str(&text).map_err(|e| {
                let mut err = ToolError::new(
                    ErrorCode::GarbledOutput,
                    format!("fal.ai accepted the generation but its answer couldn't be read: {e}. The generation may already be running and billed."),
                )
                .with_remediation("Check the generation in the fal.ai dashboard before trying again: sending it again may charge you twice.");
                err.retryable = false;
                err
            });
        }
        let text = body.unwrap_or_default();
        let detail: String = text.chars().take(300).collect();
        match status {
            401 | 403 => Err(ToolError::new(ErrorCode::BadSettings, "fal.ai didn't accept the key.")
                .with_remediation("Check the fal.ai key in Settings → AI → Game assets.")),
            402 => Err(ToolError::new(ErrorCode::OverLimit, "The fal.ai account is out of credit.").with_remediation("Add credit at fal.ai, then try again.")),
            404 => Err(ToolError::new(ErrorCode::BadSettings, format!("fal.ai didn't find the model ({status}): {detail}"))
                .with_remediation("Check the model in Settings → AI → Game assets.")),
            429 => Err(ToolError::new(ErrorCode::OverLimit, format!("fal.ai is rate limiting the generation ({status}): {detail}"))
                .with_remediation("Wait a moment, then try again.")
                .with_details(json!({"status": 429}))),
            300..=399 => Err(Fal::billed_status_error(ErrorCode::ProgramFailed, format!("fal.ai redirected the generation request ({status}): {detail}"))),
            408 | 504 => Err(Fal::billed_status_error(ErrorCode::TimedOut, format!("fal.ai didn't answer in time ({status}): {detail}"))),
            500..=599 => Err(Fal::billed_status_error(ErrorCode::EngineMissing, format!("fal.ai failed while answering ({status}): {detail}"))),
            _ => Err(ToolError::new(ErrorCode::BadInput, format!("fal.ai rejected the generation ({status}): {detail}"))
                .with_remediation("Check the prompt and model, then try again.")),
        }
    }

    /// A status that may mean fal took the generation before failing or redirecting: don't invite a
    /// retry that could charge twice.
    fn billed_status_error(code: ErrorCode, message: String) -> ToolError {
        let mut err = ToolError::new(code, format!("{message} The generation may already be running and billed."))
            .with_remediation("Check the generation in the fal.ai dashboard before trying again: sending it again may charge you twice.");
        err.retryable = false;
        err
    }

    /// The failure of one paid generation POST. Only a failure before anything was sent is a
    /// reachability problem. Anything later, from a reset or TLS error to a timeout or a signal,
    /// may have reached fal and started a generation: say it may be billed, and don't invite a
    /// second charge.
    fn post_error(e: ureq::Error) -> ToolError {
        let before_sent = match &e {
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => true,
            ureq::Error::Timeout(ureq::Timeout::Connect | ureq::Timeout::Resolve) => true,
            ureq::Error::Io(io) => matches!(io.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound),
            _ => false,
        };
        if before_sent {
            return ToolError::new(ErrorCode::EngineMissing, format!("Couldn't reach fal.ai: {e}"))
                .with_remediation("Check your internet connection and try again.");
        }
        let (code, what) = match &e {
            ureq::Error::Timeout(_) => (ErrorCode::TimedOut, "fal.ai didn't answer in time"),
            ureq::Error::Io(io) if retry::interrupted(io) => (ErrorCode::ProgramFailed, "The call to fal.ai was interrupted before it answered"),
            _ => (ErrorCode::ProgramFailed, "The call to fal.ai failed before it answered"),
        };
        let mut err = ToolError::new(code, format!("{what}: {e}. The generation may already be running and billed."))
            .with_remediation("Check the generation in the fal.ai dashboard before trying again: sending it again may charge you twice.");
        err.retryable = false;
        err
    }

    /// A failure reading fal's answer after it accepted the generation: it may already have run and
    /// been billed, whatever cut the answer short.
    fn answer_error(e: ureq::Error) -> ToolError {
        let code = if matches!(&e, ureq::Error::Timeout(_)) { ErrorCode::TimedOut } else { ErrorCode::ProgramFailed };
        let mut err = ToolError::new(
            code,
            format!("fal.ai accepted the generation but its answer couldn't be read: {e}. The generation may already be running and billed."),
        )
        .with_remediation("Check the generation in the fal.ai dashboard before trying again: sending it again may charge you twice.");
        err.retryable = false;
        err
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent("CraftAnything")
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(Duration::from_secs(10 * 60)))
        .build()
        .new_agent()
}

/// Fetch a generated file (fal answers with its address, or a `data:` URI in sync mode).
fn download(url: &str) -> Result<Vec<u8>, ToolError> {
    if let Some(data) = url.strip_prefix("data:") {
        let b64 = data
            .split_once(";base64,")
            .map(|(_, b)| b)
            .ok_or_else(|| paid_file_error(ErrorCode::GarbledOutput, None, "fal.ai sent the generated file in a form we couldn't read".to_string()))?;
        return base64_decode(b64).map_err(|e| paid_file_error(ErrorCode::GarbledOutput, None, format!("The generated file couldn't be read: {}", e.message)));
    }
    let resp = agent(Duration::from_secs(120))
        .get(url)
        .call()
        .map_err(|e| paid_file_error(ErrorCode::EngineMissing, Some(url), format!("The generated file couldn't be downloaded: {e}")))?;
    if !resp.status().is_success() {
        return Err(paid_file_error(ErrorCode::EngineMissing, Some(url), format!("Downloading the generated file failed ({}).", resp.status().as_u16())));
    }
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| paid_file_error(ErrorCode::EngineMissing, Some(url), format!("The download of the generated file broke off: {e}")))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(paid_file_error(ErrorCode::OverLimit, Some(url), "The generated file is larger than 200 MB and wasn't downloaded.".to_string()));
    }
    Ok(bytes)
}

/// A generated file that couldn't be fetched or read. fal already made it, so retrying would pay
/// for the generation again: give its address when there is one, and don't invite another.
fn paid_file_error(code: ErrorCode, url: Option<&str>, message: String) -> ToolError {
    let mut err = ToolError::new(code, format!("{}. The generation was made and paid for.", message.trim_end_matches('.')));
    err.retryable = false;
    if let Some(url) = url {
        err = err
            .with_remediation(format!("The generated file is at {url}: open that address to download it, or check your connection."))
            .with_details(json!({"url": url}));
    } else {
        err = err.with_remediation("Check the generation in the fal.ai dashboard before making it again: sending it again may charge you twice.");
    }
    err
}

fn image_url(v: &Value) -> Option<String> {
    v.pointer("/images/0/url").and_then(Value::as_str).map(str::to_string)
}

/// The GLB: Tripo lists it under `model_urls`; its main `model_mesh` can be an FBX instead.
fn model_url(v: &Value) -> Option<String> {
    let glb = v.pointer("/model_urls/glb/url").and_then(Value::as_str);
    let mesh = v.get("model_mesh").filter(|m| {
        let named = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
        named("file_name").ends_with(".glb") || named("content_type") == "model/gltf-binary"
    });
    glb.or_else(|| mesh.and_then(|m| m.get("url")?.as_str())).filter(|u| !u.is_empty()).map(str::to_string)
}

fn unexpected(what: &str, v: &Value) -> ToolError {
    ToolError::new(ErrorCode::GarbledOutput, format!("fal.ai didn't return a {what}."))
        .with_details(json!({"answer": v.to_string().chars().take(500).collect::<String>()}))
}

/// A `data:` URI for a local picture (fal accepts these for some models; links always work).
pub fn data_uri(bytes: &[u8], media_type: &str) -> String {
    format!("data:{media_type};base64,{}", base64_encode(bytes))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ToolError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|e| ToolError::new(ErrorCode::GarbledOutput, format!("invalid base64: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Catalogue;
    use craft_anything_platform::sync::Mutex;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A tiny local server that answers each request in turn and records what it was sent.
    fn serve(answers: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let (mut stream, _) = l.accept().unwrap();
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    head.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut req_body = vec![0; len];
                r.read_exact(&mut req_body).unwrap();
                s2.lock().unwrap().push(format!("{head}{}", String::from_utf8_lossy(&req_body)));
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    .unwrap();
            }
        });
        (base, seen)
    }

    /// A picture with FLUX.1 [dev].
    fn image(fal: &Fal, key: &str, prompt: &str, shape: Shape) -> Result<Generated, ToolError> {
        let c = Catalogue::built_in();
        fal.generate(key, &AssetRequest::Image { model: c.image_model(Some("flux-dev"))?, prompt, shape })
    }

    fn model_3d(fal: &Fal, key: &str, image_url: &str, textured: bool, faces: u32) -> Result<Generated, ToolError> {
        let c = Catalogue::fallback();
        fal.generate(key, &AssetRequest::Model3d { model: c.model_3d(None)?, image_url, textured, faces })
    }

    #[test]
    fn a_picture_then_a_model_from_its_address() {
        let png = base64_encode(b"\x89PNG fake");
        let glb = base64_encode(b"glTF fake");
        let (base, seen) = serve(vec![
            (200, json!({"images": [{"url": format!("data:image/png;base64,{png}"), "content_type": "image/png"}], "has_nsfw_concepts": [false]}).to_string()),
            (200, json!({"model_mesh": {"url": "https://x/m.fbx", "file_name": "m.fbx"}, "model_urls": {"glb": {"url": format!("data:model/gltf-binary;base64,{glb}")}}}).to_string()),
        ]);
        let fal = Fal { base };
        let pic = image(&fal, " k-123 ", "a wooden crate, game asset", Shape::Square).unwrap();
        assert_eq!(pic.bytes, b"\x89PNG fake");
        assert!((pic.cost_usd - 0.0262144).abs() < 1e-9, "1024×1024 at $0.025 per megapixel");
        let model = model_3d(&fal, "k-123", "https://v3.fal.media/files/crate.png", true, 2000).unwrap();
        assert_eq!(model.bytes, b"glTF fake", "the GLB, not the FBX");
        assert!((model.cost_usd - 0.50).abs() < 1e-9, "a textured Tripo P1 model");
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /fal-ai/flux/dev "), "{}", seen[0]);
        // Header names aren't case-sensitive; the key goes in fal's "Key …" form.
        assert!(seen[0].to_ascii_lowercase().contains("authorization: key k-123"), "{}", seen[0]);
        assert!(seen[0].contains("\"image_size\":\"square_hd\"") && seen[0].contains("\"output_format\":\"png\""));
        assert!(
            seen[1].starts_with("POST /tripo3d/p1/image-to-3d ") && seen[1].contains("\"texture\":true") && seen[1].contains("\"face_limit\":2000"),
            "{}",
            seen[1]
        );
        assert!(seen[1].contains("\"image_url\":\"https://v3.fal.media/files/crate.png\""));
    }

    #[test]
    fn nano_banana_pro_is_asked_for_an_aspect_ratio() {
        let png = base64_encode(b"PNG fake");
        let (base, seen) = serve(vec![(200, json!({"images": [{"url": format!("data:image/png;base64,{png}")}], "description": ""}).to_string())]);
        let c = Catalogue::built_in();
        let req = AssetRequest::Image { model: c.image_model(None).unwrap(), prompt: "a night sky", shape: Shape::Wide };
        let pic = Fal { base }.generate("k", &req).unwrap();
        assert_eq!((pic.bytes.as_slice(), pic.cost_usd), (&b"PNG fake"[..], 0.15));
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /fal-ai/nano-banana-pro "), "{}", seen[0]);
        assert!(seen[0].contains("\"aspect_ratio\":\"16:9\"") && !seen[0].contains("image_size"), "{}", seen[0]);
    }

    #[test]
    fn only_a_glb_is_taken_as_the_model() {
        let fbx_only = json!({"model_mesh": {"url": "https://x/m.fbx", "file_name": "m.fbx"}, "model_urls": {}});
        assert_eq!(model_url(&fbx_only), None);
        let glb_mesh = json!({"model_mesh": {"url": "https://x/m.glb", "file_name": "m.glb"}});
        assert_eq!(model_url(&glb_mesh).as_deref(), Some("https://x/m.glb"));
    }

    /// Only a failure before anything was sent says fal couldn't be reached. Every later failure
    /// may have reached fal and started a paid generation, whatever cut it off.
    #[test]
    fn only_a_failure_before_sending_is_unreachable() {
        let before = [
            ureq::Error::ConnectionFailed,
            ureq::Error::HostNotFound,
            ureq::Error::Timeout(ureq::Timeout::Connect),
            ureq::Error::Timeout(ureq::Timeout::Resolve),
            ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
            ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound)),
        ];
        for e in before {
            let err = Fal::post_error(e);
            assert_eq!((err.code, err.retryable), (ErrorCode::EngineMissing, false), "{err:?}");
            assert!(err.message.starts_with("Couldn't reach fal.ai"), "{err:?}");
            assert!(err.remediation.as_deref().is_some_and(|r| r.contains("internet connection")), "{err:?}");
        }

        let after = [
            (ureq::Error::Timeout(ureq::Timeout::SendBody), ErrorCode::TimedOut),
            (ureq::Error::Timeout(ureq::Timeout::RecvResponse), ErrorCode::TimedOut),
            (ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)), ErrorCode::ProgramFailed),
            (ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe)), ErrorCode::ProgramFailed),
            (ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::Interrupted)), ErrorCode::ProgramFailed),
            (ureq::Error::Tls("bad certificate"), ErrorCode::ProgramFailed),
        ];
        for (e, code) in after {
            let err = Fal::post_error(e);
            assert_eq!((err.code, err.retryable), (code, false), "{err:?}");
            assert!(!err.message.starts_with("Couldn't reach"), "{err:?}");
            assert!(err.message.contains("may already be running") && err.message.contains("billed"), "{err:?}");
            assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard") && r.contains("twice")), "{err:?}");
        }
    }

    /// Once fal has answered 200, a stalled or cut body is the same billed generation.
    #[test]
    fn an_answer_cut_short_is_billed_aware() {
        let slow = Fal::answer_error(ureq::Error::Timeout(ureq::Timeout::RecvBody));
        assert_eq!((slow.code, slow.retryable), (ErrorCode::TimedOut, false), "{slow:?}");
        assert!(slow.message.contains("may already be running") && slow.message.contains("billed"), "{slow:?}");
        let interrupted = Fal::answer_error(ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::Interrupted)));
        assert_eq!((interrupted.code, interrupted.retryable), (ErrorCode::ProgramFailed, false), "{interrupted:?}");
        assert!(interrupted.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{interrupted:?}");
    }

    /// A server that accepts the POST, reads it, and closes without sending response headers.
    fn serve_closed() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = l.accept().unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut request_body = vec![0; len];
            r.read_exact(&mut request_body).unwrap();
            drop(stream);
        });
        base
    }

    /// The POST was consumed before the connection closed, so the generation may have started.
    #[test]
    fn a_disconnect_before_the_answer_is_billed_aware() {
        let err = Fal { base: serve_closed() }.post("k", "model", &json!({})).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::ProgramFailed, false), "{err:?}");
        assert!(err.message.contains("may already be running") && err.message.contains("billed"), "{err:?}");
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{err:?}");
    }

    /// A local server that accepts one generation POST and sends 200 headers, then fewer body
    /// bytes than its Content-Length says before closing.
    fn serve_truncated(body: &'static str) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = l.accept().unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut request_body = vec![0; len];
            r.read_exact(&mut request_body).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{body}").unwrap();
        });
        base
    }

    /// fal accepted the generation, so a cut answer doesn't mean nothing ran or was billed.
    #[test]
    fn a_truncated_answer_to_a_paid_generation_is_not_unreadable() {
        let err = Fal { base: serve_truncated("{}") }.post("k", "model", &json!({})).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::ProgramFailed, false), "{err:?}");
        assert!(err.message.contains("may already be running") && err.message.contains("billed"), "{err:?}");
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{err:?}");
    }

    /// A complete but unreadable 200 answer still came from a generation fal accepted.
    #[test]
    fn a_malformed_answer_to_a_paid_generation_is_billed_aware() {
        let (base, _) = serve(vec![(200, "not json".into())]);
        let err = Fal { base }.post("k", "model", &json!({})).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::GarbledOutput, false), "{err:?}");
        assert!(err.message.contains("may already be running") && err.message.contains("billed"), "{err:?}");
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{err:?}");
    }

    /// A gateway timeout or server failure after the POST may still have started a paid
    /// generation: it must not be retried or read as a plain server error.
    #[test]
    fn an_ambiguous_server_failure_is_billed_aware() {
        for (status, code) in [(504, ErrorCode::TimedOut), (500, ErrorCode::EngineMissing)] {
            let (base, _) = serve(vec![(status, "not today".into())]);
            let err = Fal { base }.post("k", "model", &json!({})).unwrap_err();
            assert_eq!((err.code, err.retryable), (code, false), "{err:?}");
            assert!(err.message.contains("may already be running") && err.message.contains("billed"), "{err:?}");
            assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{err:?}");
        }
    }

    /// A redirect isn't followed for a paid POST: the first address may have accepted it.
    #[test]
    fn a_redirected_generation_is_billed_aware() {
        let (base, _) = serve(vec![(307, "elsewhere".into())]);
        let err = Fal { base }.post("k", "model", &json!({})).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::ProgramFailed, false), "{err:?}");
        assert!(err.message.contains("redirected") && err.message.contains("billed"), "{err:?}");
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{err:?}");
    }

    /// A request fal rejects before generating is not billed and can be corrected.
    #[test]
    fn a_rejected_generation_is_not_billed_aware() {
        let (base, _) = serve(vec![(400, "bad prompt".into())]);
        let err = Fal { base }.post("k", "model", &json!({})).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::BadInput, false), "{err:?}");
        assert!(!err.message.contains("billed"), "{err:?}");
    }

    /// A download after a paid generation can fail; it must not invite the whole paid generation
    /// again, and it must give the file's address so the person can still fetch it.
    #[test]
    fn a_failed_download_says_the_generation_was_paid_for() {
        let (base, _) = serve(vec![(500, "missing".into())]);
        let url = format!("{base}/made.png");
        let err = download(&url).unwrap_err();
        assert_eq!((err.code, err.retryable), (ErrorCode::EngineMissing, false), "{err:?}");
        assert!(err.message.contains("generation was made and paid for"), "{err:?}");
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains(&url)), "{err:?}");
        assert_eq!(err.details.as_ref().and_then(|d| d["url"].as_str()), Some(url.as_str()));

        // A `data:` answer that can't be decoded was still a paid generation, with no address to give.
        let bad = download("data:image/png,not-base64").unwrap_err();
        assert_eq!((bad.code, bad.retryable), (ErrorCode::GarbledOutput, false), "{bad:?}");
        assert!(bad.message.contains("generation was made and paid for"), "{bad:?}");
        assert!(bad.remediation.as_deref().is_some_and(|r| r.contains("dashboard")), "{bad:?}");
    }

    #[test]
    fn refusals_say_what_to_do() {
        let (base, _) = serve(vec![
            (401, "{}".into()),
            (402, "{}".into()),
            (200, json!({"images": [{"url": "data:image/png;base64,AA=="}], "has_nsfw_concepts": [true]}).to_string()),
        ]);
        let fal = Fal { base };
        assert_eq!(image(&fal, "bad", "x", Shape::Wide).unwrap_err().code, ErrorCode::BadSettings);
        assert_eq!(image(&fal, "bad", "x", Shape::Wide).unwrap_err().code, ErrorCode::OverLimit);
        assert!(image(&fal, "bad", "x", Shape::Wide).unwrap_err().message.contains("safety check"));
    }

    #[test]
    fn shapes_and_data_uris() {
        assert_eq!(Shape::parse("wide"), Some(Shape::Wide));
        assert!(Shape::parse("huge").is_none());
        let uri = data_uri(b"hi", "image/png");
        assert_eq!(uri, "data:image/png;base64,aGk=");
        assert_eq!(download(&uri).unwrap(), b"hi");
    }
}
