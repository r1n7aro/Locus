//! One preview build of one document: the SVG path (DVI/XDV with the
//! dvisvgm driver, then dvisvgm) and the PDF path, each with its own work
//! directory so their auxiliary files never mix. Dropping a running build
//! future kills its TeX process tree (managed processes live in a job that
//! closes with the handle).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;

use super::pipeline::{self, Diagnostic, Engine, LogSummary};
use super::toolchain::Toolchain;

/// TeX passes per build: the first pass of a fresh work directory usually
/// needs a second one for cross-references and the table of contents.
const MAX_PASSES: u32 = 3;
const PASS_TIMEOUT: Duration = Duration::from_secs(180);
const DVISVGM_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub struct BuildInput {
    /// Root document compiled for the preview.
    pub source: PathBuf,
    /// Working directory of TeX and dvisvgm (relative inputs and images
    /// resolve from here).
    pub source_dir: PathBuf,
    pub jobname: String,
    pub engine: Engine,
    pub toolchain: Toolchain,
    pub allow_package_install: bool,
    /// `<doc output>/work`.
    pub work_dir: PathBuf,
    /// File holding [`pipeline::svg_driver_preamble`].
    pub driver_preamble: PathBuf,
    /// Process ownership for diagnostics and shutdown.
    pub owner_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Svg,
    Pdf,
}

impl Mode {
    const fn dir(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Debug, Clone)]
