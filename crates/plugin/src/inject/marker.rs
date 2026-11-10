//! Marker scanning and HTML injection.

const SENTINEL: &[u8] = b"<!--fastcart:extensions-injected-->";
const HEAD_MARKER: &[u8] = b"<!--fastcart:extensions-head-->";
const BODY_MARKER: &[u8] = b"<!--fastcart:extensions-body-->";
const HEAD_SCRIPT: &[u8] =
    b"<script type=\"application/fastcart-extensions\" data-slot=\"head\"></script>";
const BODY_SCRIPT: &[u8] =
    b"<script type=\"application/fastcart-extensions\" data-slot=\"body\"></script>";
const MAX_DOCUMENT_PROBE: usize = 1024;

/// Generated tags for one response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InjectionConfig {
    /// Tags inserted into the head slot.
    pub head: String,
    /// Tags inserted into the body slot.
    pub body: String,
}

/// Result of one injection pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectionOutcome {
    /// Transformed document.
    pub html: Vec<u8>,
    /// Whether head tags were written.
    pub head: bool,
    /// Whether body tags were written.
    pub body: bool,
}

/// Returns whether the payload looks like a full HTML document.
#[must_use]
pub fn is_full_document(html: &[u8]) -> bool {
    let probe = &html[..html.len().min(MAX_DOCUMENT_PROBE)];
    let lower = probe.to_ascii_lowercase();
    lower.windows(5).any(|window| window == b"<!doc")
        || lower.windows(5).any(|window| window == b"<html")
}

/// Injects extension tags into a full document.
#[must_use]
pub fn inject(html: &[u8], config: &InjectionConfig) -> InjectionOutcome {
    if contains(html, SENTINEL) {
        return InjectionOutcome {
            html: html.to_vec(),
            head: false,
            body: false,
        };
    }

    let mut head = false;
    let mut body = false;
    let mut output = Vec::with_capacity(html.len() + config.head.len() + config.body.len() + 64);

    if let Some(index) = find(html, HEAD_MARKER).or_else(|| find(html, HEAD_SCRIPT)) {
        let marker_len = if html[index..].starts_with(HEAD_MARKER) {
            HEAD_MARKER.len()
        } else {
            HEAD_SCRIPT.len()
        };
        output.extend_from_slice(&html[..index]);
        if !config.head.is_empty() {
            output.extend_from_slice(SENTINEL);
            output.extend_from_slice(config.head.as_bytes());
            head = true;
        }
        output.extend_from_slice(&html[index + marker_len..]);
    } else if let Some(index) = find_last_case_insensitive(html, b"</head>") {
        output.extend_from_slice(&html[..index]);
        if !config.head.is_empty() {
            output.extend_from_slice(SENTINEL);
            output.extend_from_slice(config.head.as_bytes());
            head = true;
        }
        output.extend_from_slice(&html[index..]);
    } else {
        output.extend_from_slice(html);
    }

    if let Some(index) = find(&output, BODY_MARKER).or_else(|| find(&output, BODY_SCRIPT)) {
        let marker_len = if output[index..].starts_with(BODY_MARKER) {
            BODY_MARKER.len()
        } else {
            BODY_SCRIPT.len()
        };
        let mut replaced = Vec::with_capacity(output.len() + config.body.len() + 64);
        replaced.extend_from_slice(&output[..index]);
        if !config.body.is_empty() {
            if !contains(&output[..index], SENTINEL) {
                replaced.extend_from_slice(SENTINEL);
            }
            replaced.extend_from_slice(config.body.as_bytes());
            body = true;
        }
        replaced.extend_from_slice(&output[index + marker_len..]);
        output = replaced;
    } else if let Some(index) = find_last_case_insensitive(&output, b"</body>") {
        let mut replaced = Vec::with_capacity(output.len() + config.body.len() + 64);
        replaced.extend_from_slice(&output[..index]);
        if !config.body.is_empty() {
            if !contains(&output[..index], SENTINEL) {
                replaced.extend_from_slice(SENTINEL);
            }
            replaced.extend_from_slice(config.body.as_bytes());
            body = true;
        }
        replaced.extend_from_slice(&output[index..]);
        output = replaced;
    }

    InjectionOutcome {
        html: output,
        head,
        body,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_last_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let lower = haystack.to_ascii_lowercase();
    lower
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> InjectionConfig {
        InjectionConfig {
            head: "<link rel=\"stylesheet\" href=\"/boot.css\">".to_owned(),
            body: "<script type=\"module\" src=\"/runtime.js\"></script>".to_owned(),
        }
    }

    #[test]
    fn replaces_comment_markers() {
        let html = b"<!doctype html><html><head><!--fastcart:extensions-head--></head><body><!--fastcart:extensions-body--></body></html>";
        let result = inject(html, &config());
        assert!(result.head && result.body);
        let text = String::from_utf8(result.html).unwrap();
        assert!(!text.contains("fastcart:extensions-head"));
        assert!(text.contains("/boot.css"));
        assert!(text.contains("/runtime.js"));
    }

    #[test]
    fn replaces_script_markers() {
        let html = b"<!doctype html><html><head><script type=\"application/fastcart-extensions\" data-slot=\"head\"></script></head><body><script type=\"application/fastcart-extensions\" data-slot=\"body\"></script></body></html>";
        let result = inject(html, &config());
        assert!(result.head && result.body);
    }

    #[test]
    fn falls_back_to_closing_tags() {
        let html = b"<!doctype html><html><head><title>x</title></head><body>hello</body></html>";
        let result = inject(html, &config());
        assert!(result.head && result.body);
        let text = String::from_utf8(result.html).unwrap();
        assert!(text.contains("/boot.css</head>") || text.contains("/boot.css"));
        assert!(text.contains("/runtime.js</body>") || text.contains("/runtime.js"));
    }

    #[test]
    fn second_pass_is_idempotent() {
        let html = b"<!doctype html><html><head></head><body></body></html>";
        let first = inject(html, &config());
        let second = inject(&first.html, &config());
        assert_eq!(first.html, second.html);
        assert!(!second.head && !second.body);
    }

    #[test]
    fn fragments_are_not_documents() {
        assert!(!is_full_document(b"<div>fragment</div>"));
        assert!(is_full_document(b"<!DOCTYPE html><html></html>"));
    }
}
