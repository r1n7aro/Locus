//! Pure helpers of the LaTeX preview pipeline: engine choice, routing, the
//! SVG driver preamble, dependency and log parsing, XDV inspection and SVG
//! post-processing. Process execution lives in `build.rs`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Xelatex,
    Pdflatex,
    Lualatex,
}

impl Engine {
    pub const fn program(self) -> &'static str {
        match self {
            Self::Xelatex => "xelatex",
            Self::Pdflatex => "pdflatex",
            Self::Lualatex => "lualatex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "xelatex" | "xetex" => Some(Self::Xelatex),
            "pdflatex" | "pdftex" | "latex" => Some(Self::Pdflatex),
            "lualatex" | "luatex" | "lualatexmk" => Some(Self::Lualatex),
            _ => None,
        }
    }

    /// Extension of the engine's DVI-family output in SVG mode.
    pub const fn dvi_extension(self) -> &'static str {
        match self {
            Self::Xelatex => "xdv",
            Self::Pdflatex | Self::Lualatex => "dvi",
        }
    }
}

/// Packages that only work with PDF output (or explicitly reject the
/// dvisvgm driver). A document loading one of them goes straight to the PDF
/// path instead of failing the SVG build first.
const PDF_ONLY_PACKAGES: &[&str] = &[
    "pdfpages",
    "luatexja",
    "luatexja-fontspec",
    "luatexja-preset",
    "luatexja-otf",
    "hypgotoe",
    "bmpsize",
    "pdfcrypt",
    "animate",
    "media9",
    "movie15",
    "ocgx",
    "ocgx2",
    "pdfcomment",
    "attachfile",
    "attachfile2",
    "pdfx",
];

/// Packages that imply a LuaTeX document when no engine is requested.
const LUA_PACKAGES: &[&str] = &["luacode", "luatexja", "luatexja-fontspec", "luatexja-preset", "luamplib"];

/// Packages that mark a Unicode-font (XeTeX/LuaTeX) document.
const UNICODE_FONT_PACKAGES: &[&str] = &["fontspec", "xecjk", "ctex", "unicode-math", "polyglossia", "xeCJK"];

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid built-in regex"))
}

/// Removes `%` comments (an escaped `\%` stays) so commented-out packages and
/// magic text do not influence routing.
pub fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let bytes = line.as_bytes();
        let mut cut = line.len();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if bytes[index] == b'%' {
                cut = index;
                break;
            }
            index += 1;
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// Text before `\begin{document}`, comments removed. `None` when the file is
/// not a root document (no `\documentclass`).
pub fn preamble(source: &str) -> Option<String> {
    let clean = strip_comments(source);
    if !clean.contains("\\documentclass") {
        return None;
    }
    let end = clean.find("\\begin{document}").unwrap_or(clean.len());
    Some(clean[..end].to_string())
}

fn magic_value(source: &str, key: &str) -> Option<String> {
    static MAGIC: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(
        &MAGIC,
        r"(?i)^\s*%+\s*!\s*TEX\s+(?:TS-)?([A-Za-z]+)\s*=\s*(.+?)\s*$",
    );
    source.lines().take(30).find_map(|line| {
        let captures = pattern.captures(line)?;
        captures[1]
            .eq_ignore_ascii_case(key)
            .then(|| captures[2].trim().to_string())
    })
}

/// `% !TEX program = xelatex` (also the `TS-program` spelling).
pub fn magic_engine(source: &str) -> Option<Engine> {
    magic_value(source, "program").and_then(|value| Engine::parse(&value))
}

/// `% !TEX root = ../main.tex`: the file is part of another document.
pub fn magic_root(source: &str) -> Option<String> {
    magic_value(source, "root").filter(|value| !value.is_empty())
}

