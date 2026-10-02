#[path = "scas_gui/editor_assist.rs"]
mod editor_assist;
#[path = "scas_gui/math_preview.rs"]
mod math_preview;
#[path = "scas_gui/theme.rs"]
mod theme;

use eframe::egui::{
    self, text::LayoutJob, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
    TextFormat, TextStyle,
};
use scas::ast::statement::FunctionBody;
use scas::ast::StatementKind;
use scas::document::Document;
use scas::execute;
use scas::interpreter::Interpreter;
use scas::latex::LatexRenderer;
use scas::lexer::{Scanner, SymbolTable, TokenType};
use scas::parser::Parser;
use scas::plot::{self, CriticalPoint, PlotBounds, SampledSeries};
use scas::Limits;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

static NEXT_LATEX_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
use theme::Theme;

const STARTER: &str = "/// A live, typeset result\n\
let angle = π / 6;\n\
\\[ sin(angle) => \\]\n\
print(sin(angle));\n";
const DEFAULT_AUTO_GREEK: bool = true;
const DEFAULT_FONT_NAME: &str = "Roboto Mono";
const DEFAULT_FONT_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/RobotoMono-Regular.otf");

/// REPL inline plot drawing size, in character cells.
const PLOT_ASCII_WIDTH: usize = 64;
const PLOT_ASCII_HEIGHT: usize = 17;
/// Zoom limits for the graphing window, in data units. They keep tick steps,
/// sample density, and screen mapping well-defined no matter how long a
/// zoom gesture runs.
const MIN_VIEW_SPAN: f64 = 1.0e-9;
const MAX_VIEW_SPAN: f64 = 1.0e12;
/// Horizontal space kept for source editing while the document preview is open.
/// The preview may use the released space when the Explorer is collapsed, but
/// dragging its splitter must never crush the primary work surface.
const MIN_EDITOR_WORKSPACE_WIDTH: f32 = 420.0;
const MIN_PREVIEW_WIDTH: f32 = 220.0;
const MAX_PREVIEW_WIDTH: f32 = 900.0;

/// Curve colors for the graphing canvas, in draw order.
const GRAPH_COLORS: &[u32] = &[
    0x4f9cf9, 0xf9777f, 0x53c489, 0xc48df9, 0xf9b959, 0x5bc8d9, 0xf97fb8, 0xa3cf5b,
];

const GUI_PLOT_HELP: &str = "usage: :plot EXPRESSION[, EXPRESSION…] [from A to B] [> FILE]\n\n\
  Draws the graph inline; FILE ending in .svg receives a vector image.\n\nexamples:\n\
  :plot sin(x)\n\
  :plot x^2 - 2x from -5 to 5\n\
  :plot sin(x), cos(x) from -2*pi to 2*pi > graph.svg";

const GREEK: &[(&str, &str)] = &[
    ("\\Alpha", "Α"),
    ("\\Beta", "Β"),
    ("\\Gamma", "Γ"),
    ("\\Delta", "Δ"),
    ("\\Epsilon", "Ε"),
    ("\\Zeta", "Ζ"),
    ("\\Eta", "Η"),
    ("\\Theta", "Θ"),
    ("\\Iota", "Ι"),
    ("\\Kappa", "Κ"),
    ("\\Lambda", "Λ"),
    ("\\Mu", "Μ"),
    ("\\Nu", "Ν"),
    ("\\Xi", "Ξ"),
    ("\\Omicron", "Ο"),
    ("\\Pi", "Π"),
    ("\\Rho", "Ρ"),
    ("\\Sigma", "Σ"),
    ("\\Tau", "Τ"),
    ("\\Upsilon", "Υ"),
    ("\\Phi", "Φ"),
    ("\\Chi", "Χ"),
    ("\\Psi", "Ψ"),
    ("\\Omega", "Ω"),
    ("\\alpha", "α"),
    ("\\beta", "β"),
    ("\\gamma", "γ"),
    ("\\delta", "δ"),
    ("\\epsilon", "ε"),
    ("\\zeta", "ζ"),
    ("\\eta", "η"),
    ("\\theta", "θ"),
    ("\\iota", "ι"),
    ("\\kappa", "κ"),
    ("\\lambda", "λ"),
    ("\\mu", "μ"),
    ("\\nu", "ν"),
    ("\\xi", "ξ"),
    ("\\omicron", "ο"),
    ("\\pi", "π"),
    ("\\rho", "ρ"),
    ("\\sigma", "σ"),
    ("\\tau", "τ"),
    ("\\phi", "φ"),
    ("\\chi", "χ"),
    ("\\psi", "ψ"),
    ("\\omega", "ω"),
    ("\\varepsilon", "ϵ"),
    ("\\vartheta", "ϑ"),
    ("\\varpi", "ϖ"),
    ("\\varrho", "ϱ"),
    ("\\varsigma", "ς"),
    ("\\varphi", "ϕ"),
];

#[derive(Clone)]
struct TreeEntry {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
}

struct RunResult {
    output: String,
    failed: bool,
}

struct LatexRender {
    rgba: Vec<u8>,
    size: [usize; 2],
}

struct HighlightCache {
    source: String,
    theme: usize,
    font_size_bits: u32,
    monospace: bool,
    diagnostics_revision: u64,
    job: LayoutJob,
}

struct AnalysisResult {
    generation: u64,
    document: Result<Document, String>,
    diagnostics: Vec<EditorDiagnostic>,
}

#[derive(Clone, Debug)]
struct EditorDiagnostic {
    byte_start: usize,
    byte_end: usize,
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineNumberMode {
    Off,
    Absolute,
    Relative,
    Hybrid,
}

#[derive(Clone, Copy)]
enum ActivityIcon {
    Explorer,
    Preview,
    Problems,
    Settings,
}

/// Sampling resolution tier. Fast samples keep pans and zooms fresh while a
/// gesture is moving; a full-quality sample follows once the view settles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphQuality {
    Fast,
    Full,
}

/// One batch of graph sampling requested by the UI thread.
struct GraphBatchRequest {
    id: u64,
    content_generation: u64,
    session: Vec<String>,
    sources: Vec<String>,
    x_min: f64,
    x_max: f64,
    count: usize,
}

/// A completed batch of graph sampling from the worker.
struct GraphBatchResult {
    id: u64,
    content_generation: u64,
    x_min: f64,
    x_max: f64,
    count: usize,
    samples: Vec<Result<SampledSeries, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceTab {
    Document,
    Repl,
    Graphing,
}

struct ReplEntry {
    source: String,
    input_latex: Option<String>,
    result_latex: Option<String>,
    input_math_key: Option<u64>,
    result_math_key: Option<u64>,
    printed: String,
    error: Option<String>,
    plot: Option<PlotOutcome>,
}

/// One persistent graphing-calculator function. The expression re-samples
/// from the live REPL interpreter whenever the session changes.
struct GraphFunction {
    /// The raw expression source (`sin(x)` or `g(t) = 2t + 1`).
    source: String,
    /// Last successful sampling; `Err` holds the message shown in the list.
    sampled: Result<SampledSeries, String>,
    /// Whether the samples reflect the current source and session revision.
    stale: bool,
    /// Domain of the cached samples, for pan/zoom coverage checks.
    sampled_min: f64,
    sampled_max: f64,
    sampled_count: usize,
    /// Zeros and extrema of the current samples, drawn as dots.
    critical_points: Vec<CriticalPoint>,
}

/// A plot drawn from the REPL input with `:plot`, kept for inline display.
struct PlotOutcome {
    bounds: PlotBounds,
    ascii: String,
    wrote_file: Option<PathBuf>,
}

/// History and view state for the graphing canvas. Zooming multiplies both
/// spans; panning shifts both bounds by the same data-space amount.
#[derive(Clone, Copy)]
struct ViewState {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    auto_y: bool,
}

impl ViewState {
    fn centered(span_x: f64, span_y: f64) -> Self {
        Self {
            x_min: -span_x / 2.0,
            x_max: span_x / 2.0,
            y_min: -span_y / 2.0,
            y_max: span_y / 2.0,
            auto_y: true,
        }
    }

    fn bounds(&self) -> PlotBounds {
        PlotBounds::new(self.x_min, self.x_max, self.y_min, self.y_max)
    }

    fn x_span(&self) -> f64 {
        self.x_max - self.x_min
    }

    fn y_span(&self) -> f64 {
        self.y_max - self.y_min
    }

    #[cfg(test)]
    fn zoom(&mut self, factor: f64) {
        let cx = (self.x_min + self.x_max) / 2.0;
        let cy = (self.y_min + self.y_max) / 2.0;
        let half_x = (self.x_max - self.x_min) * factor / 2.0;
        let half_y = (self.y_max - self.y_min) * factor / 2.0;
        self.x_min = cx - half_x;
        self.x_max = cx + half_x;
        self.y_min = cy - half_y;
        self.y_max = cy + half_y;
    }
}

impl LineNumberMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Absolute => "Absolute",
            Self::Relative => "Relative",
            Self::Hybrid => "Relative + current",
        }
    }
}

struct ScasApp {
    source: String,
    output: String,
    current_file: Option<PathBuf>,
    project_root: PathBuf,
    files: Vec<TreeEntry>,
    collapsed_dirs: HashSet<PathBuf>,
    dirty: bool,
    auto_greek: bool,
    run_result: Option<Receiver<RunResult>>,
    status: String,
    themes: Vec<Theme>,
    selected_theme: usize,
    editor_font_size: f32,
    editor_monospace: bool,
    settings_open: bool,
    theme_path: String,
    font_path: String,
    preview: String,
    preview_document: Option<Document>,
    preview_error: Option<String>,
    preview_open: bool,
    files_open: bool,
    diagnostics_open: bool,
    diagnostics: Vec<EditorDiagnostic>,
    analysis_due: Option<Instant>,
    analysis_result: Option<Receiver<AnalysisResult>>,
    analysis_generation: u64,
    buffer_generation: u64,
    latex_render: Option<Receiver<(u64, Result<LatexRender, String>)>>,
    latex_pending: Option<(String, [u8; 3], u64)>,
    last_latex_key: Option<u64>,
    system_latex_preview: bool,
    latex_texture: Option<egui::TextureHandle>,
    latex_render_error: Option<String>,
    repl_math_receiver: Option<Receiver<(u64, Result<LatexRender, String>)>>,
    repl_math_pending: VecDeque<(u64, String, [u8; 3])>,
    repl_math_inflight: HashSet<u64>,
    repl_math_cache: HashMap<u64, egui::TextureHandle>,
    repl_math_error: Option<String>,
    repl_input_math_due: Option<Instant>,
    line_numbers: LineNumberMode,
    current_line: usize,
    cursor_char_index: usize,
    diagnostics_revision: u64,
    highlight_cache: Option<HighlightCache>,
    workspace_tab: WorkspaceTab,
    repl_input: String,
    repl_input_latex: Option<String>,
    repl_input_math_key: Option<u64>,
    repl_entries: Vec<ReplEntry>,
    repl_symbols: SymbolTable,
    repl_interpreter: Interpreter<Vec<u8>>,
    repl_history: Vec<String>,
    repl_history_cursor: Option<usize>,
    /// Successfully executed REPL submissions, replayed in graph workers to
    /// build an isolated interpreter without blocking the UI thread.
    repl_session: Vec<String>,
    graph_functions: Vec<GraphFunction>,
    /// Requests to the persistent graph sampling worker.
    graph_request_sender: mpsc::Sender<GraphBatchRequest>,
    /// Completed sampling batches coming back from the worker.
    graph_result_receiver: mpsc::Receiver<GraphBatchResult>,
    graph_sample_due: Option<(Instant, GraphQuality)>,
    /// Full-quality re-sample armed once fast pan/zoom samples land.
    graph_refine_due: Option<Instant>,
    /// Bumped when the function list or REPL session changes, so results
    /// sampled against older content are dropped.
    graph_content_generation: u64,
    /// Monotonic id of the newest request sent to the worker.
    graph_request_id: u64,
    /// Highest result id that has been applied to the curves.
    graph_last_applied_id: u64,
    graph_sample_count: usize,
    graph_sampling: bool,
    graph_canvas_width: f32,
    graph_auto_bounds: Option<PlotBounds>,
    graph_view: ViewState,
    graph_new_input: String,
    /// Pinned tap point on the graphing canvas, snapped to the nearest
    /// curve; `None` when the view is un-probed.
    graph_probe: Option<GraphProbe>,
}

/// A tapped point on the graphing canvas snapped to the nearest curve.
/// Stored in data coordinates so the crosshair is redrawn from the current
/// view without re-scanning every sample on each frame.
#[derive(Clone, Copy)]
struct GraphProbe {
    /// Index into `graph_functions` of the snapped curve.
    series_index: usize,
    /// Snapped position in plot coordinates.
    x: f64,
    /// Curve y at the snapped x, when the curve covers it.
    y: Option<f64>,
}

fn install_default_font(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        DEFAULT_FONT_NAME.to_owned(),
        FontData::from_owned(DEFAULT_FONT_BYTES.to_vec()),
    );
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, DEFAULT_FONT_NAME.to_owned());
    }
    context.set_fonts(fonts);
}

fn preview_widths(screen_width: f32, explorer_width: f32) -> (f32, f32) {
    let usable_width = (screen_width - 48.0 - explorer_width).max(0.0);
    let max_width = (usable_width - MIN_EDITOR_WORKSPACE_WIDTH)
        .clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH);
    let default_width = (usable_width * 0.46).clamp(MIN_PREVIEW_WIDTH, max_width);
    (default_width, max_width)
}

impl ScasApp {
    fn new(context: &egui::Context) -> Self {
        install_default_font(context);
        let themes = Theme::bundled_themes();
        themes[0].apply(context);
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let files = collect_tree(&project_root);
        let repl_symbols = SymbolTable::new();
        let repl_interpreter = Interpreter::new(repl_symbols.clone(), Vec::new());
        let (graph_request_sender, request_receiver) = mpsc::channel();
        let (result_sender, graph_result_receiver) = mpsc::channel();
        std::thread::spawn(move || run_graph_sampler(request_receiver, result_sender));
        let mut app = Self {
            source: STARTER.to_owned(),
            output: "Run the program to see its output.".to_owned(),
            current_file: None,
            project_root,
            files,
            collapsed_dirs: HashSet::new(),
            dirty: false,
            auto_greek: DEFAULT_AUTO_GREEK,
            run_result: None,
            status: "Ready".to_owned(),
            themes,
            selected_theme: 0,
            editor_font_size: 15.0,
            editor_monospace: true,
            settings_open: false,
            theme_path: "themes/dark-modern.toml".to_owned(),
            font_path: "assets/fonts/RobotoMono-Regular.otf".to_owned(),
            preview: String::new(),
            preview_document: None,
            preview_error: None,
            preview_open: true,
            files_open: true,
            diagnostics_open: false,
            diagnostics: Vec::new(),
            analysis_due: None,
            analysis_result: None,
            analysis_generation: 0,
            buffer_generation: 0,
            latex_render: None,
            latex_pending: None,
            last_latex_key: None,
            system_latex_preview: true,
            latex_texture: None,
            latex_render_error: None,
            repl_math_receiver: None,
            repl_math_pending: VecDeque::new(),
            repl_math_inflight: HashSet::new(),
            repl_math_cache: HashMap::new(),
            repl_math_error: None,
            repl_input_math_due: None,
            line_numbers: LineNumberMode::Absolute,
            current_line: 0,
            cursor_char_index: 0,
            diagnostics_revision: 0,
            highlight_cache: None,
            workspace_tab: WorkspaceTab::Document,
            repl_input: String::new(),
            repl_input_latex: None,
            repl_input_math_key: None,
            repl_entries: Vec::new(),
            repl_symbols,
            repl_interpreter,
            repl_history: Vec::new(),
            repl_history_cursor: None,
            repl_session: Vec::new(),
            graph_functions: Vec::new(),
            graph_request_sender,
            graph_result_receiver,
            graph_sample_due: None,
            graph_refine_due: None,
            graph_content_generation: 0,
            graph_request_id: 0,
            graph_last_applied_id: 0,
            graph_sample_count: 900,
            graph_sampling: false,
            graph_canvas_width: 800.0,
            graph_auto_bounds: None,
            graph_view: ViewState::centered(22.0, 12.0),
            graph_new_input: String::new(),
            graph_probe: None,
        };
        app.start_analysis();
        app
    }

    fn reset_repl(&mut self) {
        self.repl_symbols = SymbolTable::new();
        self.repl_interpreter = Interpreter::new(self.repl_symbols.clone(), Vec::new());
        self.repl_entries.clear();
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.repl_input_math_key = None;
        self.repl_input_math_due = None;
        self.repl_history.clear();
        self.repl_history_cursor = None;
        self.repl_session.clear();
        self.graph_content_generation = self.graph_content_generation.wrapping_add(1);
        self.graph_sample_due = None;
        self.graph_refine_due = None;
        self.mark_graph_functions_stale();
        self.schedule_graph_sampling(Duration::ZERO);
        self.set_status("REPL session reset");
    }

    /// Queues a complete re-sample against the latest REPL session.
    fn refresh_graph_functions(&mut self) {
        self.graph_content_generation = self.graph_content_generation.wrapping_add(1);
        self.schedule_graph_sampling(Duration::ZERO);
    }

    /// Schedules a full-quality sample off the UI thread. Requests are
    /// debounced while gestures continue; the sampling worker collapses its
    /// queue so only the newest window is measured.
    fn schedule_graph_sampling(&mut self, delay: Duration) {
        self.schedule_graph_sampling_with(delay, GraphQuality::Full);
    }

    fn schedule_graph_sampling_with(&mut self, delay: Duration, quality: GraphQuality) {
        if self.graph_functions.is_empty() {
            return;
        }
        self.graph_sample_due = Some((Instant::now() + delay, quality));
    }

    /// Expand sample coverage with hysteresis: during pans/zooms, retain the
    /// current curve buffer until the viewport approaches its padded edges.
    fn ensure_graph_coverage(&mut self) {
        let span = self.graph_view.x_span();
        if self.graph_sample_due.is_some() || self.graph_sampling {
            return;
        }
        let width = self.graph_canvas_width.max(1.0);
        let visible_min = self.graph_view.x_min;
        let visible_max = self.graph_view.x_max;
        let needs_samples = self.graph_functions.iter().any(|function| {
            let sampled_span = function.sampled_max - function.sampled_min;
            let visible_density = if sampled_span > 0.0 {
                function.sampled_count as f32 * (span / sampled_span) as f32 / width
            } else {
                0.0
            };
            function.stale
                || function.sampled.is_err()
                || function.sampled_min >= visible_min - span * 0.15
                || function.sampled_max <= visible_max + span * 0.15
                || visible_density < 1.5
        });
        if needs_samples {
            // Fast tier first so moving the window stays responsive; the
            // refine timer sharpens the curves once the view settles.
            self.schedule_graph_sampling_with(Duration::from_millis(100), GraphQuality::Fast);
            self.graph_refine_due = Some(Instant::now() + Duration::from_millis(280));
        }
    }