struct TexRun {
    ok: bool,
    summary: LogSummary,
    output: PathBuf,
    deps: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum SvgResult {
    Ready {
        /// Self-contained page files (bitmaps embedded), in page order.
        pages: Vec<PathBuf>,
        deps: Vec<PathBuf>,
        warnings: Vec<Diagnostic>,
        elapsed_ms: u64,
    },
    /// The document compiled, but the SVG would miss content; the PDF path
    /// shows it instead. `reason` is a stable code.
    Unsupported {
        reason: String,
        detail: String,
        deps: Vec<PathBuf>,
    },
    Failed {
        errors: Vec<Diagnostic>,
        deps: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub enum PdfResult {
    Ready {
        pdf: PathBuf,
        deps: Vec<PathBuf>,
        warnings: Vec<Diagnostic>,
        elapsed_ms: u64,
    },
    Failed {
        errors: Vec<Diagnostic>,
        deps: Vec<PathBuf>,
    },
}

fn tool_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: "error".to_string(),
        file: None,
        line: None,
        message: message.into(),
        context: None,
    }
}

async fn run_tex(input: &BuildInput, mode: Mode) -> TexRun {
    let dir = input.work_dir.join(mode.dir());
    let output = dir.join(format!(
        "{}.{}",
        input.jobname,
        match mode {
            Mode::Svg => input.engine.dvi_extension(),
            Mode::Pdf => "pdf",
        }
    ));
    let failed = |errors: Vec<Diagnostic>| TexRun {
        ok: false,
        summary: LogSummary {
            errors,
            ..LogSummary::default()
        },
        output: output.clone(),
        deps: Vec::new(),
    };
    if let Err(error) = tokio::fs::create_dir_all(&dir).await {
        return failed(vec![tool_error(format!("cannot create {}: {error}", dir.display()))]);
    }
    let Some(program) = input.toolchain.program(input.engine).map(str::to_string) else {
        return failed(vec![tool_error(format!(
            "{} was not found. Install a TeX distribution (MiKTeX or TeX Live) or change the LaTeX engine in Settings.",
            input.engine.program()
        ))]);
    };
    let tex_input = match mode {
        Mode::Svg => format!(
            "\\input{{{}}}\\input{{{}}}",
            pipeline::tex_path(&input.driver_preamble),
            pipeline::tex_path(&input.source)
        ),
        Mode::Pdf => format!("\\input{{{}}}", pipeline::tex_path(&input.source)),
    };

    let mut summary = LogSummary::default();
    let mut ok = false;
    for pass in 1..=MAX_PASSES {
        let _ = tokio::fs::remove_file(&output).await;
        let mut command = crate::process_util::async_command(&program);
        command
            .current_dir(&input.source_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Unwrapped log lines keep file:line errors on one line.
            .env("max_print_line", "10000")
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg("-file-line-error")
            .arg("-recorder")
            // Shell escape stays at the distribution default (restricted):
            // the dvisvgm graphics driver runs the whitelisted `extractbb`
            // for bitmap sizes, while arbitrary commands remain blocked.
            .arg(format!("-output-directory={}", dir.to_string_lossy()))
            .arg(format!("-jobname={}", input.jobname));
        if input.toolchain.is_miktex() {
            command.arg("--max-print-line=10000");
            if !input.allow_package_install {
                command.arg("--disable-installer");
            }
        }
        if mode == Mode::Svg {
            match input.engine {
                Engine::Xelatex => {
                    command.arg("-no-pdf");
                }
                Engine::Pdflatex | Engine::Lualatex => {
                    command.arg("-output-format=dvi");
                }
            }
        }
        command.arg(&tex_input);
        let owner = crate::process_util::ProcessOwner {
            working_dir: Some(input.owner_dir.clone()),
            ..Default::default()
        };
        let mut child = match crate::process_util::spawn_managed(command, owner) {
            Ok(child) => child,
            Err(error) => return failed(vec![tool_error(format!("failed to start {program}: {error}"))]),
        };
        let status = match tokio::time::timeout(PASS_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => return failed(vec![tool_error(format!("{program} failed: {error}"))]),
            Err(_) => {
                let _ = child.terminate_tree();
                return failed(vec![tool_error(format!(
                    "{} did not finish within {} s (pass {pass}).",
                    input.engine.program(),
                    PASS_TIMEOUT.as_secs()
                ))]);
            }
        };
        let log_path = dir.join(format!("{}.log", input.jobname));
        let log = tokio::fs::read(&log_path)
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        summary = pipeline::parse_log(&log);
        ok = status.success() && output.is_file();
        if !ok {
            if summary.errors.is_empty() {
                summary.errors.push(tool_error(if log.is_empty() {
                    format!("{} exited with {status} and wrote no log.", input.engine.program())
                } else {
                    format!("{} exited with {status}.", input.engine.program())
                }));
            }
            break;
        }
        if !summary.rerun {
            break;
        }
    }
    let fls = tokio::fs::read(dir.join(format!("{}.fls", input.jobname)))
        .await
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    TexRun {
        ok,
        summary,
        output,
        deps: pipeline::recorded_inputs(&fls, &input.source_dir),
    }
}

pub async fn run_svg(input: &BuildInput) -> SvgResult {
    let started = Instant::now();
    if let Some(parent) = input.driver_preamble.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(error) =
        tokio::fs::write(&input.driver_preamble, pipeline::svg_driver_preamble()).await
    {
        return SvgResult::Failed {
            errors: vec![tool_error(format!("cannot write the SVG driver preamble: {error}"))],
            deps: Vec::new(),
        };
    }
    let run = run_tex(input, Mode::Svg).await;
    if !run.ok {
        return SvgResult::Failed {
            errors: run.summary.errors,
            deps: run.deps,
        };
    }
    let mut deps = run.deps;
    let bytes = tokio::fs::read(&run.output).await.unwrap_or_default();
    let specials = pipeline::unsupported_pdf_specials(&bytes);
    if !specials.is_empty() {
        return SvgResult::Unsupported {
            reason: "pdf_specials".to_string(),
            detail: specials.join(", "),
            deps,
        };
    }
    let Some(dvisvgm) = input.toolchain.dvisvgm.clone() else {
        return SvgResult::Unsupported {
            reason: "dvisvgm_missing".to_string(),
            detail: String::new(),
            deps,
        };
    };
    let pages_dir = input.work_dir.join("svg-pages");
    let _ = tokio::fs::remove_dir_all(&pages_dir).await;
    if let Err(error) = tokio::fs::create_dir_all(&pages_dir).await {
        return SvgResult::Failed {
            errors: vec![tool_error(format!("cannot create {}: {error}", pages_dir.display()))],
            deps,
        };
    }
    let (ok, stderr) = run_dvisvgm(input, &dvisvgm, &run.output, &pages_dir).await;
    if !ok {
        return SvgResult::Unsupported {
            reason: "dvisvgm_failed".to_string(),
            detail: last_lines(&stderr, 4),
            deps,
        };
    }
    let dropped = pipeline::dvisvgm_content_warnings(&stderr);
    if !dropped.is_empty() {
        return SvgResult::Unsupported {
            reason: "dvisvgm_dropped_content".to_string(),
            detail: dropped.join("\n"),
            deps,
        };
    }
    let pages = pipeline::page_files(&pages_dir);
    if pages.is_empty() {
        return SvgResult::Unsupported {
            reason: "no_pages".to_string(),
            detail: last_lines(&stderr, 4),
            deps,
        };
    }
    for page in &pages {
        let Ok(svg) = tokio::fs::read_to_string(page).await else {
            continue;
        };
        let (embedded, images) = pipeline::embed_bitmaps(&svg, &input.source_dir);
        if !images.is_empty() {
            let _ = tokio::fs::write(page, embedded).await;
            deps.extend(images);
        }
    }
    SvgResult::Ready {
        pages,
        deps,
        warnings: run.summary.warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

async fn run_dvisvgm(input: &BuildInput, program: &str, dvi: &Path, pages_dir: &Path) -> (bool, String) {
    let mut command = crate::process_util::async_command(program);
    command
        .current_dir(&input.source_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .arg("--no-fonts")
        .arg("--bbox=papersize")
        .arg("--linkmark=none")
        .arg("--page=1-")
        .arg(format!("--output={}", pages_dir.join("p%p.svg").to_string_lossy()))
        .arg(dvi);
    let owner = crate::process_util::ProcessOwner {
        working_dir: Some(input.owner_dir.clone()),
        ..Default::default()
    };
    let mut child = match crate::process_util::spawn_managed(command, owner) {
        Ok(child) => child,
        Err(error) => return (false, format!("failed to start dvisvgm: {error}")),
    };
    let mut stderr = String::new();
    let mut pipe = child.take_stderr();
    let finished = tokio::time::timeout(DVISVGM_TIMEOUT, async {
        if let Some(pipe) = pipe.as_mut() {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes).await;
            stderr = String::from_utf8_lossy(&bytes).into_owned();
        }
        child.wait().await
    })
    .await;
    match finished {
        Ok(Ok(status)) => (status.success(), stderr),
        Ok(Err(error)) => (false, format!("dvisvgm failed: {error}")),
        Err(_) => {
            let _ = child.terminate_tree();
            (false, format!("dvisvgm did not finish within {} s", DVISVGM_TIMEOUT.as_secs()))
        }
    }
}

pub async fn run_pdf(input: &BuildInput) -> PdfResult {
    let started = Instant::now();
    let run = run_tex(input, Mode::Pdf).await;
    if run.ok {
        PdfResult::Ready {
            pdf: run.output,
            deps: run.deps,
            warnings: run.summary.warnings,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    } else {
        PdfResult::Failed {
            errors: run.summary.errors,
            deps: run.deps,
        }
    }
}

fn last_lines(text: &str, count: usize) -> String {
    let lines = text.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// Real TeX runs. Ignored by default because they need a local TeX
/// distribution with ctex, TikZ, pdfpages and dvisvgm:
/// `cargo test --lib latex_preview::build::live_tests -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    const MIXED: &str = r#"\documentclass{ctexart}
\usepackage{graphicx,xcolor,tikz,hyperref}
\begin{document}
\section{预览}\label{sec:a}
见第~\ref{sec:a}~节，\textcolor{red}{红色}，\href{https://example.com}{链接}。
\includegraphics[width=1cm]{img.png}
\begin{tikzpicture}\shade[left color=yellow,right color=green](0,0) rectangle (2,0.6);\end{tikzpicture}
\end{document}
"#;

    const PDFPAGES: &str = r#"\documentclass{article}
\usepackage{pdfpages}
\begin{document}
Before \includepdf[pages=1]{insert.pdf}
\end{document}
"#;

    const BROKEN: &str = r#"\documentclass{article}
\begin{document}
Text
\undefinedmacro
\end{document}
"#;

    const PDFLATEX_TIKZ: &str = r#"\documentclass{article}
\usepackage{tikz}
\begin{document}
\begin{tikzpicture}\shade[left color=blue,right color=white](0,0) rectangle (2,1);\end{tikzpicture}
\end{document}
"#;

    struct Fixture {
        _temp: tempfile::TempDir,
        input: BuildInput,
    }

    async fn fixture(source: &str, engine: Engine) -> Option<Fixture> {
        let toolchain = super::super::toolchain::get(true).await;
        if toolchain.program(engine).is_none() || toolchain.dvisvgm.is_none() {
            std::eprintln!("skipping: {} or dvisvgm not installed", engine.program());
            return None;
        }
        let temp = tempfile::tempdir().unwrap();
        // A space in the path exercises TeX path quoting.
        let source_dir = temp.path().join("doc dir");
        std::fs::create_dir_all(&source_dir).unwrap();
        let main = source_dir.join("main.tex");
        std::fs::write(&main, source).unwrap();
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/32x32.png");
        std::fs::copy(icon, source_dir.join("img.png")).unwrap();
        // Not created here: the build must create its own directories.
        let out = temp.path().join("out");
        Some(Fixture {
            input: BuildInput {
                source: main,
                source_dir,
                jobname: "main".to_string(),
                engine,
                toolchain,
                allow_package_install: false,
                work_dir: out.join("work"),
                driver_preamble: out.join("locus-svg-driver.tex"),
                owner_dir: temp.path().to_string_lossy().to_string(),
            },
            _temp: temp,
        })
    }

    #[tokio::test]
    #[ignore]
    async fn renders_a_mixed_document_on_both_paths() {
        let Some(fixture) = fixture(MIXED, Engine::Xelatex).await else {
            return;
        };
        match run_svg(&fixture.input).await {
            SvgResult::Ready { pages, deps, .. } => {
                assert_eq!(pages.len(), 1);
                let svg = std::fs::read_to_string(&pages[0]).unwrap();
                assert!(svg.contains("data:image/png;base64,"), "bitmap not embedded");
                assert!(svg.contains("linearGradient"), "TikZ shading missing");
                assert!(deps.iter().any(|dep| dep.ends_with("img.png")), "image not a dependency");
                assert!(deps.iter().any(|dep| dep.ends_with("main.tex")));
            }
            other => panic!("SVG path did not render: {other:?}"),
        }
        match run_pdf(&fixture.input).await {
            PdfResult::Ready { pdf, .. } => assert!(std::fs::metadata(pdf).unwrap().len() > 1000),
            other => panic!("PDF path failed: {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn pdf_only_documents_fail_svg_but_build_the_pdf() {
        let Some(fixture) = fixture(PDFPAGES, Engine::Xelatex).await else {
            return;
        };
        // An inserted page for \includepdf, produced by the PDF path itself.
        let helper = fixture.input.source_dir.join("insert.tex");
        std::fs::write(&helper, "\\documentclass{article}\\begin{document}Inserted\\end{document}\n").unwrap();
        let helper_input = BuildInput {
            source: helper,
            jobname: "insert".to_string(),
            work_dir: fixture.input.work_dir.join("helper"),
            ..fixture.input.clone()
        };
        let PdfResult::Ready { pdf, .. } = run_pdf(&helper_input).await else {
            panic!("helper PDF failed");
        };
        std::fs::copy(pdf, fixture.input.source_dir.join("insert.pdf")).unwrap();

        let preamble = pipeline::preamble(PDFPAGES).unwrap();
        assert_eq!(
            pipeline::pdf_only_reason(&preamble, Engine::Xelatex).as_deref(),
            Some("package:pdfpages")
        );
        match run_svg(&fixture.input).await {
            SvgResult::Failed { errors, .. } => {
                assert!(errors.iter().any(|error| error.message.contains("pdfpages")), "{errors:?}");
            }
            other => panic!("SVG path should fail for pdfpages: {other:?}"),
        }
        assert!(matches!(run_pdf(&fixture.input).await, PdfResult::Ready { .. }));
    }

    #[tokio::test]
    #[ignore]
    async fn document_errors_fail_both_paths_with_a_location() {
        let Some(fixture) = fixture(BROKEN, Engine::Pdflatex).await else {
            return;
        };
        let SvgResult::Failed { errors, .. } = run_svg(&fixture.input).await else {
            panic!("SVG path should fail");
        };
        assert_eq!(errors[0].line, Some(4), "{errors:?}");
        assert!(errors[0].message.contains("Undefined control sequence"));
        let PdfResult::Failed { errors, .. } = run_pdf(&fixture.input).await else {
            panic!("PDF path should fail");
        };
        assert_eq!(errors[0].line, Some(4), "{errors:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn pdflatex_dvi_branch_renders_tikz_shading() {
        let Some(fixture) = fixture(PDFLATEX_TIKZ, Engine::Pdflatex).await else {
            return;
        };
        let pages = match run_svg(&fixture.input).await {
            SvgResult::Ready { pages, .. } => pages,
            SvgResult::Failed { errors, .. } => panic!("pdfLaTeX SVG path failed: {errors:?}"),
            SvgResult::Unsupported { reason, detail, .. } => {
                panic!("pdfLaTeX SVG path unsupported: {reason}: {detail}")
            }
        };
        let svg = std::fs::read_to_string(&pages[0]).unwrap();
        assert!(svg.contains("linearGradient"));
        // The injected papersize special gives the full letter page.
        let (width, height) = pipeline::svg_page_size(&svg).expect("page size");
        assert!((width - 614.0).abs() < 4.0 && (height - 795.0).abs() < 4.0, "{width}x{height}");
    }
}