/// Package names loaded by `\usepackage` / `\RequirePackage` in `preamble`.
pub fn used_packages(preamble: &str) -> Vec<String> {
    static USE: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(
        &USE,
        r"\\(?:usepackage|RequirePackage)\s*(?:\[[^\]]*\])?\s*\{([^}]*)\}",
    );
    let mut names = Vec::new();
    for captures in pattern.captures_iter(preamble) {
        for name in captures[1].split(',') {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|known: &String| known == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Document class name, if declared.
pub fn document_class(preamble: &str) -> Option<String> {
    static CLASS: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(&CLASS, r"\\documentclass\s*(?:\[[^\]]*\])?\s*\{([^}]*)\}");
    pattern
        .captures(preamble)
        .map(|captures| captures[1].trim().to_string())
}

/// Engine for a document: a magic comment wins, then an explicit setting;
/// `auto` picks LuaLaTeX for Lua-only packages, pdfLaTeX for classic
/// `inputenc` documents without Unicode font packages, and XeLaTeX
/// otherwise (Unicode text and CJK work there without configuration).
pub fn choose_engine(setting: &str, source: &str) -> Engine {
    if let Some(engine) = magic_engine(source) {
        return engine;
    }
    if let Some(engine) = Engine::parse(setting) {
        return engine;
    }
    let Some(preamble) = preamble(source) else {
        return Engine::Xelatex;
    };
    let packages = used_packages(&preamble);
    let has = |list: &[&str]| {
        packages
            .iter()
            .any(|name| list.iter().any(|candidate| candidate.eq_ignore_ascii_case(name)))
    };
    let class = document_class(&preamble).unwrap_or_default();
    if has(LUA_PACKAGES) || class.starts_with("ltj") || preamble.contains("\\directlua") {
        return Engine::Lualatex;
    }
    if packages.iter().any(|name| name == "inputenc") && !has(UNICODE_FONT_PACKAGES) && !class.starts_with("ctex") {
        return Engine::Pdflatex;
    }
    Engine::Xelatex
}

/// Why a document skips the SVG path, decided from its preamble before any
/// build. The value is a stable code shown to the user and the agent.
pub fn pdf_only_reason(preamble: &str, engine: Engine) -> Option<String> {
    if engine == Engine::Lualatex {
        return Some("engine:lualatex".to_string());
    }
    if let Some(class) = document_class(preamble) {
        if class.starts_with("ltj") {
            return Some(format!("class:{class}"));
        }
    }
    if let Some(name) = used_packages(preamble)
        .into_iter()
        .find(|name| PDF_ONLY_PACKAGES.contains(&name.as_str()))
    {
        return Some(format!("package:{name}"));
    }
    static PDFOUTPUT: OnceLock<Regex> = OnceLock::new();
    if regex(&PDFOUTPUT, r"\\pdfoutput\s*=\s*1").is_match(preamble) {
        return Some("pdfoutput".to_string());
    }
    None
}

/// Preamble read before the document in SVG builds. It switches graphics,
/// colour and PGF to the dvisvgm driver without editing the source, so the
/// PDF build of the same file keeps its normal drivers. hyperref link boxes
/// are hidden because SVG has no annotation layer to keep them out of print.
pub fn svg_driver_preamble() -> String {
    [
        r"\def\pgfsysdriver{pgfsys-dvisvgm.def}",
        r"\PassOptionsToPackage{dvisvgm}{graphicx}",
        r"\PassOptionsToPackage{dvisvgm}{graphics}",
        r"\PassOptionsToPackage{dvisvgm}{xcolor}",
        r"\PassOptionsToPackage{dvisvgm}{color}",
        r"\ifdefined\AddToHook\AddToHook{package/hyperref/after}{\hypersetup{hidelinks}}\fi",
        // pdfLaTeX's DVI carries no page size; dvisvgm needs it for the
        // paper-sized page box (a later special from geometry wins).
        r"\AtBeginDvi{\special{papersize=\the\paperwidth,\the\paperheight}}",
        "",
    ]
    .join("\n")
}

/// Path as TeX expects it on the command line: forward slashes, quoted when
/// it contains spaces.
pub fn tex_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text
    }
}

/// `INPUT` entries of a `-recorder` `.fls` file, resolved against its `PWD`.
pub fn recorded_inputs(fls: &str, fallback_dir: &Path) -> Vec<PathBuf> {
    recorded_files(fls, fallback_dir, "INPUT ")
}

/// `OUTPUT` entries: files the build itself wrote. Packages such as
/// `filecontents` write next to the source, and those writes must not
/// trigger another build.
pub fn recorded_outputs(fls: &str, fallback_dir: &Path) -> Vec<PathBuf> {
    recorded_files(fls, fallback_dir, "OUTPUT ")
}

fn recorded_files(fls: &str, fallback_dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut pwd = fallback_dir.to_path_buf();
    let mut seen = BTreeSet::new();
    let mut inputs = Vec::new();
    for line in fls.lines() {
        if let Some(dir) = line.strip_prefix("PWD ") {
            pwd = PathBuf::from(dir.trim());
        } else if let Some(input) = line.strip_prefix(prefix) {
            let input = input.trim();
            if input.is_empty() {
                continue;
            }
            let path = PathBuf::from(input);
            let path = if path.is_absolute() { path } else { pwd.join(path) };
            let key = normalize_key(&path);
            if seen.insert(key) {
                inputs.push(path);
            }
        }
    }
    inputs
}

/// Case/separator-insensitive comparison key for paths on Windows.
pub fn normalize_key(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    while text.contains("/./") {
        text = text.replace("/./", "/");
    }
    if let Some(stripped) = text.strip_prefix("./") {
        text = stripped.to_string();
    }
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// `error` or `warning`.
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub message: String,
    /// Source excerpt TeX printed after the error (`l.12 ...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSummary {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub rerun: bool,
}

const MAX_DIAGNOSTICS: usize = 20;

/// Errors, warnings and the rerun request of a TeX log written with
/// `-file-line-error`.
pub fn parse_log(log: &str) -> LogSummary {
    static FILE_LINE: OnceLock<Regex> = OnceLock::new();
    static BANG: OnceLock<Regex> = OnceLock::new();
    static CONTEXT: OnceLock<Regex> = OnceLock::new();
    static WARNING: OnceLock<Regex> = OnceLock::new();
    static ON_LINE: OnceLock<Regex> = OnceLock::new();
    static RERUN: OnceLock<Regex> = OnceLock::new();
    let file_line = regex(&FILE_LINE, r"^(.+?\.[A-Za-z0-9]+):(\d+): (.+)$");
    let bang = regex(&BANG, r"^! (.+)$");
    let context = regex(&CONTEXT, r"^l\.(\d+) ?(.*)$");
    let warning = regex(
        &WARNING,
        r"^(?:LaTeX|Package (\S+)|Class (\S+)) Warning: (.+)$",
    );
    let on_line = regex(&ON_LINE, r"on input line (\d+)");
    let rerun = regex(
        &RERUN,
        r"(?i)(Rerun to get|Label\(s\) may have changed|Please rerun|Rerun LaTeX|rerunfilecheck.*rerun)",
    );

    let lines = log.lines().collect::<Vec<_>>();
    let mut summary = LogSummary {
        rerun: rerun.is_match(log),
        ..LogSummary::default()
    };
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let error = if let Some(captures) = file_line.captures(line) {
            Some(Diagnostic {
                severity: "error".to_string(),
                file: Some(captures[1].to_string()),
                line: captures[2].parse().ok(),
                message: captures[3].trim().to_string(),
                context: None,
            })
        } else {
            bang.captures(line).map(|captures| Diagnostic {
                severity: "error".to_string(),
                file: None,
                line: None,
                message: captures[1].trim().to_string(),
                context: None,
            })
        };
        if let Some(mut error) = error {
            // TeX prints `l.<n> <source>` within a few lines of the error.
            for follow in lines.iter().skip(index + 1).take(8) {
                if let Some(captures) = context.captures(follow) {
                    if error.line.is_none() {
                        error.line = captures[1].parse().ok();
                    }
                    let excerpt = captures[2].trim();
                    if !excerpt.is_empty() {
                        error.context = Some(excerpt.to_string());
                    }
                    break;
                }
            }
            if summary.errors.len() < MAX_DIAGNOSTICS
                && !summary.errors.iter().any(|known| known == &error)
            {
                summary.errors.push(error);
            }
        } else if let Some(captures) = warning.captures(line) {
            if summary.warnings.len() < MAX_DIAGNOSTICS {
                let mut message = captures[3].trim().to_string();
                // Warnings continue on indented lines until a blank line.
                for follow in lines.iter().skip(index + 1).take(4) {
                    let follow = follow.trim();
                    if follow.is_empty() || !follow.starts_with('(') {
                        break;
                    }
                    if let Some(rest) = follow.split_once(')') {
                        message.push(' ');
                        message.push_str(rest.1.trim());
                    }
                }
                let source = captures
                    .get(1)
                    .or_else(|| captures.get(2))
                    .map(|name| format!("{}: ", name.as_str()))
                    .unwrap_or_default();
                summary.warnings.push(Diagnostic {
                    severity: "warning".to_string(),
                    file: None,
                    line: on_line
                        .captures(&message)
                        .and_then(|found| found[1].parse().ok()),
                    message: format!("{source}{message}"),
                    context: None,
                });
            }
        }
        index += 1;
    }
    summary
}