    fn start_graph_sampling(&mut self, quality: GraphQuality) {
        if self.graph_functions.is_empty() {
            return;
        }
        let view = self.graph_view;
        let span = view.x_span();
        let x_min = view.x_min - span * 0.3;
        let x_max = view.x_max + span * 0.3;
        let width = self.graph_canvas_width.max(300.0);
        let count = match quality {
            // Base densities; the adaptive sampler refines on top of these
            // wherever the curve bends, so flat stretches stay cheap.
            GraphQuality::Fast => ((width * 2.0).ceil() as usize).clamp(400, 2_400),
            GraphQuality::Full => ((width * 3.0).ceil() as usize).clamp(600, 4_000),
        };
        self.graph_request_id = self.graph_request_id.wrapping_add(1);
        let request = GraphBatchRequest {
            id: self.graph_request_id,
            content_generation: self.graph_content_generation,
            session: self.repl_session.clone(),
            sources: self
                .graph_functions
                .iter()
                .map(|function| function.source.clone())
                .collect(),
            x_min,
            x_max,
            count,
        };
        if self.graph_request_sender.send(request).is_ok() {
            self.graph_sampling = true;
        }
        self.graph_sample_count = count;
        if quality == GraphQuality::Full {
            self.graph_refine_due = None;
        }
    }

    fn poll_graph_sampling(&mut self, context: &egui::Context) {
        let mut applied = false;
        loop {
            let event = self.graph_result_receiver.try_recv();
            match event {
                Ok(result) => {
                    self.graph_sampling = false;
                    self.apply_graph_batch(result);
                    applied = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.graph_sampling = false;
                    for function in &mut self.graph_functions {
                        function.stale = true;
                    }
                    break;
                }
            }
        }
        if applied {
            context.request_repaint();
        }
        if self
            .graph_sample_due
            .is_some_and(|(due, _)| Instant::now() >= due)
        {
            let quality = self.graph_sample_due.take().expect("checked above").1;
            self.start_graph_sampling(quality);
        }
        // Sharpen with a full-quality sample once gestures settle.
        if self
            .graph_refine_due
            .is_some_and(|due| Instant::now() >= due)
        {
            if self.graph_functions.is_empty() {
                self.graph_refine_due = None;
            } else if self.graph_sample_due.is_none() && !self.graph_sampling {
                self.graph_refine_due = None;
                self.start_graph_sampling(GraphQuality::Full);
            }
        }
        if self.graph_sampling {
            context.request_repaint_after(Duration::from_millis(24));
        } else if let Some((due, _)) = self.graph_sample_due {
            context.request_repaint_after(due.saturating_duration_since(Instant::now()));
        } else if let Some(due) = self.graph_refine_due {
            context.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }
    }

    /// Applies a completed sampling batch when it still matches the current
    /// function list and session and is newer than the last applied result.
    fn apply_graph_batch(&mut self, result: GraphBatchResult) {
        if result.content_generation != self.graph_content_generation
            || result.id <= self.graph_last_applied_id
        {
            return;
        }
        self.graph_last_applied_id = result.id;
        self.graph_auto_bounds = None;
        self.graph_sample_count = result.count;
        for (function, sampled) in self.graph_functions.iter_mut().zip(result.samples) {
            function.sampled_min = result.x_min;
            function.sampled_max = result.x_max;
            function.sampled_count = result.count;
            function.critical_points = sampled
                .as_ref()
                .ok()
                .map(|series| plot::detect_critical_points(&series.samples))
                .unwrap_or_default();
            function.sampled = sampled;
            function.stale = false;
        }
        self.graph_probe = None;
    }

    fn mark_graph_functions_stale(&mut self) {
        for function in &mut self.graph_functions {
            function.stale = true;
        }
    }

    /// Writes the current graphing canvas as an SVG beside the active file
    /// (or into the workspace root), reusing the shared SVG renderer.
    fn export_graph_svg(&mut self) {
        let live: Vec<SampledSeries> = self
            .graph_functions
            .iter()
            .filter_map(|function| function.sampled.as_ref().ok().cloned())
            .collect();
        if live.is_empty() {
            self.set_status("Nothing to export: add a function that samples successfully");
            return;
        }
        let bounds = self.current_graph_bounds();
        let stem = self
            .current_file
            .clone()
            .unwrap_or_else(|| self.project_root.join("document.scas"))
            .with_extension("svg");
        match fs::write(&stem, plot::render_svg(&live, &bounds, 800, 520)) {
            Ok(()) => self.status = format!("Exported {}", stem.display()),
            Err(error) => self.status = format!("Could not export {}: {error}", stem.display()),
        }
    }

    /// Adds a function to the graphing calculator, defining it in the REPL
    /// session when written as a definition (`g(t) = 2t + 1`).
    fn add_graph_function(&mut self, source: &str) {
        self.graph_auto_bounds = None;
        if source.contains('=') {
            self.define_in_repl_session(source);
        }
        self.graph_content_generation = self.graph_content_generation.wrapping_add(1);
        self.graph_functions.push(GraphFunction {
            source: source.to_owned(),
            sampled: Err("sampling…".to_owned()),
            stale: true,
            sampled_min: self.graph_view.x_min,
            sampled_max: self.graph_view.x_max,
            sampled_count: 0,
            critical_points: Vec::new(),
        });
        self.schedule_graph_sampling(Duration::ZERO);
        self.set_status("Function added to the plot");
    }

    /// Executes a graph-input definition in the live REPL session so later
    /// REPL entries can call it, and records it for graph sampling replays.
    /// Failures stay silent here: the graph list reports sampling errors.
    fn define_in_repl_session(&mut self, source: &str) {
        let submission = terminate_repl_source(source);
        let mut scanner = Scanner::with_symbols(&submission, self.repl_symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        if scanner.had_error() {
            return;
        }
        self.repl_symbols = scanner.into_symbols();
        self.repl_interpreter.set_symbols(self.repl_symbols.clone());
        let Ok(program) = Parser::new(tokens).parse() else {
            return;
        };
        if self
            .repl_interpreter
            .run_interactive_value(&program)
            .is_ok()
        {
            self.repl_session.push(submission);
        }
        let _ = std::mem::take(self.repl_interpreter.output_mut());
    }

    fn refresh_repl_latex(&mut self) {
        self.repl_input_latex = repl_expression_latex(&self.repl_input, &self.repl_symbols);
        self.repl_input_math_key = None;
        self.repl_input_math_due = self
            .repl_input_latex
            .as_ref()
            .map(|_| Instant::now() + Duration::from_millis(180));
    }

    fn submit_repl(&mut self) {
        let source = self.repl_input.trim().to_owned();
        if source.is_empty() {
            return;
        }
        self.repl_history.push(source.clone());
        self.repl_history_cursor = None;
        if source == ":clear" {
            self.repl_entries.clear();
            self.repl_input.clear();
            self.repl_input_latex = None;
            self.repl_input_math_key = None;
            self.set_status("REPL history cleared");
            return;
        }
        if source == ":reset" {
            self.reset_repl();
            return;
        }
        if let Some(rest) = source.strip_prefix(":plot") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                self.submit_repl_plot(rest.trim());
                return;
            }
        }

        let submission = terminate_repl_source(&source);
        let mut scanner = Scanner::with_symbols(&submission, self.repl_symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        let scan_error = scanner
            .errors()
            .iter()
            .map(|error| error.render(&submission, "<repl>"))
            .collect::<Vec<_>>()
            .join("\n");
        self.repl_symbols = scanner.into_symbols();
        self.repl_interpreter.set_symbols(self.repl_symbols.clone());

        let input_latex = self.repl_input_latex.clone();
        let repl_color = self.themes[self.selected_theme].ui.text.0;
        let input_math_key = input_latex.as_deref().map(|latex| {
            self.queue_repl_math(latex, [repl_color.r(), repl_color.g(), repl_color.b()])
        });
        self.repl_input_math_due = None;
        let mut result_latex = None;
        let mut result_math_key = None;
        let mut error = (!scan_error.is_empty()).then_some(scan_error);
        if error.is_none() {
            match Parser::new(tokens).parse() {
                Ok(program) => match self.repl_interpreter.run_interactive_value(&program) {
                    Ok(Some(value)) => {
                        result_latex =
                            Some(LatexRenderer::new(self.repl_interpreter.symbols()).value(&value));
                        result_math_key = result_latex.as_deref().map(|latex| {
                            self.queue_repl_math(
                                latex,
                                [repl_color.r(), repl_color.g(), repl_color.b()],
                            )
                        });
                    }
                    Ok(None) => {}
                    Err(runtime) => error = Some(runtime.render(&submission, "<repl>")),
                },
                Err(parse) => error = Some(parse.render(&submission, "<repl>")),
            }
        }
        let printed = String::from_utf8_lossy(&std::mem::take(self.repl_interpreter.output_mut()))
            .into_owned();
        let succeeded = error.is_none();
        // Definitions may have changed, so the graphs re-sample.
        if succeeded {
            self.repl_session.push(submission.clone());
            self.graph_content_generation = self.graph_content_generation.wrapping_add(1);
            self.mark_graph_functions_stale();
            self.schedule_graph_sampling(Duration::from_millis(50));
        }
        self.repl_entries.push(ReplEntry {
            source,
            input_latex,
            result_latex,
            input_math_key,
            result_math_key,
            printed,
            error,
            plot: None,
        });
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.repl_input_math_key = None;
        self.repl_input_math_due = None;
        self.set_status(if succeeded {
            "REPL expression evaluated"
        } else {
            "REPL expression needs attention"
        });
    }

    /// Evaluates a `:plot …` line inside the GUI REPL: samples the series,
    /// shows the ASCII drawing inline, and offers a live plotted preview.
    fn submit_repl_plot(&mut self, rest: &str) {
        if rest.is_empty() {
            self.repl_entries.push(ReplEntry {
                source: ":plot".to_owned(),
                input_latex: None,
                result_latex: None,
                input_math_key: None,
                result_math_key: None,
                printed: GUI_PLOT_HELP.to_owned(),
                error: None,
                plot: None,
            });
            self.repl_input.clear();
            self.repl_input_latex = None;
            self.repl_input_math_key = None;
            return;
        }
        let mut symbols = self.repl_symbols.clone();
        let outcome = plot::execute_plot_command(
            &mut self.repl_interpreter,
            &mut symbols,
            rest,
            PLOT_ASCII_WIDTH,
            PLOT_ASCII_HEIGHT,
            &self.project_root,
        );
        self.repl_symbols = symbols;
        self.repl_interpreter.set_symbols(self.repl_symbols.clone());
        let (entry, status) =
            match outcome {
                Ok(outcome) => {
                    let status = match &outcome.wrote_file {
                        Some(path) => format!("Plot written to {}", path.display()),
                        None => "Plot drawn in the REPL".to_owned(),
                    };
                    (
                        ReplEntry {
                            source: format!(":plot {rest}"),
                            input_latex: None,
                            result_latex: None,
                            input_math_key: None,
                            result_math_key: None,
                            printed: format!(
                                "{}\n{}",
                                outcome.ascii,
                                outcome.wrote_file.as_ref().map_or_else(
                                    String::new,
                                    |path| format!("wrote {}", path.display())
                                )
                            ),
                            error: None,
                            plot: Some(PlotOutcome {
                                bounds: outcome.bounds,
                                ascii: outcome.ascii,
                                wrote_file: outcome.wrote_file,
                            }),
                        },
                        status,
                    )
                }
                Err(message) => (
                    ReplEntry {
                        source: format!(":plot {rest}"),
                        input_latex: None,
                        result_latex: None,
                        input_math_key: None,
                        result_math_key: None,
                        printed: String::new(),
                        error: Some(message),
                        plot: None,
                    },
                    "Plot request needs attention".to_owned(),
                ),
            };
        self.repl_entries.push(entry);
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.repl_input_math_key = None;
        self.repl_input_math_due = None;
        self.set_status(&status);
    }

