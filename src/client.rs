use anyhow::{anyhow, Context, Result};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use reqwest::{header, Client as Http, Method, Response, StatusCode};
use serde::Serialize;
use serde_json::Value;

use crate::config::Resolved;

/// Characters percent-encoded inside a single URL path segment so a user/server
/// -supplied id/uuid/username can't reshape the request path — i.e. inject a
/// query (`?`) or fragment (`#`), add `/` separators, or smuggle `%2e%2e`
/// traversal. Unreserved chars (alnum, `-`, `.`, `_`, `~`) are left intact so a
/// normal UUID/domain round-trips unchanged.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Percent-encode a dynamic path segment for safe interpolation into a request
/// path. Returns a `Display` adapter so it drops straight into `format!`.
pub fn enc(segment: &str) -> impl std::fmt::Display + '_ {
    utf8_percent_encode(segment, PATH_SEGMENT)
}

/// Convert a Serialize-able query struct into a flat list of key=value pairs,
/// dropping any `None` Option fields. The pairs are URL-decoded so they round-trip
/// safely through reqwest's `.query()` (which is itself URL-encoded by serde_urlencoded).
///
/// Use this to materialize query parameters up front, so a long-lived poller
/// can re-issue the same request without re-borrowing the original struct.
pub fn args_to_pairs<T: Serialize + ?Sized>(args: &T) -> Result<Vec<(String, String)>> {
    let encoded = serde_urlencoded::to_string(args)
        .context("encoding query args as application/x-www-form-urlencoded")?;
    Ok(url::form_urlencoded::parse(encoded.as_bytes())
        .into_owned()
        .collect())
}

#[derive(Clone, Copy, Debug)]
pub enum Auth {
    /// `api_...` token for the management API
    Api,
    /// `transactional_...` token for `/email` and `/email-batch`
    Transactional,
    /// No authentication header (e.g. POST /inbound/forward-destinations/verify)
    None,
}

#[derive(Clone)]
pub struct ApiClient {
    http: Http,
    base_url: String,
    cfg: Resolved,
}

impl ApiClient {
    pub fn new(cfg: Resolved) -> Result<Self> {
        let http = Http::builder()
            .user_agent(format!("jetemail-cli/{}", env!("CARGO_PKG_VERSION")))
            // The JetEmail API does not 3xx-redirect normal calls. Disable
            // redirect-following explicitly so the bearer token can never be
            // replayed to a different host via a redirect (rather than relying on
            // reqwest's implicit cross-host header-stripping default).
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building HTTP client")?;
        let base_url = cfg.base_url.trim_end_matches('/').to_string();
        Ok(Self {
            http,
            base_url,
            cfg,
        })
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    fn auth_header(&self, auth: Auth) -> Result<Option<String>> {
        match auth {
            Auth::Api => Ok(Some(format!("Bearer {}", self.cfg.require_api_key()?))),
            Auth::Transactional => Ok(Some(format!(
                "Bearer {}",
                self.cfg.require_transactional_key()?
            ))),
            Auth::None => Ok(None),
        }
    }

    pub async fn request_json<Q: Serialize + ?Sized, B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        auth: Auth,
        query: Option<&Q>,
        body: Option<&B>,
        extra_headers: &[(&str, &str)],
    ) -> Result<Value> {
        let resp = self
            .send(method, path, auth, query, body, extra_headers, None)
            .await?;
        decode_json(resp).await
    }

    pub async fn request_text<Q: Serialize + ?Sized, B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        auth: Auth,
        query: Option<&Q>,
        body: Option<&B>,
        extra_headers: &[(&str, &str)],
    ) -> Result<String> {
        let resp = self
            .send(method, path, auth, query, body, extra_headers, None)
            .await?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            return Err(api_error(status, &text));
        }
        Ok(text)
    }

    pub async fn request_csv<Q: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        auth: Auth,
        query: Option<&Q>,
        csv_body: &str,
    ) -> Result<Value> {
        let resp = self
            .send::<Q, ()>(
                method,
                path,
                auth,
                query,
                None,
                &[],
                Some(("text/csv", csv_body.to_string())),
            )
            .await?;
        decode_json(resp).await
    }

    async fn send<Q: Serialize + ?Sized, B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        auth: Auth,
        query: Option<&Q>,
        body: Option<&B>,
        extra_headers: &[(&str, &str)],
        raw_body: Option<(&str, String)>,
    ) -> Result<Response> {
        let url = self.url(path);
        let mut req = self.http.request(method, &url);
        if let Some(h) = self.auth_header(auth)? {
            req = req.header(header::AUTHORIZATION, h);
        }
        for (k, v) in extra_headers {
            req = req.header(*k, *v);
        }
        if let Some(q) = query {
            req = req.query(q);
        }
        if let Some((mime, raw)) = raw_body {
            req = req.header(header::CONTENT_TYPE, mime).body(raw);
        } else if let Some(b) = body {
            req = req.json(b);
        }
        req.send()
            .await
            .with_context(|| format!("sending request to {url}"))
    }
}

async fn decode_json(resp: Response) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await.context("reading response body")?;
    if !status.is_success() {
        return Err(api_error(status, &text));
    }
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| anyhow!("invalid JSON in response ({e}): {text}"))
}

fn api_error(status: StatusCode, body: &str) -> anyhow::Error {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return anyhow!("HTTP {status}");
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string());
        anyhow!("HTTP {status}\n{pretty}")
    } else {
        anyhow!("HTTP {status}: {trimmed}")
    }
}