/// `pdf:` special commands that dvisvgm either handles (links, page size) or
/// that carry document metadata only. Anything else in an XDV (literal
/// drawing code, PDF colour stacks, XObjects, transforms) is silently dropped
/// by dvisvgm, so its presence means the SVG would miss content.
const SAFE_PDF_SPECIALS: &[&str] = &[
    "ann",
    "bann",
    "eann",
    "dest",
    "docinfo",
    "docview",
    "pagesize",
    "majorversion",
    "minorversion",
    "outline",
    "out",
    "bookmark",
    "link",
    "names",
    "mapline",
    "mapfile",
    "tounicode",
    "obj",
    "put",
    "close",
];

/// Distinct `pdf:` specials in a DVI/XDV file that dvisvgm cannot render.
pub fn unsupported_pdf_specials(bytes: &[u8]) -> Vec<String> {
    let needle = b"pdf:";
    let mut found = BTreeSet::new();
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] != needle {
            index += 1;
            continue;
        }
        let mut cursor = index + needle.len();
        while cursor < bytes.len() && bytes[cursor] == b' ' {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
            cursor += 1;
        }
        if cursor > start {
            let word = String::from_utf8_lossy(&bytes[start..cursor]).to_ascii_lowercase();
            if !SAFE_PDF_SPECIALS.contains(&word.as_str()) {
                found.insert(word);
            }
        }
        index = cursor.max(index + 1);
    }
    found.into_iter().collect()
}