    fn run(&mut self) {
        if self.run_result.is_some() {
            return;
        }
        let source = self.source.clone();
        let source_name = self
            .current_file
            .as_deref()
            .and_then(Path::to_str)
            .unwrap_or("<editor>")
            .to_owned();
        let (sender, receiver) = mpsc::channel();
        self.run_result = Some(receiver);
        self.set_status("Running…");
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let error = execute(&source, &mut bytes).err();
            let mut output = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(error) = error {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&error.render(&source, &source_name));
                let _ = sender.send(RunResult {
                    output,
                    failed: true,
                });
            } else {
                if output.is_empty() {
                    output.push_str("Program completed without output.");
                }
                let _ = sender.send(RunResult {
                    output,
                    failed: false,
                });
            }
        });
    }

    fn poll_run(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.run_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.output = result.output;
                self.set_status(if result.failed {
                    "Run failed"
                } else {
                    "Run finished"
                });
                self.run_result = None;
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => {
                self.output.clear();
                self.output
                    .push_str("The execution worker stopped unexpectedly.");
                self.set_status("Run failed");
                self.run_result = None;
            }
        }
    }

    fn open(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(source) => {
                self.source = source;
                self.current_file = Some(path.clone());
                self.dirty = false;
                self.cursor_char_index = 0;
                self.buffer_generation = self.buffer_generation.wrapping_add(1);
                self.analysis_generation = self.analysis_generation.wrapping_add(1);
                self.analysis_due = None;
                self.status = format!("Opened {}", path.display());
                self.start_analysis();
            }
            Err(error) => self.status = format!("Could not open {}: {error}", path.display()),
        }
    }

    fn save(&mut self) {
        let Some(path) = self.current_file.as_ref() else {
            self.set_status("Choose a .scas file in the tree before saving");
            return;
        };
        match fs::write(path, &self.source) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", path.display());
            }
            Err(error) => self.status = format!("Could not save {}: {error}", path.display()),
        }
    }

    fn export_stem(&self) -> PathBuf {
        self.current_file
            .clone()
            .unwrap_or_else(|| self.project_root.join("document.scas"))
    }

    fn export_tex(&mut self) -> Option<PathBuf> {
        let document = match scas::document::build(&self.source) {
            Ok(document) => document,
            Err(error) => {
                self.status = format!("Cannot export: {error}");
                return None;
            }
        };
        let path = self.export_stem().with_extension("tex");
        match fs::write(&path, document.latex()) {
            Ok(()) => {
                self.status = format!("Exported {}", path.display());
                Some(path)
            }
            Err(error) => {
                self.status = format!("Could not export {}: {error}", path.display());
                None
            }
        }
    }

    fn export_pdf(&mut self) {
        let Some(tex_path) = self.export_tex() else {
            return;
        };
        let Some(directory) = tex_path.parent() else {
            self.set_status("Could not determine the export directory");
            return;
        };
        let Some(file_name) = tex_path.file_name() else {
            self.set_status("Could not determine the TeX filename");
            return;
        };
        match Command::new("pdflatex")
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg(file_name)
            .current_dir(directory)
            .output()
        {
            Ok(output) if output.status.success() => {
                self.status = format!("Exported {}", tex_path.with_extension("pdf").display());
            }
            Ok(output) => {
                let message = String::from_utf8_lossy(&output.stdout);
                let detail = message
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("pdflatex failed");
                self.status = format!("PDF export failed: {detail}");
            }
            Err(error) => {
                self.status = format!("PDF export requires pdflatex: {error}");
            }
        }
    }

    fn new_file(&mut self) {
        self.source.clear();
        self.current_file = None;
        self.dirty = false;
        self.cursor_char_index = 0;
        self.buffer_generation = self.buffer_generation.wrapping_add(1);
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        self.analysis_due = None;
        self.set_status("New buffer");
        self.start_analysis();
    }

    fn set_status(&mut self, status: &str) {
        self.status.clear();
        self.status.push_str(status);
    }

    fn insert_editor_template(
        &mut self,
        context: &egui::Context,
        editor_id: egui::Id,
        start_char: usize,
        end_char: usize,
        template: &str,
    ) {
        let source_chars = self.source.chars().count();
        let start_char = start_char.min(source_chars);
        let end_char = end_char.clamp(start_char, source_chars);
        let marker = template.find("$0");
        let cursor_offset = marker
            .map(|index| template[..index].chars().count())
            .unwrap_or_else(|| template.chars().count());
        let insertion = marker.map_or_else(
            || template.to_owned(),
            |index| {
                let mut text = String::with_capacity(template.len().saturating_sub(2));
                text.push_str(&template[..index]);
                text.push_str(&template[index + 2..]);
                text
            },
        );
        let start_byte = char_to_byte(&self.source, start_char);
        let end_byte = char_to_byte(&self.source, end_char);
        let before = self.source.clone();
        self.source.replace_range(start_byte..end_byte, &insertion);
        self.cursor_char_index = start_char + cursor_offset;

        let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
        let old_range = state
            .cursor
            .char_range()
            .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(end_char)));
        let mut undoer = state.undoer();
        undoer.add_undo(&(old_range, before));
        state.set_undoer(undoer);
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(self.cursor_char_index),
            )));
        TextEdit::store_state(context, editor_id, state);
        context.memory_mut(|memory| memory.request_focus(editor_id));
        self.highlight_cache = None;
        self.dirty = true;
        self.schedule_analysis(context);
    }

    fn start_analysis(&mut self) {
        if self.analysis_result.is_some() {
            return;
        }
        let source = self.source.clone();
        let generation = self.analysis_generation;
        let (sender, receiver) = mpsc::channel();
        self.analysis_result = Some(receiver);
        std::thread::spawn(move || {
            let diagnostics = source_diagnostics(&source);
            let document = scas::document::build_with_limits(&source, live_preview_limits())
                .map_err(|error| error.to_string());
            let _ = sender.send(AnalysisResult {
                generation,
                document,
                diagnostics,
            });
        });
    }

    fn apply_analysis(&mut self, result: AnalysisResult) {
        if result.generation != self.analysis_generation {
            return;
        }
        let mut diagnostics = result.diagnostics;
        match result.document {
            Ok(document) => {
                self.preview = document.latex();
                let text_color = self.themes[self.selected_theme].ui.text.0;
                if self.preview_open && self.system_latex_preview {
                    self.start_latex_render(
                        document.body(),
                        [text_color.r(), text_color.g(), text_color.b()],
                    );
                }
                for diagnostic in &document.diagnostics {
                    let byte_start = diagnostic.span.byte_start.min(self.source.len());
                    let byte_end = diagnostic
                        .span
                        .byte_end
                        .max(byte_start.saturating_add(1))
                        .min(self.source.len());
                    diagnostics.push(EditorDiagnostic {
                        byte_start,
                        byte_end,
                        line: diagnostic.span.line,
                        column: diagnostic.span.column,
                        message: diagnostic.message.clone(),
                    });
                }
                self.diagnostics = diagnostics;
                self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
                self.highlight_cache = None;
                self.preview_document = Some(document);
                self.preview_error = None;
            }
            Err(error) => {
                self.diagnostics = diagnostics;
                self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
                self.highlight_cache = None;
                // Keep the last successful document visible while the user is
                // in the middle of typing an incomplete expression.
                self.preview_error = Some(error.to_string());
            }
        }
    }

    fn schedule_analysis(&mut self, context: &egui::Context) {
        const DEBOUNCE: Duration = Duration::from_millis(400);
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        self.analysis_due = Some(Instant::now() + DEBOUNCE);
        // Do not show stale errors while the user is repairing the source.
        self.diagnostics.clear();
        self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
        self.highlight_cache = None;
        self.preview_error = None;
        context.request_repaint_after(DEBOUNCE);
    }

    fn poll_analysis(&mut self, context: &egui::Context) {
        if let Some(receiver) = &self.analysis_result {
            match receiver.try_recv() {
                Ok(result) => {
                    self.analysis_result = None;
                    self.apply_analysis(result);
                }
                Err(TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(100));
                }
                Err(TryRecvError::Disconnected) => {
                    self.analysis_result = None;
                    self.preview_error = Some("analysis worker stopped unexpectedly".to_owned());
                }
            }
        }

        if let Some(due) = self.analysis_due {
            let now = Instant::now();
            if now >= due && self.analysis_result.is_none() {
                self.analysis_due = None;
                self.start_analysis();
            } else if now < due {
                context.request_repaint_after(due.duration_since(now));
            }
        }
    }

    fn start_latex_render(&mut self, body: String, color: [u8; 3]) {
        if !self.system_latex_preview || !self.preview_open {
            return;
        }
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        color.hash(&mut hasher);
        let key = hasher.finish();
        if self.last_latex_key == Some(key)
            || self
                .latex_pending
                .as_ref()
                .is_some_and(|(_, _, pending_key)| *pending_key == key)
        {
            return;
        }
        if self.latex_render.is_some() {
            self.latex_pending = Some((body, color, key));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.latex_render = Some(receiver);
        self.latex_render_error = None;
        std::thread::spawn(move || {
            let result = render_latex_png(&body, color);
            let _ = sender.send((key, result));
        });
    }

    fn queue_repl_math(&mut self, latex: &str, color: [u8; 3]) -> u64 {
        let mut hasher = DefaultHasher::new();
        latex.hash(&mut hasher);
        color.hash(&mut hasher);
        let key = hasher.finish();
        if self.repl_math_cache.contains_key(&key) || self.repl_math_inflight.contains(&key) {
            return key;
        }
        if !self
            .repl_math_pending
            .iter()
            .any(|(pending, _, _)| *pending == key)
        {
            self.repl_math_pending
                .push_back((key, latex.to_owned(), color));
        }
        key
    }

    fn start_next_repl_math(&mut self) {
        if self.repl_math_receiver.is_some() {
            return;
        }
        let Some((key, latex, color)) = self.repl_math_pending.pop_front() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.repl_math_receiver = Some(receiver);
        self.repl_math_inflight.insert(key);
        std::thread::spawn(move || {
            let result = render_repl_math_png(&latex, color);
            let _ = sender.send((key, result));
        });
    }

    fn poll_repl_math(&mut self, context: &egui::Context) {
        if !self.system_latex_preview {
            return;
        }
        if self
            .repl_input_math_due
            .is_some_and(|_| self.repl_input_latex.is_none())
        {
            self.repl_input_math_due = None;
            self.repl_input_math_key = None;
        }
        if self
            .repl_input_math_due
            .is_some_and(|due| Instant::now() >= due)
        {
            self.repl_input_math_due = None;
            if let Some(latex) = self.repl_input_latex.clone() {
                let color = self.themes[self.selected_theme].ui.text.0;
                self.repl_input_math_key =
                    Some(self.queue_repl_math(&latex, [color.r(), color.g(), color.b()]));
            }
        }
        if let Some(receiver) = &self.repl_math_receiver {
            match receiver.try_recv() {
                Ok((key, Ok(render))) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(render.size, &render.rgba);
                    let texture = context.load_texture(
                        format!("repl-math-{key}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.repl_math_cache.insert(key, texture);
                    self.repl_math_error = None;
                    self.repl_math_receiver = None;
                    self.repl_math_inflight.remove(&key);
                }
                Ok((key, Err(error))) => {
                    self.repl_math_error = Some(error);
                    self.repl_math_receiver = None;
                    self.repl_math_inflight.remove(&key);
                }
                Err(TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(30));
                }
                Err(TryRecvError::Disconnected) => {
                    self.repl_math_error = Some("Computer Modern renderer stopped".to_owned());
                    self.repl_math_receiver = None;
                    self.repl_math_inflight.clear();
                }
            }
        }
        self.start_next_repl_math();
        if self.repl_math_receiver.is_some() || !self.repl_math_pending.is_empty() {
            context.request_repaint_after(Duration::from_millis(30));
        } else if let Some(due) = self.repl_input_math_due {
            context.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }
    }

    fn poll_latex_render(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.latex_render else {
            return;
        };
        let event = receiver.try_recv();
        match event {
            Ok((key, Ok(render))) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(render.size, &render.rgba);
                self.latex_texture = Some(context.load_texture(
                    "latex-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                self.last_latex_key = Some(key);
                self.latex_render_error = None;
                self.latex_render = None;
            }
            Ok((key, Err(error))) => {
                self.latex_render_error = Some(error);
                self.last_latex_key = Some(key);
                self.latex_render = None;
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.latex_render_error = Some("LaTeX preview worker stopped".to_owned());
                self.latex_render = None;
            }
        }
        if self.latex_render.is_none() {
            if let Some((body, color, _)) = self.latex_pending.take() {
                self.start_latex_render(body, color);
            }
        }
    }

    fn title(&self) -> String {
        let name = self
            .current_file
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled");
        format!("{name}{}", if self.dirty { " •" } else { "" })
    }

    fn load_theme(&mut self, context: &egui::Context) {
        let path = PathBuf::from(self.theme_path.trim());
        match Theme::load(&path) {
            Ok(theme) => {
                self.status = format!("Loaded theme {}", theme.name);
                self.themes.push(theme);
                self.selected_theme = self.themes.len() - 1;
                self.themes[self.selected_theme].apply(context);
                if let Some(document) = &self.preview_document {
                    let color = self.themes[self.selected_theme].ui.text.0;
                    self.start_latex_render(document.body(), [color.r(), color.g(), color.b()]);
                }
            }
            Err(error) => self.status = error,
        }
    }

    fn load_font(&mut self, context: &egui::Context) {
        let path = PathBuf::from(self.font_path.trim());
        match fs::read(&path) {
            Ok(bytes) => {
                let name = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("custom-editor-font")
                    .to_owned();
                let mut fonts = FontDefinitions::default();
                fonts
                    .font_data
                    .insert(name.clone(), FontData::from_owned(bytes));
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .insert(0, name.clone());
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(0, name.clone());
                context.set_fonts(fonts);
                self.status = format!("Loaded font {name}");
            }
            Err(error) => self.status = format!("Could not load {}: {error}", path.display()),
        }
    }

    fn show_repl(&mut self, ui: &mut egui::Ui, context: &egui::Context, theme: &Theme) {
        let enter_pressed = context.input(|input| {
            input.key_pressed(egui::Key::Enter)
                && !input.modifiers.shift
                && !input.modifiers.ctrl
                && !input.modifiers.command
        });
        let history_up = context.input(|input| input.key_pressed(egui::Key::ArrowUp));
        let history_down = context.input(|input| input.key_pressed(egui::Key::ArrowDown));
        let history_height = (ui.available_height() - 154.0).max(120.0);
        egui::ScrollArea::vertical()
            .id_source("gui-repl-history")
            .max_height(history_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.repl_entries.is_empty() {
                    ui.add_space(28.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Start a calculation")
                                .strong()
                                .size(18.0)
                                .color(theme.ui.text.0),
                        );
                        ui.label(
                            RichText::new(
                                "Enter an expression below. Definitions stay available for the session.",
                            )
                            .size(12.0)
                            .color(theme.ui.muted.0),
                        );
                        ui.add_space(6.0);
                        ui.monospace("Try:  f(x) = x^2 - 2;   then   plot(f(x), x, -4, 4) =>;");
                    });
                }
                for (index, entry) in self.repl_entries.iter().enumerate() {
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(12.0, 12.0))
                        .show(ui, |ui| {
                            // LaTeX first: the typeset input is the primary
                            // display; the raw source stays as a faint echo.
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{}", index + 1))
                                        .monospace()
                                        .size(10.5)
                                        .color(theme.ui.muted.0),
                                );
                                if self.system_latex_preview {
                                    if let Some(key) = entry.input_math_key {
                                        if let Some(texture) = self.repl_math_cache.get(&key) {
                                            show_cm_texture(ui, texture, ui.available_width());
                                        } else if let Some(error) = &self.repl_math_error {
                                            ui.colored_label(theme.syntax.operator.0, error);
                                        } else {
                                            ui.label(
                                                RichText::new("Typesetting in Computer Modern…")
                                                    .italics()
                                                    .color(theme.ui.muted.0),
                                            );
                                        }
                                    } else if entry.input_latex.is_none() {
                                        ui.label(
                                            RichText::new(&entry.source)
                                                .monospace()
                                                .color(theme.ui.text.0),
                                        );
                                    }
                                } else if let Some(latex) = &entry.input_latex {
                                    math_preview::show_expression(
                                        ui,
                                        latex,
                                        theme.ui.text.0,
                                        22.0,
                                    );
                                } else {
                                    ui.label(
                                        RichText::new(&entry.source)
                                            .monospace()
                                            .size(13.0)
                                            .color(theme.ui.text.0),
                                    );
                                }
                                if self.system_latex_preview {
                                    if let Some(key) = entry.result_math_key {
                                        if let Some(texture) = self.repl_math_cache.get(&key) {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| show_cm_texture(ui, texture, ui.available_width()),
                                            );
                                        }
                                    }
                                } else if let Some(latex) = &entry.result_latex {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            math_preview::show_expression(
                                                ui,
                                                latex,
                                                theme.ui.accent.0,
                                                22.0,
                                            );
                                        },
                                    );
                                }
                            });
                            if entry.input_latex.is_some() {
                                ui.label(
                                    RichText::new(&entry.source)
                                        .monospace()
                                        .size(9.5)
                                        .color(theme.ui.muted.0),
                                );
                            }
                            if let Some(plot_outcome) = &entry.plot {
                                show_inline_plot(ui, plot_outcome, theme);
                            }
                            if !entry.printed.trim().is_empty() && entry.plot.is_none() {
                                ui.label(
                                    RichText::new(entry.printed.trim_end())
                                        .monospace()
                                        .size(12.0),
                                );
                            }
                            if let Some(error) = &entry.error {
                                ui.colored_label(theme.syntax.operator.0, error);
                            }
                        });
                    ui.separator();
                    ui.add_space(8.0);
                }
            });

        ui.add_space(8.0);
        let input_id = egui::Id::new("gui-repl-input");
        let mut submit = false;
        egui::Frame::none()
            .fill(theme.ui.panel.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .stroke(egui::Stroke::new(
                1.0,
                theme.ui.accent.0.gamma_multiply(0.65),
            ))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("LIVE MATH ENTRY")
                            .strong()
                            .size(10.5)
                            .color(theme.ui.accent.0),
                    );
                    ui.label(
                        RichText::new("Enter to evaluate · ↑/↓ history · :plot draws a graph")
                            .size(10.5)
                            .color(theme.ui.muted.0),
                    );
                });
                ui.add_space(5.0);
                // LaTeX-first entry: the live typeset preview is the main
                // display, and the editable source sits underneath as a
                // compact one-line input.
                egui::Frame::none()
                    .fill(theme.ui.editor.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 9.0))
                    .show(ui, |ui| {
                        ui.set_min_height(56.0);
                        if !self.system_latex_preview {
                            if let Some(latex) = &self.repl_input_latex {
                                math_preview::show_expression(ui, latex, theme.ui.text.0, 26.0);
                            } else if self.repl_input.trim().is_empty() {
                                ui.label(
                                    RichText::new("Your expression will typeset here as you work")
                                        .italics()
                                        .color(theme.ui.muted.0),
                                );
                            } else {
                                ui.label(
                                    RichText::new("Keep typing to complete the expression…")
                                        .italics()
                                        .color(theme.ui.muted.0),
                                );
                            }
                        } else if let Some(key) = self.repl_input_math_key {
                            if let Some(texture) = self.repl_math_cache.get(&key) {
                                show_cm_texture(ui, texture, ui.available_width());
                            } else if let Some(error) = &self.repl_math_error {
                                ui.colored_label(theme.syntax.operator.0, error);
                            } else {
                                ui.label(RichText::new("Typesetting in Computer Modern…").italics().color(theme.ui.muted.0));
                            }
                        } else if self.repl_input.trim().is_empty() {
                            ui.label(
                                RichText::new("Your expression will typeset here as you work")
                                    .italics()
                                    .color(theme.ui.muted.0),
                            );
                        } else {
                            ui.label(
                                RichText::new("Keep typing to complete the expression…")
                                    .italics()
                                    .color(theme.ui.muted.0),
                            );
                        }
                    });
                ui.add_space(4.0);
                let response = ui.add(
                    TextEdit::singleline(&mut self.repl_input)
                        .id(input_id)
                        .font(TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type SCAS math, for example integral(sin(x), x) — or :plot sin(x)"),
                );
                let input_focused = response.has_focus();
                // Tab completes builtin names (and `:plot …` expressions)
                // using the same prefix engine as the editor.
                let cursor_chars = self.repl_input.chars().count();
                let repl_matches = editor_assist::completions(&self.repl_input, cursor_chars);
                let tab_complete = input_focused
                    && repl_matches.len > 0
                    && context.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    });
                if let Some(completion) = tab_complete.then(|| repl_matches.first()).flatten() {
                    let insertion = completion.insertion.text();
                    let marker = insertion.find("$0");
                    let replacement = marker.map_or_else(
                        || insertion.clone(),
                        |index| {
                            let mut text = String::with_capacity(insertion.len() - 2);
                            text.push_str(&insertion[..index]);
                            text.push_str(&insertion[index + 2..]);
                            text
                        },
                    );
                    let start_byte =
                        char_to_byte(&self.repl_input, repl_matches.start_char);
                    self.repl_input.replace_range(start_byte.., &replacement);
                    self.refresh_repl_latex();
                    context.memory_mut(|memory| memory.request_focus(input_id));
                    context.request_repaint();
                }
                if response.changed() {
                    if self.auto_greek {
                        expand_greek_shortcuts(&mut self.repl_input);
                    }
                    self.refresh_repl_latex();
                    self.repl_history_cursor = None;
                    context.request_repaint();
                }
                if input_focused && history_up {
                    self.repl_history_step(-1);
                    context.request_repaint();
                }
                if input_focused && history_down {
                    self.repl_history_step(1);
                    context.request_repaint();
                }
                submit = input_focused && enter_pressed;
                if let Some(error) = &self.repl_math_error {
                    ui.colored_label(theme.syntax.operator.0, format!("Computer Modern unavailable: {error}"));
                }
                ui.horizontal(|ui| {
                    if ui.button("Evaluate").clicked() {
                        submit = true;
                    }
                    ui.label(
                        RichText::new("Commands: :plot sin(x) · :plot sin(x), cos(x) > graph.svg · :clear · :reset")
                            .size(10.5)
                            .color(theme.ui.muted.0),
                    );
                });
            });
        if submit {
            self.submit_repl();
            context.memory_mut(|memory| memory.request_focus(input_id));
        }
    }

    fn repl_history_step(&mut self, direction: isize) {
        if self.repl_history.is_empty() {
            return;
        }
        let cursor = self.repl_history_cursor.unwrap_or(self.repl_history.len());
        let next = cursor as isize + direction;
        if next < 0 {
            return;
        }
        let next = (next as usize).min(self.repl_history.len());
        if next == self.repl_history.len() {
            self.repl_history_cursor = None;
            self.repl_input.clear();
        } else {
            self.repl_history_cursor = Some(next);
            let recalled = self.repl_history[next].clone();
            self.repl_input = recalled;
            self.refresh_repl_latex();
        }
    }

    fn show_graphing(&mut self, ui: &mut egui::Ui, context: &egui::Context, theme: &Theme) {
        self.ensure_graph_coverage();

        egui::SidePanel::right("graph-functions")
            .default_width(300.0)
            .width_range(220.0..=460.0)
            .resizable(true)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 8.0)),
            )
            .show_inside(ui, |ui| {
                ui.label(
                    RichText::new("FUNCTIONS")
                        .strong()
                        .size(11.0)
                        .color(theme.ui.accent.0),
                );
                ui.label(
                    RichText::new("Evaluated in the live REPL session")
                        .size(10.5)
                        .color(theme.ui.muted.0),
                );
                ui.add_space(6.0);
                let input_id = egui::Id::new("graph-new-input");
                let mut add = false;
                let response = ui.add(
                    TextEdit::singleline(&mut self.graph_new_input)
                        .id(input_id)
                        .font(TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .hint_text("sin(x)  or  g(t) = 2t + 1"),
                );
                if response.changed() && self.auto_greek {
                    expand_greek_shortcuts(&mut self.graph_new_input);
                }
                add |= response.lost_focus() && context.input(|input| input.key_pressed(egui::Key::Enter));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Add function").clicked() {
                        add = true;
                    }
                    if ui.button("Fit").on_hover_text("Refit the y window to the data").clicked() {
                        self.graph_view.auto_y = true;
                    }
                    if ui
                        .button("Reset view")
                        .on_hover_text("Restore the default -11 to 11 window")
                        .clicked()
                    {
                        self.graph_view = ViewState::centered(22.0, 12.0);
                        self.graph_view.auto_y = false;
                    }
                });
                if ui
                    .checkbox(&mut self.graph_view.auto_y, "Auto-fit y window")
                    .changed()
 {
                    context.request_repaint();
                }
                ui.add_space(6.0);
                ui.separator();

                let mut removal = None;
                let mut replot_request = false;
                egui::ScrollArea::vertical()
                    .id_source("graph-function-list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.graph_functions.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("No functions yet")
                                    .size(12.0)
                                    .color(theme.ui.muted.0),
                            );
                            ui.label(
                                RichText::new("Add one above, or plot from the REPL with\n:plot sin(x), cos(x) from -6 to 6")
                                    .size(11.0)
                                    .color(theme.ui.muted.0),
                            );
                        }
                        for index in 0..self.graph_functions.len() {
                            let (sampled_ok, stale, label, detail) = {
                                let function = &self.graph_functions[index];
                                match &function.sampled {
                                    Ok(series) => (
                                        true,
                                        function.stale,
                                        series.label.clone(),
                                        format!("variable: {}", series.variable),
                                    ),
                                    Err(message) => (
                                        false,
                                        function.stale,
                                        function.source.clone(),
                                        message.clone(),
                                    ),
                                }
                            };
                            egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(4.0, 4.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let dot = GRAPH_COLORS[index % GRAPH_COLORS.len()];
                                        let (r, g, b) = (
                                            ((dot >> 16) & 0xff) as u8,
                                            ((dot >> 8) & 0xff) as u8,
                                            (dot & 0xff) as u8,
                                        );
                                        let swatch_height = ui.spacing().interact_size.y * 0.35;
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(16.0, swatch_height),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_filled(
                                            rect,
                                            2.0,
                                            egui::Color32::from_rgb(r, g, b),
                                        );
                                        ui.label(
                                            RichText::new(&label)
                                                .monospace()
                                                .size(12.0)
                                                .color(if stale { theme.ui.muted.0 } else { theme.ui.text.0 }),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                                                removal = Some(index);
                                            }
                                            if stale {
                                                if ui.small_button("↻").on_hover_text("Re-sample against the current session").clicked() {
                                                    self.graph_functions[index].stale = true;
                                                    replot_request = true;
                                                }
                                                ui.label(RichText::new("stale").size(10.0).color(theme.ui.muted.0));
                                            }
                                        });
                                    });
                                    if !sampled_ok {
                                        ui.label(
                                            RichText::new(&detail)
                                                .size(10.5)
                                                .color(theme.syntax.operator.0),
                                        );
                                    } else if !detail.is_empty() {
                                        ui.label(RichText::new(&detail).size(10.0).color(theme.ui.muted.0));
                                    }
                                });
                            ui.separator();
                        }
                    });
                if replot_request {
                    self.refresh_graph_functions();
                }
                if let Some(index) = removal {
                    self.graph_functions.remove(index);
                    self.graph_content_generation = self.graph_content_generation.wrapping_add(1);
                    self.graph_sample_due = None;
                    self.graph_refine_due = None;
                    self.schedule_graph_sampling(Duration::ZERO);
                }
                if add {
                    let source = self.graph_new_input.trim().to_owned();
                    if !source.is_empty() {
                        self.add_graph_function(&source);
                        self.graph_new_input.clear();
                        self.set_status(if source.contains('=') {
                            "Function defined in the REPL session and plotted"
                        } else {
                            "Function added to the plot"
                        });
                        context.memory_mut(|memory| memory.request_focus(input_id));
                    }
                }
            });

        let plot_rect = ui.available_rect_before_wrap();
        if (self.graph_canvas_width - plot_rect.width()).abs() > 1.0 {
            self.graph_canvas_width = plot_rect.width();
            self.schedule_graph_sampling(Duration::from_millis(100));
        }
        let canvas = ui.allocate_rect(plot_rect, egui::Sense::click_and_drag());

        // Interaction first: drag pans, scroll wheel zooms, click inspects.
        let mut panned = false;
        // Pointer delta is frame-local; `Response::drag_delta` accumulates
        // from the start of the gesture and would apply earlier motion again.
        let drag = if canvas.dragged() {
            context.input(|input| input.pointer.delta())
        } else {
            egui::Vec2::ZERO
        };
        if drag != egui::Vec2::ZERO {
            let scale_x = self.graph_view.x_span() / f64::from(plot_rect.width().max(1.0));
            let scale_y = self.graph_view.y_span() / f64::from(plot_rect.height().max(1.0));
            self.graph_view.auto_y = false;
            self.graph_view.x_min -= f64::from(drag.x) * scale_x;
            self.graph_view.x_max -= f64::from(drag.x) * scale_x;
            self.graph_view.y_min += f64::from(drag.y) * scale_y;
            self.graph_view.y_max += f64::from(drag.y) * scale_y;
            self.graph_auto_bounds = None;
            panned = true;
            self.ensure_graph_coverage();
        }
        let (scroll, pointer) = ui.input(|input| {
            (
                input.raw_scroll_delta.y,
                input.pointer.hover_pos().unwrap_or(plot_rect.center()),
            )
        });
        if canvas.hovered() && scroll.abs() > 0.0 {
            let factor = f64::from((-scroll * 0.0015).exp()).clamp(0.5, 2.0);
            let new_x_span = self.graph_view.x_span() * factor;
            let new_y_span = self.graph_view.y_span() * factor;
            // Ignore wheel ticks that would drive the window outside sane
            // spans so ticks, sampling, and mapping stay well-defined.
            if (MIN_VIEW_SPAN..=MAX_VIEW_SPAN).contains(&new_x_span)
                && (MIN_VIEW_SPAN..=MAX_VIEW_SPAN).contains(&new_y_span)
            {
                let x_ratio =
                    f64::from((pointer.x - plot_rect.left()) / plot_rect.width().max(1.0));
                let y_ratio =
                    f64::from((pointer.y - plot_rect.top()) / plot_rect.height().max(1.0));
                let anchor_x = self.graph_view.x_min + x_ratio * self.graph_view.x_span();
                let anchor_y = self.graph_view.y_max - y_ratio * self.graph_view.y_span();
                self.graph_view.auto_y = false;
                self.graph_view.x_min = anchor_x - (anchor_x - self.graph_view.x_min) * factor;
                self.graph_view.x_max = anchor_x + (self.graph_view.x_max - anchor_x) * factor;
                self.graph_view.y_min = anchor_y - (anchor_y - self.graph_view.y_min) * factor;
                self.graph_view.y_max = anchor_y + (self.graph_view.y_max - anchor_y) * factor;
                self.graph_auto_bounds = None;
                panned = true;
                self.ensure_graph_coverage();
            }
        }
        if panned {
            self.graph_probe = None;
        }

        let bounds = self.current_graph_bounds();
        let painter = ui.painter_at(plot_rect);

        // A tap (click without a preceding drag) pins the nearest curve
        // value or critical point and shows a crosshair with its x/y. The
        // snap happens once; the crosshair itself is redrawn from data
        // coordinates every frame.
        if canvas.clicked() {
            self.graph_probe = canvas
                .interact_pointer_pos()
                .filter(|pointer| plot_rect.contains(*pointer))
                .and_then(|pointer| {
                    snap_graph_probe(pointer, &bounds, plot_rect, &self.graph_functions)
                });
        }

        draw_graph_canvas(painter, &bounds, &self.graph_functions, theme, plot_rect);

        if let Some(probe) = self.graph_probe {
            draw_graph_probe(
                ui.painter_at(plot_rect),
                &probe,
                &bounds,
                plot_rect,
                theme,
                &self.graph_functions,
            );
        }

        // Footer readout over the canvas.
        egui::Area::new(egui::Id::new("graph-footer"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-14.0, -12.0])
            .show(ui.ctx(), |ui| {
                egui::Frame::none()
                    .fill(theme.ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.ui.muted.0.gamma_multiply(0.35),
                    ))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "x: {} … {}   y: {} … {}   drag to pan · scroll to zoom · click a curve",
                                plot::format_number(bounds.x_min),
                                plot::format_number(bounds.x_max),
                                plot::format_number(bounds.y_min),
                                plot::format_number(bounds.y_max),
                            ))
                            .size(10.5)
                            .color(theme.ui.muted.0),
                        );
                    });
            });
    }

    fn current_graph_bounds(&mut self) -> PlotBounds {
        let view = self.graph_view;
        if !view.auto_y {
            return view.bounds();
        }
        if let Some(bounds) = self.graph_auto_bounds {
            return PlotBounds::new(view.x_min, view.x_max, bounds.y_min, bounds.y_max);
        }
        let live: Vec<SampledSeries> = self
            .graph_functions
            .iter()
            .filter_map(|function| function.sampled.as_ref().ok().cloned())
            .collect();
        if live.is_empty() {
            return view.bounds();
        }
        let bounds = plot::auto_y_bounds(&live, view.x_min, view.x_max);
        self.graph_auto_bounds = Some(bounds);
        bounds
    }

    fn settings(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut chosen_theme = None;
        let mut load_theme = false;
        let mut load_font = false;
        let previous_system_preview = self.system_latex_preview;
        egui::Window::new("Editor settings")
            .open(&mut open)
            .resizable(false)
            .show(context, |ui| {
                ui.label("Color scheme");
                egui::ComboBox::from_id_source("theme-picker")
                    .selected_text(&self.themes[self.selected_theme].name)
                    .show_ui(ui, |ui| {
                        for (index, theme) in self.themes.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.selected_theme, index, &theme.name)
                                .changed()
                            {
                                chosen_theme = Some(index);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Theme file");
                    ui.text_edit_singleline(&mut self.theme_path);
                    load_theme = ui.button("Load").clicked();
                });
                ui.separator();
                ui.label("Line numbers");
                egui::ComboBox::from_id_source("line-number-mode")
                    .selected_text(self.line_numbers.label())
                    .show_ui(ui, |ui| {
                        for mode in [
                            LineNumberMode::Off,
                            LineNumberMode::Absolute,
                            LineNumberMode::Relative,
                            LineNumberMode::Hybrid,
                        ] {
                            ui.selectable_value(&mut self.line_numbers, mode, mode.label());
                        }
                    });
                ui.separator();
                ui.label("Preview performance");
                ui.checkbox(&mut self.system_latex_preview, "Computer Modern preview");
                ui.small(
                    "Uses system LaTeX for faithful typesetting. Off uses the fast structured fallback.",
                );
                ui.separator();
                ui.label("Editor font");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.editor_monospace, true, "Monospace");
                    ui.radio_value(&mut self.editor_monospace, false, "Proportional");
                    ui.add(egui::Slider::new(&mut self.editor_font_size, 9.0..=32.0).suffix(" pt"));
                });
                ui.horizontal(|ui| {
                    ui.label("Font file");
                    ui.text_edit_singleline(&mut self.font_path);
                    load_font = ui.button("Load").clicked();
                });
                ui.small("Theme files use TOML; font files may be .ttf or .otf.");
            });
        self.settings_open = open;
        if !previous_system_preview && self.system_latex_preview {
            if let Some(document) = &self.preview_document {
                let body = document.body();
                let color = self.themes[self.selected_theme].ui.text.0;
                self.start_latex_render(body, [color.r(), color.g(), color.b()]);
            }
            self.refresh_repl_latex();
        } else if previous_system_preview && !self.system_latex_preview {
            self.latex_pending = None;
            self.repl_math_pending.clear();
            self.repl_input_math_key = None;
        }
        if let Some(index) = chosen_theme {
            self.themes[index].apply(context);
            self.status = format!("Using {}", self.themes[index].name);
            if let Some(document) = &self.preview_document {
                let color = self.themes[self.selected_theme].ui.text.0;
                self.start_latex_render(document.body(), [color.r(), color.g(), color.b()]);
            }
        }
        if load_theme {
            self.load_theme(context);
        }
        if load_font {
            self.load_font(context);
        }
    }
}

