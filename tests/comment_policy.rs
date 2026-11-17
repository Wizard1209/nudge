use std::fs;
use std::path::{Path, PathBuf};

const RULE: &str = "Source comments are not written. The one allowed form is a block of at most four \
lines whose last line is `proof: <https URL | repo-relative path::fn name or test title>`; \
tool directives (@ts-expect-error, eslint-disable, svelte-ignore, ...) are exempt.";
const MAX_LISTED: usize = 40;
const SKIPPED_DIRS: [&str; 3] = ["target", "node_modules", "dist"];
const DIRECTIVES: &str = "@ts-expect-error|@ts-ignore|@ts-nocheck|<reference|eslint-disable|\
eslint-enable|eslint |prettier-ignore|v8 ignore|istanbul ignore|c8 ignore|@vitest-environment|@vite-ignore|\
svelte-ignore|#!";
const REGEX_AFTER: &str =
    "return typeof instanceof in of new delete void throw case do else yield await";

#[derive(Clone, Copy)]
enum Lang {
    Rust,
    Js,
    Svelte,
}

enum Mark {
    Code(usize),
    Comment(usize, usize),
}

struct Block {
    first: usize,
    last: usize,
    text: String,
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    end: usize,
    marks: Vec<Mark>,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

fn utf8_len(c: u8) -> usize {
    match c {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

impl<'a> Lexer<'a> {
    fn at(&self, k: usize) -> u8 {
        self.s.get(self.i + k).copied().unwrap_or(0)
    }

    fn starts(&self, p: &str) -> bool {
        self.s
            .get(self.i..self.end)
            .is_some_and(|rest| rest.starts_with(p.as_bytes()))
    }

    fn code(&mut self) {
        match self.marks.last_mut() {
            Some(Mark::Code(at)) => *at = self.i,
            _ => self.marks.push(Mark::Code(self.i)),
        }
    }

    fn skip_until(&mut self, p: &str) {
        while self.i < self.end && !self.starts(p) {
            self.i += 1;
        }
    }

    fn line_comment(&mut self) {
        let start = self.i;
        self.skip_until("\n");
        self.marks.push(Mark::Comment(start, self.i));
    }

    fn block_comment(&mut self, open: &str, close: &str, nests: bool) {
        let start = self.i;
        let mut depth = 0;
        while self.i < self.end {
            if self.starts(open) && (nests || depth == 0) {
                depth += 1;
                self.i += open.len();
            } else if self.starts(close) {
                depth -= 1;
                self.i += close.len();
                if depth == 0 {
                    break;
                }
            } else {
                self.i += 1;
            }
        }
        self.marks.push(Mark::Comment(start, self.i.min(self.end)));
    }

    fn quoted(&mut self, quote: u8) {
        self.code();
        self.i += 1;
        while self.i < self.end && self.s[self.i] != quote {
            self.i += if self.s[self.i] == b'\\' { 2 } else { 1 };
        }
        self.code();
        self.i += 1;
    }

    fn word(&mut self) -> &'a [u8] {
        let start = self.i;
        self.code();
        while self.i < self.end && is_ident(self.s[self.i]) {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    fn rust(&mut self) {
        while self.i < self.end {
            match self.s[self.i] {
                c if c.is_ascii_whitespace() => self.i += 1,
                b'/' if self.at(1) == b'/' => self.line_comment(),
                b'/' if self.at(1) == b'*' => self.block_comment("/*", "*/", true),
                b'"' => self.quoted(b'"'),
                b'\'' => self.char_or_lifetime(),
                c if is_ident(c) => {
                    if matches!(self.word(), b"r" | b"br" | b"cr") {
                        self.raw_string();
                    }
                }
                _ => {
                    self.code();
                    self.i += 1;
                }
            }
        }
    }

    fn raw_string(&mut self) {
        let hashes = self.s[self.i..].iter().take_while(|&&c| c == b'#').count();
        if self.at(hashes) != b'"' {
            return;
        }
        let close = format!("\"{}", "#".repeat(hashes));
        self.i += hashes + 1;
        self.skip_until(&close);
        self.code();
        self.i += close.len();
    }

    fn char_or_lifetime(&mut self) {
        self.code();
        self.i += 1;
        if self.at(0) == b'\\' {
            self.i += 2;
            self.skip_until("'");
            self.i += 1;
        } else {
            let len = utf8_len(self.at(0));
            if self.at(len) == b'\'' {
                self.i += len + 1;
            }
        }
    }

    fn js(&mut self, closes_on_brace: bool) {
        let mut depth = 0usize;
        let mut regex_ok = true;
        if self.i == 0 && self.starts("#!") {
            self.line_comment();
        }
        while self.i < self.end {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() {
                self.i += 1;
                continue;
            }
            if c == b'/' && self.at(1) == b'/' {
                self.line_comment();
                continue;
            }
            if c == b'/' && self.at(1) == b'*' {
                self.block_comment("/*", "*/", false);
                continue;
            }
            regex_ok = match c {
                b'/' if regex_ok => {
                    self.regex();
                    false
                }
                b'"' | b'\'' => {
                    self.quoted(c);
                    false
                }
                b'`' => {
                    self.template();
                    false
                }
                c if is_ident(c) => {
                    let word = self.word();
                    REGEX_AFTER.split(' ').any(|k| k.as_bytes() == word)
                }
                b'}' if depth == 0 && closes_on_brace => {
                    self.code();
                    self.i += 1;
                    return;
                }
                _ => {
                    match c {
                        b'{' => depth += 1,
                        b'}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    self.code();
                    self.i += 1;
                    !matches!(c, b')' | b']' | b'}')
                }
            };
        }
    }

    fn regex(&mut self) {
        self.code();
        self.i += 1;
        let mut in_class = false;
        while self.i < self.end {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'\\' => self.i += 1,
                b'[' => in_class = true,
                b']' => in_class = false,
                b'/' if !in_class => break,
                b'\n' => break,
                _ => {}
            }
        }
    }

    fn template(&mut self) {
        self.code();
        self.i += 1;
        while self.i < self.end {
            match self.s[self.i] {
                b'\\' => self.i += 2,
                b'`' => break,
                b'$' if self.at(1) == b'{' => {
                    self.i += 2;
                    self.js(true);
                }
                _ => self.i += 1,
            }
        }
        self.code();
        self.i += 1;
    }

    fn css(&mut self) {
        while self.i < self.end {
            match self.s[self.i] {
                c if c.is_ascii_whitespace() => self.i += 1,
                b'/' if self.at(1) == b'*' => self.block_comment("/*", "*/", false),
                quote @ (b'"' | b'\'') => self.quoted(quote),
                _ => {
                    self.code();
                    self.i += 1;
                }
            }
        }
    }

    fn svelte(&mut self) {
        let len = self.s.len();
        while self.i < len {
            if self.starts("<!--") {
                self.block_comment("<!--", "-->", false);
            } else if self.starts("<script") || self.starts("<style") {
                let css = self.starts("<style");
                let close: &[u8] = if css { b"</style" } else { b"</script" };
                self.skip_until(">");
                self.code();
                self.i += 1;
                self.end = self.s[self.i.min(len)..]
                    .windows(close.len())
                    .position(|w| w == close)
                    .map_or(len, |at| self.i + at);
                if css {
                    self.css()
                } else {
                    self.js(false)
                }
                self.i = self.end;
                self.end = len;
            } else if self.s[self.i] == b'{' {
                self.code();
                self.i += 1;
                if matches!(self.at(0), b'#' | b'/' | b':' | b'@') {
                    self.i += 1;
                }
                self.js(true);
            } else {
                if !self.s[self.i].is_ascii_whitespace() {
                    self.code();
                }
                self.i += 1;
            }
        }
    }
}

fn body(line: &str) -> &str {
    let text = line
        .trim()
        .trim_start_matches("<!--")
        .trim_start_matches(['/', '*', '!'])
        .trim_end();
    text.strip_suffix("-->")
        .or_else(|| text.strip_suffix("*/"))
        .unwrap_or(text)
        .trim()
}

fn is_directive(text: &str) -> bool {
    text.lines().any(|line| {
        let body = body(line);
        DIRECTIVES.split('|').any(|d| body.starts_with(d))
    })
}

fn blocks(src: &[u8], lang: Lang) -> Vec<Block> {
    let mut lexer = Lexer {
        s: src,
        i: 0,
        end: src.len(),
        marks: Vec::new(),
    };
    match lang {
        Lang::Rust => lexer.rust(),
        Lang::Js => lexer.js(false),
        Lang::Svelte => lexer.svelte(),
    }
    let newlines: Vec<usize> = (0..src.len()).filter(|&i| src[i] == b'\n').collect();
    let line = |at: usize| newlines.partition_point(|&n| n < at) + 1;
    let mut found: Vec<Block> = Vec::new();
    let mut code_line = 0;
    let mut extendable = false;
    for mark in lexer.marks {
        let (start, end) = match mark {
            Mark::Code(at) => {
                code_line = line(at);
                extendable = false;
                continue;
            }
            Mark::Comment(start, end) => (start, end),
        };
        let text = String::from_utf8_lossy(&src[start..end]).into_owned();
        let (first, last) = (line(start), line(end));
        if is_directive(&text) {
            code_line = last;
            extendable = false;
            continue;
        }
        let trailing = code_line == first;
        match found.last_mut() {
            Some(block) if extendable && !trailing && first <= block.last + 1 => {
                block.last = last;
                block.text.push('\n');
                block.text.push_str(&text);
            }
            _ => found.push(Block { first, last, text }),
        }
        extendable = !trailing;
    }
    found
}

fn defines_fn(file: &str, name: &str) -> bool {
    let needle = format!("fn {name}");
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    file.match_indices(&needle).any(|(at, _)| {
        !file[at + needle.len()..].starts_with(ident) && !file[..at].ends_with(ident)
    })
}

fn proven(root: &Path, block: &Block) -> bool {
    let Some(reference) = block
        .text
        .lines()
        .last()
        .and_then(|line| body(line).strip_prefix("proof:"))
        .map(str::trim)
    else {
        return false;
    };
    if block.last - block.first > 3 {
        return false;
    }
    if let Some(url) = reference.strip_prefix("https://") {
        return !url.is_empty() && !url.contains(char::is_whitespace);
    }
    let Some((path, name)) = reference.split_once("::") else {
        return false;
    };
    let Ok(file) = fs::read_to_string(root.join(path)) else {
        return false;
    };
    if path.ends_with(".rs") {
        defines_fn(&file, name)
    } else {
        !name.is_empty() && file.contains(name)
    }
}

fn lang(path: &Path) -> Option<Lang> {
    match path.extension()?.to_str()? {
        "rs" => Some(Lang::Rust),
        "ts" | "js" | "mjs" | "cjs" => Some(Lang::Js),
        "svelte" => Some(Lang::Svelte),
        _ => None,
    }
}

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && !SKIPPED_DIRS.contains(&&*name) {
                sources(&path, found);
            }
        } else if lang(&path).is_some() {
            found.push(path);
        }
    }
}

fn violations(root: &Path, src: &[u8], lang: Lang) -> Vec<usize> {
    blocks(src, lang)
        .into_iter()
        .filter(|block| !proven(root, block))
        .map(|block| block.first)
        .collect()
}

#[test]
fn every_source_comment_carries_proof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(root, &mut files);
    files.sort();
    let found: Vec<String> = files
        .iter()
        .flat_map(|path| {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let src = fs::read(path).unwrap();
            violations(root, &src, lang(path).unwrap())
                .into_iter()
                .map(move |line| format!("{rel}:{line}"))
        })
        .collect();
    let listed: String = found
        .iter()
        .take(MAX_LISTED)
        .map(|f| f.clone() + "\n")
        .collect();
    let more = found.len().saturating_sub(MAX_LISTED);
    assert!(
        found.is_empty(),
        "{listed}{}{} comment blocks break the rule. {RULE}",
        if more > 0 {
            format!("... and {more} more\n")
        } else {
            String::new()
        },
        found.len()
    );
}

