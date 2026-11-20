//! Incremental marker scanner for streaming HTML.

const SENTINEL: &[u8] = b"<!--fastcart:extensions-injected-->";
const HEAD_MARKER: &[u8] = b"<!--fastcart:extensions-head-->";
const BODY_MARKER: &[u8] = b"<!--fastcart:extensions-body-->";
const HEAD_SCRIPT: &[u8] =
    b"<script type=\"application/fastcart-extensions\" data-slot=\"head\"></script>";
const BODY_SCRIPT: &[u8] =
    b"<script type=\"application/fastcart-extensions\" data-slot=\"body\"></script>";
const TAIL: usize = 32 * 1024;
const DECISION_BYTES: usize = 1024;

use super::InjectionConfig;
use super::is_full_document;

/// Streaming injector state.
///
/// Feed chunks with [`StreamInjector::push`] and call [`StreamInjector::finish`]
/// at EOF. Output bytes are yielded as soon as they are safe.
pub struct StreamInjector {
    config: InjectionConfig,
    buffer: Vec<u8>,
    decided: bool,
    passthrough: bool,
    head_done: bool,
    body_done: bool,
    injected: bool,
}

impl StreamInjector {
    /// Creates an injector for one response.
    #[must_use]
    pub fn new(config: InjectionConfig) -> Self {
        Self {
            config,
            buffer: Vec::new(),
            decided: false,
            passthrough: false,
            head_done: false,
            body_done: false,
            injected: false,
        }
    }

    /// Consumes one chunk and returns safe output chunks.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut output = Vec::new();
        if !self.decided {
            if self.buffer.len() < DECISION_BYTES {
                return output;
            }
            self.decide();
        }
        if self.passthrough {
            if !self.buffer.is_empty() {
                output.push(std::mem::take(&mut self.buffer));
            }
            return output;
        }
        self.scan(&mut output, false);
        output
    }

    /// Flushes the remaining buffer at EOF.
    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        let mut output = Vec::new();
        if !self.decided {
            self.decide();
        }
        if self.passthrough {
            if !self.buffer.is_empty() {
                output.push(std::mem::take(&mut self.buffer));
            }
            return output;
        }
        self.scan(&mut output, true);
        if !self.buffer.is_empty() {
            output.push(std::mem::take(&mut self.buffer));
        }
        output
    }

    fn decide(&mut self) {
        self.decided = true;
        if !is_full_document(&self.buffer) {
            self.passthrough = true;
        }
    }

    fn scan(&mut self, output: &mut Vec<Vec<u8>>, eof: bool) {
        loop {
            if self.buffer.is_empty() {
                return;
            }
            if find(&self.buffer, SENTINEL).is_some() {
                self.passthrough = true;
                if !self.buffer.is_empty() {
                    output.push(std::mem::take(&mut self.buffer));
                }
                return;
            }

            if !self.head_done
                && let Some((index, length)) = marker(&self.buffer, HEAD_MARKER, HEAD_SCRIPT)
            {
                emit(output, &self.buffer[..index]);
                self.inject_head(output);
                self.buffer.drain(..index + length);
                self.head_done = true;
                continue;
            }

            if !self.body_done
                && let Some((index, length)) = marker(&self.buffer, BODY_MARKER, BODY_SCRIPT)
            {
                emit(output, &self.buffer[..index]);
                self.inject_body(output);
                self.buffer.drain(..index + length);
                self.body_done = true;
                continue;
            }

            if !self.head_done
                && let Some(index) = find_case_insensitive(&self.buffer, b"</head>")
            {
                emit(output, &self.buffer[..index]);
                self.inject_head(output);
                let end = index + b"</head>".len();
                emit(output, &self.buffer[index..end]);
                self.buffer.drain(..end);
                self.head_done = true;
                continue;
            }

            if eof && !self.body_done {
                if let Some(index) = rfind_case_insensitive(&self.buffer, b"</body>") {
                    emit(output, &self.buffer[..index]);
                    self.inject_body(output);
                    let tail = self.buffer[index..].to_vec();
                    self.buffer.clear();
                    output.push(tail);
                    self.body_done = true;
                    continue;
                }
                output.push(std::mem::take(&mut self.buffer));
                return;
            }

            let keep = if self.body_done { 0 } else { TAIL };
            if self.buffer.len() > keep {
                let emit_len = self.buffer.len() - keep;
                let chunk: Vec<u8> = self.buffer.drain(..emit_len).collect();
                output.push(chunk);
                continue;
            }
            return;
        }
    }

    fn inject_head(&mut self, output: &mut Vec<Vec<u8>>) {
        if self.config.head.is_empty() {
            return;
        }
        let mut chunk = Vec::with_capacity(self.config.head.len() + SENTINEL.len());
        if !self.injected {
            chunk.extend_from_slice(SENTINEL);
            self.injected = true;
        }
        chunk.extend_from_slice(self.config.head.as_bytes());
        output.push(chunk);
    }

    fn inject_body(&mut self, output: &mut Vec<Vec<u8>>) {
        if self.config.body.is_empty() {
            return;
        }
        let mut chunk = Vec::with_capacity(self.config.body.len() + SENTINEL.len());
        if !self.injected {
            chunk.extend_from_slice(SENTINEL);
            self.injected = true;
        }
        chunk.extend_from_slice(self.config.body.as_bytes());
        output.push(chunk);
    }
}

fn emit(output: &mut Vec<Vec<u8>>, bytes: &[u8]) {
    if !bytes.is_empty() {
        output.push(bytes.to_vec());
    }
}

fn marker(buffer: &[u8], comment: &[u8], script: &[u8]) -> Option<(usize, usize)> {
    if let Some(index) = find(buffer, comment) {
        return Some((index, comment.len()));
    }
    find(buffer, script).map(|index| (index, script.len()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let lower = haystack.to_ascii_lowercase();
    lower
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
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

    fn run(html: &[u8], chunk_size: usize, config: InjectionConfig) -> Vec<u8> {
        let mut injector = StreamInjector::new(config);
        let mut output = Vec::new();
        for chunk in html.chunks(chunk_size) {
            for part in injector.push(chunk) {
                output.extend_from_slice(&part);
            }
        }
        for part in injector.finish() {
            output.extend_from_slice(&part);
        }
        output
    }

    #[test]
    fn works_across_one_byte_chunks() {
        let html = b"<!doctype html><html><head><!--fastcart:extensions-head--></head><body><!--fastcart:extensions-body--></body></html>";
        let output = run(html, 1, config());
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("/boot.css"));
        assert!(text.contains("/runtime.js"));
        assert!(text.contains("fastcart:extensions-injected"));
        assert!(!text.contains("extensions-head"));
    }

    #[test]
    fn falls_back_without_markers() {
        let html = b"<!doctype html><html><head><title>x</title></head><body>hello</body></html>";
        let text = String::from_utf8(run(html, 7, config())).unwrap();
        assert!(text.contains("/boot.css"));
        assert!(text.contains("/runtime.js"));
    }

    #[test]
    fn fragments_pass_through() {
        let html = b"<div>fragment</div>";
        let output = run(html, 3, config());
        assert_eq!(output, html);
    }

    #[test]
    fn sentinel_disables_injection() {
        let html = b"<!doctype html><html><head><!--fastcart:extensions-injected--></head><body></body></html>";
        let output = run(html, 5, config());
        assert_eq!(output, html);
    }
}