/// Persistent graph sampling worker. One interpreter stays alive across
/// batches and replays each successful REPL submission only once, so pans,
/// zooms, and edits re-sample curves instead of rebuilding the session for
/// every request. Queued requests supersede each other: only the newest
/// window is measured, and an in-flight batch is abandoned as soon as the
/// function list or session content changes. Sampling runs off the UI
/// thread so expensive expressions never stall gestures.
fn run_graph_sampler(requests: Receiver<GraphBatchRequest>, results: Sender<GraphBatchResult>) {
    let mut symbols = SymbolTable::new();
    let mut interpreter: Interpreter<Vec<u8>> = Interpreter::new(symbols.clone(), Vec::new());
    let mut replayed: Vec<String> = Vec::new();
    let mut pending: Option<GraphBatchRequest> = None;
    while let Some(request) = pending.take().or_else(|| requests.recv().ok()) {
        // Collapse the queue: only the newest window matters.
        let mut request = request;
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        // Rebuild the interpreter when the session diverges (e.g. `:reset`).
        if !request.session.starts_with(&replayed) {
            symbols = SymbolTable::new();
            interpreter = Interpreter::new(symbols.clone(), Vec::new());
            replayed.clear();
        }
        // Replay only the submissions this interpreter has not executed yet.
        for source in request.session[replayed.len()..].iter().cloned() {
            let submission = terminate_repl_source(&source);
            let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
            let tokens = scanner.scan_tokens().clone();
            if !scanner.had_error() {
                symbols = scanner.into_symbols();
                interpreter.set_symbols(symbols.clone());
                if let Ok(program) = Parser::new(tokens).parse() {
                    let _ = interpreter.run_interactive_value(&program);
                }
            }
            replayed.push(source);
        }

        let mut samples = Vec::with_capacity(request.sources.len());
        let mut superseded = false;
        for source in &request.sources {
            // Abandon the batch once the function list or session changed;
            // newer windows merely queue up and never cancel a good run.
            let mut should_cancel = || {
                while let Ok(newer) = requests.try_recv() {
                    pending = Some(newer);
                }
                pending
                    .as_ref()
                    .is_some_and(|newer| newer.content_generation != request.content_generation)
            };
            let outcome = plot::sample_source_cancellable(
                &mut interpreter,
                &mut symbols,
                source,
                request.x_min,
                request.x_max,
                request.count,
                &mut should_cancel,
            );
            let Some(outcome) = outcome else {
                superseded = true;
                break;
            };
            samples.push(outcome);
        }
        if superseded {
            continue;
        }
        let _ = results.send(GraphBatchResult {
            id: request.id,
            content_generation: request.content_generation,
            x_min: request.x_min,
            x_max: request.x_max,
            count: request.count,
            samples,
        });
    }
}

fn show_cm_texture(ui: &mut egui::Ui, texture: &egui::TextureHandle, max_width: f32) {
    let natural = texture.size_vec2();
    let max_width = if max_width.is_finite() {
        max_width.max(1.0)
    } else {
        natural.x.max(1.0)
    };
    let scale = (max_width / natural.x.max(1.0)).min(1.0);
    ui.add(egui::Image::from_texture(texture).fit_to_exact_size(natural * scale));
}

/// Builds an isolated plot interpreter by replaying successful REPL source,
/// then samples every graph function in that single interpreter. Used by the
/// tests to exercise the sampling pipeline synchronously.
#[cfg(test)]
fn sample_graph_batch(
    session: &[String],
    sources: &[String],
    mut symbols: SymbolTable,
    x_min: f64,
    x_max: f64,
    count: usize,
) -> Vec<Result<SampledSeries, String>> {
    let mut interpreter: Interpreter<Vec<u8>> = Interpreter::new(symbols.clone(), Vec::new());
    for source in session {
        let submission = terminate_repl_source(source);
        let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        if scanner.had_error() {
            continue;
        }
        symbols = scanner.into_symbols();
        interpreter.set_symbols(symbols.clone());
        let Ok(program) = Parser::new(tokens).parse() else {
            continue;
        };
        let _ = interpreter.run_interactive_value(&program);
    }
    sources
        .iter()
        .map(|source| {
            let mut local_symbols = symbols.clone();
            plot::sample_source(
                &mut interpreter,
                &mut local_symbols,
                source,
                x_min,
                x_max,
                count,
            )
        })
        .collect()
}

/// Draws the graphing-canvas axes, grid, and curves from cached samples.
fn draw_graph_canvas(
    painter: egui::Painter,
    bounds: &PlotBounds,
    functions: &[GraphFunction],
    theme: &Theme,
    rect: egui::Rect,
) {
    let background = theme.ui.editor.0;
    painter.rect_filled(rect, 0.0, background);
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let to_screen_x = |x: f64| rect.left() + ((x - bounds.x_min) / x_span) as f32 * rect.width();
    let to_screen_y = |y: f64| rect.top() + ((bounds.y_max - y) / y_span) as f32 * rect.height();

    let grid_minor = theme.ui.muted.0.gamma_multiply(0.16);
    let grid_axis = theme.ui.muted.0.gamma_multiply(0.45);
    let step_x = plot::nice_step(x_span, 10.0);
    let step_y = plot::nice_step(y_span, 8.0);
    let mut tick = (bounds.x_min / step_x).ceil() * step_x;
    while tick <= bounds.x_max + step_x * 0.001 {
        if tick.is_finite() {
            let screen = to_screen_x(tick);
            let is_axis = tick.abs() <= step_x * 0.001;
            painter.line_segment(
                [
                    egui::pos2(screen, rect.top()),
                    egui::pos2(screen, rect.bottom()),
                ],
                egui::Stroke::new(1.0, if is_axis { grid_axis } else { grid_minor }),
            );
            if !is_axis {
                painter.text(
                    egui::pos2(screen, rect.bottom() - 6.0),
                    egui::Align2::CENTER_TOP,
                    plot::format_number(tick),
                    FontId::monospace(10.0),
                    theme.ui.muted.0,
                );
            }
        }
        tick += step_x;
    }
    let mut tick = (bounds.y_min / step_y).ceil() * step_y;
    while tick <= bounds.y_max + step_y * 0.001 {
        if tick.is_finite() {
            let screen = to_screen_y(tick);
            let is_axis = tick.abs() <= step_y * 0.001;
            painter.line_segment(
                [
                    egui::pos2(rect.left(), screen),
                    egui::pos2(rect.right(), screen),
                ],
                egui::Stroke::new(1.0, if is_axis { grid_axis } else { grid_minor }),
            );
            if !is_axis {
                painter.text(
                    egui::pos2(rect.left() + 6.0, screen),
                    egui::Align2::LEFT_TOP,
                    plot::format_number(tick),
                    FontId::monospace(10.0),
                    theme.ui.muted.0,
                );
            }
        }
        tick += step_y;
    }
    for (index, function) in functions.iter().enumerate() {
        let Ok(series) = &function.sampled else {
            continue;
        };
        let color = curve_color(index);
        let stroke = egui::Stroke::new(2.2, color);
        let first = series
            .samples
            .partition_point(|(x, _)| *x < bounds.x_min)
            .saturating_sub(1);
        let last = series
            .samples
            .partition_point(|(x, _)| *x <= bounds.x_max)
            .saturating_add(1)
            .min(series.samples.len());
        // During a zoom-in, draw a viewport-resolution subset of
        // the wider cache instead of submitting every offscreen
        // high-density sample to the painter.
        let target = (rect.width().ceil() as usize).saturating_mul(2).max(2);
        let visible = last.saturating_sub(first);
        let samples = &series.samples[first..last];
        let indices = if visible > target {
            decimate_visible_samples(samples, bounds, rect.width().ceil() as usize, y_span)
        } else {
            (0..samples.len()).collect()
        };
        let mut path = Vec::with_capacity(indices.len());
        let mut previous_y: Option<f64> = None;
        for index in indices {
            let (x, y) = samples[index];
            let Some(y) = y else {
                if path.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(&mut path), stroke));
                }
                path.clear();
                previous_y = None;
                continue;
            };
            if previous_y.is_some_and(|previous| (y - previous).abs() > y_span * 2.0) {
                if path.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(&mut path), stroke));
                }
                path.clear();
            }
            path.push(egui::pos2(to_screen_x(x), to_screen_y(y)));
            previous_y = Some(y);
        }
        if path.len() > 1 {
            painter.add(egui::Shape::line(path, stroke));
        }
        // Desmos-style dots on zeros and local extrema.
        for point in &function.critical_points {
            if !bounds.contains_x(point.x) || !bounds.contains_y(point.y) {
                continue;
            }
            let screen = egui::pos2(to_screen_x(point.x), to_screen_y(point.y));
            painter.circle_filled(screen, 4.0, color);
            painter.circle_filled(screen, 2.0, background);
        }
    }
}