/// dvisvgm warnings that mean content was dropped (missing Ghostscript for
/// PostScript specials, unsupported specials).
pub fn dvisvgm_content_warnings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            // A missing page size only changes the page box, not content.
            lower.contains("warning")
                && !lower.contains("papersize")
                && (lower.contains("special") || lower.contains("ghostscript") || lower.contains("postscript"))
        })
        .map(str::to_string)
        .take(5)
        .collect()
}

fn bitmap_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("svg") => Some("image/svg+xml"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// Replaces file references of `<image>` elements with data URIs so the page
/// renders standalone (an SVG shown through `<img>` cannot load external
/// files). Returns the rewritten SVG and the files it embedded, which become
/// dependencies of the document.
pub fn embed_bitmaps(svg: &str, base_dir: &Path) -> (String, Vec<PathBuf>) {
    use base64::Engine as _;
    static HREF: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(&HREF, r#"(xlink:href|href)=(['"])([^'"]+)(['"])"#);
    let mut embedded = Vec::new();
    let rewritten = pattern.replace_all(svg, |captures: &regex::Captures<'_>| {
        let original = captures[0].to_string();
        let target = &captures[3];
        if target.starts_with('#') || target.starts_with("data:") || target.contains("://") {
            return original;
        }
        let candidate = PathBuf::from(target);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            base_dir.join(candidate)
        };
        let Some(mime) = bitmap_mime(&path) else {
            return original;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return original;
        };
        embedded.push(path);
        format!(
            "{}={}data:{};base64,{}{}",
            &captures[1],
            &captures[2],
            mime,
            base64::engine::general_purpose::STANDARD.encode(bytes),
            &captures[4]
        )
    });
    (rewritten.into_owned(), embedded)
}

