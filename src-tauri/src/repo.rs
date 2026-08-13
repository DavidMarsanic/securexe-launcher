use crate::error::LauncherError;
use crate::signature;

/// A parsed, validated `securexe://run?repo=owner/repo&commit=<sha>&exp=<unix_ts>&sig=<hex>` request.
///
/// `owner`/`repo`/`commit` are validated against a strict charset before this
/// struct can be constructed, because they get interpolated into a local
/// filesystem path (`~/.securexe/apps/<slug>/<commit>/...`). A malicious page
/// could otherwise pass something like `repo=..%2F..%2F..%2Fetc&commit=passwd`
/// through the scheme and walk the cache path outside `~/.securexe`.
///
/// `exp`/`sig` are also required and checked against the website's signing
/// key (see signature.rs) — without a valid signature, *any* webpage could
/// construct a `securexe://run` link for any repo, since the OS gives the
/// launcher no way to know which site a custom-scheme link was clicked
/// from. The signature is what limits working links to ones minted by
/// brightencode.com's own backend.
pub struct RunRequest {
    pub owner: String,
    pub repo: String,
    pub commit: Option<String>,
}

impl RunRequest {
    /// Storage-form slug (`owner__repo`) — required by the orchestrator's
    /// `/download` endpoint (it 404s on `owner/repo`, unlike `/manifest`).
    pub fn slug(&self) -> String {
        format!("{}__{}", self.owner, self.repo)
    }

    pub fn repo_path(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Safe for both a URL path segment and a filesystem path segment: no `/`,
/// no `..` traversal, no hidden/empty segments.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Also used by flow.rs to validate the commit the *orchestrator* resolves
/// (when the request didn't pin one) before it's used as a path segment.
pub fn is_safe_commit(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// A parsed, signature-verified `securexe://link?user=...&exp=...&sig=...`
/// request — hands off a device-linking token minted by the website
/// (lib/signing.ts's signDeviceLinkToken) rather than a repo to run.
pub struct LinkRequest {
    pub user: String,
    pub exp: String,
    pub sig: String,
}

/// GitHub usernames are alphanumeric plus single hyphens, capped at 39
/// chars — this is intentionally a little looser than that (up to 64, no
/// leading/trailing-hyphen check) since it's not being used as a filesystem
/// or URL path segment here, just sanity-bounding what gets stored locally
/// and sent on to the worker.
fn is_safe_username(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn parse_link_url(raw: &str) -> Result<LinkRequest, LauncherError> {
    let url = url::Url::parse(raw).map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

    if url.scheme() != "securexe" {
        return Err(LauncherError::InvalidUrl(format!(
            "unsupported scheme '{}'",
            url.scheme()
        )));
    }

    let action = url.host_str().unwrap_or_default();
    if action != "link" {
        return Err(LauncherError::InvalidUrl(format!(
            "unsupported action '{action}'"
        )));
    }

    let mut user_param: Option<String> = None;
    let mut exp_param: Option<String> = None;
    let mut sig_param: Option<String> = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "user" => user_param = Some(value.into_owned()),
            "exp" => exp_param = Some(value.into_owned()),
            "sig" => sig_param = Some(value.into_owned()),
            _ => {}
        }
    }

    let user = user_param.ok_or_else(|| LauncherError::InvalidUrl("missing user".into()))?;
    if !is_safe_username(&user) {
        return Err(LauncherError::InvalidUrl(format!("invalid user '{user}'")));
    }

    let exp = exp_param.ok_or_else(|| LauncherError::Unauthorized("missing exp".into()))?;
    let sig = sig_param.ok_or_else(|| LauncherError::Unauthorized("missing sig".into()))?;

    signature::verify_link(&user, &exp, &sig)?;

    Ok(LinkRequest { user, exp, sig })
}

pub fn parse_run_url(raw: &str) -> Result<RunRequest, LauncherError> {
    let url = url::Url::parse(raw).map_err(|e| LauncherError::InvalidUrl(e.to_string()))?;

    if url.scheme() != "securexe" {
        return Err(LauncherError::InvalidUrl(format!(
            "unsupported scheme '{}'",
            url.scheme()
        )));
    }

    let action = url.host_str().unwrap_or_default();
    if action != "run" {
        return Err(LauncherError::InvalidUrl(format!(
            "unsupported action '{action}'"
        )));
    }

    let mut repo_param: Option<String> = None;
    let mut commit_param: Option<String> = None;
    let mut exp_param: Option<String> = None;
    let mut sig_param: Option<String> = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "repo" => repo_param = Some(value.into_owned()),
            "commit" => commit_param = Some(value.into_owned()),
            "exp" => exp_param = Some(value.into_owned()),
            "sig" => sig_param = Some(value.into_owned()),
            _ => {}
        }
    }

    let repo_param = repo_param.ok_or_else(|| LauncherError::InvalidUrl("missing repo".into()))?;
    let mut parts = repo_param.splitn(2, '/');
    let owner = parts.next().unwrap_or_default().to_string();
    let repo = parts.next().unwrap_or_default().to_string();

    if !is_safe_segment(&owner) || !is_safe_segment(&repo) {
        return Err(LauncherError::InvalidUrl(format!(
            "invalid repo '{repo_param}'"
        )));
    }

    let commit = match commit_param {
        Some(c) if !c.is_empty() => {
            if !is_safe_commit(&c) {
                return Err(LauncherError::InvalidUrl(format!("invalid commit '{c}'")));
            }
            Some(c)
        }
        _ => None,
    };

    let exp = exp_param.ok_or_else(|| LauncherError::Unauthorized("missing exp".into()))?;
    let sig = sig_param.ok_or_else(|| LauncherError::Unauthorized("missing sig".into()))?;

    let repo_path = format!("{owner}/{repo}");
    signature::verify(&repo_path, commit.as_deref(), &exp, &sig)?;

    Ok(RunRequest { owner, repo, commit })
}