/// Keeps a pixel-scale envelope when the cached curve is denser than the
/// display. Selecting each bucket's first, last, minimum, and maximum samples
/// preserves narrow peaks and valleys while bounding painter work by width.
fn decimate_visible_samples(
    samples: &[(f64, Option<f64>)],
    bounds: &PlotBounds,
    pixel_width: usize,
    y_span: f64,
) -> Vec<usize> {
    let pixel_width = pixel_width.max(1);
    let x_span = bounds.x_span();
    let bucket_for = |x: f64| {
        (((x - bounds.x_min) / x_span) * pixel_width as f64)
            .floor()
            .clamp(0.0, pixel_width.saturating_sub(1) as f64) as usize
    };
    // (first, last, minimum, maximum, pixel bucket)
    let mut bucket: Option<(usize, usize, usize, usize, usize)> = None;
    let mut output = Vec::with_capacity(pixel_width.saturating_mul(4));
    let flush = |bucket: &mut Option<(usize, usize, usize, usize, usize)>,
                 output: &mut Vec<usize>| {
        if let Some((first, last, minimum, maximum, _)) = bucket.take() {
            let mut selected = [first, minimum, maximum, last];
            selected.sort_unstable();
            for index in selected {
                if output.last() != Some(&index) {
                    output.push(index);
                }
            }
        }
    };

    for (index, (x, sample_y)) in samples.iter().enumerate() {
        let Some(y) = sample_y.filter(|y| y.is_finite()) else {
            flush(&mut bucket, &mut output);
            if output
                .last()
                .map_or(true, |previous| samples[*previous].1.is_some())
            {
                output.push(index);
            }
            continue;
        };
        let pixel = bucket_for(*x);
        if let Some((_, last, _, _, bucket_pixel)) = bucket {
            let previous_y = samples[last].1.unwrap_or(y);
            if bucket_pixel != pixel || (y - previous_y).abs() > y_span * 2.0 {
                flush(&mut bucket, &mut output);
            }
        }
        match bucket.as_mut() {
            Some((_, last, minimum, maximum, _)) => {
                *last = index;
                if y < samples[*minimum].1.unwrap_or(y) {
                    *minimum = index;
                }
                if y > samples[*maximum].1.unwrap_or(y) {
                    *maximum = index;
                }
            }
            None => bucket = Some((index, index, index, index, pixel)),
        }
    }
    flush(&mut bucket, &mut output);
    output
}

/// Draw-order color for a graphing-canvas curve.
fn curve_color(index: usize) -> egui::Color32 {
    let raw = GRAPH_COLORS[index % GRAPH_COLORS.len()];
    egui::Color32::from_rgb(
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
    )
}

/// Snaps a tapped screen point to the nearest curve: finds the closest
/// sample in screen space across every plotted series and reports the
/// series index and the snapped position in data coordinates.
fn snap_graph_probe(
    pointer: egui::Pos2,
    bounds: &PlotBounds,
    rect: egui::Rect,
    functions: &[GraphFunction],
) -> Option<GraphProbe> {
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let to_screen_x = |x: f64| rect.left() + ((x - bounds.x_min) / x_span) as f32 * rect.width();
    let to_screen_y = |y: f64| rect.top() + ((bounds.y_max - y) / y_span) as f32 * rect.height();
    let mut best: Option<(f32, GraphProbe)> = None;
    for (index, function) in functions.iter().enumerate() {
        let Ok(series) = &function.sampled else {
            continue;
        };
        // Only samples inside the visible window can be tapped.
        let first = series.samples.partition_point(|(x, _)| *x < bounds.x_min);
        let last = series.samples.partition_point(|(x, _)| *x <= bounds.x_max);
        for (x, y) in &series.samples[first..last] {
            let Some(y) = y else {
                continue;
            };
            let sample = egui::pos2(to_screen_x(*x), to_screen_y(*y));
            let distance = pointer.distance(sample);
            if best.map_or(true, |(best_distance, _)| distance < best_distance) {
                best = Some((
                    distance,
                    GraphProbe {
                        series_index: index,
                        x: *x,
                        y: Some(*y),
                    },
                ));
            }
        }
    }
    best.map(|(_, probe)| probe)
}

/// Draws the crosshair and coordinate label for a snapped probe point.
fn draw_graph_probe(
    painter: egui::Painter,
    probe: &GraphProbe,
    bounds: &PlotBounds,
    rect: egui::Rect,
    theme: &Theme,
    functions: &[GraphFunction],
) {
    let Some(series) = functions
        .get(probe.series_index)
        .and_then(|function| function.sampled.as_ref().ok())
    else {
        return;
    };
    let Some(y) = probe.y else {
        return;
    };
    let x = probe.x;
    let y_span = bounds.y_span();
    let x_span = bounds.x_span();
    if x_span <= 0.0 || y_span <= 0.0 {
        return;
    }
    let data_to_screen_y =
        |y: f64| rect.top() + (1.0 - (y - bounds.y_min) / y_span) as f32 * rect.height();
    let data_to_screen_x =
        |x: f64| rect.left() + ((x - bounds.x_min) / x_span) as f32 * rect.width();
    let screen = egui::pos2(data_to_screen_x(x), data_to_screen_y(y));
    if !rect.contains(screen) {
        return;
    }
    let color = curve_color(probe.series_index);
    let accent = theme.ui.accent.0;

    // Guides from the point down to the x axis and across to the y axis.
    let x_axis_y = data_to_screen_y(0.0).clamp(rect.top(), rect.bottom());
    let y_axis_x = data_to_screen_x(0.0).clamp(rect.left(), rect.right());
    let guide = egui::Stroke::new(1.0, accent.gamma_multiply(0.55));
    painter.line_segment(
        [
            screen,
            egui::pos2(screen.x, x_axis_y.clamp(rect.top(), rect.bottom())),
        ],
        guide,
    );
    painter.line_segment(
        [
            screen,
            egui::pos2(y_axis_x.clamp(rect.left(), rect.right()), screen.y),
        ],
        guide,
    );
    painter.circle_filled(screen, 5.0, color);
    painter.circle_filled(screen, 2.5, theme.ui.editor.0);

    let label = format!(
        "{} = {}   y = {}",
        series.variable,
        plot::format_number(x),
        plot::format_number(y)
    );
    let galley = painter.layout_no_wrap(label, FontId::monospace(11.0), theme.ui.text.0);
    let padding = 6.0;
    let size = galley.size() + egui::vec2(padding * 2.0, padding * 2.0);
    let above = screen.y - size.y - 12.0 > rect.top();
    let mut anchor = if above {
        egui::pos2(screen.x, screen.y - 12.0)
    } else {
        egui::pos2(screen.x, screen.y + 12.0)
    };
    anchor.x = anchor
        .x
        .clamp(rect.left() + size.x / 2.0, rect.right() - size.x / 2.0);
    let label_rect = egui::Rect::from_center_size(anchor, size);
    painter.rect_filled(label_rect, 4.0, theme.ui.panel.0);
    painter.rect_stroke(label_rect, 4.0, egui::Stroke::new(1.0, color));
    painter.galley(
        label_rect.left_top() + egui::vec2(padding, padding),
        galley,
        theme.ui.text.0,
    );
}

/// Renders a REPL `:plot` outcome as an inline card under the entry.
fn show_inline_plot(ui: &mut egui::Ui, outcome: &PlotOutcome, theme: &Theme) {
    ui.add_space(6.0);
    egui::Frame::none()
        .fill(theme.ui.editor.0)
        .inner_margin(egui::Margin::same(10.0))
        .stroke(egui::Stroke::new(1.0, theme.ui.muted.0.gamma_multiply(0.3)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("PLOT")
                        .strong()
                        .size(10.0)
                        .color(theme.ui.accent.0),
                );
                ui.label(
                    RichText::new(format!(
                        "x: {} … {}   y: {} … {}",
                        plot::format_number(outcome.bounds.x_min),
                        plot::format_number(outcome.bounds.x_max),
                        plot::format_number(outcome.bounds.y_min),
                        plot::format_number(outcome.bounds.y_max),
                    ))
                    .size(10.0)
                    .color(theme.ui.muted.0),
                );
                if let Some(path) = &outcome.wrote_file {
                    ui.label(
                        RichText::new(format!("wrote {}", path.display()))
                            .size(10.0)
                            .color(theme.ui.muted.0),
                    );
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(&outcome.ascii)
                    .monospace()
                    .size(11.0)
                    .color(theme.ui.text.0),
            );
        });
    ui.add_space(4.0);
}

fn terminate_repl_source(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.ends_with(';') || trimmed.ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    }
}

fn repl_expression_latex(source: &str, symbols: &SymbolTable) -> Option<String> {
    let source = source.trim();
    if source.is_empty() || source.starts_with(':') {
        return None;
    }
    let submission = terminate_repl_source(source);
    let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
    let tokens = scanner.scan_tokens().clone();
    if scanner.had_error() {
        return None;
    }
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().ok()?;
    let renderer = LatexRenderer::new(&symbols);
    match &program.last()?.kind {
        StatementKind::Expr(expression) | StatementKind::Print(expression) => {
            Some(renderer.expression(expression))
        }
        StatementKind::Let {
            name,
            value: Some(value),
        } => Some(format!(
            "{} = {}",
            renderer.identifier(symbols.resolve(*name)),
            renderer.expression(value)
        )),
        StatementKind::Function {
            name,
            params,
            body: FunctionBody::Expression(body),
        } => {
            let params = params
                .iter()
                .map(|param| renderer.identifier(symbols.resolve(*param)))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "{}\\left({params}\\right) = {}",
                renderer.identifier(symbols.resolve(*name)),
                renderer.expression(body)
            ))
        }
        _ => None,
    }
}