/// Page size in points from the root `<svg width='612pt' height='792pt'>`.
pub fn svg_page_size(svg: &str) -> Option<(f64, f64)> {
    static SIZE: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(
        &SIZE,
        r#"<svg\b[^>]*?\bwidth=['"]([0-9.]+)(?:pt)?['"][^>]*?\bheight=['"]([0-9.]+)(?:pt)?['"]"#,
    );
    let captures = pattern.captures(svg)?;
    Some((captures[1].parse().ok()?, captures[2].parse().ok()?))
}

/// `p<N>.svg` files of `dir`, ordered by page number.
pub fn page_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut pages = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let number = name.strip_prefix('p')?.strip_suffix(".svg")?.parse::<u32>().ok()?;
            Some((number, entry.path()))
        })
        .collect::<Vec<_>>();
    pages.sort_by_key(|(number, _)| *number);
    pages.into_iter().map(|(_, path)| path).collect()
}

/// Build artefacts a document's own compile writes next to the source. Their
/// changes must not trigger a preview rebuild.
pub fn is_build_artefact(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    const SUFFIXES: &[&str] = &[
        ".aux", ".log", ".out", ".toc", ".lof", ".lot", ".fls", ".fdb_latexmk", ".synctex.gz",
        ".synctex", ".xdv", ".dvi", ".bbl", ".blg", ".bcf", ".run.xml", ".nav", ".snm", ".vrb",
        ".idx", ".ilg", ".ind", ".glo", ".gls", ".glg", ".ist", ".thm", ".loa", ".lol", ".tmp",
    ];
    SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// File types whose appearance or change in a watched directory can affect
/// the document (sources, local packages, bibliography data, images).
pub fn is_relevant_source(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "tex" | "ltx" | "sty" | "cls" | "def" | "cfg" | "bib" | "bst" | "bbx" | "cbx" | "lbx"
            | "png" | "jpg" | "jpeg" | "gif" | "pdf" | "eps" | "svg" | "tikz" | "pgf" | "dat"
            | "csv" | "txt" | "lua" | "ttf" | "otf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_but_keeps_escaped_percent() {
        let text = "a % comment\n50\\% off % note\n";
        assert_eq!(strip_comments(text), "a \n50\\% off \n");
    }

    #[test]
    fn reads_magic_comments() {
        let source = "% !TEX program = lualatex\n%!TEX root = ../main.tex\n\\documentclass{article}";
        assert_eq!(magic_engine(source), Some(Engine::Lualatex));
        assert_eq!(magic_root(source).as_deref(), Some("../main.tex"));
        assert_eq!(magic_engine("% !TEX TS-program = xelatex\n"), Some(Engine::Xelatex));
        assert_eq!(magic_root("\\documentclass{article}"), None);
    }

    #[test]
    fn chooses_engine_from_magic_setting_and_preamble() {
        let classic = "\\documentclass{article}\n\\usepackage[utf8]{inputenc}\n\\begin{document}x\\end{document}";
        let cjk = "\\documentclass{ctexart}\n\\usepackage[utf8]{inputenc}\n\\begin{document}x\\end{document}";
        let lua = "\\documentclass{article}\n\\usepackage{luacode}\n\\begin{document}x\\end{document}";
        let plain = "\\documentclass{article}\n\\begin{document}x\\end{document}";
        assert_eq!(choose_engine("auto", classic), Engine::Pdflatex);
        assert_eq!(choose_engine("auto", cjk), Engine::Xelatex);
        assert_eq!(choose_engine("auto", lua), Engine::Lualatex);
        assert_eq!(choose_engine("auto", plain), Engine::Xelatex);
        assert_eq!(choose_engine("pdflatex", plain), Engine::Pdflatex);
        let magic = format!("% !TEX program = lualatex\n{plain}");
        assert_eq!(choose_engine("pdflatex", &magic), Engine::Lualatex);
    }

    #[test]
    fn lists_packages_and_routes_pdf_only_documents() {
        let preamble = "\\documentclass[11pt]{ctexart}\n\\usepackage[margin=1in]{geometry}\n\\usepackage{amsmath, graphicx,pdfpages}\n% \\usepackage{animate}\n";
        let clean = super::preamble(&format!("{preamble}\\begin{{document}}")).unwrap();
        assert_eq!(used_packages(&clean), vec!["geometry", "amsmath", "graphicx", "pdfpages"]);
        assert_eq!(document_class(&clean).as_deref(), Some("ctexart"));
        assert_eq!(pdf_only_reason(&clean, Engine::Xelatex).as_deref(), Some("package:pdfpages"));
        assert_eq!(pdf_only_reason("\\documentclass{article}", Engine::Lualatex).as_deref(), Some("engine:lualatex"));
        assert_eq!(pdf_only_reason("\\documentclass{ltjsarticle}", Engine::Xelatex).as_deref(), Some("class:ltjsarticle"));
        assert_eq!(pdf_only_reason("\\documentclass{article}\\pdfoutput=1", Engine::Pdflatex).as_deref(), Some("pdfoutput"));
        assert_eq!(pdf_only_reason("\\documentclass{article}\\usepackage{tikz}", Engine::Xelatex), None);
        assert!(super::preamble("\\section{Only a chapter}").is_none());
    }

    #[test]
    fn driver_preamble_switches_every_graphics_driver() {
        let text = svg_driver_preamble();
        for needle in [
            "pgfsys-dvisvgm.def",
            "{dvisvgm}{graphicx}",
            "{dvisvgm}{xcolor}",
            "hidelinks",
            "papersize=",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        // Single backslashes: this is TeX source, not an escaped string.
        assert!(text.contains("\\PassOptionsToPackage") && !text.contains("\\\\Pass"));
    }

    #[test]
    fn parses_recorder_inputs_relative_to_pwd() {
        let fls = "PWD C:/docs/thesis\nINPUT C:/texmf/tex/latex/base/article.cls\nINPUT ./chapters/intro.tex\nINPUT chapters/intro.tex\nOUTPUT main.aux\nINPUT img/a.png\n";
        let inputs = recorded_inputs(fls, Path::new("C:/fallback"));
        let keys = inputs.iter().map(|path| normalize_key(path)).collect::<Vec<_>>();
        assert_eq!(keys.len(), 3);
        assert!(keys[1].ends_with("docs/thesis/chapters/intro.tex"));
        assert!(keys[2].ends_with("docs/thesis/img/a.png"));
        let outputs = recorded_outputs(fls, Path::new("C:/fallback"));
        assert_eq!(outputs.len(), 1);
        assert!(normalize_key(&outputs[0]).ends_with("docs/thesis/main.aux"));
    }

    #[test]
    fn parses_file_line_errors_warnings_and_rerun() {
        let log = "This is XeTeX\n./main.tex:12: Undefined control sequence.\nl.12 \\foo\n          bar\nLaTeX Warning: Reference `sec:a' on page 1 undefined on input line 7.\n\nPackage hyperref Warning: Token not allowed\n(hyperref)                removing `math shift' on input line 9.\n\nLaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.\n";
        let summary = parse_log(log);
        assert_eq!(summary.errors.len(), 1);
        let error = &summary.errors[0];
        assert_eq!(error.file.as_deref(), Some("./main.tex"));
        assert_eq!(error.line, Some(12));
        assert_eq!(error.message, "Undefined control sequence.");
        assert_eq!(error.context.as_deref(), Some("\\foo"));
        assert!(summary.rerun);
        assert_eq!(summary.warnings[0].line, Some(7));
        assert!(summary.warnings[1].message.starts_with("hyperref: Token not allowed"));
        assert_eq!(summary.warnings[1].line, Some(9));

        let windows = parse_log("C:/Users/a b/doc.tex:3: LaTeX Error: File `x.sty' not found.\n");
        assert_eq!(windows.errors[0].file.as_deref(), Some("C:/Users/a b/doc.tex"));
        assert_eq!(windows.errors[0].line, Some(3));

        let bang = parse_log("! Emergency stop.\n<*> main.tex\n");
        assert_eq!(bang.errors[0].message, "Emergency stop.");
        assert!(!bang.rerun);
    }

    #[test]
    fn finds_unsupported_pdf_specials_only() {
        let wrapped = b"\x00pdf:bann <<>>\x00pdf:dest (a)\x00pdf:docinfo\x00pdf:pagesize width 1pt\x00";
        assert!(unsupported_pdf_specials(wrapped).is_empty());
        let raw = b"pdf: bcontent\x00pdf:code q 1 0 0 1 cm\x00pdf:bcolor [1 0 0]\x00pdf:code Q\x00";
        assert_eq!(unsupported_pdf_specials(raw), vec!["bcolor", "bcontent", "code"]);
    }

    #[test]
    fn detects_dropped_content_warnings() {
        let stderr = "pre-processing DVI file (format version 7)\nWARNING: processing of PostScript specials is disabled (Ghostscript not found)\n";
        assert_eq!(dvisvgm_content_warnings(stderr).len(), 1);
        assert!(dvisvgm_content_warnings("1 of 1 page converted in 0.1 seconds").is_empty());
        assert!(dvisvgm_content_warnings("WARNING: no valid papersize special found").is_empty());
    }

    #[test]
    fn embeds_relative_bitmaps_as_data_uris() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("img.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let svg = "<svg><image width='1' xlink:href='img.png'/><use xlink:href='#g1'/><image href=\"missing.png\"/></svg>";
        let (out, embedded) = embed_bitmaps(svg, temp.path());
        assert!(out.contains("xlink:href='data:image/png;base64,iVBORw=='"));
        assert!(out.contains("xlink:href='#g1'"));
        assert!(out.contains("href=\"missing.png\""));
        assert_eq!(embedded.len(), 1);
    }

    #[test]
    fn reads_page_size_and_orders_pages() {
        let svg = "<?xml version='1.0'?>\n<svg version='1.1' xmlns='http://www.w3.org/2000/svg' width='612pt' height='792pt' viewBox='0 0 612 792'>";
        assert_eq!(svg_page_size(svg), Some((612.0, 792.0)));
        let temp = tempfile::tempdir().unwrap();
        for name in ["p10.svg", "p2.svg", "p1.svg", "other.svg"] {
            std::fs::write(temp.path().join(name), "").unwrap();
        }
        let names = page_files(temp.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["p1.svg", "p2.svg", "p10.svg"]);
    }

    #[test]
    fn filters_artefacts_and_relevant_sources() {
        assert!(is_build_artefact(Path::new("main.synctex.gz")));
        assert!(is_build_artefact(Path::new("main.aux")));
        assert!(!is_build_artefact(Path::new("main.tex")));
        assert!(is_relevant_source(Path::new("fig/plot.PNG")));
        assert!(!is_relevant_source(Path::new("notes.docx")));
    }
}