fn flagged(lang: Lang, src: &str) -> Vec<usize> {
    violations(Path::new(env!("CARGO_MANIFEST_DIR")), src.as_bytes(), lang)
}

#[test]
fn rust_literals_hide_comment_markers() {
    let src = r##"let a = "// not /* a comment";
let b = r#"/* " // still a string"#;
let c = (b"//", br#"/*"#, b'/');
let d = ('/', '"', '\'', '\\', 'é');
fn e<'a>(x: &'a str) -> &'a str { x } // one
/* two /* nested */ still two */ let f = 1;
/// three
//! three
"##;
    assert_eq!(flagged(Lang::Rust, src), [5, 6, 7]);
}

#[test]
fn js_literals_hide_comment_markers() {
    let src = r#"const a = '//' + "/*" + `${`//${"*/"}`} // ${b}`;
const re = /\/\//g, half = a / 2; // one
const r2 = x.replace(/[/*]/, "") /* two */;
"#;
    assert_eq!(flagged(Lang::Js, src), [2, 3]);
}

#[test]
fn svelte_counts_html_comments_in_markup_only() {
    let src = r#"<script lang="ts">
  let url = "https://x"; // one
</script>
<a href="https://example.com">// not a comment {url}</a>
<!-- two -->
<!-- svelte-ignore a11y_missing_attribute -->
{#if url}{/if}
<style>
  a::after { content: "/* no */"; } /* three */
</style>
"#;
    assert_eq!(flagged(Lang::Svelte, src), [2, 5, 9]);
}

#[test]
fn blocks_split_on_code_gaps_and_directives() {
    let src = "#!/usr/bin/env node\n// one\n// one\n\n// two\n// @ts-expect-error\nx();\n\
/* three */ /* three */\nx(); // four\n// five\n";
    assert_eq!(flagged(Lang::Js, src), [2, 5, 8, 9, 10]);
}

#[test]
fn proof_line_must_close_a_short_block_and_resolve() {
    let allowed = |src: &str| flagged(Lang::Rust, src).is_empty();
    assert!(allowed("// a fact\n// proof: https://example.com/doc\n"));
    assert!(allowed(
        "// a fact\n// proof: tests/comment_policy.rs::proof_line_must_close_a_short_block_and_resolve\n"
    ));
    assert!(allowed(
        "/* a fact\n   proof: README.md::interval journaling for focus */\n"
    ));
    assert!(!allowed("// proof: tests/absent.rs::f\n"));
    assert!(!allowed("// proof: tests/comment_policy.rs::absent\n"));
    assert!(!allowed(
        "// 1\n// 2\n// 3\n// 4\n// proof: https://example.com\n"
    ));
    assert!(!allowed("// proof: https://example.com\n// a fact\n"));
}