impl eframe::App for ScasApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_run(context);
        self.poll_analysis(context);
        self.poll_latex_render(context);
        self.poll_graph_sampling(context);
        self.poll_repl_math(context);
        self.settings(context);

        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            match self.workspace_tab {
                WorkspaceTab::Document => self.run(),
                WorkspaceTab::Repl => self.submit_repl(),
                WorkspaceTab::Graphing => {
                    let source = self.graph_new_input.trim().to_owned();
                    if !source.is_empty() {
                        self.add_graph_function(&source);
                        self.graph_new_input.clear();
                    }
                }
            }
        }

        let theme = self.themes[self.selected_theme].clone();
        let surface_title = match self.workspace_tab {
            WorkspaceTab::Document => self.title(),
            WorkspaceTab::Repl => "Interactive REPL".to_owned(),
            WorkspaceTab::Graphing => "Graphing calculator".to_owned(),
        };
        egui::TopBottomPanel::top("command-bar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.ui.muted.0.gamma_multiply(0.22),
                    )),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("SCAS")
                            .strong()
                            .size(13.0)
                            .color(theme.ui.text.0),
                    );
                    let button_frame = ui.visuals().button_frame;
                    let menu_widgets = ui.visuals().widgets.clone();
                    let menu_hover = theme.ui.accent.0.gamma_multiply(0.20);
                    let menu_active = theme.ui.accent.0.gamma_multiply(0.32);
                    ui.visuals_mut().button_frame = true;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.hovered.weak_bg_fill = menu_hover;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.active.weak_bg_fill = menu_active;
                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.open.weak_bg_fill = menu_hover;
                    ui.visuals_mut().widgets.open.bg_stroke = egui::Stroke::NONE;
                    ui.menu_button("File", |ui| {
                        if ui.button("New File").clicked() {
                            self.new_file();
                            ui.close_menu();
                        }
                        if ui.button("Save").clicked() {
                            self.save();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Export TeX…").clicked() {
                            self.export_tex();
                            ui.close_menu();
                        }
                        if ui.button("Export PDF…").clicked() {
                            self.export_pdf();
                            ui.close_menu();
                        }
                        if self.workspace_tab == WorkspaceTab::Graphing
                            && ui.button("Export Graph as SVG…").clicked()
                        {
                            self.export_graph_svg();
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui.checkbox(&mut self.files_open, "Explorer").clicked() {
                            ui.close_menu();
                        }
                        if ui
                            .checkbox(&mut self.preview_open, "Document Preview")
                            .clicked()
                        {
                            ui.close_menu();
                        }
                        if ui
                            .checkbox(&mut self.diagnostics_open, "Problems")
                            .clicked()
                        {
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Run", |ui| {
                        let label = match self.workspace_tab {
                            WorkspaceTab::Document => "Run Document",
                            WorkspaceTab::Repl => "Evaluate REPL Entry",
                            WorkspaceTab::Graphing => "Add Graph Function",
                        };
                        if ui
                            .add_enabled(
                                !(self.workspace_tab == WorkspaceTab::Document
                                    && self.run_result.is_some()),
                                egui::Button::new(label),
                            )
                            .clicked()
                        {
                            match self.workspace_tab {
                                WorkspaceTab::Document => self.run(),
                                WorkspaceTab::Repl => self.submit_repl(),
                                WorkspaceTab::Graphing => {
                                    let source = self.graph_new_input.trim().to_owned();
                                    if !source.is_empty() {
                                        self.add_graph_function(&source);
                                        self.graph_new_input.clear();
                                    }
                                }
                            }
                            ui.close_menu();
                        }
                    });
                    ui.visuals_mut().widgets = menu_widgets;
                    ui.visuals_mut().button_frame = button_frame;
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!("{surface_title} — SCAS"))
                            .size(12.0)
                            .color(theme.ui.muted.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let running = self.run_result.is_some();
                        let action_label = match self.workspace_tab {
                            WorkspaceTab::Document if running => "Running…",
                            WorkspaceTab::Document => "Run",
                            WorkspaceTab::Repl => "Evaluate",
                            WorkspaceTab::Graphing => "Add Function",
                        };
                        if ui
                            .add_enabled(
                                !(self.workspace_tab == WorkspaceTab::Document && running),
                                egui::Button::new(action_label)
                                    .fill(theme.ui.accent.0)
                                    .stroke(egui::Stroke::NONE),
                            )
                            .on_hover_text("Run the active workspace · Ctrl+Enter")
                            .clicked()
                        {
                            match self.workspace_tab {
                                WorkspaceTab::Document => self.run(),
                                WorkspaceTab::Repl => self.submit_repl(),
                                WorkspaceTab::Graphing => {
                                    let source = self.graph_new_input.trim().to_owned();
                                    if !source.is_empty() {
                                        self.add_graph_function(&source);
                                        self.graph_new_input.clear();
                                    }
                                }
                            }
                        }
                        if ui
                            .add(egui::Button::new(RichText::new("⚙").size(16.0)).frame(false))
                            .on_hover_text("Workspace settings")
                            .clicked()
                        {
                            self.settings_open = true;
                        }
                    });
                });
            });

        egui::SidePanel::left("activity-bar")
            .exact_width(48.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.background.0)
                    .inner_margin(egui::Margin::symmetric(4.0, 6.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.ui.muted.0.gamma_multiply(0.22),
                    )),
            )
            .show(context, |ui| {
                let explorer = activity_button(ui, ActivityIcon::Explorer, self.files_open, &theme)
                    .on_hover_text(if self.files_open {
                        "Hide Explorer"
                    } else {
                        "Show Explorer"
                    });
                if explorer.clicked() {
                    self.files_open = !self.files_open;
                }
                if activity_button(ui, ActivityIcon::Preview, self.preview_open, &theme)
                    .on_hover_text("Toggle mathematical preview")
                    .clicked()
                {
                    self.preview_open = !self.preview_open;
                }
                if activity_button(ui, ActivityIcon::Problems, self.diagnostics_open, &theme)
                    .on_hover_text("Toggle diagnostics")
                    .clicked()
                {
                    self.diagnostics_open = !self.diagnostics_open;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if activity_button(ui, ActivityIcon::Settings, self.settings_open, &theme)
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.settings_open = true;
                    }
                });
            });

        if self.files_open {
            egui::SidePanel::left("files")
                .default_width(240.0)
                .width_range(180.0..=320.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.panel.0)
                        .inner_margin(egui::Margin::symmetric(8.0, 7.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.20),
                        )),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("EXPLORER").strong().size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("×")
                                .on_hover_text("Hide Explorer")
                                .clicked()
                            {
                                self.files_open = false;
                            }
                            if ui
                                .small_button("↻")
                                .on_hover_text("Refresh files")
                                .clicked()
                            {
                                self.files = collect_tree(&self.project_root);
                            }
                        });
                    });
                    let root_name = self
                        .project_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("workspace")
                        .to_uppercase();
                    ui.add_space(5.0);
                    ui.label(RichText::new(root_name).strong().size(11.0));
                    ui.separator();
                    let mut selected = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut hidden_below = None;
                        for entry in self.files.clone() {
                            if let Some(depth) = hidden_below {
                                if entry.depth > depth {
                                    continue;
                                }
                                hidden_below = None;
                            }
                            let collapsed =
                                entry.is_dir && self.collapsed_dirs.contains(&entry.path);
                            let is_selected =
                                !entry.is_dir && self.current_file.as_ref() == Some(&entry.path);
                            let row_size =
                                egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                            let (row_rect, row_response) =
                                ui.allocate_exact_size(row_size, egui::Sense::click());
                            if is_selected {
                                ui.painter().rect_filled(
                                    row_rect,
                                    3.0,
                                    theme.ui.accent.0.gamma_multiply(0.26),
                                );
                            } else if row_response.hovered() {
                                ui.painter().rect_filled(
                                    row_rect,
                                    3.0,
                                    theme.ui.accent.0.gamma_multiply(0.16),
                                );
                            }
                            let mut row_ui = ui.child_ui(
                                row_rect,
                                egui::Layout::left_to_right(egui::Align::Center),
                                None,
                            );
                            row_ui.add_space(3.0 + entry.depth as f32 * 12.0);
                            let name = entry
                                .path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("?");
                            if entry.is_dir {
                                let disclosure = tree_disclosure(&mut row_ui, collapsed, &theme);
                                paint_tree_icon(&mut row_ui, true, &theme);
                                let name_response = row_ui.add(
                                    egui::Label::new(
                                        RichText::new(name).strong().color(theme.ui.text.0),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if row_response.clicked()
                                    || disclosure.clicked()
                                    || name_response.clicked()
                                {
                                    if collapsed {
                                        self.collapsed_dirs.remove(&entry.path);
                                    } else {
                                        self.collapsed_dirs.insert(entry.path.clone());
                                    }
                                }
                            } else {
                                row_ui.add_space(14.0);
                                paint_tree_icon(&mut row_ui, false, &theme);
                                let name_response = row_ui.add(
                                    egui::Label::new(name).sense(egui::Sense::click()),
                                );
                                if row_response.clicked() || name_response.clicked() {
                                    selected = Some(entry.path.clone());
                                }
                            }
                            if collapsed {
                                hidden_below = Some(entry.depth);
                            }
                        }
                    });
                    if let Some(path) = selected {
                        self.open(path);
                    }
                });
        }

        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(22.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.accent.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    let busy = self.run_result.is_some()
                        || self.analysis_result.is_some()
                        || self.analysis_due.is_some();
                    let status_mark = if busy { "WORKING" } else { "READY" };
                    let status_color = theme.contrast_on_accent();
                    ui.label(
                        RichText::new(status_mark)
                            .strong()
                            .size(11.0)
                            .color(status_color),
                    );
                    ui.label(RichText::new(&self.status).size(11.0).color(status_color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let workspace_status = match self.workspace_tab {
                            WorkspaceTab::Document => format!(
                                "SCAS   UTF-8   Ln {}   Problems {}",
                                self.current_line + 1,
                                self.diagnostics.len()
                            ),
                            WorkspaceTab::Repl => {
                                format!("SCAS   REPL   Entries {}", self.repl_entries.len())
                            }
                            WorkspaceTab::Graphing => format!(
                                "SCAS   GRAPHING{}   Functions {}   x {} … {}",
                                if self.graph_sampling {
                                    "   · Sampling"
                                } else {
                                    ""
                                },
                                self.graph_functions.len(),
                                plot::format_number(self.graph_view.x_min),
                                plot::format_number(self.graph_view.x_max)
                            ),
                        };
                        ui.label(
                            RichText::new(workspace_status)
                                .size(11.0)
                                .color(status_color),
                        );
                    });
                });
            });

        if self.workspace_tab == WorkspaceTab::Document {
            egui::TopBottomPanel::bottom("output")
                .default_height(170.0)
                .height_range(80.0..=420.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.editor.0)
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.22),
                        )),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                !self.diagnostics_open,
                                RichText::new("OUTPUT").strong().size(11.0),
                            )
                            .clicked()
                        {
                            self.diagnostics_open = false;
                        }
                        if ui
                            .selectable_label(
                                self.diagnostics_open,
                                RichText::new(format!("PROBLEMS {}", self.diagnostics.len()))
                                    .strong()
                                    .size(11.0),
                            )
                            .clicked()
                        {
                            self.diagnostics_open = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !self.diagnostics_open && ui.small_button("Copy").clicked() {
                                ui.output_mut(|clipboard| {
                                    clipboard.copied_text.clone_from(&self.output)
                                });
                                self.set_status("Output copied");
                            }
                        });
                    });
                    ui.separator();
                    if self.diagnostics_open {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if self.diagnostics.is_empty() {
                                ui.label(
                                    RichText::new(if self.analysis_due.is_some() {
                                        "Checking the document after typing pauses…"
                                    } else {
                                        "No problems detected in this document."
                                    })
                                    .color(theme.ui.muted.0),
                                );
                            } else {
                                for diagnostic in &self.diagnostics {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("ERROR")
                                                .strong()
                                                .size(10.5)
                                                .color(theme.syntax.operator.0),
                                        );
                                        ui.monospace(format!(
                                            "{}:{}  {}",
                                            diagnostic.line, diagnostic.column, diagnostic.message
                                        ));
                                    });
                                }
                            }
                        });
                    } else {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&self.output).monospace())
                                        .selectable(true)
                                        .extend(),
                                );
                            });
                    }
                });
        }

        if self.workspace_tab == WorkspaceTab::Document && self.preview_open {
            let explorer_width = if self.files_open {
                egui::containers::panel::PanelState::load(context, egui::Id::new("files"))
                    .map_or(240.0, |state| state.size().x)
            } else {
                0.0
            };
            let (preview_width, preview_max_width) =
                preview_widths(context.screen_rect().width(), explorer_width);
            egui::SidePanel::right("latex-preview")
                .default_width(preview_width)
                .width_range(MIN_PREVIEW_WIDTH..=preview_max_width)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.panel.0)
                        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.22),
                        )),
                )
                .show(context, |ui| {
                    let mut enable_system_preview = false;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("DOCUMENT PREVIEW")
                                .strong()
                                .size(11.0)
                                .color(self.themes[self.selected_theme].ui.text.0),
                        );
                        let state = if self.analysis_due.is_some() || self.analysis_result.is_some()
                        {
                            "UPDATING"
                        } else {
                            "LIVE"
                        };
                        ui.label(
                            RichText::new(state)
                                .size(10.5)
                                .color(self.themes[self.selected_theme].ui.accent.0),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        let response = ui
                            .toggle_value(&mut self.system_latex_preview, "Computer Modern")
                            .on_hover_text("Use cached, serialized system LaTeX rendering");
                        enable_system_preview = response.changed() && self.system_latex_preview;
                        if ui.small_button("Copy TeX").clicked() {
                            ui.output_mut(|output| output.copied_text.clone_from(&self.preview));
                        }
                    });
                    if enable_system_preview {
                        if let Some(document) = &self.preview_document {
                            let body = document.body();
                            let color = self.themes[self.selected_theme].ui.text.0;
                            self.start_latex_render(body, [color.r(), color.g(), color.b()]);
                        }
                    }
                    if let Some(error) = &self.preview_error {
                        ui.colored_label(
                            self.themes[self.selected_theme].syntax.operator.0,
                            format!("Editing: {error} (showing last good preview)"),
                        );
                    }
                    ui.separator();
                    if self
                        .preview_document
                        .as_ref()
                        .map_or(true, |document| document.fragments.is_empty())
                    {
                        ui.label("Add a render line, for example:");
                        ui.monospace(r"\[ (x + 1) / (2x^2) \]");
                    } else {
                        let preview_width = ui.available_width();
                        let preview_scale = (preview_width / 280.0).clamp(1.0, 2.8);
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if let Some(document) = &self.preview_document {
                                    if self.system_latex_preview {
                                        if let Some(texture) = &self.latex_texture {
                                            let natural_size = texture.size_vec2();
                                            let page_width = (preview_width - 52.0).max(180.0);
                                            let image_scale =
                                                (page_width / natural_size.x.max(1.0)).min(4.5);
                                            let display_size = natural_size * image_scale;
                                            ui.with_layout(
                                                egui::Layout::top_down(egui::Align::Min),
                                                |ui| {
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        RichText::new("Rendered document")
                                                            .strong()
                                                            .size(15.0),
                                                    );
                                                    ui.label(
                                                        RichText::new(
                                                            "Live output from the current SCAS source",
                                                        )
                                                        .size(11.0)
                                                        .color(theme.ui.muted.0),
                                                    );
                                                    ui.add_space(8.0);
                                                    egui::Frame::none()
                                                        .fill(theme.ui.editor.0)
                                                        .inner_margin(egui::Margin::same(18.0))
                                                        .show(ui, |ui| {
                                                            ui.add(
                                                                egui::Image::from_texture(texture)
                                                                    .fit_to_exact_size(
                                                                        display_size,
                                                                    ),
                                                            );
                                                        });
                                                },
                                            );
                                        } else {
                                            math_preview::show_document(
                                                ui,
                                                document,
                                                self.themes[self.selected_theme].ui.text.0,
                                                self.themes[self.selected_theme].ui.muted.0,
                                                preview_scale,
                                            );
                                        }
                                    } else {
                                        math_preview::show_document(
                                            ui,
                                            document,
                                            self.themes[self.selected_theme].ui.text.0,
                                            self.themes[self.selected_theme].ui.muted.0,
                                            preview_scale,
                                        );
                                    }
                                    if self.latex_render.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.small("Typesetting Computer Modern preview…");
                                        });
                                    }
                                    if let Some(error) = &self.latex_render_error {
                                        ui.colored_label(
                                            self.themes[self.selected_theme].syntax.operator.0,
                                            format!("{error}; showing structured fallback"),
                                        );
                                    }
                                    for diagnostic in &document.diagnostics {
                                        ui.colored_label(
                                            self.themes[self.selected_theme].syntax.operator.0,
                                            &diagnostic.message,
                                        );
                                    }
                                }
                                ui.add_space(12.0);
                                ui.collapsing("LaTeX source", |ui| {
                                    ui.add(
                                        TextEdit::multiline(&mut self.preview)
                                            .font(TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                            });
                    }
                });
        }

        let editor_background = self.themes[self.selected_theme].ui.editor.0;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(editor_background)
                    .inner_margin(egui::Margin::same(0.0)),
            )
            .show(context, |ui| {
                let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
                let mut chosen_snippet = None;
                let document_title = self.title();
                egui::Frame::none()
                    .fill(self.themes[self.selected_theme].ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        self.themes[self.selected_theme]
                            .ui
                            .muted
                            .0
                            .gamma_multiply(0.20),
                    ))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (tab, label) in [
                                (WorkspaceTab::Document, document_title.as_str()),
                                (WorkspaceTab::Repl, "REPL"),
                                (WorkspaceTab::Graphing, "Graphing"),
                            ] {
                                let response = ui.selectable_label(
                                    self.workspace_tab == tab,
                                    RichText::new(label).strong().size(12.0),
                                );
                                if self.workspace_tab == tab {
                                    ui.painter().line_segment(
                                        [response.rect.left_bottom(), response.rect.right_bottom()],
                                        egui::Stroke::new(2.0, theme.ui.accent.0),
                                    );
                                }
                                if response.clicked() {
                                    self.workspace_tab = tab;
                                    if tab == WorkspaceTab::Repl {
                                        context.memory_mut(|memory| {
                                            memory.request_focus(egui::Id::new("gui-repl-input"))
                                        });
                                    }
                                }
                            }
                            if self.workspace_tab == WorkspaceTab::Document && self.dirty {
                                ui.label(
                                    RichText::new("Modified").size(11.0).color(theme.ui.muted.0),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.workspace_tab == WorkspaceTab::Repl {
                                        if ui.small_button("Reset session").clicked() {
                                            self.reset_repl();
                                        }
                                        if ui.small_button("Clear history").clicked() {
                                            self.repl_entries.clear();
                                            self.set_status("REPL history cleared");
                                        }
                                        return;
                                    }
                                    if self.workspace_tab != WorkspaceTab::Document {
                                        return;
                                    }
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close editor")
                                        .clicked()
                                    {
                                        if self.dirty {
                                            self.set_status(
                                                "Save the modified document before closing",
                                            );
                                        } else {
                                            self.new_file();
                                        }
                                    }
                                    ui.menu_button("{}", |ui| {
                                            ui.set_min_width(250.0);
                                            for snippet in editor_assist::SNIPPETS {
                                                if ui
                                                    .button(snippet.label)
                                                    .on_hover_text(snippet.detail)
                                                    .clicked()
                                                {
                                                    chosen_snippet = Some(*snippet);
                                                    ui.close_menu();
                                                }
                                            }
                                        })
                                        .response
                                        .on_hover_text("Insert a code snippet");
                                    ui.menu_button("Ω", |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                                for (symbol, insertion, hint) in [
                                                    ("π", "π", "\\pi"),
                                                    ("φ", "φ", "\\phi"),
                                                    ("θ", "θ", "\\theta"),
                                                    ("λ", "λ", "\\lambda"),
                                                    ("Σ", "Σ", "\\Sigma"),
                                                    ("Γ", "Γ", "\\Gamma"),
                                                    ("∫", "integral(", "Integral function"),
                                                    ("∇", "gradient(", "Gradient function"),
                                                ] {
                                                    if ui
                                                        .button(RichText::new(symbol).size(17.0))
                                                        .on_hover_text(hint)
                                                        .clicked()
                                                    {
                                                        self.source.push_str(insertion);
                                                        self.dirty = true;
                                                        self.schedule_analysis(context);
                                                        ui.close_menu();
                                                    }
                                                }
                                            });
                                        })
                                        .response
                                        .on_hover_text("Insert a mathematical symbol");
                                    let greek_hint = if self.auto_greek {
                                        "Greek shortcut expansion is on"
                                    } else {
                                        "Greek shortcut expansion is off"
                                    };
                                    ui.toggle_value(&mut self.auto_greek, "α")
                                        .on_hover_text(greek_hint);
                                },
                            );
                        });
                    });
                match self.workspace_tab {
                    WorkspaceTab::Repl => {
                        self.show_repl(ui, context, &theme);
                        return;
                    }
                    WorkspaceTab::Graphing => {
                        self.show_graphing(ui, context, &theme);
                        return;
                    }
                    WorkspaceTab::Document => {}
                }
                if let Some(snippet) = chosen_snippet {
                    let insertion = snippet.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        self.cursor_char_index,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
                ui.separator();
                let editor_focused = context.memory(|memory| memory.has_focus(editor_id));
                // Completion scans are only useful while the editor has
                // focus; skip them while the REPL or graphing tab is active.
                let matches = if editor_focused {
                    editor_assist::completions(&self.source, self.cursor_char_index)
                } else {
                    editor_assist::Matches::empty(self.cursor_char_index)
                };
                let tab_accept = editor_focused
                    && matches.len > 0
                    && context.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    });
                if let Some(completion) = tab_accept.then(|| matches.first()).flatten() {
                    let insertion = completion.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        matches.start_char,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
                let font_id = FontId::new(
                    self.editor_font_size,
                    if self.editor_monospace {
                        FontFamily::Monospace
                    } else {
                        FontFamily::Proportional
                    },
                );
                let cache_is_current = self.highlight_cache.as_ref().is_some_and(|cache| {
                    cache.source == self.source
                        && cache.theme == self.selected_theme
                        && cache.font_size_bits == self.editor_font_size.to_bits()
                        && cache.monospace == self.editor_monospace
                        && cache.diagnostics_revision == self.diagnostics_revision
                });
                if !cache_is_current {
                    self.highlight_cache = Some(HighlightCache {
                        source: self.source.clone(),
                        theme: self.selected_theme,
                        font_size_bits: self.editor_font_size.to_bits(),
                        monospace: self.editor_monospace,
                        diagnostics_revision: self.diagnostics_revision,
                        job: highlight(
                            &self.source,
                            &self.themes[self.selected_theme],
                            font_id.clone(),
                            &self.diagnostics,
                        ),
                    });
                }
                let cache = self
                    .highlight_cache
                    .as_ref()
                    .expect("highlight cache was populated");
                let cached_source = cache.source.as_str();
                let cached_job = &cache.job;
                let fallback_theme = &self.themes[self.selected_theme];
                let fallback_font = &font_id;
                let fallback_diagnostics = &self.diagnostics;
                let mut layouter = |ui: &egui::Ui, source: &str, wrap_width: f32| {
                    let mut job = if source == cached_source {
                        cached_job.clone()
                    } else {
                        highlight(
                            source,
                            fallback_theme,
                            fallback_font.clone(),
                            fallback_diagnostics,
                        )
                    };
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                let total_lines = self.source.split('\n').count().max(1);
                let digits = total_lines.to_string().len() as f32;
                let gutter_width = if self.line_numbers == LineNumberMode::Off {
                    4.0
                } else {
                    20.0 + digits * self.editor_font_size * 0.62
                };
                if TextEdit::load_state(context, editor_id).is_none() {
                    let mut state = egui::text_edit::TextEditState::default();
                    state.set_undoer(editor_undoer(&self.source));
                    TextEdit::store_state(context, editor_id, state);
                }
                let output = egui::ScrollArea::vertical()
                    .id_source("source-editor-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        TextEdit::multiline(&mut self.source)
                            .id(editor_id)
                            .font(font_id.clone())
                            .code_editor()
                            .frame(false)
                            .desired_width(ui.available_width())
                            .lock_focus(true)
                            .margin(egui::Margin {
                                left: gutter_width,
                                right: 4.0,
                                top: 4.0,
                                bottom: 4.0,
                            })
                            .layouter(&mut layouter)
                            .desired_rows(total_lines.max(1))
                            .show(ui)
                    })
                    .inner;
                if let Some(cursor) = output.cursor_range {
                    self.cursor_char_index = cursor.primary.ccursor.index;
                    self.current_line = self
                        .source
                        .chars()
                        .take(self.cursor_char_index)
                        .filter(|character| *character == '\n')
                        .count()
                        .min(total_lines.saturating_sub(1));
                }
                let popup_matches = if output.response.has_focus() {
                    editor_assist::completions(&self.source, self.cursor_char_index)
                } else {
                    matches
                };
                let mut popup_completion = None;
                if output.response.has_focus() && popup_matches.len > 0 {
                    if let Some(cursor) = output.cursor_range {
                        let caret = output.galley.pos_from_cursor(&cursor.primary);
                        let anchor =
                            output.galley_pos + egui::vec2(caret.left(), caret.bottom() + 4.0);
                        egui::Area::new(egui::Id::new("editor-completion-popup"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(anchor)
                            .movable(false)
                            .constrain(true)
                            .show(context, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(290.0);
                                    for (index, assist) in popup_matches.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            let label = if index == 0 {
                                                format!("{}   Tab", assist.label)
                                            } else {
                                                assist.label.to_owned()
                                            };
                                            if ui.selectable_label(false, label).clicked() {
                                                popup_completion = Some(assist);
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(assist.detail)
                                                            .size(11.0)
                                                            .color(
                                                                self.themes[self.selected_theme]
                                                                    .ui
                                                                    .muted
                                                                    .0,
                                                            ),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                });
                            });
                    }
                }
                if self.line_numbers != LineNumberMode::Off {
                    paint_line_numbers(
                        ui,
                        &output,
                        self.current_line,
                        self.line_numbers,
                        &self.themes[self.selected_theme],
                        self.editor_font_size,
                        gutter_width,
                    );
                }
                if let Some(pointer) = output.response.hover_pos() {
                    let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
                    let byte = char_to_byte(&self.source, cursor.ccursor.index);
                    if let Some(diagnostic) = self
                        .diagnostics
                        .iter()
                        .find(|item| item.byte_start <= byte && byte < item.byte_end)
                    {
                        output.response.clone().on_hover_text(&diagnostic.message);
                    }
                }
                if output.response.changed() {
                    self.highlight_cache = None;
                    if self.auto_greek {
                        let before_expansion = self.source.clone();
                        expand_greek_shortcuts(&mut self.source);
                        if self.source != before_expansion {
                            if let Some(cursor) = output.cursor_range {
                                let mut state = output.state.clone();
                                let mut undoer = state.undoer();
                                undoer.add_undo(&(cursor.as_ccursor_range(), before_expansion));
                                state.set_undoer(undoer);
                                TextEdit::store_state(context, editor_id, state);
                            }
                        }
                    }
                    self.dirty = true;
                    self.schedule_analysis(context);
                }
                if let Some(completion) = popup_completion {
                    let insertion = completion.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        popup_matches.start_char,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
            });
    }
}

#[allow(dead_code)]
fn readable_latex(latex: &str) -> String {
    fn group(text: &str, start: usize) -> Option<(&str, usize)> {
        let mut depth = 0usize;
        for (offset, character) in text[start..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some((&text[start + 1..start + offset], start + offset + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn superscript(text: &str) -> String {
        text.chars()
            .map(|character| match character {
                '0' => '⁰',
                '1' => '¹',
                '2' => '²',
                '3' => '³',
                '4' => '⁴',
                '5' => '⁵',
                '6' => '⁶',
                '7' => '⁷',
                '8' => '⁸',
                '9' => '⁹',
                '+' => '⁺',
                '-' => '⁻',
                'n' => 'ⁿ',
                _ => character,
            })
            .collect()
    }

    let commands = [
        (r"\upalpha", "α"),
        (r"\upbeta", "β"),
        (r"\upgamma", "γ"),
        (r"\updelta", "δ"),
        (r"\upepsilon", "ε"),
        (r"\uptheta", "θ"),
        (r"\uplambda", "λ"),
        (r"\upmu", "μ"),
        (r"\uppi", "π"),
        (r"\uprho", "ρ"),
        (r"\upsigma", "σ"),
        (r"\upphi", "φ"),
        (r"\upomega", "ω"),
        (r"\alpha", "α"),
        (r"\beta", "β"),
        (r"\gamma", "γ"),
        (r"\delta", "δ"),
        (r"\theta", "θ"),
        (r"\lambda", "λ"),
        (r"\mu", "μ"),
        (r"\pi", "π"),
        (r"\rho", "ρ"),
        (r"\sigma", "σ"),
        (r"\phi", "φ"),
        (r"\omega", "ω"),
        (r"\Gamma", "Γ"),
        (r"\Delta", "Δ"),
        (r"\Theta", "Θ"),
        (r"\Lambda", "Λ"),
        (r"\Sigma", "Σ"),
        (r"\Phi", "Φ"),
        (r"\Omega", "Ω"),
        (r"\cdot", "·"),
        (r"\times", "×"),
        (r"\le", "≤"),
        (r"\ge", "≥"),
        (r"\ne", "≠"),
        (r"\land", "∧"),
        (r"\lor", "∨"),
        (r"\infty", "∞"),
        (r"\sin", "sin"),
        (r"\cos", "cos"),
        (r"\tan", "tan"),
        (r"\ln", "ln"),
        (r"\log", "log"),
        (r"\exp", "exp"),
    ];
    let mut output = String::new();
    let mut cursor = 0usize;
    while cursor < latex.len() {
        let rest = &latex[cursor..];
        if rest.starts_with(r"\mathrm{") {
            let start = cursor + r"\mathrm".len();
            if let Some((identifier, after)) = group(latex, start) {
                output.push_str(&readable_latex(identifier));
                cursor = after;
                continue;
            }
        }
        if rest.starts_with(r"\frac{") {
            let numerator_start = cursor + r"\frac".len();
            if let Some((numerator, after_numerator)) = group(latex, numerator_start) {
                if latex[after_numerator..].starts_with('{') {
                    if let Some((denominator, after_denominator)) = group(latex, after_numerator) {
                        output.push('(');
                        output.push_str(&readable_latex(numerator));
                        output.push_str(")⁄(");
                        output.push_str(&readable_latex(denominator));
                        output.push(')');
                        cursor = after_denominator;
                        continue;
                    }
                }
            }
        }
        if rest.starts_with(r"\sqrt{") {
            let start = cursor + r"\sqrt".len();
            if let Some((radicand, after)) = group(latex, start) {
                output.push_str("√(");
                output.push_str(&readable_latex(radicand));
                output.push(')');
                cursor = after;
                continue;
            }
        }
        if rest.starts_with("^{") {
            if let Some((exponent, after)) = group(latex, cursor + 1) {
                output.push_str(&superscript(&readable_latex(exponent)));
                cursor = after;
                continue;
            }
        }
        if let Some((command, replacement)) = commands.iter().find(|(command, _)| {
            rest.starts_with(command)
                && !rest[command.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphabetic)
        }) {
            output.push_str(replacement);
            cursor += command.len();
            continue;
        }
        if rest.starts_with(r"\left") || rest.starts_with(r"\right") {
            cursor += if rest.starts_with(r"\left") { 5 } else { 6 };
            continue;
        }
        if rest.starts_with(r"\[") || rest.starts_with(r"\]") {
            cursor += 2;
            continue;
        }
        if rest.starts_with(r"\,") {
            output.push(' ');
            cursor += 2;
            continue;
        }
        let character = rest.chars().next().expect("cursor is inside text");
        if character != '{' && character != '}' {
            output.push(character);
        }
        cursor += character.len_utf8();
    }
    output.trim().to_owned()
}

fn activity_button(
    ui: &mut egui::Ui,
    icon: ActivityIcon,
    selected: bool,
    theme: &Theme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 38.0), egui::Sense::click());
    let fill = if selected {
        theme.ui.selection.0
    } else if response.hovered() {
        theme.ui.panel.0
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    let color = if selected || response.hovered() {
        theme.ui.text.0
    } else {
        theme.ui.muted.0
    };
    let stroke = egui::Stroke::new(1.6, color);
    let center = rect.center();
    let icon_rect = egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0));
    match icon {
        ActivityIcon::Explorer => {
            let back = icon_rect.translate(egui::vec2(-2.5, -2.5)).shrink(2.0);
            let front = icon_rect.translate(egui::vec2(2.5, 2.5)).shrink(2.0);
            ui.painter().rect_stroke(back, 1.0, stroke);
            ui.painter().rect_stroke(front, 1.0, stroke);
        }
        ActivityIcon::Preview => {
            let page = icon_rect.shrink(2.5);
            ui.painter().rect_stroke(page, 1.0, stroke);
            for y in [page.top() + 5.0, page.top() + 9.0, page.top() + 13.0] {
                ui.painter().line_segment(
                    [
                        egui::pos2(page.left() + 3.0, y),
                        egui::pos2(page.right() - 3.0, y),
                    ],
                    egui::Stroke::new(1.1, color),
                );
            }
        }
        ActivityIcon::Problems => {
            let top = egui::pos2(center.x, icon_rect.top() + 1.5);
            let left = egui::pos2(icon_rect.left() + 1.0, icon_rect.bottom() - 2.0);
            let right = egui::pos2(icon_rect.right() - 1.0, icon_rect.bottom() - 2.0);
            ui.painter().line_segment([top, left], stroke);
            ui.painter().line_segment([left, right], stroke);
            ui.painter().line_segment([right, top], stroke);
            ui.painter().line_segment(
                [center - egui::vec2(0.0, 3.5), center + egui::vec2(0.0, 2.0)],
                stroke,
            );
            ui.painter()
                .circle_filled(center + egui::vec2(0.0, 6.0), 1.1, color);
        }
        ActivityIcon::Settings => {
            ui.painter().circle_stroke(center, 6.0, stroke);
            ui.painter().circle_stroke(center, 2.2, stroke);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                ui.painter()
                    .line_segment([center + direction * 7.0, center + direction * 9.5], stroke);
            }
        }
    }
    response
}

fn tree_disclosure(ui: &mut egui::Ui, collapsed: bool, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::click());
    let center = rect.center();
    let points = if collapsed {
        vec![
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(3.0, 0.0),
            center + egui::vec2(-2.0, 4.0),
        ]
    } else {
        vec![
            center + egui::vec2(-4.0, -2.0),
            center + egui::vec2(4.0, -2.0),
            center + egui::vec2(0.0, 3.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        if response.hovered() {
            theme.ui.text.0
        } else {
            theme.ui.muted.0
        },
        egui::Stroke::NONE,
    ));
    response
}

fn paint_tree_icon(ui: &mut egui::Ui, directory: bool, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
    let color = theme.ui.muted.0;
    if directory {
        let body = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(1.0, 6.0),
            rect.right_bottom() - egui::vec2(1.0, 2.0),
        );
        ui.painter()
            .rect_stroke(body, 1.0, egui::Stroke::new(1.2, color));
        ui.painter().line_segment(
            [
                egui::pos2(body.left() + 1.0, body.top()),
                egui::pos2(body.left() + 6.0, body.top() - 3.0),
            ],
            egui::Stroke::new(1.2, color),
        );
    } else {
        let page = rect.shrink2(egui::vec2(3.0, 2.5));
        ui.painter()
            .rect_stroke(page, 1.0, egui::Stroke::new(1.1, color));
        ui.painter().line_segment(
            [
                egui::pos2(page.left() + 2.0, page.top() + 5.0),
                egui::pos2(page.right() - 2.0, page.top() + 5.0),
            ],
            egui::Stroke::new(1.0, color),
        );
    }
}

fn paint_line_numbers(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    current_line: usize,
    mode: LineNumberMode,
    theme: &Theme,
    font_size: f32,
    gutter_width: f32,
) {
    // TextEdit clips its galley to the editable text area. The gutter occupies
    // the left margin, so explicitly extend the paint clip across that margin.
    let gutter_left = output.galley_pos.x - gutter_width;
    let editor_clip = egui::Rect::from_min_max(
        egui::pos2(gutter_left, output.response.rect.top()),
        output.response.rect.right_bottom(),
    )
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(editor_clip);
    let separator_x = output.galley_pos.x - 10.0;
    let gutter_rect = egui::Rect::from_min_max(
        egui::pos2(gutter_left, output.response.rect.top()),
        egui::pos2(separator_x, output.response.rect.bottom()),
    );
    painter.rect_filled(gutter_rect, 0.0, theme.ui.editor.0);
    painter.line_segment(
        [
            egui::pos2(separator_x, output.text_clip_rect.top()),
            egui::pos2(separator_x, output.text_clip_rect.bottom()),
        ],
        egui::Stroke::new(1.0, theme.ui.muted.0.gamma_multiply(0.35)),
    );
    let mut source_line = 0usize;
    let mut first_visual_row = true;
    for galley_row in &output.galley.rows {
        let active = source_line == current_line;
        if active {
            let row_rect = egui::Rect::from_min_max(
                egui::pos2(
                    separator_x + 1.0,
                    output.galley_pos.y + galley_row.rect.top(),
                ),
                egui::pos2(
                    output.text_clip_rect.right(),
                    output.galley_pos.y + galley_row.rect.bottom(),
                ),
            );
            painter.rect_filled(row_rect, 0.0, theme.ui.accent.0.gamma_multiply(0.075));
        }
        let value = first_visual_row
            .then(|| displayed_line_number(mode, source_line, current_line))
            .flatten();
        if let Some(value) = value {
            let position = egui::pos2(
                separator_x - 8.0,
                output.galley_pos.y + galley_row.rect.center().y,
            );
            painter.text(
                position,
                egui::Align2::RIGHT_CENTER,
                value.to_string(),
                FontId::monospace(font_size * 0.82),
                if active {
                    theme.ui.accent.0
                } else {
                    theme.ui.text.0.gamma_multiply(0.72)
                },
            );
        }
        first_visual_row = galley_row.ends_with_newline;
        if galley_row.ends_with_newline {
            source_line += 1;
        }
    }
}

fn displayed_line_number(mode: LineNumberMode, row: usize, current_line: usize) -> Option<usize> {
    match mode {
        LineNumberMode::Off => None,
        LineNumberMode::Absolute => Some(row + 1),
        LineNumberMode::Relative => Some(row.abs_diff(current_line)),
        LineNumberMode::Hybrid if row == current_line => Some(row + 1),
        LineNumberMode::Hybrid => Some(row.abs_diff(current_line)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Highlight {
    Plain,
    Keyword,
    Number,
    String,
    Comment,
    DocComment,
    Identifier,
    Operator,
    Delimiter,
    RenderMarker,
}

#[derive(Clone, Copy, Debug)]
struct HighlightSpan {
    start: usize,
    end: usize,
    kind: Highlight,
}

fn highlight(
    source: &str,
    theme: &Theme,
    font_id: FontId,
    diagnostics: &[EditorDiagnostic],
) -> LayoutJob {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut spans = tokens
        .iter()
        .filter_map(|token| {
            token_highlight(token.token_type).map(|kind| HighlightSpan {
                start: token.byte_start,
                end: token.byte_end,
                kind,
            })
        })
        .collect::<Vec<_>>();

    // Ordinary comments are trivia by design, so the lexer does not emit them.
    // Recover only those gaps here; token spans prevent `//` inside strings from
    // being mistaken for a comment.
    for (line_start, line) in source.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    }) {
        let bytes = line.as_bytes();
        let mut index = 0;
        while index + 1 < bytes.len() {
            if bytes[index] == b'/' && bytes[index + 1] == b'/' {
                let candidate = line_start + index;
                let covered = spans
                    .iter()
                    .any(|span| span.start <= candidate && candidate < span.end);
                if !covered {
                    let end = line_start + line.trim_end_matches('\n').len();
                    spans.push(HighlightSpan {
                        start: candidate,
                        end,
                        kind: Highlight::Comment,
                    });
                    break;
                }
            }
            index += 1;
        }
    }
    spans.sort_by_key(|span| (span.start, span.end));

    let mut job = LayoutJob::default();
    let mut cursor = 0;
    for span in spans {
        if span.start < cursor || span.end > source.len() || span.start >= span.end {
            continue;
        }
        append_highlight(
            &mut job,
            &source[cursor..span.start],
            Highlight::Plain,
            theme,
            &font_id,
        );
        append_highlight(
            &mut job,
            &source[span.start..span.end],
            span.kind,
            theme,
            &font_id,
        );
        cursor = span.end;
    }
    append_highlight(
        &mut job,
        &source[cursor..],
        Highlight::Plain,
        theme,
        &font_id,
    );
    let diagnostic_color = theme.syntax.operator.0;
    let original_sections = std::mem::take(&mut job.sections);
    for section in original_sections {
        let mut boundaries = vec![section.byte_range.start, section.byte_range.end];
        for diagnostic in diagnostics {
            if section.byte_range.start < diagnostic.byte_start
                && diagnostic.byte_start < section.byte_range.end
            {
                boundaries.push(diagnostic.byte_start);
            }
            if section.byte_range.start < diagnostic.byte_end
                && diagnostic.byte_end < section.byte_range.end
            {
                boundaries.push(diagnostic.byte_end);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        for range in boundaries.windows(2) {
            let mut part = section.clone();
            part.byte_range = range[0]..range[1];
            if diagnostics.iter().any(|diagnostic| {
                part.byte_range.start < diagnostic.byte_end
                    && diagnostic.byte_start < part.byte_range.end
            }) {
                part.format.underline = egui::Stroke::new(1.5, diagnostic_color);
            }
            job.sections.push(part);
        }
    }
    job
}

fn source_diagnostics(source: &str) -> Vec<EditorDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    for error in scanner.errors() {
        let start = byte_offset(source, error.line, error.column);
        let end = byte_end_for_chars(source, start, error.length.max(1));
        diagnostics.push(EditorDiagnostic {
            byte_start: start,
            byte_end: end,
            line: error.line,
            column: error.column,
            message: error.message.clone(),
        });
    }
    if diagnostics.is_empty() {
        for error in Parser::new(tokens).parse_recovering().errors {
            let start = byte_offset(source, error.line, error.column);
            let end = byte_end_for_chars(source, start, error.length.max(1));
            diagnostics.push(EditorDiagnostic {
                byte_start: start,
                byte_end: end,
                line: error.line,
                column: error.column,
                message: error.message,
            });
        }
    }
    diagnostics
}

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let line_start = source
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    line_start
        + source[line_start..]
            .chars()
            .take(column.saturating_sub(1))
            .map(char::len_utf8)
            .sum::<usize>()
}

fn byte_end_for_chars(source: &str, start: usize, count: usize) -> usize {
    let width = source[start.min(source.len())..]
        .chars()
        .take(count)
        .map(char::len_utf8)
        .sum::<usize>();
    (start + width.max(1)).min(source.len())
}

fn char_to_byte(source: &str, char_index: usize) -> usize {
    source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(index, _)| index)
}

fn append_highlight(
    job: &mut LayoutJob,
    text: &str,
    kind: Highlight,
    theme: &Theme,
    font_id: &FontId,
) {
    if text.is_empty() {
        return;
    }
    let syntax = &theme.syntax;
    let color = match kind {
        Highlight::Plain => syntax.plain.0,
        Highlight::Keyword => syntax.keyword.0,
        Highlight::Number => syntax.number.0,
        Highlight::String => syntax.string.0,
        Highlight::Comment => syntax.comment.0,
        Highlight::DocComment => syntax.doc_comment.0,
        Highlight::Identifier => syntax.identifier.0,
        Highlight::Operator => syntax.operator.0,
        Highlight::Delimiter => syntax.delimiter.0,
        Highlight::RenderMarker => syntax.render_marker.0,
    };
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font_id.clone(),
            color,
            ..Default::default()
        },
    );
}

fn token_highlight(token: TokenType) -> Option<Highlight> {
    use TokenType::*;
    Some(match token {
        Eof => return None,
        Let | Fn | If | Else | For | While | Break | Continue | Return | Print | And | Or | Xor
        | True | False | Nil | Exit | Help | Clear => Highlight::Keyword,
        Number => Highlight::Number,
        Str => Highlight::String,
        DocComment => Highlight::DocComment,
        Identifier => Highlight::Identifier,
        RenderLeftBrace | RenderRightBrace | RenderLeftBracket | RenderRightBracket => {
            Highlight::RenderMarker
        }
        LeftParen | RightParen | LeftBracket | RightBracket | LeftBrace | RightBrace | Comma
        | Dot | Colon | Semicolon => Highlight::Delimiter,
        FlowOperator | FatArrow | Plus | Minus | Star | Slash | Mod | Pow | Ampersand | Pipe
        | Tilde | ShiftLeft | ShiftRight | ShiftLeftEqual | ShiftRightEqual | Equal | PlusEqual
        | MinusEqual | StarEqual | SlashEqual | ModEqual | PowEqual | AmpersandEqual
        | PipeEqual | TildeEqual | EqualEqual | BangEqual | Less | LessEqual | Greater
        | Apostrophe | GreaterEqual | Bang => Highlight::Operator,
    })
}

fn collect_tree(root: &Path) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    visit_directory(root, 0, &mut entries);
    entries
}

fn visit_directory(directory: &Path, depth: usize, output: &mut Vec<TreeEntry>) {
    if depth > 12 {
        return;
    }
    let Ok(read_dir) = fs::read_dir(directory) else {
        return;
    };
    let mut paths = read_dir
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.retain(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        !name.starts_with('.') && name != "target" && name != "fuzz"
    });
    paths.sort_by_key(|path| (!path.is_dir(), path.file_name().map(|name| name.to_owned())));
    for path in paths {
        if path.is_dir() {
            output.push(TreeEntry {
                path: path.clone(),
                depth,
                is_dir: true,
            });
            visit_directory(&path, depth + 1, output);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "scas")
        {
            output.push(TreeEntry {
                path,
                depth,
                is_dir: false,
            });
        }
    }
}

fn expand_greek_shortcuts(source: &mut String) {
    if !source.contains('\\') {
        return;
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut changed = false;
    while let Some(offset) = source[cursor..].find('\\') {
        let slash = cursor + offset;
        output.push_str(&source[cursor..slash]);
        let rest = &source[slash..];
        let replacement = GREEK
            .iter()
            .filter(|(shortcut, _)| rest.starts_with(shortcut))
            .filter(|(shortcut, _)| {
                !rest[shortcut.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphabetic)
            })
            .max_by_key(|(shortcut, _)| shortcut.len());
        if let Some((shortcut, symbol)) = replacement {
            output.push_str(symbol);
            cursor = slash + shortcut.len();
            changed = true;
        } else {
            output.push('\\');
            cursor = slash + 1;
        }
    }
    if changed {
        output.push_str(&source[cursor..]);
        *source = output;
    }
}

fn editor_undoer(source: &str) -> egui::util::undoer::Undoer<(egui::text::CCursorRange, String)> {
    let mut undoer = egui::util::undoer::Undoer::with_settings(egui::util::undoer::Settings {
        max_undos: 200,
        // Record each completed input frame. A long typing burst or paste
        // must never leave the empty buffer as the only undo destination.
        stable_time: 0.0,
        auto_save_interval: 2.0,
    });
    undoer.add_undo(&(egui::text::CCursorRange::default(), source.to_owned()));
    undoer
}

fn live_preview_limits() -> Limits {
    Limits {
        max_steps: 120_000,
        max_call_depth: 96,
        max_exponent: 10_000,
        max_shift: 100_000,
        max_decimal_precision: 512,
        max_transcendental_precision: 256,
        max_number_bits: 1_000_000,
        ..Limits::default()
    }
}

fn render_repl_math_png(latex: &str, color: [u8; 3]) -> Result<LatexRender, String> {
    render_latex_png(&format!("${latex}$"), color)
}

fn render_latex_png(body: &str, color: [u8; 3]) -> Result<LatexRender, String> {
    let directory = std::env::temp_dir().join(format!(
        "scas-latex-{}-{}",
        std::process::id(),
        NEXT_LATEX_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create LaTeX preview directory: {error}"))?;
    let source = format!(
        "\\documentclass[preview,border=10pt]{{standalone}}\n\
         \\usepackage{{amsmath}}\n\
         \\usepackage{{amssymb}}\n\
         \\usepackage{{upgreek}}\n\
         \\usepackage{{booktabs}}\n\
         \\usepackage{{xcolor}}\n\
         \\begin{{document}}\n\\color[rgb]{{{:.4},{:.4},{:.4}}}\n{body}\n\\end{{document}}\n",
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0
    );
    let result = (|| {
        fs::write(directory.join("preview.tex"), source)
            .map_err(|error| format!("could not write LaTeX preview: {error}"))?;
        let latex = Command::new("latex")
            .args(["-interaction=nonstopmode", "-halt-on-error", "preview.tex"])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start LaTeX: {error}"))?;
        if !latex.status.success() {
            return Err("LaTeX could not typeset the current document".to_owned());
        }
        let dvipng = Command::new("dvipng")
            .args([
                "-T",
                "tight",
                "-D",
                "180",
                "-bg",
                "Transparent",
                "-o",
                "preview.png",
                "preview.dvi",
            ])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start dvipng: {error}"))?;
        if !dvipng.status.success() {
            return Err("dvipng could not rasterize the LaTeX preview".to_owned());
        }
        let bytes = fs::read(directory.join("preview.png"))
            .map_err(|error| format!("could not read LaTeX preview: {error}"))?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| format!("could not decode LaTeX preview: {error}"))?
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        Ok(LatexRender {
            rgba: image.into_raw(),
            size,
        })
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SCAS",
        options,
        Box::new(|creation| Ok(Box::new(ScasApp::new(&creation.egui_ctx)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color_at(job: &LayoutJob, needle: &str) -> egui::Color32 {
        let offset = job.text.find(needle).unwrap();
        job.sections
            .iter()
            .find(|section| section.byte_range.contains(&offset))
            .unwrap()
            .format
            .color
    }

    #[test]
    fn expands_supported_greek_shortcuts_without_touching_render_markers() {
        let mut source =
            "let angle = \\theta; print(\\pi); let variants = [\\varphi, \\Xi]; \\[ angle \\]"
                .to_owned();
        expand_greek_shortcuts(&mut source);
        assert_eq!(
            source,
            "let angle = θ; print(π); let variants = [ϕ, Ξ]; \\[ angle \\]"
        );
    }

    #[test]
    fn editor_undo_is_seeded_with_the_loaded_document() {
        let initial = "first line\nsecond line";
        let mut undoer = editor_undoer(initial);
        let edited = (egui::text::CCursorRange::default(), format!("{initial}!"));
        undoer.add_undo(&edited);
        let restored = undoer.undo(&edited).expect("one edit should be undoable");
        assert_eq!(restored.1, initial);
    }

    #[test]
    fn preview_resize_preserves_the_editor_workspace() {
        let (default_width, max_width) = preview_widths(2048.0, 480.0);
        assert_eq!(max_width, MAX_PREVIEW_WIDTH);
        assert!(default_width <= max_width);
        assert!(2048.0 - 48.0 - 480.0 - max_width >= MIN_EDITOR_WORKSPACE_WIDTH);

        let (_, max_without_explorer) = preview_widths(2048.0, 0.0);
        assert_eq!(max_without_explorer, MAX_PREVIEW_WIDTH);

        let (narrow_default, narrow_max) = preview_widths(1024.0, 240.0);
        assert_eq!(narrow_default, narrow_max);
        assert_eq!(narrow_max, 316.0);
        assert!(1024.0 - 48.0 - 240.0 - narrow_max >= MIN_EDITOR_WORKSPACE_WIDTH);
    }

    #[test]
    fn highlights_lexer_tokens_and_distinguishes_render_markers() {
        let theme = Theme::bundled_themes().remove(0);
        let source = "let x = 42; \\[ x =>; \\]";
        let job = highlight(source, &theme, FontId::monospace(15.0), &[]);
        assert_eq!(color_at(&job, "let"), theme.syntax.keyword.0);
        assert_eq!(color_at(&job, "42"), theme.syntax.number.0);
        assert_eq!(color_at(&job, "\\["), theme.syntax.render_marker.0);
        assert_ne!(color_at(&job, "\\["), theme.syntax.delimiter.0);
    }

    #[test]
    fn highlights_doc_and_ordinary_comments_without_coloring_slashes_in_strings() {
        let theme = Theme::bundled_themes().remove(0);
        let source = "/// Math prose\nlet url = \"https://example.test\"; // note\n";
        let job = highlight(source, &theme, FontId::monospace(15.0), &[]);
        assert_eq!(color_at(&job, "///"), theme.syntax.doc_comment.0);
        assert_eq!(color_at(&job, "// note"), theme.syntax.comment.0);
        assert_eq!(color_at(&job, "https"), theme.syntax.string.0);
    }

    #[test]
    fn live_preview_assembles_prose_and_rendered_expressions() {
        let preview =
            scas::document::build("/// Quadratic formula\n\\[ (x + 1) / (2x^2) \\]\nprint(x);\n")
                .unwrap()
                .body();
        assert_eq!(
            preview,
            "Quadratic formula\n\n\\[\\frac{\\left(x + 1\\right)}{\\left(2x^{2}\\right)}\\]"
        );
        assert_eq!(
            readable_latex(&preview),
            "Quadratic formula\n\n((x + 1))⁄((2x²))"
        );
    }

    #[test]
    fn graph_functions_sample_from_the_repl_session_and_track_staleness() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        app.add_graph_function("sin(x)");
        assert!(app.graph_functions[0].stale);
        let series = sample_graph_batch(
            &[],
            &["sin(x)".to_owned()],
            app.repl_symbols.clone(),
            app.graph_view.x_min - 1.0,
            app.graph_view.x_max + 1.0,
            401,
        )
        .remove(0)
        .unwrap();
        app.graph_functions[0].sampled = Ok(series);
        app.graph_functions[0].stale = false;
        assert!(!app.graph_functions[0].stale);

        // Any successful submission marks graphs stale so they re-sample.
        app.repl_input = "let k = 3;".to_string();
        app.submit_repl();
        assert!(app.graph_functions[0].stale);

        app.refresh_graph_functions();
        assert!(app.graph_functions[0].stale);
        let samples = sample_graph_batch(
            &app.repl_session,
            &[app.graph_functions[0].source.clone()],
            app.repl_symbols.clone(),
            app.graph_view.x_min - 1.0,
            app.graph_view.x_max + 1.0,
            401,
        );
        app.graph_functions[0].sampled = samples.into_iter().next().unwrap();
        app.graph_functions[0].critical_points = app.graph_functions[0]
            .sampled
            .as_ref()
            .ok()
            .map(|series| plot::detect_critical_points(&series.samples))
            .unwrap_or_default();
        app.graph_functions[0].stale = false;
        let series = app.graph_functions[0].sampled.as_ref().unwrap();
        assert_eq!(series.variable, "x");

        // The sampled span tracks the view and critical points are filled.
        let view = app.graph_view;
        assert!(
            app.graph_functions[0].sampled_max - app.graph_functions[0].sampled_min
                >= view.x_span()
        );
        assert!(!app.graph_functions[0].critical_points.is_empty());
    }

    #[test]
    fn graph_view_growth_resamples_to_cover_the_visible_window() {
        let mut app = ScasApp::new(&egui::Context::default());
        app.add_graph_function("sin(x)");
        app.graph_functions[0].sampled_min = app.graph_view.x_min - app.graph_view.x_span() * 0.3;
        app.graph_functions[0].sampled_max = app.graph_view.x_max + app.graph_view.x_span() * 0.3;
        app.graph_functions[0].sampled_count = 1_000;
        let sampled_min = app.graph_functions[0].sampled_min;
        let sampled_max = app.graph_functions[0].sampled_max;
        let series = sample_graph_batch(
            &[],
            &["sin(x)".to_owned()],
            app.repl_symbols.clone(),
            sampled_min,
            sampled_max,
            1_000,
        )
        .remove(0)
        .unwrap();
        app.graph_functions[0].sampled = Ok(series);
        app.graph_functions[0].critical_points =
            plot::detect_critical_points(&app.graph_functions[0].sampled.as_ref().unwrap().samples);
        app.graph_functions[0].stale = false;

        app.graph_view.zoom(0.1); // Narrow views need denser samples.
        app.ensure_graph_coverage();
        assert!(app.graph_sample_due.is_some());

        app.graph_view = ViewState::centered(200.0, 120.0); // 10x zoom-out.
        app.ensure_graph_coverage();
        assert!(app.graph_sample_due.is_some());
    }

    #[test]
    fn probe_snaps_to_the_nearest_curve_sample() {
        let mut app = ScasApp::new(&egui::Context::default());
        app.add_graph_function("sin(x)");
        let bounds = app.graph_view.bounds();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 500.0));
        // The y=0 line maps to the vertical middle of the window.
        let origin = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.5);
        let series = sample_graph_batch(
            &[],
            &["sin(x)".to_owned()],
            app.repl_symbols.clone(),
            -12.0,
            12.0,
            721,
        )
        .remove(0)
        .unwrap();
        app.graph_functions[0].sampled = Ok(series);
        let probe = snap_graph_probe(origin, &bounds, rect, &app.graph_functions)
            .expect("the sine curve crosses the screen centre");
        assert_eq!(probe.series_index, 0);
        let y = probe.y.expect("sine is defined at the snapped x");
        assert!(y.abs() < 0.01, "snapped y should be near zero, got {y}");
    }

    #[test]
    fn graph_decimation_preserves_pixel_bucket_extrema_and_domain_gaps() {
        let samples = vec![
            (0.0, Some(0.0)),
            (0.1, Some(1.0)),
            (0.2, Some(8.0)),
            (0.3, Some(-5.0)),
            (0.4, Some(2.0)),
            (0.5, None),
            (0.6, Some(1.0)),
            (0.7, Some(2.0)),
            (0.8, Some(3.0)),
            (0.9, Some(4.0)),
        ];
        let bounds = PlotBounds::new(0.0, 1.0, -6.0, 9.0);
        let selected = decimate_visible_samples(&samples, &bounds, 2, bounds.y_span());
        assert!(selected.windows(2).any(|window| window == [4, 5]));
        assert!(selected.contains(&2), "retain the narrow maximum");
        assert!(selected.contains(&3), "retain the narrow minimum");
        assert!(
            selected.contains(&5),
            "retain a gap so paths do not bridge it"
        );
        assert!(selected.len() < samples.len());
    }

    #[test]
    fn graph_view_zoom_and_pan_preserve_the_center() {
        let mut view = ViewState::centered(20.0, 10.0);
        assert_eq!(view.x_min, -10.0);
        assert_eq!(view.x_max, 10.0);
        view.zoom(0.5);
        assert!((view.x_max - view.x_min - 10.0).abs() < 1.0e-9);
        assert!((view.y_max - view.y_min - 5.0).abs() < 1.0e-9);
        let center = (view.x_min + view.x_max) / 2.0;
        assert!(center.abs() < 1.0e-9);
        view.x_min += 3.0;
        view.x_max += 3.0;
        assert!(((view.x_min + view.x_max) / 2.0 - 3.0).abs() < 1.0e-9);
    }

    #[test]
    fn repl_submissions_accept_plot_commands_and_history() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        app.repl_input = ":plot x^2 from -2 to 2".to_string();
        app.submit_repl();
        assert_eq!(app.repl_entries.len(), 1);
        assert!(app.repl_entries[0].plot.is_some());
        assert!(app.repl_entries[0].error.is_none());

        app.repl_input = ":plot".to_string();
        app.submit_repl();
        assert!(app.repl_entries[1].printed.contains("usage:"));

        app.repl_history_step(-1);
        assert_eq!(app.repl_input, ":plot");
        app.repl_history_step(-1);
        assert_eq!(app.repl_input, ":plot x^2 from -2 to 2");
        app.repl_history_step(1);
        assert_eq!(app.repl_input, ":plot");
    }

    #[test]
    fn repl_input_is_rendered_from_the_live_ast() {
        let symbols = SymbolTable::new();
        let power = repl_expression_latex("(x + 1)^2", &symbols).unwrap();
        assert!(power.contains(r"^{2}"));
        assert!(power.contains("x + 1"));

        let matrix = repl_expression_latex("let A = [1, 2; 3, 4]", &symbols).unwrap();
        assert!(matrix.contains('='));
        assert!(matrix.contains(r"\begin{pmatrix}"));

        assert!(repl_expression_latex("(x +", &symbols).is_none());
    }

    #[test]
    fn starter_opens_with_a_typeset_document() {
        let document = scas::document::build(STARTER).unwrap();
        assert!(!document.fragments.is_empty());
        assert!(document.body().contains(r"\sin"));
        assert!(document.body().contains(r"\frac{1}{2}"));
    }

    #[test]
    fn diagnostic_ranges_are_unicode_aware_and_underlined() {
        let source = "let π = ;";
        let start = byte_offset(source, 1, 9);
        assert_eq!(&source[start..], ";");
        let diagnostic = EditorDiagnostic {
            byte_start: start,
            byte_end: start + 1,
            line: 1,
            column: 9,
            message: "expected expression".to_owned(),
        };
        let job = highlight(
            source,
            &Theme::bundled_themes()[0],
            FontId::monospace(15.0),
            &[diagnostic],
        );
        let semicolon = job.text.find(';').unwrap();
        assert!(job.sections.iter().any(|section| {
            section.byte_range.contains(&semicolon) && section.format.underline.width > 0.0
        }));
    }

    #[test]
    fn line_number_modes_produce_editor_gutter_values() {
        assert_eq!(
            displayed_line_number(LineNumberMode::Absolute, 4, 2),
            Some(5)
        );
        assert_eq!(
            displayed_line_number(LineNumberMode::Relative, 4, 2),
            Some(2)
        );
        assert_eq!(displayed_line_number(LineNumberMode::Hybrid, 2, 2), Some(3));
        assert_eq!(displayed_line_number(LineNumberMode::Hybrid, 4, 2), Some(2));
        assert_eq!(displayed_line_number(LineNumberMode::Off, 4, 2), None);
    }

    #[test]
    fn system_latex_produces_a_real_computer_modern_preview_when_available() {
        if Command::new("latex").arg("--version").output().is_err()
            || Command::new("dvipng").arg("--version").output().is_err()
        {
            return;
        }
        let document = scas::document::build(
            r#"\{
rref([1, 2; 3, 4]) =>;
solve(z^2-z, z) =>;
\}"#,
        )
        .unwrap();
        let rendered = render_latex_png(&document.body(), [20, 20, 20]).unwrap();
        assert!(rendered.size[0] > 20);
        assert!(rendered.size[1] > 20);
        assert_eq!(rendered.rgba.len(), rendered.size[0] * rendered.size[1] * 4);

        // REPL snippets are rendered in math mode via the same system LaTeX
        // pipeline, so fractions and symbols use actual Computer Modern glyphs.
        let repl = render_repl_math_png(r"\frac{1}{2} + \sqrt{2}", [20, 20, 20]).unwrap();
        assert!(repl.size[0] > 20);
        assert!(repl.size[1] > 10);

        let plot_document = scas::document::build(r"\[ plot(sin(x), x, -pi, pi) \]").unwrap();
        let plot = render_latex_png(&plot_document.body(), [20, 20, 20]).unwrap();
        assert!(plot.size[0] > 100);
        assert!(plot.size[1] > 50);
    }
}
