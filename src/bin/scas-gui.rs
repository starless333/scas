#[path = "scas_gui/editor_assist.rs"]
mod editor_assist;
#[path = "scas_gui/math_preview.rs"]
mod math_preview;
#[path = "scas_gui/terminal.rs"]
mod terminal;
#[path = "scas_gui/theme.rs"]
mod theme;

use eframe::egui::{
    self, text::LayoutJob, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
    TextFormat, TextStyle,
};
use scas::ast::statement::FunctionBody;
use scas::ast::StatementKind;
use scas::document::{DiagnosticSeverity, Document};
use scas::execute;
use scas::intelligence::{evaluate_hover, SourceIndex};
use scas::interpreter::Interpreter;
use scas::latex::LatexRenderer;
use scas::lexer::{Scanner, SymbolTable, TokenType};
use scas::parser::Parser;
use scas::plot::{self, CriticalPoint, PlotBounds, SampledSeries};
use scas::Limits;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

static NEXT_LATEX_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
use terminal::TerminalSession;
use theme::Theme;

const STARTER: &str = "/// A live, typeset result\n\
let angle = π / 6;\n\
\\[ sin(angle) => \\]\n\
print(sin(angle));\n";
const DEFAULT_AUTO_GREEK: bool = true;
const DEFAULT_EDITOR_FONT_SIZE: f32 = 27.0;
const DEFAULT_CODE_FONT_NAME: &str = "JetBrains Mono";
const DEFAULT_CODE_FONT_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
const CODICON_FONT_NAME: &str = "VS Code Codicons";
const CODICON_FONT_BYTES: &[u8] = include_bytes!("../../assets/icons/codicon.ttf");
const PREVIEW_UI_FONT_NAME: &str = "Segoe UI";
const INTERFACE_FONT_SIZE: f32 = 13.0;
const EXPLORER_FOOTER_HEIGHT: f32 = 112.0;

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
const MIN_DOCUMENT_PREVIEW_ZOOM: f32 = 0.5;
const MAX_DOCUMENT_PREVIEW_ZOOM: f32 = 16.0;
const DOCUMENT_PREVIEW_ZOOM_STEP: f32 = 1.25;
/// Keeps a long document name from crowding the workspace tabs and tools.
/// The complete name remains available from the tab tooltip.
const MAX_DOCUMENT_TAB_CHARS: usize = 18;

/// Curve colors for the graphing canvas, in draw order.
const GRAPH_COLORS: &[u32] = &[
    0x4f9cf9, 0xf9777f, 0x53c489, 0xc48df9, 0xf9b959, 0x5bc8d9, 0xf97fb8, 0xa3cf5b,
];
const SVG_GRAPH_COLORS: &[u32] = &[
    0x1f6feb, 0xd73a49, 0x0a8f4d, 0x8250df, 0xd97706, 0x0e7490, 0xdb2777, 0x4d7c0f,
];

const GUI_PLOT_HELP: &str =
    "usage: :plot [terminal|svg] EXPRESSION[, EXPRESSION…] [from A to B] [> FILE]\n\n\
  Draws the graph inline; svg mode or FILE ending in .svg produces a vector image.\n\nexamples:\n\
  :plot sin(x)\n\
  :plot x^2 - 2x from -5 to 5\n\
  :plot svg sin(x), cos(x) from -6 to 6 > graph.svg";

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

type LatexDocumentResult = (u64, f32, Result<Vec<Arc<LatexRender>>, String>);

#[derive(Default)]
struct LatexRenderCache {
    entries: HashMap<u64, (String, [u8; 3], u32, Arc<LatexRender>)>,
    order: VecDeque<u64>,
    bytes: usize,
}

static LATEX_RENDER_CACHE: OnceLock<Mutex<LatexRenderCache>> = OnceLock::new();

struct HighlightCache {
    source: String,
    theme: usize,
    font_size_bits: u32,
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
    severity: DiagnosticSeverity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineNumberMode {
    Off,
    Absolute,
    Relative,
    Hybrid,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
struct GuiPreferences {
    theme: String,
    custom_theme_path: Option<PathBuf>,
    editor_font_size: f32,
    custom_font_path: Option<PathBuf>,
    line_numbers: String,
    auto_greek: bool,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            theme: "Dark Modern".to_owned(),
            custom_theme_path: None,
            editor_font_size: DEFAULT_EDITOR_FONT_SIZE,
            custom_font_path: None,
            line_numbers: LineNumberMode::Absolute.key().to_owned(),
            auto_greek: DEFAULT_AUTO_GREEK,
        }
    }
}

impl GuiPreferences {
    fn normalize(mut self) -> Self {
        if !self.editor_font_size.is_finite() {
            self.editor_font_size = DEFAULT_EDITOR_FONT_SIZE;
        }
        self.editor_font_size = self.editor_font_size.clamp(9.0, 32.0);
        if LineNumberMode::from_key(&self.line_numbers).is_none() {
            self.line_numbers = LineNumberMode::Absolute.key().to_owned();
        }
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Problems,
    Output,
    Terminal,
    References,
}

#[derive(Clone, Copy)]
enum ActivityIcon {
    Explorer,
    Search,
    Repl,
    Graphing,
    Settings,
}

#[derive(Clone, Copy)]
enum ChromeIcon {
    Sidebar,
    Panel,
    Preview,
    Minimize,
    Maximize,
    Close,
    Run,
    NewFile,
    Refresh,
    Collapse,
    TabClose,
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
    series: Vec<SampledSeries>,
    ascii: String,
    svg: bool,
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

    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Hybrid => "hybrid",
        }
    }

    fn from_key(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "absolute" => Some(Self::Absolute),
            "relative" => Some(Self::Relative),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
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
    root_collapsed: bool,
    dirty: bool,
    pending_action: Option<PendingAction>,
    save_as_open: bool,
    save_as_path: String,
    allow_close: bool,
    auto_greek: bool,
    run_result: Option<Receiver<RunResult>>,
    status: String,
    themes: Vec<Theme>,
    theme_paths: Vec<Option<PathBuf>>,
    selected_theme: usize,
    startup_maximize_pending: bool,
    editor_font_size: f32,
    settings_open: bool,
    theme_path: String,
    font_path: String,
    active_font_path: Option<PathBuf>,
    preferences_path: Option<PathBuf>,
    saved_preferences: GuiPreferences,
    preferences_save_due: Option<Instant>,
    preview: String,
    preview_document: Option<Document>,
    preview_error: Option<String>,
    preview_open: bool,
    document_preview_zoom: f32,
    document_preview_scroll_offset: egui::Vec2,
    latex_texture_rects: Vec<egui::Rect>,
    output_open: bool,
    files_open: bool,
    quick_open: bool,
    quick_open_ready_to_dismiss: bool,
    quick_open_query: String,
    bottom_tab: BottomTab,
    terminal: Option<TerminalSession>,
    terminal_error: Option<String>,
    references_target: Option<String>,
    rename_target: Option<String>,
    rename_input: String,
    hover_key: Option<(u64, usize)>,
    hover_since: Instant,
    hover_value: Option<String>,
    preview_scroll_target: Option<usize>,
    preview_last_cursor: usize,
    diagnostics: Vec<EditorDiagnostic>,
    analysis_due: Option<Instant>,
    analysis_result: Option<Receiver<AnalysisResult>>,
    analysis_generation: u64,
    buffer_generation: u64,
    latex_render: Option<Receiver<LatexDocumentResult>>,
    latex_pending: Option<(String, [u8; 3], u64)>,
    last_latex_key: Option<u64>,
    last_latex_source_key: Option<u64>,
    latex_render_zoom: f32,
    latex_texture_zoom: f32,
    latex_observed_zoom: f32,
    latex_zoom_due: Option<Instant>,
    latex_textures: Vec<egui::TextureHandle>,
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
    /// Pinned tap point on the graphing canvas. Critical points take
    /// precedence over ordinary curve samples so roots, holes, and extrema
    /// remain inspectable after the pointer moves away.
    graph_probe: Option<GraphProbe>,
}

enum PendingAction {
    NewFile,
    Open(PathBuf),
    Close,
}

/// A tapped point on the graphing canvas snapped to the nearest curve.
/// Stored in data coordinates so the crosshair is redrawn from the current
/// view without re-scanning every sample on each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GraphProbe {
    /// Index into `graph_functions` of the snapped curve.
    series_index: usize,
    /// Snapped position in plot coordinates.
    x: f64,
    /// Curve y at the snapped x, when the curve covers it.
    y: Option<f64>,
    /// Notable-point classification when this probe targets a highlighted
    /// root, hole, or local extremum rather than an ordinary curve sample.
    critical_kind: Option<plot::CriticalKind>,
}

struct PlatformUiFontSpec {
    name: &'static str,
    candidates: Vec<PathBuf>,
}

fn platform_ui_font_spec(
    os: &str,
    system_root: Option<&Path>,
    user_home: Option<&Path>,
) -> PlatformUiFontSpec {
    match os {
        "windows" => {
            let root = system_root.unwrap_or_else(|| Path::new(r"C:\Windows"));
            PlatformUiFontSpec {
                name: PREVIEW_UI_FONT_NAME,
                candidates: vec![root.join("Fonts").join("segoeui.ttf")],
            }
        }
        "macos" => PlatformUiFontSpec {
            name: "SF Pro Text",
            candidates: vec![
                PathBuf::from("/System/Library/Fonts/SFNS.ttf"),
                PathBuf::from("/System/Library/Fonts/SFNSText.ttf"),
                PathBuf::from("/System/Library/Fonts/SFNSDisplay.ttf"),
            ],
        },
        _ => {
            let mut candidates = vec![
                PathBuf::from("/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf"),
                PathBuf::from("/usr/share/fonts/truetype/ubuntu/Ubuntu-Regular.ttf"),
                PathBuf::from("/usr/share/fonts/opentype/ubuntu/Ubuntu-R.ttf"),
                PathBuf::from("/usr/share/fonts/truetype/ubuntu/Ubuntu[wdth,wght].ttf"),
            ];
            if let Some(home) = user_home {
                candidates.push(home.join(".local/share/fonts/truetype/ubuntu/Ubuntu-R.ttf"));
                candidates.push(home.join(".fonts/Ubuntu-R.ttf"));
            }
            PlatformUiFontSpec {
                name: "Ubuntu",
                candidates,
            }
        }
    }
}

fn default_font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        CODICON_FONT_NAME.to_owned(),
        FontData::from_static(CODICON_FONT_BYTES),
    );
    fonts.families.insert(
        FontFamily::Name(CODICON_FONT_NAME.into()),
        vec![CODICON_FONT_NAME.to_owned()],
    );
    fonts.font_data.insert(
        DEFAULT_CODE_FONT_NAME.to_owned(),
        FontData::from_static(DEFAULT_CODE_FONT_BYTES),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, DEFAULT_CODE_FONT_NAME.to_owned());

    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    let user_home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let ui_font = platform_ui_font_spec(
        std::env::consts::OS,
        system_root.as_deref(),
        user_home.as_deref(),
    );
    if let Some(bytes) = ui_font
        .candidates
        .iter()
        .find_map(|path| fs::read(path).ok())
    {
        fonts
            .font_data
            .insert(ui_font.name.to_owned(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, ui_font.name.to_owned());
    }
    if cfg!(windows) {
        let preview_family = fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        fonts.families.insert(
            FontFamily::Name(PREVIEW_UI_FONT_NAME.into()),
            preview_family,
        );
    }
    fonts
}

fn install_default_fonts(context: &egui::Context) {
    context.set_fonts(default_font_definitions());
}

fn preview_ui_font(size: f32) -> FontId {
    if cfg!(windows) {
        FontId::new(size, FontFamily::Name(PREVIEW_UI_FONT_NAME.into()))
    } else {
        FontId::proportional(size)
    }
}

fn preview_widths(screen_width: f32, explorer_width: f32) -> (f32, f32) {
    let usable_width = (screen_width - 48.0 - explorer_width).max(0.0);
    let max_width = (usable_width - MIN_EDITOR_WORKSPACE_WIDTH).max(MIN_PREVIEW_WIDTH);
    let default_width = (usable_width * 0.46).clamp(MIN_PREVIEW_WIDTH, max_width);
    (default_width, max_width)
}

fn adjusted_document_preview_zoom(current: f32, factor: f32) -> f32 {
    (current * factor).clamp(MIN_DOCUMENT_PREVIEW_ZOOM, MAX_DOCUMENT_PREVIEW_ZOOM)
}

fn zoom_scroll_offset_about_pointer(
    offset: egui::Vec2,
    pointer: egui::Pos2,
    page_rects: &[egui::Rect],
    factor: f32,
) -> egui::Vec2 {
    if !factor.is_finite() || (factor - 1.0).abs() <= f32::EPSILON {
        return offset;
    }
    let Some((page_index, page_rect)) = page_rects
        .iter()
        .enumerate()
        .find(|(_, rect)| rect.contains(pointer))
    else {
        return offset;
    };
    let preceding_page_height = page_rects[..page_index]
        .iter()
        .map(egui::Rect::height)
        .sum::<f32>();
    let local = pointer - page_rect.min;
    let delta = egui::vec2(local.x, preceding_page_height + local.y) * (factor - 1.0);
    egui::vec2((offset.x + delta.x).max(0.0), (offset.y + delta.y).max(0.0))
}

fn preferences_path_for(
    os: &str,
    xdg_config_home: Option<&Path>,
    app_data: Option<&Path>,
    user_home: Option<&Path>,
) -> Option<PathBuf> {
    let directory = match os {
        "windows" => app_data.map(|path| path.join("SCAS")),
        "macos" => user_home.map(|path| path.join("Library/Application Support/SCAS")),
        _ => xdg_config_home
            .map(|path| path.join("scas"))
            .or_else(|| user_home.map(|path| path.join(".config/scas"))),
    }?;
    Some(directory.join("preferences.toml"))
}

fn preferences_path() -> Option<PathBuf> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    let user_home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    preferences_path_for(
        std::env::consts::OS,
        xdg_config_home.as_deref(),
        app_data.as_deref(),
        user_home.as_deref(),
    )
}

fn load_preferences(path: Option<&Path>) -> (GuiPreferences, Option<String>) {
    let Some(path) = path else {
        return (
            GuiPreferences::default(),
            Some(
                "Preferences will reset on restart because no config directory is available"
                    .to_owned(),
            ),
        );
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (GuiPreferences::default(), None)
        }
        Err(error) => {
            return (
                GuiPreferences::default(),
                Some(format!("Could not read preferences: {error}")),
            )
        }
    };
    match toml::from_str::<GuiPreferences>(&source) {
        Ok(preferences) => (preferences.normalize(), None),
        Err(error) => (
            GuiPreferences::default(),
            Some(format!("Ignored invalid preferences: {error}")),
        ),
    }
}

fn save_preferences(path: &Path, preferences: &GuiPreferences) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "preferences path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let source = toml::to_string_pretty(preferences)
        .map_err(|error| format!("could not encode preferences: {error}"))?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, &source)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    if let Err(rename_error) = fs::rename(&temporary, path) {
        fs::write(path, source).map_err(|error| {
            format!(
                "could not save {}: {error} ({rename_error})",
                path.display()
            )
        })?;
        let _ = fs::remove_file(temporary);
    }
    Ok(())
}

fn font_definitions_from_path(path: &Path) -> Result<(FontDefinitions, String), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not load font {}: {error}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("custom-editor-font")
        .to_owned();
    let mut fonts = default_font_definitions();
    fonts
        .font_data
        .insert(name.clone(), FontData::from_owned(bytes));
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, name.clone());
    Ok((fonts, name))
}

impl ScasApp {
    fn new(context: &egui::Context) -> Self {
        install_default_fonts(context);
        let preferences_path = preferences_path();
        let (preferences, preference_warning) = load_preferences(preferences_path.as_deref());
        let mut warnings = preference_warning.into_iter().collect::<Vec<_>>();
        let mut themes = Theme::bundled_themes();
        let mut theme_paths = vec![None; themes.len()];
        let mut selected_theme = themes
            .iter()
            .position(|theme| theme.name == preferences.theme)
            .unwrap_or(0);
        if let Some(path) = preferences.custom_theme_path.as_deref() {
            match Theme::load(path) {
                Ok(theme) => {
                    themes.push(theme);
                    theme_paths.push(Some(path.to_owned()));
                    selected_theme = themes.len() - 1;
                }
                Err(error) => warnings.push(format!("Saved theme was unavailable: {error}")),
            }
        }
        themes[selected_theme].apply(context);
        let mut active_font_path = None;
        if let Some(path) = preferences.custom_font_path.as_deref() {
            match font_definitions_from_path(path) {
                Ok((fonts, _)) => {
                    context.set_fonts(fonts);
                    active_font_path = Some(path.to_owned());
                }
                Err(error) => warnings.push(format!("Saved font was unavailable: {error}")),
            }
        }
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let files = collect_tree(&project_root);
        let collapsed_dirs = files
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| entry.path.clone())
            .collect();
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
            collapsed_dirs,
            root_collapsed: false,
            dirty: false,
            pending_action: None,
            save_as_open: false,
            save_as_path: String::new(),
            allow_close: false,
            auto_greek: preferences.auto_greek,
            run_result: None,
            status: if warnings.is_empty() {
                "Ready".to_owned()
            } else {
                warnings.join("; ")
            },
            themes,
            theme_paths,
            selected_theme,
            startup_maximize_pending: true,
            editor_font_size: preferences.editor_font_size,
            settings_open: false,
            theme_path: preferences
                .custom_theme_path
                .as_deref()
                .unwrap_or_else(|| Path::new("themes/dark-modern.toml"))
                .display()
                .to_string(),
            font_path: preferences
                .custom_font_path
                .as_deref()
                .unwrap_or_else(|| Path::new("assets/fonts/JetBrainsMono-Regular.ttf"))
                .display()
                .to_string(),
            active_font_path,
            preferences_path,
            saved_preferences: GuiPreferences::default(),
            preferences_save_due: None,
            preview: String::new(),
            preview_document: None,
            preview_error: None,
            preview_open: true,
            document_preview_zoom: 1.0,
            document_preview_scroll_offset: egui::Vec2::ZERO,
            latex_texture_rects: Vec::new(),
            output_open: false,
            files_open: true,
            quick_open: false,
            quick_open_ready_to_dismiss: false,
            quick_open_query: String::new(),
            bottom_tab: BottomTab::Output,
            terminal: None,
            terminal_error: None,
            references_target: None,
            rename_target: None,
            rename_input: String::new(),
            hover_key: None,
            hover_since: Instant::now(),
            hover_value: None,
            preview_scroll_target: None,
            preview_last_cursor: 0,
            diagnostics: Vec::new(),
            analysis_due: None,
            analysis_result: None,
            analysis_generation: 0,
            buffer_generation: 0,
            latex_render: None,
            latex_pending: None,
            last_latex_key: None,
            last_latex_source_key: None,
            latex_render_zoom: 1.0,
            latex_texture_zoom: 1.0,
            latex_observed_zoom: 1.0,
            latex_zoom_due: None,
            latex_textures: Vec::new(),
            latex_render_error: None,
            repl_math_receiver: None,
            repl_math_pending: VecDeque::new(),
            repl_math_inflight: HashSet::new(),
            repl_math_cache: HashMap::new(),
            repl_math_error: None,
            repl_input_math_due: None,
            line_numbers: LineNumberMode::from_key(&preferences.line_numbers)
                .unwrap_or(LineNumberMode::Absolute),
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
        app.saved_preferences = app.preference_snapshot();
        app.start_analysis();
        app
    }

    fn preference_snapshot(&self) -> GuiPreferences {
        GuiPreferences {
            theme: self.themes[self.selected_theme].name.clone(),
            custom_theme_path: self.theme_paths[self.selected_theme].clone(),
            editor_font_size: self.editor_font_size,
            custom_font_path: self.active_font_path.clone(),
            line_numbers: self.line_numbers.key().to_owned(),
            auto_greek: self.auto_greek,
        }
    }

    fn persist_preferences(&mut self) {
        let preferences = self.preference_snapshot();
        self.preferences_save_due = None;
        if preferences == self.saved_preferences {
            return;
        }
        if let Some(path) = self.preferences_path.as_deref() {
            if let Err(error) = save_preferences(path, &preferences) {
                self.status = format!("Could not save preferences: {error}");
            }
        }
        self.saved_preferences = preferences;
    }

    fn queue_preferences_save(&mut self, context: &egui::Context) {
        if self.preference_snapshot() == self.saved_preferences {
            self.preferences_save_due = None;
            return;
        }
        let due = *self
            .preferences_save_due
            .get_or_insert_with(|| Instant::now() + Duration::from_millis(250));
        let now = Instant::now();
        if now >= due {
            self.persist_preferences();
        } else {
            context.request_repaint_after(due - now);
        }
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
        // Preserve an inspected point across background re-sampling. Critical
        // points are reconciled to the closest newly sampled point of the
        // same kind so zooming or refinement cannot make a pinned label blip.
        if let Some(probe) = self.graph_probe {
            self.graph_probe = self
                .graph_functions
                .get(probe.series_index)
                .and_then(|function| {
                    let Some(kind) = probe.critical_kind else {
                        return Some(probe);
                    };
                    function
                        .critical_points
                        .iter()
                        .filter(|point| point.kind == kind)
                        .min_by(|left, right| {
                            (left.x - probe.x)
                                .abs()
                                .total_cmp(&(right.x - probe.x).abs())
                        })
                        .map(|point| GraphProbe {
                            series_index: probe.series_index,
                            x: point.x,
                            y: Some(point.y),
                            critical_kind: Some(point.kind),
                        })
                        .or(Some(probe))
                });
        }
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
        self.submit_repl_with_execution_source(None);
    }

    fn submit_repl_with_execution_source(&mut self, execution_source: Option<&str>) {
        let source = self.repl_input.trim().to_owned();
        if source.is_empty() {
            return;
        }
        self.repl_history.push(source.clone());
        self.repl_history_cursor = None;
        if source == ":help" {
            self.push_repl_command_result(
                source,
                "Commands:\n  :help             show this help\n  :precision N      set decimal precision\n  :plot EXPR        draw a graph (use :plot for examples)\n  :clear            clear displayed history\n  :reset            reset definitions and precision\n\nEnter evaluates. Ctrl+Enter evaluates numerically.",
                None,
            );
            return;
        }
        if source == ":precision" {
            self.push_repl_command_result(
                source,
                &format!(
                    "decimal precision: {} digits\nusage: :precision N (N must be greater than zero)",
                    self.repl_interpreter.decimal_precision()
                ),
                None,
            );
            return;
        }
        if let Some(argument) = source.strip_prefix(":precision ") {
            let result = argument
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(NonZeroU64::new);
            match result {
                Some(precision) => match self.repl_interpreter.set_decimal_precision(precision) {
                    Ok(()) => self.push_repl_command_result(
                        source,
                        &format!(
                            "decimal precision: {} digits",
                            self.repl_interpreter.decimal_precision()
                        ),
                        None,
                    ),
                    Err(error) => self.push_repl_command_result(
                        source,
                        "",
                        Some(format!("limit error: {error}")),
                    ),
                },
                None => self.push_repl_command_result(
                    source,
                    "",
                    Some("usage: :precision N (N must be greater than zero)".to_owned()),
                ),
            }
            return;
        }
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

        let submission = terminate_repl_source(execution_source.unwrap_or(&source));
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
        let mut result_math_key = None;
        let mut error = (!scan_error.is_empty()).then_some(scan_error);
        if error.is_none() {
            match Parser::new(tokens).parse() {
                Ok(program) => match self.repl_interpreter.run_interactive_value(&program) {
                    Ok(Some(value)) => {
                        let latex =
                            LatexRenderer::new(self.repl_interpreter.symbols()).value(&value);
                        result_math_key = Some(self.queue_repl_math(
                            &latex,
                            [repl_color.r(), repl_color.g(), repl_color.b()],
                        ));
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

    fn push_repl_command_result(&mut self, source: String, printed: &str, error: Option<String>) {
        self.repl_entries.push(ReplEntry {
            source,
            input_latex: None,
            input_math_key: None,
            result_math_key: None,
            printed: printed.to_owned(),
            error,
            plot: None,
        });
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.repl_input_math_key = None;
        self.repl_input_math_due = None;
    }

    fn submit_repl_numerically(&mut self) {
        let source = self.repl_input.trim().to_owned();
        if source.is_empty() {
            return;
        }
        if source.starts_with(':') {
            self.set_status("Ctrl+Enter numerically evaluates expressions; use Enter for commands");
            return;
        }
        let numerical_source = format!("evalf({source})");
        self.submit_repl_with_execution_source(Some(&numerical_source));
    }

    /// Evaluates a `:plot …` line inside the GUI REPL and keeps its samples
    /// for an inline text or vector graph.
    fn submit_repl_plot(&mut self, rest: &str) {
        if rest.is_empty() {
            self.repl_entries.push(ReplEntry {
                source: ":plot".to_owned(),
                input_latex: None,
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
                                series: outcome.series,
                                ascii: outcome.ascii,
                                svg: outcome.svg.is_some()
                                    || outcome.wrote_file.as_ref().is_some_and(|path| {
                                        path.extension().is_some_and(|extension| {
                                            extension.eq_ignore_ascii_case("svg")
                                        })
                                    }),
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
        self.output_open = true;
        self.bottom_tab = BottomTab::Output;
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

    fn show_terminal_tab(&mut self, context: &egui::Context) {
        self.workspace_tab = WorkspaceTab::Document;
        self.output_open = true;
        self.bottom_tab = BottomTab::Terminal;
        if self.terminal.is_none() {
            match TerminalSession::start(&self.project_root) {
                Ok(session) => {
                    self.terminal = Some(session);
                    self.terminal_error = None;
                }
                Err(error) => self.terminal_error = Some(format!("Could not start shell: {error}")),
            }
        }
        context.memory_mut(|memory| memory.request_focus(egui::Id::new(terminal::TERMINAL_ID)));
    }

    fn jump_to_diagnostic(&mut self, context: &egui::Context, diagnostic: &EditorDiagnostic) {
        self.jump_to_byte(context, diagnostic.byte_start);
    }

    fn jump_to_byte(&mut self, context: &egui::Context, byte: usize) {
        self.workspace_tab = WorkspaceTab::Document;
        let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
        let char_index = self.source[..previous_char_boundary(&self.source, byte)]
            .chars()
            .count();
        let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(char_index),
            )));
        TextEdit::store_state(context, editor_id, state);
        context.memory_mut(|memory| memory.request_focus(editor_id));
        self.cursor_char_index = char_index;
        self.current_line = self.source[..previous_char_boundary(&self.source, byte)]
            .bytes()
            .filter(|character| *character == b'\n')
            .count();
    }

    fn symbol_at_cursor(&self) -> Option<String> {
        let index = SourceIndex::new(&self.source);
        let byte = char_to_byte(&self.source, self.cursor_char_index);
        index
            .at(byte)
            .or_else(|| byte.checked_sub(1).and_then(|before| index.at(before)))
            .map(|item| item.name.clone())
    }

    fn apply_symbol_rename(&mut self, context: &egui::Context) {
        let Some(target) = self.rename_target.take() else {
            return;
        };
        let replacement = self.rename_input.trim().to_owned();
        let index = SourceIndex::new(&self.source);
        let Some(spans) = index.rename(&target, &replacement) else {
            self.set_status("Rename unavailable: choose a unique symbol and a valid new name");
            return;
        };
        if spans.is_empty() || target == replacement {
            return;
        }
        let before = self.source.clone();
        let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
        let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
        let old_range = state.cursor.char_range().unwrap_or_else(|| {
            egui::text::CCursorRange::one(egui::text::CCursor::new(self.cursor_char_index))
        });
        let mut undoer = state.undoer();
        undoer.add_undo(&(old_range, before));
        state.set_undoer(undoer);
        for span in spans.iter().rev() {
            self.source.replace_range(span.clone(), &replacement);
        }
        TextEdit::store_state(context, editor_id, state);
        self.highlight_cache = None;
        self.dirty = true;
        self.schedule_analysis(context);
        self.set_status(&format!(
            "Renamed {target} to {replacement} ({} locations)",
            spans.len()
        ));
    }

    fn show_rename(&mut self, context: &egui::Context) {
        if self.rename_target.is_none() {
            return;
        }
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Rename symbol")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(context, |ui| {
                ui.label(format!(
                    "Rename {} across this document",
                    self.rename_target.as_deref().unwrap_or("")
                ));
                let response = ui.text_edit_singleline(&mut self.rename_input);
                response.request_focus();
                apply = ui.button("Rename").clicked()
                    || (response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                ui.label("Only unambiguous document-level symbols can be renamed.");
            });
        if !open {
            self.rename_target = None;
        }
        if apply {
            self.apply_symbol_rename(context);
        }
    }

    fn open(&mut self, path: PathBuf) {
        if self.current_file.as_ref() == Some(&path) {
            self.workspace_tab = WorkspaceTab::Document;
            return;
        }
        self.request_action(PendingAction::Open(path), None);
    }

    fn open_unchecked(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(source) => {
                self.source = source;
                self.current_file = Some(path.clone());
                self.workspace_tab = WorkspaceTab::Document;
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

    fn save(&mut self) -> bool {
        let Some(path) = self.current_file.as_ref() else {
            self.open_save_as();
            return false;
        };
        match fs::write(path, &self.source) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", path.display());
                true
            }
            Err(error) => {
                self.status = format!("Could not save {}: {error}", path.display());
                false
            }
        }
    }

    fn open_save_as(&mut self) {
        if self.save_as_path.is_empty() {
            self.save_as_path = self
                .current_file
                .clone()
                .unwrap_or_else(|| self.project_root.join("untitled.scas"))
                .display()
                .to_string();
        }
        self.save_as_open = true;
    }

    fn save_as(&mut self) -> bool {
        let typed = self.save_as_path.trim();
        if typed.is_empty() {
            self.set_status("Enter a path for the .scas file");
            return false;
        }
        let path = PathBuf::from(typed);
        let path = if path.is_absolute() {
            path
        } else {
            self.project_root.join(path)
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("scas") {
            self.set_status("Save As requires a .scas filename");
            return false;
        }
        if path.exists() && self.current_file.as_ref() != Some(&path) {
            self.set_status("That file already exists; choose a new filename");
            return false;
        }
        match fs::write(&path, &self.source) {
            Ok(()) => {
                self.current_file = Some(path.clone());
                self.dirty = false;
                self.save_as_open = false;
                self.save_as_path.clear();
                self.files = collect_tree(&self.project_root);
                self.status = format!("Saved {}", path.display());
                true
            }
            Err(error) => {
                self.status = format!("Could not save {}: {error}", path.display());
                false
            }
        }
    }

    fn request_action(&mut self, action: PendingAction, context: Option<&egui::Context>) {
        if self.dirty {
            self.pending_action = Some(action);
        } else {
            self.perform_action(action, context);
        }
    }

    fn perform_action(&mut self, action: PendingAction, context: Option<&egui::Context>) {
        match action {
            PendingAction::NewFile => self.new_file_unchecked(),
            PendingAction::Open(path) => self.open_unchecked(path),
            PendingAction::Close => {
                self.persist_preferences();
                self.allow_close = true;
                if let Some(context) = context {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn finish_pending_action(&mut self, context: &egui::Context) {
        if let Some(action) = self.pending_action.take() {
            self.perform_action(action, Some(context));
        }
    }

    fn show_save_and_close_dialogs(&mut self, context: &egui::Context) {
        if self.save_as_open {
            egui::Window::new("Save As")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    ui.label("Save the current document as a .scas file");
                    ui.add_sized(
                        [420.0, 25.0],
                        egui::TextEdit::singleline(&mut self.save_as_path),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() && self.save_as() {
                            self.finish_pending_action(context);
                        }
                        if ui.button("Cancel").clicked() {
                            self.save_as_open = false;
                            self.pending_action = None;
                        }
                    });
                    if self.status.starts_with("Could not save")
                        || self.status.starts_with("Save As requires")
                        || self.status.starts_with("That file already exists")
                        || self.status.starts_with("Enter a path")
                    {
                        ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
                    }
                });
        } else if self.pending_action.is_some() {
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    ui.label("Save changes to this document?");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() && self.save() {
                            self.finish_pending_action(context);
                        }
                        if ui.button("Discard").clicked() {
                            self.finish_pending_action(context);
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_action = None;
                        }
                    });
                });
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
        self.request_action(PendingAction::NewFile, None);
    }

    fn new_file_unchecked(&mut self) {
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
            let document = scas::document::build_live_with_limits(&source, live_preview_limits())
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
                if self.preview_open {
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
                        severity: diagnostic.severity,
                    });
                }
                diagnostics.sort_by_key(|diagnostic| {
                    (diagnostic.line, diagnostic.column, diagnostic.byte_start)
                });
                diagnostics.dedup_by(|a, b| a.byte_start == b.byte_start && a.message == b.message);
                self.diagnostics = diagnostics;
                self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
                self.highlight_cache = None;
                self.preview_document = Some(document);
                self.preview_error = None;
            }
            Err(error) => {
                diagnostics.sort_by_key(|diagnostic| {
                    (diagnostic.line, diagnostic.column, diagnostic.byte_start)
                });
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
        if !self.preview_open {
            return;
        }
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        color.hash(&mut hasher);
        let source_key = hasher.finish();
        if self.last_latex_source_key != Some(source_key) {
            // A texture from another document must never be shown as its preview.
            self.latex_textures.clear();
            self.latex_texture_rects.clear();
            self.last_latex_source_key = Some(source_key);
            self.latex_render_zoom = 1.0;
            self.latex_texture_zoom = 1.0;
            self.latex_zoom_due = Some(Instant::now() + Duration::from_millis(250));
        }
        self.latex_render_zoom.to_bits().hash(&mut hasher);
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
        self.last_latex_key = None;
        let zoom = self.latex_render_zoom;
        let (sender, receiver) = mpsc::channel();
        self.latex_render = Some(receiver);
        self.latex_render_error = None;
        std::thread::spawn(move || {
            let result = render_latex_document_pngs(&body, color, zoom);
            let _ = sender.send((key, zoom, result));
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
            Ok((key, zoom, Ok(renders))) => {
                if self.last_latex_key.is_some_and(|expected| expected != key)
                    || self
                        .latex_pending
                        .as_ref()
                        .is_some_and(|(_, _, pending_key)| *pending_key != key)
                {
                    self.latex_render = None;
                } else {
                    self.latex_textures = renders
                        .into_iter()
                        .enumerate()
                        .map(|(index, render)| {
                            let image =
                                egui::ColorImage::from_rgba_unmultiplied(render.size, &render.rgba);
                            context.load_texture(
                                format!("latex-preview-{index}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            )
                        })
                        .collect();
                    self.latex_texture_rects.clear();
                    self.last_latex_key = Some(key);
                    self.latex_texture_zoom = zoom;
                    self.latex_render_error = None;
                    self.latex_render = None;
                }
            }
            Ok((key, _, Err(error))) => {
                if self.last_latex_key.is_some_and(|expected| expected != key)
                    || self
                        .latex_pending
                        .as_ref()
                        .is_some_and(|(_, _, pending_key)| *pending_key != key)
                {
                    self.latex_render = None;
                } else {
                    self.latex_render_error = Some(error);
                    self.last_latex_key = Some(key);
                    self.latex_render = None;
                }
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
                let saved_path = fs::canonicalize(&path).unwrap_or(path);
                self.status = format!("Loaded theme {}", theme.name);
                self.themes.push(theme);
                self.theme_paths.push(Some(saved_path));
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
        match font_definitions_from_path(&path) {
            Ok((fonts, name)) => {
                context.set_fonts(fonts);
                self.active_font_path = Some(fs::canonicalize(&path).unwrap_or(path));
                self.highlight_cache = None;
                self.status = format!("Loaded font {name}");
            }
            Err(error) => self.status = error,
        }
    }

    fn run_active(&mut self) {
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

    fn show_title_bar(&mut self, context: &egui::Context, theme: &Theme) {
        egui::TopBottomPanel::top("vscode-title-bar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.title_bar_color())
                    .inner_margin(egui::Margin::symmetric(7.0, 4.0)),
            )
            .show(context, |ui| {
                let available = ui.available_width();
                let wide = available > 1200.0;
                let left_width = (available * 0.32).clamp(410.0, 690.0);
                let center_width = (available * 0.38).clamp(350.0, 970.0);
                ui.horizontal_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if paint_scas_logo(ui, theme).drag_started() {
                                context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                            }
                            ui.add_space(9.0);
                            let old_button_frame = ui.visuals().button_frame;
                            ui.visuals_mut().button_frame = false;
                            ui.menu_button("File", |ui| {
                                if ui.button("New File").clicked() {
                                    self.new_file();
                                    ui.close_menu();
                                }
                                if ui.button("Quick Open…  Ctrl+P").clicked() {
                                    self.quick_open = true;
                                    ui.close_menu();
                                }
                                if ui.button("Save  Ctrl+S").clicked() {
                                    self.save();
                                    ui.close_menu();
                                }
                                if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                                    self.open_save_as();
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
                            ui.menu_button("Edit", |ui| {
                                if ui
                                    .checkbox(&mut self.auto_greek, "Greek shortcuts")
                                    .clicked()
                                {
                                    ui.close_menu();
                                }
                                if ui.button("Settings…").clicked() {
                                    self.settings_open = true;
                                    ui.close_menu();
                                }
                            });
                            ui.menu_button("View", |ui| {
                                ui.checkbox(&mut self.files_open, "Explorer  Ctrl+B");
                                ui.checkbox(&mut self.preview_open, "Document Preview");
                                ui.checkbox(&mut self.output_open, "Output Panel  Ctrl+J");
                                if ui.button("Problems").clicked() {
                                    self.bottom_tab = BottomTab::Problems;
                                    self.output_open = true;
                                    ui.close_menu();
                                }
                                if ui.button("Terminal").clicked() {
                                    self.show_terminal_tab(context);
                                    ui.close_menu();
                                }
                            });
                            if wide {
                                ui.menu_button("Go", |ui| {
                                    if ui.button("Go to File…  Ctrl+P").clicked() {
                                        self.quick_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Editor").clicked() {
                                        self.workspace_tab = WorkspaceTab::Document;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.menu_button("Run", |ui| {
                                if ui.button("Run Active  Ctrl+Enter").clicked() {
                                    self.run_active();
                                    ui.close_menu();
                                }
                                if ui.button("Graphing Calculator").clicked() {
                                    self.workspace_tab = WorkspaceTab::Graphing;
                                    ui.close_menu();
                                }
                            });
                            if wide {
                                ui.menu_button("Terminal", |ui| {
                                    if ui.button("New Terminal").clicked() {
                                        self.show_terminal_tab(context);
                                        ui.close_menu();
                                    }
                                    if ui.button("Interactive REPL").clicked() {
                                        self.workspace_tab = WorkspaceTab::Repl;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.visuals_mut().button_frame = old_button_frame;
                            let drag_width = ui.available_width();
                            if drag_width > 5.0 {
                                let (_, drag) = ui.allocate_exact_size(
                                    egui::vec2(drag_width, 27.0),
                                    egui::Sense::click_and_drag(),
                                );
                                if drag.drag_started() {
                                    context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                                }
                                if drag.double_clicked() {
                                    let maximized = context
                                        .input(|input| input.viewport().maximized.unwrap_or(false));
                                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(
                                        !maximized,
                                    ));
                                }
                            }
                        },
                    );
                    ui.add_space(5.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(center_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let project = self
                                .project_root
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("SCAS");
                            let response = ui.add_sized(
                                [center_width - 2.0, 26.0],
                                egui::Button::new(
                                    RichText::new(project).size(12.0).color(theme.ui.text.0),
                                )
                                .fill(theme.title_bar_color().gamma_multiply(0.90))
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    theme.ui.muted.0.gamma_multiply(0.28),
                                )),
                            );
                            if response.on_hover_text("Quick Open · Ctrl+P").clicked() {
                                self.quick_open = true;
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if chrome_button(ui, ChromeIcon::Close, theme)
                            .on_hover_text("Close")
                            .clicked()
                        {
                            self.request_action(PendingAction::Close, Some(context));
                        }
                        let maximized =
                            context.input(|input| input.viewport().maximized.unwrap_or(false));
                        if chrome_button(ui, ChromeIcon::Maximize, theme)
                            .on_hover_text(if maximized { "Restore" } else { "Maximize" })
                            .clicked()
                        {
                            context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if chrome_button(ui, ChromeIcon::Minimize, theme)
                            .on_hover_text("Minimize")
                            .clicked()
                        {
                            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(7.0);
                        if chrome_button(ui, ChromeIcon::Preview, theme)
                            .on_hover_text("Toggle preview")
                            .clicked()
                        {
                            self.preview_open = !self.preview_open;
                        }
                        if chrome_button(ui, ChromeIcon::Panel, theme)
                            .on_hover_text("Toggle output panel")
                            .clicked()
                        {
                            self.output_open = !self.output_open;
                        }
                        if chrome_button(ui, ChromeIcon::Sidebar, theme)
                            .on_hover_text("Toggle Explorer")
                            .clicked()
                        {
                            self.files_open = !self.files_open;
                        }
                    });
                });
            });
    }

    fn show_quick_open(&mut self, context: &egui::Context) {
        if !self.quick_open {
            return;
        }
        let mut open = self.quick_open;
        let mut selected = None;
        let mut dismiss = false;
        let can_dismiss_on_outside_click = self.quick_open_ready_to_dismiss;
        let window = egui::Window::new("Quick Open")
            .id(egui::Id::new("quick-open-window"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 58.0))
            .open(&mut open)
            .show(context, |ui| {
                ui.set_min_width(470.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.quick_open_query)
                        .id_source("quick-open-query")
                        .font(TextStyle::Body)
                        .hint_text("Search SCAS files by name")
                        .desired_width(f32::INFINITY),
                );
                if !response.has_focus() {
                    response.request_focus();
                }
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    dismiss = true;
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(290.0)
                    .show(ui, |ui| {
                        let query = self.quick_open_query.to_lowercase();
                        for entry in self
                            .files
                            .iter()
                            .filter(|entry| !entry.is_dir)
                            .filter(|entry| {
                                entry.path.to_string_lossy().to_lowercase().contains(&query)
                            })
                            .take(12)
                        {
                            let name = entry
                                .path
                                .strip_prefix(&self.project_root)
                                .unwrap_or(&entry.path)
                                .display()
                                .to_string();
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(name)
                                        .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                                )
                                .clicked()
                            {
                                selected = Some(entry.path.clone());
                            }
                        }
                    });
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    let query = self.quick_open_query.to_lowercase();
                    selected = self
                        .files
                        .iter()
                        .find(|entry| {
                            !entry.is_dir
                                && entry.path.to_string_lossy().to_lowercase().contains(&query)
                        })
                        .map(|entry| entry.path.clone());
                }
            });
        let click_position = context.input(|input| {
            input
                .pointer
                .any_click()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        if let Some(window) = window.as_ref() {
            dismiss |= quick_open_clicked_outside(
                window.response.rect,
                click_position,
                can_dismiss_on_outside_click,
            );
        }
        if let Some(path) = selected {
            self.open(path);
            self.workspace_tab = WorkspaceTab::Document;
            open = false;
        }
        self.quick_open = open && !dismiss;
        self.quick_open_ready_to_dismiss = self.quick_open;
        if !self.quick_open {
            self.quick_open_ready_to_dismiss = false;
            self.quick_open_query.clear();
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
        // Keep the complete composer above the application status bar.  The
        // live Computer Modern preview can make the composer taller than its
        // structured fallback, so give that region priority and let history
        // shrink on short windows instead of clipping the command row.
        let history_height = (ui.available_height() - 224.0).max(32.0);
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
                                if let Some(key) = entry.result_math_key {
                                    if let Some(texture) = self.repl_math_cache.get(&key) {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| show_cm_texture(ui, texture, ui.available_width()),
                                        );
                                    }
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
            .inner_margin(egui::Margin::symmetric(12.0, 12.0))
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
                        RichText::new(
                            "Enter evaluates · Ctrl+Enter evaluates numerically · ↑/↓ history · :plot draws a graph",
                        )
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
                        if let Some(latex) = &self.repl_input_latex {
                            let mut rendered_texture = false;
                            if let Some(key) = self.repl_input_math_key {
                                if let Some(texture) = self.repl_math_cache.get(&key) {
                                    show_cm_texture(ui, texture, ui.available_width());
                                    rendered_texture = true;
                                }
                            }
                            // Never replace valid math with a loading message:
                            // show the structured LaTeX immediately, then swap
                            // in Computer Modern when its texture is ready.
                            if !rendered_texture {
                                math_preview::show_expression(
                                    ui,
                                    latex,
                                    theme.ui.text.0,
                                    26.0,
                                );
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
                submit = enter_pressed && (input_focused || response.lost_focus());
                if let Some(error) = &self.repl_math_error {
                    ui.colored_label(theme.syntax.operator.0, format!("Computer Modern unavailable: {error}"));
                }
                ui.horizontal(|ui| {
                    if ui.button("Evaluate").clicked() {
                        submit = true;
                    }
                    ui.label(
                        RichText::new("Commands: :help · :precision N · :plot sin(x) · :clear · :reset")
                            .size(10.5)
                            .color(theme.ui.muted.0),
                    );
                });
            });
        ui.add_space(18.0);
        if submit {
            self.submit_repl();
            context.memory_mut(|memory| memory.request_focus(input_id));
            // The text edit was painted before submit_repl cleared its model.
            // Request another frame so the cleared composer and new history
            // entry replace that stale pre-submit frame immediately.
            context.request_repaint();
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
                    self.graph_probe = self.graph_probe.and_then(|mut probe| {
                        if probe.series_index == index {
                            None
                        } else {
                            if probe.series_index > index {
                                probe.series_index -= 1;
                            }
                            Some(probe)
                        }
                    });
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
                self.ensure_graph_coverage();
            }
        }
        let bounds = self.current_graph_bounds();
        let painter = ui.painter_at(plot_rect);
        let hovered_critical = canvas
            .hover_pos()
            .filter(|pointer| plot_rect.contains(*pointer))
            .and_then(|pointer| {
                snap_graph_critical_point(pointer, &bounds, plot_rect, &self.graph_functions, 12.0)
            });
        let hovered_series = canvas
            .hover_pos()
            .filter(|pointer| plot_rect.contains(*pointer))
            .and_then(|pointer| {
                hovered_critical
                    .map(|probe| probe.series_index)
                    .or_else(|| {
                        hovered_graph_series(
                            pointer,
                            &bounds,
                            plot_rect,
                            &self.graph_functions,
                            12.0,
                        )
                    })
            });

        // A tap (click without a preceding drag) pins a highlighted critical
        // point before falling back to the nearest ordinary curve sample.
        if canvas.clicked() {
            self.graph_probe = canvas
                .interact_pointer_pos()
                .filter(|pointer| plot_rect.contains(*pointer))
                .and_then(|pointer| {
                    snap_graph_critical_point(
                        pointer,
                        &bounds,
                        plot_rect,
                        &self.graph_functions,
                        12.0,
                    )
                    .or_else(|| {
                        snap_graph_probe(pointer, &bounds, plot_rect, &self.graph_functions)
                    })
                });
        }

        draw_graph_canvas(
            painter,
            &bounds,
            &self.graph_functions,
            theme,
            plot_rect,
            hovered_series,
        );

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
        if let Some(probe) = hovered_critical.filter(|probe| Some(*probe) != self.graph_probe) {
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
                                "x: {} … {}   y: {} … {}   drag to pan · scroll to zoom · hover a point · click to pin",
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
                ui.checkbox(&mut self.auto_greek, "Expand LaTeX-style Greek shortcuts");
                ui.separator();
                ui.label("Editor font");
                ui.horizontal(|ui| {
                    ui.label("Monospace");
                    ui.add(egui::Slider::new(&mut self.editor_font_size, 9.0..=32.0).suffix(" pt"));
                });
                ui.horizontal(|ui| {
                    ui.label("Font file");
                    ui.text_edit_singleline(&mut self.font_path);
                    load_font = ui.button("Load").clicked();
                });
                ui.small("JetBrains Mono is the default. Font files may be .ttf or .otf.");
                ui.small("Changes are saved automatically for the next launch.");
            });
        self.settings_open = open;
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
    hovered_series: Option<usize>,
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
        // Reveal notable points only while the curve is being inspected.
        // Filled grey means a true zero; an empty ring is reserved for a
        // removable discontinuity so the two cases cannot be confused.
        if hovered_series != Some(index) {
            continue;
        }
        for point in &function.critical_points {
            if !bounds.contains_x(point.x) || !bounds.contains_y(point.y) {
                continue;
            }
            let screen = egui::pos2(to_screen_x(point.x), to_screen_y(point.y));
            match point.kind {
                plot::CriticalKind::Zero => {
                    painter.circle_filled(screen, 4.5, theme.ui.muted.0);
                }
                plot::CriticalKind::Hole => {
                    painter.circle_filled(screen, 5.0, background);
                    painter.circle_stroke(screen, 5.0, egui::Stroke::new(1.8, theme.ui.muted.0));
                }
                plot::CriticalKind::Maximum | plot::CriticalKind::Minimum => {
                    painter.circle_filled(screen, 4.0, color);
                }
            }
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
                        critical_kind: None,
                    },
                ));
            }
        }
    }
    best.map(|(_, probe)| probe)
}

/// Returns the highlighted root, hole, or local extremum under the pointer.
/// Critical-point hit testing is independent of curve samples so a removable
/// discontinuity remains clickable even though the function is undefined at
/// the hole itself.
fn snap_graph_critical_point(
    pointer: egui::Pos2,
    bounds: &PlotBounds,
    rect: egui::Rect,
    functions: &[GraphFunction],
    max_distance: f32,
) -> Option<GraphProbe> {
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let to_screen = |x: f64, y: f64| {
        egui::pos2(
            rect.left() + ((x - bounds.x_min) / x_span) as f32 * rect.width(),
            rect.top() + ((bounds.y_max - y) / y_span) as f32 * rect.height(),
        )
    };
    let mut best: Option<(f32, GraphProbe)> = None;
    for (series_index, function) in functions.iter().enumerate() {
        for point in &function.critical_points {
            if !bounds.contains_x(point.x) || !bounds.contains_y(point.y) {
                continue;
            }
            let distance = pointer.distance(to_screen(point.x, point.y));
            if distance <= max_distance
                && best.map_or(true, |(best_distance, _)| distance < best_distance)
            {
                best = Some((
                    distance,
                    GraphProbe {
                        series_index,
                        x: point.x,
                        y: Some(point.y),
                        critical_kind: Some(point.kind),
                    },
                ));
            }
        }
    }
    best.map(|(_, probe)| probe)
}

/// Returns the curve under the pointer when its nearest sampled point is
/// within a small screen-space radius. This drives hover-only point markers
/// without making a distant curve appear selected.
fn hovered_graph_series(
    pointer: egui::Pos2,
    bounds: &PlotBounds,
    rect: egui::Rect,
    functions: &[GraphFunction],
    max_distance: f32,
) -> Option<usize> {
    let probe = snap_graph_probe(pointer, bounds, rect, functions)?;
    let y = probe.y?;
    let screen = egui::pos2(
        rect.left() + ((probe.x - bounds.x_min) / bounds.x_span()) as f32 * rect.width(),
        rect.top() + ((bounds.y_max - y) / bounds.y_span()) as f32 * rect.height(),
    );
    (pointer.distance(screen) <= max_distance).then_some(probe.series_index)
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
    match probe.critical_kind {
        Some(plot::CriticalKind::Zero) => {
            painter.circle_filled(screen, 5.0, theme.ui.muted.0);
        }
        Some(plot::CriticalKind::Hole) => {
            painter.circle_filled(screen, 5.5, theme.ui.editor.0);
            painter.circle_stroke(screen, 5.5, egui::Stroke::new(2.0, theme.ui.muted.0));
        }
        Some(plot::CriticalKind::Maximum) | Some(plot::CriticalKind::Minimum) | None => {
            painter.circle_filled(screen, 5.0, color);
        }
    }

    let point_name = match probe.critical_kind {
        Some(plot::CriticalKind::Zero) => "root · ",
        Some(plot::CriticalKind::Hole) => "hole · ",
        Some(plot::CriticalKind::Maximum) => "local maximum · ",
        Some(plot::CriticalKind::Minimum) => "local minimum · ",
        None => "",
    };
    let label = format!(
        "{point_name}{} = {}   y = {}",
        series.variable,
        plot::format_number(x),
        plot::format_number(y)
    );
    let label_color = match probe.critical_kind {
        Some(plot::CriticalKind::Zero) | Some(plot::CriticalKind::Hole) => theme.ui.muted.0,
        _ => color,
    };
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
    painter.rect_stroke(label_rect, 4.0, egui::Stroke::new(1.0, label_color));
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
                    RichText::new(if outcome.svg { "SVG PLOT" } else { "PLOT" })
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
            if outcome.svg {
                show_inline_svg_plot(ui, outcome);
            } else {
                ui.label(
                    RichText::new(&outcome.ascii)
                        .monospace()
                        .size(11.0)
                        .color(theme.ui.text.0),
                );
            }
        });
    ui.add_space(4.0);
}

/// Paints the SVG plot's axes and samples directly in egui so the REPL keeps
/// a crisp vector preview without depending on an external SVG converter.
fn show_inline_svg_plot(ui: &mut egui::Ui, outcome: &PlotOutcome) {
    let width = ui.available_width().clamp(240.0, 800.0);
    let height = (width * 0.65).clamp(180.0, 520.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);

    let plot_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(46.0, 14.0),
        rect.max - egui::vec2(14.0, 32.0),
    );
    let bounds = outcome.bounds;
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 {
        return;
    }
    let screen_x =
        |x: f64| plot_rect.left() + ((x - bounds.x_min) / x_span) as f32 * plot_rect.width();
    let screen_y =
        |y: f64| plot_rect.top() + ((bounds.y_max - y) / y_span) as f32 * plot_rect.height();
    let grid = egui::Color32::from_rgb(228, 231, 235);
    let axis = egui::Color32::from_rgb(87, 96, 106);
    let step_x = plot::nice_step(x_span, 9.0);
    let step_y = plot::nice_step(y_span, 7.0);
    let mut tick = (bounds.x_min / step_x).ceil() * step_x;
    while tick <= bounds.x_max + step_x * 0.001 {
        let x = screen_x(tick);
        painter.line_segment(
            [
                egui::pos2(x, plot_rect.top()),
                egui::pos2(x, plot_rect.bottom()),
            ],
            egui::Stroke::new(1.0, grid),
        );
        painter.text(
            egui::pos2(x, plot_rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            plot::format_number(tick),
            FontId::monospace(10.0),
            axis,
        );
        tick += step_x;
    }
    tick = (bounds.y_min / step_y).ceil() * step_y;
    while tick <= bounds.y_max + step_y * 0.001 {
        let y = screen_y(tick);
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), y),
                egui::pos2(plot_rect.right(), y),
            ],
            egui::Stroke::new(1.0, grid),
        );
        painter.text(
            egui::pos2(plot_rect.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            plot::format_number(tick),
            FontId::monospace(10.0),
            axis,
        );
        tick += step_y;
    }
    if bounds.contains_x(0.0) {
        let x = screen_x(0.0);
        painter.line_segment(
            [
                egui::pos2(x, plot_rect.top()),
                egui::pos2(x, plot_rect.bottom()),
            ],
            egui::Stroke::new(1.5, axis),
        );
    }
    if bounds.contains_y(0.0) {
        let y = screen_y(0.0);
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), y),
                egui::pos2(plot_rect.right(), y),
            ],
            egui::Stroke::new(1.5, axis),
        );
    }
    let curves = painter.with_clip_rect(plot_rect);
    for (index, series) in outcome.series.iter().enumerate() {
        let raw = SVG_GRAPH_COLORS[index % SVG_GRAPH_COLORS.len()];
        let color = egui::Color32::from_rgb(
            ((raw >> 16) & 0xff) as u8,
            ((raw >> 8) & 0xff) as u8,
            (raw & 0xff) as u8,
        );
        let stroke = egui::Stroke::new(2.0, color);
        let mut path = Vec::new();
        let mut previous_y: Option<f64> = None;
        for (x, y) in &series.samples {
            let Some(y) = y.filter(|y| y.is_finite()) else {
                if path.len() > 1 {
                    curves.add(egui::Shape::line(std::mem::take(&mut path), stroke));
                }
                path.clear();
                previous_y = None;
                continue;
            };
            if previous_y.is_some_and(|previous| (y - previous).abs() > y_span * 2.0) {
                if path.len() > 1 {
                    curves.add(egui::Shape::line(std::mem::take(&mut path), stroke));
                }
                path.clear();
            }
            path.push(egui::pos2(screen_x(*x), screen_y(y)));
            previous_y = Some(y);
        }
        if path.len() > 1 {
            curves.add(egui::Shape::line(path, stroke));
        }
    }
    ui.horizontal_wrapped(|ui| {
        for (index, series) in outcome.series.iter().enumerate() {
            let raw = SVG_GRAPH_COLORS[index % SVG_GRAPH_COLORS.len()];
            let color = egui::Color32::from_rgb(
                ((raw >> 16) & 0xff) as u8,
                ((raw >> 8) & 0xff) as u8,
                (raw & 0xff) as u8,
            );
            ui.label(RichText::new(&series.label).color(color).size(11.0));
        }
    });
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
        if self.startup_maximize_pending {
            context.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.startup_maximize_pending = false;
        }
        if context.input(|input| input.viewport().close_requested()) && !self.allow_close {
            if self.dirty {
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.request_action(PendingAction::Close, Some(context));
            } else {
                self.persist_preferences();
                self.allow_close = true;
            }
        }
        self.poll_run(context);
        if self.terminal.as_mut().is_some_and(TerminalSession::poll) {
            context.request_repaint();
        }
        if self.terminal.is_some()
            && self.output_open
            && self.workspace_tab == WorkspaceTab::Document
            && self.bottom_tab == BottomTab::Terminal
        {
            context.request_repaint_after(Duration::from_millis(50));
        }
        self.poll_analysis(context);
        self.poll_latex_render(context);
        self.poll_graph_sampling(context);
        self.poll_repl_math(context);
        self.settings(context);

        let terminal_focused =
            context.memory(|memory| memory.has_focus(egui::Id::new(terminal::TERMINAL_ID)));
        if !terminal_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            self.save();
        }
        if !terminal_focused
            && context.input_mut(|input| {
                input.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                )
            })
        {
            self.open_save_as();
        }
        if !terminal_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::P))
        {
            self.quick_open = true;
        }
        if !terminal_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B))
        {
            self.files_open = !self.files_open;
        }
        let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
        let editor_focused = context.memory(|memory| memory.has_focus(editor_id));
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F12))
        {
            if let Some(name) = self.symbol_at_cursor() {
                let index = SourceIndex::new(&self.source);
                if let Some(definition) = index.definition(&name) {
                    self.jump_to_byte(context, definition.span.start);
                } else {
                    self.set_status("No unambiguous definition in this document");
                }
            }
        }
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F12))
        {
            if let Some(name) = self.symbol_at_cursor() {
                if SourceIndex::new(&self.source).definition(&name).is_some() {
                    self.references_target = Some(name);
                    self.bottom_tab = BottomTab::References;
                    self.output_open = true;
                } else {
                    self.set_status("No unambiguous references in this document");
                }
            }
        }
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F2))
        {
            if let Some(name) = self.symbol_at_cursor() {
                self.rename_input.clone_from(&name);
                self.rename_target = Some(name);
            }
        }
        if !terminal_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::J))
        {
            self.output_open = !self.output_open;
        }
        if !terminal_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter))
        {
            match self.workspace_tab {
                WorkspaceTab::Document => self.run(),
                WorkspaceTab::Repl => self.submit_repl_numerically(),
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
        self.show_title_bar(context, &theme);
        self.show_save_and_close_dialogs(context);
        self.show_rename(context);

        egui::SidePanel::left("activity-bar")
            .exact_width(46.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme.activity_bar_color())
                    .inner_margin(egui::Margin::symmetric(3.0, 6.0))
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
                if activity_button(ui, ActivityIcon::Search, self.quick_open, &theme)
                    .on_hover_text("Quick Open · Ctrl+P")
                    .clicked()
                {
                    self.quick_open = true;
                }
                if activity_button(
                    ui,
                    ActivityIcon::Repl,
                    self.workspace_tab == WorkspaceTab::Repl,
                    &theme,
                )
                .on_hover_text("Interactive REPL")
                .clicked()
                {
                    self.workspace_tab = WorkspaceTab::Repl;
                }
                if activity_button(
                    ui,
                    ActivityIcon::Graphing,
                    self.workspace_tab == WorkspaceTab::Graphing,
                    &theme,
                )
                .on_hover_text("Graphing calculator")
                .clicked()
                {
                    self.workspace_tab = WorkspaceTab::Graphing;
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
                .default_width(300.0)
                .width_range(200.0..=420.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.panel.0)
                        .inner_margin(egui::Margin::symmetric(12.0, 3.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.20),
                        )),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Explorer")
                                .strong()
                                .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button_frame = ui.visuals().button_frame;
                            ui.visuals_mut().button_frame = false;
                            ui.menu_button(
                                RichText::new('\u{ea7c}').font(codicon_font(16.0)),
                                |ui| {
                                    if ui.button("New File").clicked() {
                                        self.new_file();
                                        ui.close_menu();
                                    }
                                    if ui.button("Refresh Files").clicked() {
                                        self.files = collect_tree(&self.project_root);
                                        ui.close_menu();
                                    }
                                    if ui.button("Collapse Folders").clicked() {
                                        self.collapsed_dirs.extend(
                                            self.files
                                                .iter()
                                                .filter(|entry| entry.is_dir)
                                                .map(|entry| entry.path.clone()),
                                        );
                                        ui.close_menu();
                                    }
                                },
                            );
                            ui.visuals_mut().button_frame = button_frame;
                        });
                    });
                    let root_name = self
                        .project_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("workspace")
                        .to_owned();
                    ui.add_space(1.0);
                    ui.horizontal(|ui| {
                        let disclosure = tree_disclosure(ui, self.root_collapsed, &theme);
                        let root_label = ui.add(
                            egui::Label::new(
                                RichText::new(root_name)
                                    .strong()
                                    .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if disclosure.clicked() || root_label.clicked() {
                            self.root_collapsed = !self.root_collapsed;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if chrome_button(ui, ChromeIcon::Collapse, &theme)
                                .on_hover_text("Collapse folders")
                                .clicked()
                            {
                                self.collapsed_dirs.extend(
                                    self.files
                                        .iter()
                                        .filter(|entry| entry.is_dir)
                                        .map(|entry| entry.path.clone()),
                                );
                            }
                            if chrome_button(ui, ChromeIcon::Refresh, &theme)
                                .on_hover_text("Refresh files")
                                .clicked()
                            {
                                self.files = collect_tree(&self.project_root);
                            }
                            if chrome_button(ui, ChromeIcon::NewFile, &theme)
                                .on_hover_text("New SCAS buffer")
                                .clicked()
                            {
                                self.new_file();
                                self.workspace_tab = WorkspaceTab::Document;
                            }
                        });
                    });
                    let mut selected = None;
                    let explorer_content = ui.available_rect_before_wrap();
                    let tree_height =
                        (explorer_content.height() - EXPLORER_FOOTER_HEIGHT).max(100.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), tree_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if self.root_collapsed {
                                return;
                            }
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
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
                                    let is_selected = !entry.is_dir
                                        && self.current_file.as_ref() == Some(&entry.path);
                                    let row_size = egui::vec2(
                                        ui.available_width(),
                                        ui.spacing().interact_size.y,
                                    );
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
                                        let disclosure =
                                            tree_disclosure(&mut row_ui, collapsed, &theme);
                                        paint_tree_icon(&mut row_ui, true, &theme);
                                        let name_response = row_ui.add(
                                            egui::Label::new(
                                                RichText::new(name)
                                                    .strong()
                                                    .font(FontId::proportional(INTERFACE_FONT_SIZE))
                                                    .color(theme.ui.text.0),
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
                                        let name_response =
                                            row_ui.add(
                                                egui::Label::new(RichText::new(name).font(
                                                    FontId::proportional(INTERFACE_FONT_SIZE),
                                                ))
                                                .sense(egui::Sense::click()),
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
                        },
                    );
                    if let Some(path) = selected {
                        self.open(path);
                        self.workspace_tab = WorkspaceTab::Document;
                    }
                    ui.separator();
                    egui::CollapsingHeader::new(
                        RichText::new("Outline").font(FontId::proportional(INTERFACE_FONT_SIZE)),
                    )
                    .id_source("explorer-outline")
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("SCAS document symbols")
                                .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                        );
                    });
                    egui::CollapsingHeader::new(
                        RichText::new("Timeline").font(FontId::proportional(INTERFACE_FONT_SIZE)),
                    )
                    .id_source("explorer-timeline")
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("No timeline entries")
                                .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                        );
                    });
                });
        }

        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(26.0)
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
                    let status_color = theme.contrast_on_accent();
                    let error_count = self
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
                        .count();
                    let info_count = self.diagnostics.len() - error_count;
                    let error_badge = status_badge(ui, '\u{ea87}', status_color);
                    ui.label(
                        RichText::new(error_count.to_string())
                            .size(11.0)
                            .color(status_color),
                    );
                    let info_badge = status_badge(ui, '\u{ea74}', status_color);
                    ui.label(
                        RichText::new(info_count.to_string())
                            .size(11.0)
                            .color(status_color),
                    );
                    if error_badge.clicked() || info_badge.clicked() {
                        self.workspace_tab = WorkspaceTab::Document;
                        self.output_open = true;
                        self.bottom_tab = BottomTab::Problems;
                    }
                    if ui.available_width() > 760.0 {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(if busy { "Working…" } else { &self.status })
                                .size(11.0)
                                .color(status_color),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let workspace_status = match self.workspace_tab {
                            WorkspaceTab::Document => format!(
                                "Ln {}, Col {}    Spaces: 4    UTF-8    CRLF    {{ }} SCAS",
                                self.current_line + 1,
                                self.source
                                    .chars()
                                    .take(self.cursor_char_index)
                                    .collect::<String>()
                                    .rsplit('\n')
                                    .next()
                                    .map_or(1, |line| line.chars().count() + 1)
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

        if self.workspace_tab == WorkspaceTab::Document && self.output_open {
            egui::TopBottomPanel::bottom("output")
                .default_height(260.0)
                .height_range(120.0..=650.0)
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
                                self.bottom_tab == BottomTab::Output,
                                RichText::new("OUTPUT")
                                    .strong()
                                    .font(FontId::proportional(11.0)),
                            )
                            .clicked()
                        {
                            self.bottom_tab = BottomTab::Output;
                        }
                        if ui
                            .selectable_label(
                                self.bottom_tab == BottomTab::Problems,
                                RichText::new(format!("PROBLEMS {}", self.diagnostics.len()))
                                    .strong()
                                    .font(FontId::proportional(11.0)),
                            )
                            .clicked()
                        {
                            self.bottom_tab = BottomTab::Problems;
                        }
                        if ui
                            .selectable_label(
                                self.bottom_tab == BottomTab::Terminal,
                                RichText::new("TERMINAL")
                                    .strong()
                                    .font(FontId::proportional(11.0)),
                            )
                            .clicked()
                        {
                            self.show_terminal_tab(context);
                        }
                        if self.references_target.is_some()
                            && ui
                                .selectable_label(
                                    self.bottom_tab == BottomTab::References,
                                    RichText::new("REFERENCES")
                                        .strong()
                                        .font(FontId::proportional(11.0)),
                                )
                                .clicked()
                        {
                            self.bottom_tab = BottomTab::References;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.bottom_tab == BottomTab::Terminal {
                                if ui
                                    .small_button("New")
                                    .on_hover_text("Restart shell")
                                    .clicked()
                                {
                                    self.terminal = None;
                                    self.show_terminal_tab(context);
                                }
                                if ui.small_button("Clear").clicked() {
                                    if let Some(terminal) = &mut self.terminal {
                                        terminal.clear();
                                    }
                                }
                            }
                            if self.bottom_tab == BottomTab::Output
                                && ui
                                    .small_button(
                                        RichText::new("Copy")
                                            .font(FontId::proportional(INTERFACE_FONT_SIZE)),
                                    )
                                    .clicked()
                            {
                                ui.output_mut(|clipboard| {
                                    clipboard.copied_text.clone_from(&self.output)
                                });
                                self.set_status("Output copied");
                            }
                        });
                    });
                    ui.separator();
                    if self.bottom_tab == BottomTab::Problems {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if self.diagnostics.is_empty() {
                                ui.label(
                                    RichText::new(if self.analysis_due.is_some() {
                                        "Checking the document after typing pauses…"
                                    } else {
                                        "No problems detected in this document."
                                    })
                                    .font(FontId::proportional(INTERFACE_FONT_SIZE))
                                    .color(theme.ui.muted.0),
                                );
                            } else {
                                let file_name = self
                                    .current_file
                                    .as_ref()
                                    .and_then(|path| path.file_name())
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("Untitled.scas");
                                ui.label(
                                    RichText::new(format!(
                                        "{file_name}  ({})",
                                        self.diagnostics.len()
                                    ))
                                    .strong()
                                    .size(12.0),
                                );
                                let mut selected = None;
                                for (index, diagnostic) in self.diagnostics.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let color = diagnostic_color(diagnostic.severity, &theme);
                                        ui.label(
                                            RichText::new(
                                                if diagnostic.severity == DiagnosticSeverity::Error
                                                {
                                                    '\u{ea87}'
                                                } else {
                                                    '\u{ea74}'
                                                },
                                            )
                                            .font(codicon_font(13.0))
                                            .color(color),
                                        );
                                        let location =
                                            format!("[{}:{}]", diagnostic.line, diagnostic.column);
                                        if ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(format!(
                                                        "{}  {location}",
                                                        diagnostic.message
                                                    ))
                                                    .size(12.0)
                                                    .color(theme.ui.text.0),
                                                )
                                                .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text("Go to problem in source")
                                            .clicked()
                                        {
                                            selected = Some(index);
                                        }
                                    });
                                }
                                if let Some(index) = selected {
                                    let diagnostic = self.diagnostics[index].clone();
                                    self.jump_to_diagnostic(context, &diagnostic);
                                }
                            }
                        });
                    } else if self.bottom_tab == BottomTab::References {
                        let mut selected = None;
                        if let Some(name) = &self.references_target {
                            let index = SourceIndex::new(&self.source);
                            let references = index.references(name);
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} — {} locations",
                                        name,
                                        references.len()
                                    ))
                                    .strong(),
                                );
                                for occurrence in references {
                                    let line = self
                                        .source
                                        .lines()
                                        .nth(occurrence.line.saturating_sub(1))
                                        .unwrap_or("");
                                    let label = format!(
                                        "{}:{}  {}",
                                        occurrence.line,
                                        occurrence.column,
                                        line.trim()
                                    );
                                    if ui
                                        .selectable_label(false, label)
                                        .on_hover_text("Go to source")
                                        .clicked()
                                    {
                                        selected = Some(occurrence.span.start);
                                    }
                                }
                            });
                        }
                        if let Some(byte) = selected {
                            self.jump_to_byte(context, byte);
                        }
                    } else if self.bottom_tab == BottomTab::Terminal {
                        if let Some(error) = &self.terminal_error {
                            ui.colored_label(theme.syntax.operator.0, error);
                        } else if let Some(terminal) = &mut self.terminal {
                            if let Err(error) = terminal.ui(ui, theme.ui.text.0, theme.ui.editor.0)
                            {
                                self.terminal_error =
                                    Some(format!("Terminal disconnected: {error}"));
                            }
                        }
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
            let mut selected_preview_byte = None;
            let mut preview_scroll_target = self.preview_scroll_target.take();
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
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("DOCUMENT PREVIEW")
                                .strong()
                                .font(preview_ui_font(11.0))
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
                                .font(preview_ui_font(10.5))
                                .color(self.themes[self.selected_theme].ui.accent.0),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button(
                                RichText::new("Copy TeX")
                                    .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                            )
                            .clicked()
                        {
                            ui.output_mut(|output| output.copied_text.clone_from(&self.preview));
                        }
                        ui.add_space(4.0);
                        if ui
                            .add_enabled(
                                self.document_preview_zoom > MIN_DOCUMENT_PREVIEW_ZOOM,
                                egui::Button::new(
                                    RichText::new("−")
                                        .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                                )
                                .frame(false)
                                .min_size(egui::vec2(24.0, 0.0)),
                            )
                            .on_hover_text("Zoom out of rendered document")
                            .clicked()
                        {
                            self.document_preview_zoom = adjusted_document_preview_zoom(
                                self.document_preview_zoom,
                                1.0 / DOCUMENT_PREVIEW_ZOOM_STEP,
                            );
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!(
                                        "{:.0}%",
                                        self.document_preview_zoom * 100.0
                                    ))
                                    .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Reset rendered document to 100%")
                            .clicked()
                        {
                            self.document_preview_zoom = 1.0;
                        }
                        if ui
                            .add_enabled(
                                self.document_preview_zoom < MAX_DOCUMENT_PREVIEW_ZOOM,
                                egui::Button::new(
                                    RichText::new("+")
                                        .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                                )
                                .frame(false)
                                .min_size(egui::vec2(24.0, 0.0)),
                            )
                            .on_hover_text("Zoom in to rendered document")
                            .clicked()
                        {
                            self.document_preview_zoom = adjusted_document_preview_zoom(
                                self.document_preview_zoom,
                                DOCUMENT_PREVIEW_ZOOM_STEP,
                            );
                        }
                    });
                    if let Some(error) = &self.preview_error {
                        ui.label(
                            RichText::new(format!("Editing: {error} (showing last good preview)"))
                                .font(preview_ui_font(INTERFACE_FONT_SIZE))
                                .color(self.themes[self.selected_theme].syntax.operator.0),
                        );
                    }
                    ui.separator();
                    if self
                        .preview_document
                        .as_ref()
                        .map_or(true, |document| document.fragments.is_empty())
                    {
                        ui.label(
                            RichText::new("Add a render line, for example:")
                                .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                        );
                        ui.monospace(r"\[ (x + 1) / (2x^2) \]");
                    } else {
                        let preview_width = ui.available_width();
                        let preview_view_rect = ui.available_rect_before_wrap();
                        let (gesture_zoom, wheel_zoom, zoom_pointer) = ui.input(|input| {
                            let Some(pointer) = input
                                .pointer
                                .hover_pos()
                                .filter(|position| preview_view_rect.contains(*position))
                            else {
                                return (1.0, false, None);
                            };
                            let pinch = input.zoom_delta();
                            if (pinch - 1.0).abs() > f32::EPSILON {
                                (pinch, input.raw_scroll_delta.y != 0.0, Some(pointer))
                            } else if input.raw_scroll_delta.y != 0.0 {
                                (
                                    (input.raw_scroll_delta.y / 300.0).exp(),
                                    true,
                                    Some(pointer),
                                )
                            } else {
                                (1.0, false, None)
                            }
                        });
                        if wheel_zoom {
                            // The trackpad gesture belongs to the document, not
                            // the surrounding ScrollArea's vertical scrollbar.
                            ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
                        }
                        if (gesture_zoom - 1.0).abs() > f32::EPSILON {
                            let previous_zoom = self.document_preview_zoom;
                            let next_zoom =
                                adjusted_document_preview_zoom(previous_zoom, gesture_zoom);
                            if let Some(pointer) = zoom_pointer {
                                self.document_preview_scroll_offset =
                                    zoom_scroll_offset_about_pointer(
                                        self.document_preview_scroll_offset,
                                        pointer,
                                        &self.latex_texture_rects,
                                        next_zoom / previous_zoom,
                                    );
                            }
                            self.document_preview_zoom = next_zoom;
                        }
                        if self.document_preview_zoom.to_bits()
                            != self.latex_observed_zoom.to_bits()
                        {
                            self.latex_observed_zoom = self.document_preview_zoom;
                            self.latex_zoom_due =
                                Some(Instant::now() + Duration::from_millis(250));
                        }
                        if let Some(due) = self.latex_zoom_due {
                            let now = Instant::now();
                            if now >= due {
                                if self.latex_textures.is_empty() {
                                    if self.latex_render_error.is_some() {
                                        self.latex_zoom_due = None;
                                    } else {
                                        self.latex_zoom_due =
                                            Some(now + Duration::from_millis(100));
                                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                                    }
                                } else {
                                    self.latex_zoom_due = None;
                                    // Respect the GPU texture limit while using as
                                    // much resolution as the current view permits.
                                    let largest = self
                                        .latex_textures
                                        .iter()
                                        .flat_map(|texture| texture.size())
                                        .max()
                                        .unwrap_or(1) as f32;
                                    let pixels = self
                                        .latex_textures
                                        .iter()
                                        .map(|texture| {
                                            let [width, height] = texture.size();
                                            width.saturating_mul(height)
                                        })
                                        .fold(0usize, usize::saturating_add)
                                        .max(1) as f32;
                                    let max_side = ui.input(|input| input.max_texture_side) as f32;
                                    let side_limit = max_side * 0.9 / largest;
                                    let memory_limit = (16_000_000.0 / pixels).sqrt();
                                    let supported_zoom =
                                        (side_limit.min(memory_limit) * self.latex_texture_zoom)
                                            .max(1.0);
                                    self.latex_render_zoom = self
                                        .document_preview_zoom
                                        .min(supported_zoom)
                                        .max(1.0);
                                }
                            } else {
                                ui.ctx().request_repaint_after(due.duration_since(now));
                            }
                        }
                        let preview_scale = (preview_width / 280.0).clamp(1.0, 2.8)
                            * self.document_preview_zoom;
                        let preview_zoom = self.document_preview_zoom;
                        if let Some(document) = &self.preview_document {
                            self.start_latex_render(
                                document.body(),
                                [theme.ui.text.0.r(), theme.ui.text.0.g(), theme.ui.text.0.b()],
                            );
                        }
                        let mut next_texture_rects = Vec::new();
                        let scroll_output = egui::ScrollArea::both()
                            .id_source("document-preview-scroll")
                            .scroll_offset(self.document_preview_scroll_offset)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if let Some(document) = &self.preview_document {
                                    if !self.latex_textures.is_empty() {
                                        let page_width = (preview_width - 52.0).max(180.0);
                                        let widest_chunk = self
                                            .latex_textures
                                            .iter()
                                            .map(|texture| texture.size_vec2().x)
                                            .fold(1.0, f32::max);
                                        let image_scale = (page_width / widest_chunk).min(4.5)
                                            * preview_zoom;
                                        ui.with_layout(
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                ui.add_space(10.0);
                                                ui.label(
                                                    RichText::new("Rendered document")
                                                        .strong()
                                                        .font(preview_ui_font(15.0)),
                                                );
                                                ui.label(
                                                    RichText::new(
                                                        "Live output from the current SCAS source",
                                                    )
                                                    .font(preview_ui_font(11.0))
                                                    .color(theme.ui.muted.0),
                                                );
                                                ui.add_space(8.0);
                                                egui::Frame::none()
                                                    .fill(theme.ui.editor.0)
                                                    .inner_margin(egui::Margin::same(18.0))
                                                    .show(ui, |ui| {
                                                        let chunks = document_preview_chunks(document);
                                                        for (index, texture) in self.latex_textures.iter().enumerate() {
                                                            let display_size =
                                                                texture.size_vec2() * image_scale;
                                                            let response = ui.add(
                                                                egui::Image::from_texture(texture)
                                                                    .fit_to_exact_size(
                                                                        display_size,
                                                                    )
                                                                    .sense(egui::Sense::click()),
                                                            );
                                                            next_texture_rects.push(response.rect);
                                                            if let Some((_, range)) = chunks.get(index) {
                                                                if let Some(fragment) = document.fragments.get(range.start) {
                                                                    let response = response.on_hover_text(format!("Go to source near line {}", fragment.span.line));
                                                                    if response.clicked() { selected_preview_byte = Some(fragment.span.byte_start); }
                                                                    if preview_scroll_target.is_some_and(|target| range.contains(&target)) {
                                                                        ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                                                                        preview_scroll_target = None;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    });
                                            },
                                        );
                                    } else {
                                        let selected = math_preview::show_document(
                                            ui,
                                            document,
                                            self.themes[self.selected_theme].ui.text.0,
                                            self.themes[self.selected_theme].ui.muted.0,
                                            preview_scale,
                                            preview_scroll_target.take(),
                                        );
                                        if let Some(index) = selected {
                                            selected_preview_byte = document.fragments.get(index).map(|fragment| fragment.span.byte_start);
                                        }
                                    }
                                    if self.latex_render.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(
                                                RichText::new(
                                                    "Typesetting preview…",
                                                )
                                                .font(preview_ui_font(11.0))
                                                .color(theme.ui.muted.0),
                                            );
                                        });
                                    }
                                    if let Some(error) = &self.latex_render_error {
                                        ui.label(
                                            RichText::new(format!(
                                                "{error}; showing structured fallback"
                                            ))
                                            .font(preview_ui_font(INTERFACE_FONT_SIZE))
                                            .color(
                                                self.themes[self.selected_theme].syntax.operator.0,
                                            ),
                                        );
                                    }
                                    for diagnostic in &document.diagnostics {
                                        ui.label(
                                            RichText::new(&diagnostic.message)
                                                .font(preview_ui_font(INTERFACE_FONT_SIZE))
                                                .color(
                                                    self.themes[self.selected_theme]
                                                        .syntax
                                                        .operator
                                                        .0,
                                                ),
                                        );
                                    }
                                }
                                ui.add_space(12.0);
                                egui::CollapsingHeader::new(
                                    RichText::new("LaTeX source")
                                        .font(preview_ui_font(INTERFACE_FONT_SIZE)),
                                )
                                .id_source("latex-preview-source")
                                .show(ui, |ui| {
                                    ui.add(
                                        TextEdit::multiline(&mut self.preview)
                                            .font(TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                            });
                        self.document_preview_scroll_offset = scroll_output.state.offset;
                        self.latex_texture_rects = next_texture_rects;
                    }
                });
            self.preview_scroll_target = preview_scroll_target;
            if let Some(byte) = selected_preview_byte {
                self.jump_to_byte(context, byte);
            }
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
                let compact_document_title =
                    compact_tab_title(&document_title, MAX_DOCUMENT_TAB_CHARS);
                egui::Frame::none().fill(theme.ui.panel.0).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let visible_tabs = if self.workspace_tab == WorkspaceTab::Document {
                            vec![WorkspaceTab::Document]
                        } else {
                            vec![WorkspaceTab::Document, self.workspace_tab]
                        };
                        for tab in visible_tabs {
                            let active = self.workspace_tab == tab;
                            let label = match tab {
                                WorkspaceTab::Document => compact_document_title.as_str(),
                                WorkspaceTab::Repl => "REPL",
                                WorkspaceTab::Graphing => "Graphing",
                            };
                            egui::Frame::none()
                                .fill(if active {
                                    theme.ui.editor.0
                                } else {
                                    theme.ui.panel.0
                                })
                                .inner_margin(egui::Margin::symmetric(10.0, 2.0))
                                .show(ui, |ui| {
                                    ui.horizontal_centered(|ui| {
                                        ui.set_min_height(30.0);
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        ui.label(RichText::new("SC").strong().size(12.0).color(
                                            if active {
                                                theme.ui.accent.0
                                            } else {
                                                theme.ui.muted.0
                                            },
                                        ));
                                        if ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(label).size(12.0).color(
                                                        if active {
                                                            theme.ui.text.0
                                                        } else {
                                                            theme.ui.muted.0
                                                        },
                                                    ),
                                                )
                                                .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text(if tab == WorkspaceTab::Document {
                                                &document_title
                                            } else {
                                                label
                                            })
                                            .clicked()
                                        {
                                            self.workspace_tab = tab;
                                        }
                                        if chrome_button(ui, ChromeIcon::TabClose, &theme)
                                            .on_hover_text("Close tab")
                                            .clicked()
                                        {
                                            if tab == WorkspaceTab::Document {
                                                self.new_file();
                                            } else {
                                                self.workspace_tab = WorkspaceTab::Document;
                                            }
                                        }
                                    });
                                });
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button(
                                RichText::new('\u{ea7c}').font(codicon_font(16.0)),
                                |ui| {
                                    if ui.button("Show Explorer").clicked() {
                                        self.files_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Show Preview").clicked() {
                                        self.preview_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Show Output").clicked() {
                                        self.output_open = true;
                                        ui.close_menu();
                                    }
                                },
                            );
                            if chrome_button(ui, ChromeIcon::Preview, &theme)
                                .on_hover_text("Toggle mathematical preview")
                                .clicked()
                            {
                                self.preview_open = !self.preview_open;
                            }
                            if chrome_button(ui, ChromeIcon::Run, &theme)
                                .on_hover_text("Run active workspace · Ctrl+Enter")
                                .clicked()
                            {
                                self.run_active();
                            }
                            if self.workspace_tab == WorkspaceTab::Document {
                                ui.menu_button("Ω", |ui| {
                                    for (symbol, insertion) in
                                        [("π", "π"), ("θ", "θ"), ("λ", "λ"), ("∫", "integral(")]
                                    {
                                        if ui.button(symbol).clicked() {
                                            self.source.push_str(insertion);
                                            self.dirty = true;
                                            self.schedule_analysis(context);
                                            ui.close_menu();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("Insert a mathematical symbol");
                                ui.menu_button("{}", |ui| {
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
                            }
                        });
                    });
                });
                let (crumb_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 23.0),
                    egui::Sense::hover(),
                );
                let mut crumb_ui = ui.child_ui(
                    crumb_rect,
                    egui::Layout::left_to_right(egui::Align::Center),
                    None,
                );
                crumb_ui.add_space(18.0);
                let location = self
                    .current_file
                    .as_ref()
                    .and_then(|path| path.strip_prefix(&self.project_root).ok())
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Untitled".to_owned());
                crumb_ui.label(
                    RichText::new("SC")
                        .strong()
                        .size(10.0)
                        .color(theme.ui.accent.0),
                );
                crumb_ui.label(RichText::new(location).size(12.0).color(theme.ui.muted.0));
                crumb_ui.label(RichText::new("› …").size(12.0).color(theme.ui.muted.0));
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
                let editor_focused = context.memory(|memory| memory.has_focus(editor_id));
                // Completion scans are only useful while the editor has
                // focus; skip them while the REPL or graphing tab is active.
                let matches = if editor_focused {
                    editor_assist::completions(&self.source, self.cursor_char_index)
                } else {
                    editor_assist::Matches::empty(self.cursor_char_index)
                };
                let (user_start, user_matches) = if editor_focused && matches.len == 0 {
                    editor_assist::user_function_completions(&self.source, self.cursor_char_index)
                } else {
                    (self.cursor_char_index, Vec::new())
                };
                let tab_accept = editor_focused
                    && (matches.len > 0 || !user_matches.is_empty())
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
                } else if tab_accept {
                    if let Some((name, _)) = user_matches.first() {
                        self.insert_editor_template(
                            context,
                            editor_id,
                            user_start,
                            self.cursor_char_index,
                            &format!("{name}($0)"),
                        );
                    }
                }
                let font_id = FontId::monospace(self.editor_font_size);
                let cache_is_current = self.highlight_cache.as_ref().is_some_and(|cache| {
                    cache.source == self.source
                        && cache.theme == self.selected_theme
                        && cache.font_size_bits == self.editor_font_size.to_bits()
                        && cache.diagnostics_revision == self.diagnostics_revision
                });
                if !cache_is_current {
                    self.highlight_cache = Some(HighlightCache {
                        source: self.source.clone(),
                        theme: self.selected_theme,
                        font_size_bits: self.editor_font_size.to_bits(),
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
                let gutter_width =
                    editor_gutter_width(total_lines, self.editor_font_size, self.line_numbers);
                if TextEdit::load_state(context, editor_id).is_none() {
                    let mut state = egui::text_edit::TextEditState::default();
                    state.set_undoer(editor_undoer(&self.source));
                    TextEdit::store_state(context, editor_id, state);
                }
                prepare_editor_line_cut(context, editor_id, &self.source, editor_focused);
                let scroll_output = egui::ScrollArea::vertical()
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
                    });
                let editor_viewport = scroll_output.inner_rect;
                let output = scroll_output.inner;
                if let Some(cursor) = output.cursor_range {
                    self.cursor_char_index = cursor.primary.ccursor.index;
                    if output.response.has_focus()
                        && self.cursor_char_index != self.preview_last_cursor
                    {
                        self.preview_last_cursor = self.cursor_char_index;
                        let byte = char_to_byte(&self.source, self.cursor_char_index);
                        if let Some(document) = &self.preview_document {
                            self.preview_scroll_target =
                                document.fragments.iter().position(|fragment| {
                                    fragment.span.byte_start <= byte
                                        && byte < fragment.span.byte_end
                                });
                        }
                    }
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
                let (user_popup_start, user_popup_matches) = if output.response.has_focus()
                    && popup_matches.len == 0
                {
                    editor_assist::user_function_completions(&self.source, self.cursor_char_index)
                } else {
                    (self.cursor_char_index, Vec::new())
                };
                let mut popup_completion = None;
                let mut popup_user_completion = None;
                if output.response.has_focus()
                    && (popup_matches.len > 0 || !user_popup_matches.is_empty())
                {
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
                                    for (index, (name, detail)) in
                                        user_popup_matches.iter().enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            let label = if index == 0 {
                                                format!("{name}   Tab")
                                            } else {
                                                name.clone()
                                            };
                                            if ui.selectable_label(false, label).clicked() {
                                                popup_user_completion = Some(name.clone());
                                            }
                                            ui.label(RichText::new(detail).size(11.0).color(
                                                self.themes[self.selected_theme].ui.muted.0,
                                            ));
                                        });
                                    }
                                });
                            });
                    }
                }
                if output.response.has_focus()
                    && popup_matches.len == 0
                    && user_popup_matches.is_empty()
                {
                    let byte = char_to_byte(&self.source, self.cursor_char_index);
                    if let Some(call) = SourceIndex::new(&self.source).call_at(&self.source, byte) {
                        if !call.parameters.is_empty() {
                            if let Some(cursor) = output.cursor_range {
                                let caret = output.galley.pos_from_cursor(&cursor.primary);
                                let anchor = output.galley_pos
                                    + egui::vec2(caret.left(), caret.bottom() + 4.0);
                                egui::Area::new(egui::Id::new("editor-signature-popup"))
                                    .order(egui::Order::Foreground)
                                    .fixed_pos(anchor)
                                    .constrain(true)
                                    .show(context, |ui| {
                                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.monospace(format!("{}(", call.name));
                                                for (index, parameter) in
                                                    call.parameters.iter().enumerate()
                                                {
                                                    if index > 0 {
                                                        ui.monospace(", ");
                                                    }
                                                    let text = RichText::new(parameter).monospace();
                                                    ui.label(if index == call.active_parameter {
                                                        text.strong().color(theme.ui.accent.0)
                                                    } else {
                                                        text
                                                    });
                                                }
                                                ui.monospace(")");
                                            });
                                        });
                                    });
                            }
                        }
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
                        editor_viewport,
                    );
                }
                paint_inline_diagnostics(
                    ui,
                    &output,
                    &self.diagnostics,
                    &self.themes[self.selected_theme],
                    gutter_width,
                    editor_viewport,
                );
                if let Some(pointer) = output.response.hover_pos() {
                    let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
                    let byte = char_to_byte(&self.source, cursor.ccursor.index);
                    if let Some(diagnostic) = self
                        .diagnostics
                        .iter()
                        .find(|item| item.byte_start <= byte && byte < item.byte_end)
                    {
                        output.response.clone().on_hover_text(&diagnostic.message);
                    } else {
                        let key = (self.analysis_generation, byte);
                        if self.hover_key != Some(key) {
                            self.hover_key = Some(key);
                            self.hover_since = Instant::now();
                            self.hover_value = None;
                        } else if self.hover_since.elapsed() >= Duration::from_millis(450) {
                            if self.hover_value.is_none() {
                                self.hover_value = Some(
                                    evaluate_hover(&self.source, byte)
                                        .map(|(_, value)| value)
                                        .unwrap_or_default(),
                                );
                            }
                            if let Some(value) = &self.hover_value {
                                if !value.is_empty() {
                                    output.response.clone().on_hover_text(format!("= {value}"));
                                }
                            }
                        } else {
                            context.request_repaint_after(Duration::from_millis(460));
                        }
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
                if let Some(name) = popup_user_completion {
                    self.insert_editor_template(
                        context,
                        editor_id,
                        user_popup_start,
                        self.cursor_char_index,
                        &format!("{name}($0)"),
                    );
                }
            });
        self.show_quick_open(context);
        self.queue_preferences_save(context);
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

fn paint_scas_logo(ui: &mut egui::Ui, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(27.0, 27.0), egui::Sense::drag());
    let c = rect.center();
    let stroke = egui::Stroke::new(2.4, theme.ui.accent.0);
    for (a, b) in [
        (egui::vec2(-8.0, -7.0), egui::vec2(7.0, -7.0)),
        (egui::vec2(-8.0, -7.0), egui::vec2(-1.0, 0.0)),
        (egui::vec2(-1.0, 0.0), egui::vec2(-8.0, 7.0)),
        (egui::vec2(-8.0, 7.0), egui::vec2(7.0, 7.0)),
    ] {
        ui.painter().line_segment([c + a, c + b], stroke);
    }
    ui.painter()
        .circle_filled(c + egui::vec2(8.0, -7.0), 1.8, theme.ui.accent.0);
    ui.painter()
        .circle_filled(c + egui::vec2(8.0, 7.0), 1.8, theme.ui.accent.0);
    response
}

fn status_badge(ui: &mut egui::Ui, glyph: char, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    paint_codicon(ui, rect.center(), glyph, 14.0, color);
    response.on_hover_text("Show Problems")
}
fn codicon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(CODICON_FONT_NAME.into()))
}

fn paint_codicon(ui: &egui::Ui, center: egui::Pos2, glyph: char, size: f32, color: egui::Color32) {
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        glyph,
        codicon_font(size),
        color,
    );
}

fn chrome_icon_glyph(icon: ChromeIcon) -> char {
    match icon {
        ChromeIcon::Sidebar => '\u{ebf3}',  // layout-sidebar-left
        ChromeIcon::Panel => '\u{ebf2}',    // layout-panel
        ChromeIcon::Preview => '\u{ebf4}',  // layout-sidebar-right
        ChromeIcon::Minimize => '\u{eaba}', // chrome-minimize
        ChromeIcon::Maximize => '\u{eab9}', // chrome-maximize
        ChromeIcon::Close => '\u{eab8}',    // chrome-close
        ChromeIcon::Run => '\u{eb2c}',      // play
        ChromeIcon::NewFile => '\u{ea7f}',  // file-add
        ChromeIcon::Refresh => '\u{eb37}',  // refresh
        ChromeIcon::Collapse => '\u{eac5}', // collapse-all
        ChromeIcon::TabClose => '\u{ea76}', // close
    }
}

fn activity_icon_glyph(icon: ActivityIcon) -> char {
    match icon {
        ActivityIcon::Explorer => '\u{eaf0}', // files
        ActivityIcon::Search => '\u{ea6d}',   // search
        ActivityIcon::Repl => '\u{ea85}',     // terminal
        ActivityIcon::Graphing => '\u{ebe2}', // graph-line
        ActivityIcon::Settings => '\u{eb51}', // settings-gear
    }
}

fn chrome_button(ui: &mut egui::Ui, icon: ChromeIcon, theme: &Theme) -> egui::Response {
    let width = match icon {
        ChromeIcon::Minimize | ChromeIcon::Maximize | ChromeIcon::Close => 29.0,
        ChromeIcon::TabClose => 20.0,
        _ => 27.0,
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 27.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            2.0,
            if matches!(icon, ChromeIcon::Close) {
                egui::Color32::from_rgb(190, 53, 53)
            } else {
                theme.ui.muted.0.gamma_multiply(0.20)
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    let color = if matches!(icon, ChromeIcon::Close) && response.hovered() {
        egui::Color32::WHITE
    } else {
        theme.ui.text.0
    };
    paint_codicon(ui, rect.center(), chrome_icon_glyph(icon), 16.0, color);
    response
}

fn activity_button(
    ui: &mut egui::Ui,
    icon: ActivityIcon,
    selected: bool,
    theme: &Theme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 38.0), egui::Sense::click());
    let fill = if selected {
        theme.title_bar_color()
    } else if response.hovered() {
        theme.ui.panel.0
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 4.0, fill);
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
    paint_codicon(ui, rect.center(), activity_icon_glyph(icon), 22.0, color);
    response
}

fn tree_disclosure(ui: &mut egui::Ui, collapsed: bool, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::click());
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    let glyph = if collapsed {
        '\u{eab6}' // chevron-right
    } else {
        '\u{eab4}' // chevron-down
    };
    paint_codicon(
        ui,
        rect.center(),
        glyph,
        14.0,
        if response.hovered() {
            theme.ui.text.0
        } else {
            theme.ui.muted.0
        },
    );
    response
}

fn paint_tree_icon(ui: &mut egui::Ui, directory: bool, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
    let glyph = if directory { '\u{ea83}' } else { '\u{ea7b}' };
    paint_codicon(ui, rect.center(), glyph, 15.0, theme.ui.muted.0);
}

fn editor_gutter_width(total_lines: usize, font_size: f32, mode: LineNumberMode) -> f32 {
    if mode == LineNumberMode::Off {
        20.0
    } else {
        let digits = total_lines.max(1).to_string().len() as f32;
        34.0 + digits * font_size * 0.48
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
    editor_viewport: egui::Rect,
) {
    // TextEdit clips its galley to the editable text area. The gutter occupies
    // the left margin, so explicitly extend the paint clip across that margin.
    let gutter_left = output.galley_pos.x - gutter_width;
    let editor_clip = egui::Rect::from_min_max(
        egui::pos2(gutter_left, output.response.rect.top()),
        output.response.rect.right_bottom(),
    )
    .intersect(editor_viewport)
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(editor_clip);
    let separator_x = output.galley_pos.x - 9.0;
    let gutter_rect = egui::Rect::from_min_max(
        egui::pos2(gutter_left, output.response.rect.top()),
        egui::pos2(separator_x, output.response.rect.bottom()),
    );
    painter.rect_filled(gutter_rect, 0.0, theme.ui.editor.0);
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
            painter.rect_stroke(
                row_rect,
                0.0,
                egui::Stroke::new(1.0, theme.ui.muted.0.gamma_multiply(0.10)),
            );
        }
        let value = first_visual_row
            .then(|| displayed_line_number(mode, source_line, current_line))
            .flatten();
        if let Some(value) = value {
            let position = egui::pos2(
                separator_x - 12.0,
                output.galley_pos.y + galley_row.rect.center().y,
            );
            painter.text(
                position,
                egui::Align2::RIGHT_CENTER,
                value.to_string(),
                FontId::monospace(font_size * 0.78),
                if active {
                    theme.ui.accent.0
                } else {
                    theme.ui.accent.0.gamma_multiply(0.72)
                },
            );
        }
        first_visual_row = galley_row.ends_with_newline;
        if galley_row.ends_with_newline {
            source_line += 1;
        }
    }
}

fn diagnostic_color(severity: DiagnosticSeverity, theme: &Theme) -> egui::Color32 {
    match severity {
        DiagnosticSeverity::Error => {
            if theme.dark {
                egui::Color32::from_rgb(244, 77, 81)
            } else {
                egui::Color32::from_rgb(180, 38, 43)
            }
        }
        DiagnosticSeverity::Info => theme.ui.accent.0,
    }
}

fn paint_inline_diagnostics(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    diagnostics: &[EditorDiagnostic],
    theme: &Theme,
    gutter_width: f32,
    editor_viewport: egui::Rect,
) {
    if diagnostics.is_empty() {
        return;
    }
    let clip = egui::Rect::from_min_max(
        egui::pos2(
            output.galley_pos.x - gutter_width,
            output.response.rect.top(),
        ),
        output.response.rect.right_bottom(),
    )
    .intersect(editor_viewport)
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let mut source_line = 1usize;
    let mut first_visual_row = true;
    for row in &output.galley.rows {
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.line == source_line);
        if let Some(diagnostic) = diagnostic {
            let color = diagnostic_color(diagnostic.severity, theme);
            let row_rect = egui::Rect::from_min_max(
                egui::pos2(
                    output.galley_pos.x - 2.0,
                    output.galley_pos.y + row.rect.top(),
                ),
                egui::pos2(
                    output.text_clip_rect.right(),
                    output.galley_pos.y + row.rect.bottom(),
                ),
            );
            painter.rect_filled(row_rect, 0.0, color.gamma_multiply(0.07));
        }
        if first_visual_row {
            if let Some(diagnostic) = diagnostic {
                let color = diagnostic_color(diagnostic.severity, theme);
                let y = output.galley_pos.y + row.rect.center().y;
                let x = output.galley_pos.x - gutter_width + 8.0;
                painter.text(
                    egui::pos2(x, y),
                    egui::Align2::CENTER_CENTER,
                    if diagnostic.severity == DiagnosticSeverity::Error {
                        '\u{ea87}'
                    } else {
                        '\u{ea74}'
                    },
                    codicon_font(12.0),
                    color,
                );
                let text_x = output.galley_pos.x + row.rect.right() + 16.0;
                let right = output.text_clip_rect.right() - 8.0;
                if right - text_x >= 90.0 {
                    let message = diagnostic.message.lines().next().unwrap_or("");
                    let max_chars = ((right - text_x) / 7.2).floor().max(1.0) as usize;
                    let display = if message.chars().count() > max_chars {
                        format!(
                            "{}…",
                            message
                                .chars()
                                .take(max_chars.saturating_sub(1))
                                .collect::<String>()
                        )
                    } else {
                        message.to_owned()
                    };
                    let rect = egui::Rect::from_min_max(
                        egui::pos2(text_x - 5.0, output.galley_pos.y + row.rect.top() + 1.0),
                        egui::pos2(right, output.galley_pos.y + row.rect.bottom() - 1.0),
                    );
                    painter.rect_filled(rect, 1.0, color.gamma_multiply(0.13));
                    painter.text(
                        egui::pos2(text_x, y),
                        egui::Align2::LEFT_CENTER,
                        display,
                        FontId::proportional(12.0),
                        color,
                    );
                }
            }
        }
        first_visual_row = row.ends_with_newline;
        if row.ends_with_newline {
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
    // Analysis runs asynchronously, so diagnostics can briefly belong to the
    // preceding buffer revision. Snap their offsets to the current UTF-8 text
    // before turning them into LayoutJob section boundaries; egui requires
    // every boundary to fall between complete code points.
    let diagnostic_ranges = diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let start = previous_char_boundary(source, diagnostic.byte_start);
            let end = next_char_boundary(source, diagnostic.byte_end);
            (start < end).then_some((start..end, diagnostic.severity))
        })
        .collect::<Vec<_>>();
    let original_sections = std::mem::take(&mut job.sections);
    for section in original_sections {
        let mut boundaries = vec![section.byte_range.start, section.byte_range.end];
        for (diagnostic, _) in &diagnostic_ranges {
            if section.byte_range.start < diagnostic.start
                && diagnostic.start < section.byte_range.end
            {
                boundaries.push(diagnostic.start);
            }
            if section.byte_range.start < diagnostic.end && diagnostic.end < section.byte_range.end
            {
                boundaries.push(diagnostic.end);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        for range in boundaries.windows(2) {
            let mut part = section.clone();
            part.byte_range = range[0]..range[1];
            if let Some((_, severity)) = diagnostic_ranges.iter().find(|(diagnostic, _)| {
                part.byte_range.start < diagnostic.end && diagnostic.start < part.byte_range.end
            }) {
                let color = diagnostic_color(*severity, theme);
                part.format.underline = egui::Stroke::new(1.5, color);
                part.format.background = color.gamma_multiply(0.13);
            }
            job.sections.push(part);
        }
    }
    job
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
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
            severity: DiagnosticSeverity::Error,
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
                severity: DiagnosticSeverity::Error,
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

fn line_cut_range(source: &str, cursor_index: usize) -> egui::text::CCursorRange {
    let chars: Vec<char> = source.chars().collect();
    let cursor = cursor_index.min(chars.len());
    let start = chars[..cursor]
        .iter()
        .rposition(|character| *character == '\n')
        .map_or(0, |index| index + 1);
    let end = chars[cursor..]
        .iter()
        .position(|character| *character == '\n')
        .map_or(chars.len(), |offset| cursor + offset + 1);
    let (start, end) = if start == end && start > 0 {
        (start - 1, end)
    } else {
        (start, end)
    };
    egui::text::CCursorRange::two(
        egui::text::CCursor::new(start),
        egui::text::CCursor::new(end),
    )
}

fn prepare_editor_line_cut(
    context: &egui::Context,
    editor_id: egui::Id,
    source: &str,
    editor_focused: bool,
) {
    if !editor_focused
        || !context.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Cut))
        })
    {
        return;
    }
    if let Some(mut state) = TextEdit::load_state(context, editor_id) {
        let cursor_index = match state.cursor.char_range() {
            Some(range) if range.primary.index != range.secondary.index => return,
            Some(range) => range.primary.index,
            None => 0,
        };
        state
            .cursor
            .set_char_range(Some(line_cut_range(source, cursor_index)));
        TextEdit::store_state(context, editor_id, state);
    }
}

fn compact_tab_title(title: &str, max_chars: usize) -> String {
    if title.chars().count() <= max_chars {
        return title.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }

    let (name, dirty_suffix) = title
        .strip_suffix(" •")
        .map_or((title, ""), |name| (name, " •"));
    let dirty_chars = dirty_suffix.chars().count();
    let name_limit = max_chars.saturating_sub(dirty_chars);
    if name_limit <= 1 {
        return format!(
            "{}{}",
            "…".chars().take(name_limit).collect::<String>(),
            dirty_suffix
        );
    }

    let extension = name
        .rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
        .map(|(_, extension)| format!(".{extension}"));
    if let Some(extension) = extension {
        let extension_chars = extension.chars().count();
        if extension_chars + 1 < name_limit {
            let prefix_chars = name_limit - extension_chars - 1;
            let prefix = name.chars().take(prefix_chars).collect::<String>();
            return format!("{prefix}…{extension}{dirty_suffix}");
        }
    }

    let prefix = name.chars().take(name_limit - 1).collect::<String>();
    format!("{prefix}…{dirty_suffix}")
}

fn quick_open_clicked_outside(
    window_rect: egui::Rect,
    click_position: Option<egui::Pos2>,
    ready_to_dismiss: bool,
) -> bool {
    ready_to_dismiss && click_position.is_some_and(|position| !window_rect.contains(position))
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
        Let | Fn | If | Else | For | While | Break | Continue | Return | Print | Import | And
        | Or | Xor | True | False | Nil | Exit | Help | Clear => Highlight::Keyword,
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

/// Split generated documents at complete fragment and alignment boundaries.
/// Result-heavy `=>` blocks can be far larger than their source; bounded,
/// stable chunks avoid pathological TeX pages and are reusable after edits.
fn latex_preview_chunks(body: &str) -> Vec<String> {
    latex_preview_chunks_tagged(body, &[])
        .into_iter()
        .map(|(chunk, _)| chunk)
        .collect()
}

fn document_preview_chunks(document: &Document) -> Vec<(String, std::ops::Range<usize>)> {
    let mut starts = Vec::with_capacity(document.fragments.len());
    let mut offset = 0;
    for fragment in &document.fragments {
        starts.push(offset);
        offset += fragment.latex.len() + 2;
    }
    latex_preview_chunks_tagged(&document.body(), &starts)
}

fn latex_preview_chunks_tagged(
    body: &str,
    fragment_starts: &[usize],
) -> Vec<(String, std::ops::Range<usize>)> {
    const MAX_ROWS: usize = 32;
    const MAX_BYTES: usize = 12_000;

    let mut fragments: Vec<(String, bool, usize)> = Vec::new();
    let mut offset = 0;
    for raw in body.split("\n\n") {
        let fragment_index = fragment_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        offset += raw.len() + 2;
        if raw.trim().is_empty() {
            continue;
        }
        let fragment = raw;
        let fragment = fragment.trim();
        let parts = [
            (r"\begin{align*}", r"\end{align*}"),
            (r"\[\begin{aligned}", r"\end{aligned}\]"),
        ]
        .into_iter()
        .find_map(|(begin, end)| {
            fragment
                .strip_prefix(begin)
                .and_then(|rows| rows.strip_suffix(end))
                .map(|rows| (begin, rows, end))
        })
        .map_or_else(
            || vec![(fragment.to_owned(), false)],
            |(begin, rows, end)| {
                let alignment_rows = split_top_level_alignment_rows(rows);
                let total_rows = alignment_rows.len();
                if total_rows <= MAX_ROWS && fragment.len() <= MAX_BYTES {
                    return vec![(fragment.to_owned(), false)];
                }
                let mut parts = Vec::new();
                let mut part = String::new();
                let mut row_count = 0;
                for row in alignment_rows {
                    if !part.is_empty()
                        && (row_count >= MAX_ROWS || part.len() + row.len() > MAX_BYTES)
                    {
                        parts.push((format!("{begin}\n{}{end}", part.trim()), true));
                        part.clear();
                        row_count = 0;
                    }
                    part.push_str(row);
                    row_count += 1;
                }
                if !part.trim().is_empty() {
                    parts.push((format!("{begin}\n{}{end}", part.trim()), true));
                }
                parts
            },
        );

        fragments.extend(
            parts
                .into_iter()
                .map(|(text, isolated)| (text, isolated, fragment_index)),
        );
    }
    if fragments.is_empty() {
        return vec![(body.to_owned(), 0..fragment_starts.len().max(1))];
    }
    // Count-based grouping stays stable when one result changes length, which
    // is what makes unchanged chunks reusable. The largest fragment selects a
    // safe group size for the whole revision.
    let largest = fragments
        .iter()
        .filter(|(_, isolated, _)| !isolated)
        .map(|(fragment, _, _)| fragment.len())
        .max()
        .unwrap_or(1);
    let fragments_per_chunk = match largest {
        0..=512 => 64,
        513..=2_048 => 16,
        2_049..=8_192 => 4,
        _ => 1,
    };
    let mut chunks = Vec::new();
    let mut grouped = Vec::new();
    for (fragment, isolated, index) in fragments {
        if isolated {
            if !grouped.is_empty() {
                chunks.push(group_preview_fragments(&grouped));
                grouped.clear();
            }
            chunks.push((fragment, index..index + 1));
        } else {
            grouped.push((fragment, index));
            if grouped.len() >= fragments_per_chunk {
                chunks.push(group_preview_fragments(&grouped));
                grouped.clear();
            }
        }
    }
    if !grouped.is_empty() {
        chunks.push(group_preview_fragments(&grouped));
    }
    chunks
}

fn group_preview_fragments(parts: &[(String, usize)]) -> (String, std::ops::Range<usize>) {
    let start = parts.first().map_or(0, |(_, index)| *index);
    let end = parts.last().map_or(start, |(_, index)| index + 1);
    (
        parts
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
        start..end,
    )
}

/// Split an alignment at row separators belonging to its outer environment.
/// A rendered derivation is itself an `aligned` environment and may contain
/// many `\\` rows; those must stay together when the preview is paginated.
fn split_top_level_alignment_rows(rows: &str) -> Vec<&str> {
    const BEGIN: &str = r"\begin{aligned}";
    const END: &str = r"\end{aligned}";
    const SEPARATOR: &str = "\\\\\n";

    let mut result = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let mut nested_depth = 0_u32;
    while cursor < rows.len() {
        let rest = &rows[cursor..];
        if rest.starts_with(BEGIN) {
            nested_depth += 1;
            cursor += BEGIN.len();
        } else if rest.starts_with(END) {
            nested_depth = nested_depth.saturating_sub(1);
            cursor += END.len();
        } else if nested_depth == 0 && rest.starts_with(SEPARATOR) {
            cursor += SEPARATOR.len();
            result.push(&rows[start..cursor]);
            start = cursor;
        } else {
            cursor += rest
                .chars()
                .next()
                .expect("cursor remains on a character boundary")
                .len_utf8();
        }
    }
    if start < rows.len() {
        result.push(&rows[start..]);
    }
    result
}

fn latex_render_cache_key(body: &str, color: [u8; 3], zoom: f32) -> u64 {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    color.hash(&mut hasher);
    zoom.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn render_latex_png_cached(
    body: &str,
    color: [u8; 3],
    zoom: f32,
) -> Result<Arc<LatexRender>, String> {
    const MAX_CACHE_ENTRIES: usize = 32;
    const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
    let key = latex_render_cache_key(body, color, zoom);
    let cache = LATEX_RENDER_CACHE.get_or_init(|| Mutex::new(LatexRenderCache::default()));
    {
        let cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_body, cached_color, cached_zoom, render)) = cache.entries.get(&key) {
            if cached_body == body && *cached_color == color && *cached_zoom == zoom.to_bits() {
                return Ok(Arc::clone(render));
            }
        }
    }

    let render = Arc::new(render_latex_png(body, color, zoom)?);
    let render_bytes = render.rgba.len();
    if render_bytes > MAX_CACHE_BYTES {
        return Ok(render);
    }
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !cache.entries.contains_key(&key) {
        while cache.entries.len() >= MAX_CACHE_ENTRIES
            || cache.bytes.saturating_add(render_bytes) > MAX_CACHE_BYTES
        {
            if let Some(oldest) = cache.order.pop_front() {
                if let Some((_, _, _, removed)) = cache.entries.remove(&oldest) {
                    cache.bytes = cache.bytes.saturating_sub(removed.rgba.len());
                }
            } else {
                break;
            }
        }
        cache.order.push_back(key);
    } else if let Some((_, _, _, previous)) = cache.entries.get(&key) {
        cache.bytes = cache.bytes.saturating_sub(previous.rgba.len());
    }
    cache.bytes += render_bytes;
    cache.entries.insert(
        key,
        (body.to_owned(), color, zoom.to_bits(), Arc::clone(&render)),
    );
    Ok(render)
}

fn render_repl_math_png(latex: &str, color: [u8; 3]) -> Result<LatexRender, String> {
    render_latex_png(&format!("${latex}$"), color, 1.0)
}

fn render_latex_document_pngs(
    body: &str,
    color: [u8; 3],
    zoom: f32,
) -> Result<Vec<Arc<LatexRender>>, String> {
    latex_preview_chunks(body)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            render_latex_png_cached(&chunk, color, zoom)
                .map_err(|error| format!("LaTeX preview chunk {}: {error}", index + 1))
        })
        .collect()
}

fn render_latex_png(body: &str, color: [u8; 3], zoom: f32) -> Result<LatexRender, String> {
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
         \\DeclareUnicodeCharacter{{039B}}{{\\ensuremath{{\\Lambda}}}}\n\
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
            let log = String::from_utf8_lossy(&latex.stdout);
            let detail = log
                .lines()
                .find(|line| line.starts_with('!'))
                .unwrap_or("unknown TeX error");
            return Err(format!(
                "LaTeX could not typeset the current document: {detail}"
            ));
        }
        let vector = Command::new("dvisvgm")
            .args([
                "--no-fonts",
                "--bbox=preview",
                "--output=preview.svg",
                "preview.dvi",
            ])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start dvisvgm: {error}"))?;
        if !vector.status.success() {
            return Err("dvisvgm could not convert the LaTeX preview to SVG".to_owned());
        }
        let raster = Command::new("rsvg-convert")
            .args([
                "--zoom",
                &format!("{}", 2.5 * zoom),
                "--output",
                "preview.png",
                "preview.svg",
            ])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start rsvg-convert: {error}"))?;
        if !raster.status.success() {
            return Err("rsvg-convert could not rasterize the SVG preview".to_owned());
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
            .with_min_inner_size([760.0, 480.0])
            .with_decorations(false)
            .with_maximized(true),
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
        assert_eq!(max_width, 1100.0);
        assert!(default_width <= max_width);
        assert!(2048.0 - 48.0 - 480.0 - max_width >= MIN_EDITOR_WORKSPACE_WIDTH);

        let (_, max_without_explorer) = preview_widths(2048.0, 0.0);
        assert_eq!(max_without_explorer, 1580.0);

        let (narrow_default, narrow_max) = preview_widths(1024.0, 240.0);
        assert_eq!(narrow_default, narrow_max);
        assert_eq!(narrow_max, 316.0);
        assert!(1024.0 - 48.0 - 240.0 - narrow_max >= MIN_EDITOR_WORKSPACE_WIDTH);
    }

    #[test]
    fn document_preview_zoom_steps_and_stays_bounded() {
        assert_eq!(adjusted_document_preview_zoom(1.0, 1.25), 1.25);
        assert_eq!(adjusted_document_preview_zoom(1.25, 0.8), 1.0);
        assert_eq!(
            adjusted_document_preview_zoom(1.0, 0.01),
            MIN_DOCUMENT_PREVIEW_ZOOM
        );
        assert_eq!(
            adjusted_document_preview_zoom(1.0, 100.0),
            MAX_DOCUMENT_PREVIEW_ZOOM
        );
    }

    #[test]
    fn document_preview_zoom_keeps_the_pointer_anchored_to_its_page() {
        let pages = [
            egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(500.0, 300.0)),
            egui::Rect::from_min_max(egui::pos2(100.0, 320.0), egui::pos2(500.0, 520.0)),
        ];
        assert_eq!(
            zoom_scroll_offset_about_pointer(
                egui::vec2(40.0, 60.0),
                egui::pos2(250.0, 420.0),
                &pages,
                1.5,
            ),
            egui::vec2(115.0, 210.0)
        );
        assert_eq!(
            zoom_scroll_offset_about_pointer(
                egui::vec2(10.0, 10.0),
                egui::pos2(300.0, 200.0),
                &pages,
                0.5,
            ),
            egui::Vec2::ZERO
        );
        assert_eq!(
            zoom_scroll_offset_about_pointer(
                egui::vec2(40.0, 60.0),
                egui::pos2(20.0, 20.0),
                &pages,
                2.0,
            ),
            egui::vec2(40.0, 60.0)
        );
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
    fn mit_session_document_renders_in_gui_preview() {
        if Command::new("latex").arg("--version").output().is_err()
            || Command::new("dvisvgm").arg("--version").output().is_err()
            || Command::new("rsvg-convert")
                .arg("--version")
                .output()
                .is_err()
        {
            return;
        }
        let source = include_str!("../../examples/mit-18-06-session-22.scas");
        let document = scas::document::build_live_with_limits(source, live_preview_limits())
            .expect("MIT source should build in the live preview");
        let renders = render_latex_document_pngs(&document.body(), [220, 220, 220], 1.0)
            .expect("MIT document should typeset in the GUI preview");
        assert!(!renders.is_empty());
        assert!(renders
            .iter()
            .all(|render| render.size[0] > 0 && render.size[1] > 0));
    }

    #[test]
    fn latex_preview_chunks_map_back_to_source_fragments() {
        let source = "/// First section\n\\[ 2 + 3 => \\]\n/// Second section\n\\[ 4 + 5 => \\]";
        let document = scas::document::build(source).unwrap();
        let chunks = document_preview_chunks(&document);
        assert_eq!(
            chunks
                .iter()
                .map(|(body, _)| body)
                .cloned()
                .collect::<Vec<_>>(),
            latex_preview_chunks(&document.body())
        );
        assert_eq!(chunks.first().unwrap().1.start, 0);
        assert_eq!(chunks.last().unwrap().1.end, document.fragments.len());
    }

    #[test]
    fn tall_builtin_gallery_is_split_into_safe_computer_modern_chunks() {
        let source = include_str!("../../tests/render_all_builtins.scas");
        let document = scas::document::build_with_limits(source, live_preview_limits()).unwrap();
        assert!(document.fragments.len() >= 2);
        let chunks = latex_preview_chunks(&document.body());
        assert!(chunks.len() > 2);
        assert!(chunks.iter().skip(1).all(|chunk| {
            chunk
                .strip_prefix(r"\begin{align*}")
                .and_then(|rows| rows.strip_suffix(r"\end{align*}"))
                .is_some_and(|rows| split_top_level_alignment_rows(rows).len() <= 32)
        }));
    }

    #[test]
    fn preview_chunking_keeps_nested_derivation_rows_together() {
        let rows = "\\begin{aligned}a&=b \\\\\nc&=d\\end{aligned} \\\\\ne&=f";
        let split = split_top_level_alignment_rows(rows);
        assert_eq!(split.len(), 2);
        assert!(split[0].contains("c&=d\\end{aligned}"));
        assert_eq!(split[1], "e&=f");
    }

    #[test]
    fn ordinary_documents_still_use_the_computer_modern_preview() {
        let document = scas::document::build(r"\[ integral(x^2, x) => \]").unwrap();
        assert_eq!(latex_preview_chunks(&document.body()), [document.body()]);
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
        app.graph_functions[0].critical_points =
            plot::detect_critical_points(&app.graph_functions[0].sampled.as_ref().unwrap().samples);
        let probe = snap_graph_probe(origin, &bounds, rect, &app.graph_functions)
            .expect("the sine curve crosses the screen centre");
        assert_eq!(probe.series_index, 0);
        assert_eq!(probe.critical_kind, None);
        let y = probe.y.expect("sine is defined at the snapped x");
        assert!(y.abs() < 0.01, "snapped y should be near zero, got {y}");
        let root = snap_graph_critical_point(origin, &bounds, rect, &app.graph_functions, 12.0)
            .expect("the highlighted root should have its own hit target");
        assert_eq!(root.critical_kind, Some(plot::CriticalKind::Zero));
        assert!(root.x.abs() < 0.01);

        // A hole must remain clickable even though there is no finite curve
        // sample at the discontinuity itself.
        app.graph_functions[0].critical_points.push(CriticalPoint {
            x: 1.0,
            y: 2.0,
            kind: plot::CriticalKind::Hole,
        });
        let hole_screen = egui::pos2(
            rect.left() + ((1.0 - bounds.x_min) / bounds.x_span()) as f32 * rect.width(),
            rect.top() + ((bounds.y_max - 2.0) / bounds.y_span()) as f32 * rect.height(),
        );
        let hole =
            snap_graph_critical_point(hole_screen, &bounds, rect, &app.graph_functions, 12.0)
                .expect("a highlighted hole should be clickable without a curve sample");
        assert_eq!(hole.critical_kind, Some(plot::CriticalKind::Hole));
        assert_eq!((hole.x, hole.y), (1.0, Some(2.0)));
        assert_eq!(
            hovered_graph_series(origin, &bounds, rect, &app.graph_functions, 12.0),
            Some(0)
        );
        assert_eq!(
            hovered_graph_series(
                egui::pos2(rect.left() + 10.0, rect.top() + 10.0),
                &bounds,
                rect,
                &app.graph_functions,
                12.0,
            ),
            None
        );
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
    fn unsaved_document_survives_new_and_open_until_decided() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        "let edited = 1;".clone_into(&mut app.source);
        app.dirty = true;
        app.new_file();
        assert_eq!(app.source, "let edited = 1;");
        assert!(matches!(app.pending_action, Some(PendingAction::NewFile)));

        let path =
            std::env::temp_dir().join(format!("scas-unsaved-open-{}.scas", std::process::id()));
        fs::write(&path, "let other = 2;").unwrap();
        app.open(path.clone());
        assert_eq!(app.source, "let edited = 1;");
        assert!(matches!(app.pending_action, Some(PendingAction::Open(_))));
        app.finish_pending_action(&context);
        assert_eq!(app.source, "let other = 2;");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_as_creates_new_file_without_overwriting_existing_file() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        let path = std::env::temp_dir().join(format!("scas-save-as-{}.scas", std::process::id()));
        let _ = fs::remove_file(&path);
        "let saved = 3;".clone_into(&mut app.source);
        app.dirty = true;
        app.save_as_path = path.display().to_string();
        assert!(app.save_as());
        assert_eq!(fs::read_to_string(&path).unwrap(), "let saved = 3;");
        assert!(!app.dirty);

        app.current_file = None;
        "let replacement = 4;".clone_into(&mut app.source);
        app.save_as_path = path.display().to_string();
        assert!(!app.save_as());
        assert_eq!(fs::read_to_string(&path).unwrap(), "let saved = 3;");
        fs::remove_file(path).unwrap();
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
        assert!(app.repl_input.is_empty());

        ":plot svg sin(x) from -3 to 3".clone_into(&mut app.repl_input);
        app.submit_repl();
        let vector_plot = app.repl_entries[1].plot.as_ref().unwrap();
        assert!(vector_plot.svg);
        assert_eq!(vector_plot.series.len(), 1);
        assert!(vector_plot.series[0]
            .samples
            .iter()
            .any(|(_, y)| y.is_some()));

        app.repl_input = ":plot".to_string();
        app.submit_repl();
        assert!(app.repl_entries[2].printed.contains("usage:"));
        assert!(app.repl_input.is_empty());

        app.repl_history_step(-1);
        assert_eq!(app.repl_input, ":plot");
        app.repl_history_step(-1);
        assert_eq!(app.repl_input, ":plot svg sin(x) from -3 to 3");
        app.repl_history_step(-1);
        assert_eq!(app.repl_input, ":plot x^2 from -2 to 2");
        app.repl_history_step(1);
        assert_eq!(app.repl_input, ":plot svg sin(x) from -3 to 3");
    }

    #[test]
    fn svg_repl_plot_paints_curves_in_the_history_buffer() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        ":plot svg sin(x) from -3 to 3".clone_into(&mut app.repl_input);
        app.submit_repl();
        let plot = app.repl_entries[0].plot.as_ref().unwrap();
        let theme = &app.themes[app.selected_theme];
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    show_inline_plot(ui, plot, theme);
                });
            },
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Path(_))),
            "SVG mode should paint a vector curve"
        );
    }

    #[test]
    fn editor_repl_help_and_precision_commands() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        ":help".clone_into(&mut app.repl_input);
        app.submit_repl();
        assert!(app.repl_entries[0].printed.contains(":precision N"));
        assert!(app.repl_entries[0].printed.contains("Ctrl+Enter"));

        ":precision 12".clone_into(&mut app.repl_input);
        app.submit_repl();
        assert_eq!(app.repl_interpreter.decimal_precision(), 12);
        assert!(app.repl_entries[1].printed.contains("12 digits"));

        ":precision 0".clone_into(&mut app.repl_input);
        app.submit_repl();
        assert_eq!(app.repl_interpreter.decimal_precision(), 12);
        assert!(app.repl_entries[2]
            .error
            .as_deref()
            .unwrap()
            .contains("usage"));
    }

    #[test]
    fn platform_font_defaults_match_each_desktop_os() {
        let windows = platform_ui_font_spec("windows", Some(Path::new(r"D:\Windows")), None);
        assert_eq!(windows.name, "Segoe UI");
        assert!(windows.candidates[0].ends_with("Fonts/segoeui.ttf"));

        let macos = platform_ui_font_spec("macos", None, None);
        assert_eq!(macos.name, "SF Pro Text");
        assert!(macos
            .candidates
            .iter()
            .any(|path| path.ends_with("SFNS.ttf")));

        let linux = platform_ui_font_spec("linux", None, Some(Path::new("/home/test")));
        assert_eq!(linux.name, "Ubuntu");
        assert!(linux
            .candidates
            .iter()
            .any(|path| path.ends_with("Ubuntu-R.ttf")));

        let fonts = default_font_definitions();
        assert_eq!(
            fonts.families[&FontFamily::Monospace][0],
            DEFAULT_CODE_FONT_NAME
        );
    }

    #[test]
    fn cut_without_selection_targets_only_the_cursor_line() {
        let source = "first\nαβ\nthird";
        let range = line_cut_range(source, 7);
        let [start, end] = range.sorted();
        assert_eq!((start.index, end.index), (6, 9));
        assert_eq!(
            &source[char_to_byte(source, start.index)..char_to_byte(source, end.index)],
            "αβ\n"
        );

        let range = line_cut_range(source, source.chars().count());
        let [start, end] = range.sorted();
        assert_eq!(
            &source[char_to_byte(source, start.index)..char_to_byte(source, end.index)],
            "third"
        );

        let range = line_cut_range("first\n", 6);
        let [start, end] = range.sorted();
        assert_eq!((start.index, end.index), (5, 6));
    }

    #[test]
    fn text_editor_cut_preserves_other_lines_and_copies_the_current_line() {
        let context = egui::Context::default();
        let editor_id = egui::Id::new("cut-integration-editor");
        let mut source = "first\nαβ\nthird".to_owned();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.add(TextEdit::multiline(&mut source).id(editor_id));
            });
        });
        let mut state = TextEdit::load_state(&context, editor_id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(7),
            )));
        TextEdit::store_state(&context, editor_id, state);
        context.memory_mut(|memory| memory.request_focus(editor_id));
        let output = context.run(
            egui::RawInput {
                events: vec![egui::Event::Cut],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    prepare_editor_line_cut(context, editor_id, &source, true);
                    ui.add(TextEdit::multiline(&mut source).id(editor_id));
                });
            },
        );
        assert_eq!(source, "first\nthird");
        assert_eq!(output.platform_output.copied_text, "αβ\n");
    }

    #[test]
    fn ctrl_enter_wraps_repl_expressions_in_evalf() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        "pi / 6".clone_into(&mut app.repl_input);
        app.refresh_repl_latex();
        app.submit_repl_numerically();
        assert_eq!(app.repl_entries.len(), 1);
        assert_eq!(app.repl_entries[0].source, "pi / 6");
        assert_eq!(app.repl_history[0], "pi / 6");
        assert!(!app.repl_entries[0]
            .input_latex
            .as_deref()
            .unwrap_or_default()
            .contains("evalf"));
        assert!(app.repl_session[0].contains("evalf(pi / 6)"));
        assert!(app.repl_entries[0].error.is_none());
        assert!(app.repl_input.is_empty());

        ":plot sin(x)".clone_into(&mut app.repl_input);
        app.submit_repl_numerically();
        assert_eq!(app.repl_entries.len(), 1);
        assert_eq!(app.repl_input, ":plot sin(x)");
        assert!(app.status.contains("use Enter for commands"));
    }

    #[test]
    fn quick_open_dismisses_only_after_an_outside_click() {
        let window = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(500.0, 300.0));
        assert!(!quick_open_clicked_outside(
            window,
            Some(egui::pos2(20.0, 20.0)),
            false,
        ));
        assert!(!quick_open_clicked_outside(
            window,
            Some(egui::pos2(200.0, 100.0)),
            true,
        ));
        assert!(quick_open_clicked_outside(
            window,
            Some(egui::pos2(20.0, 20.0)),
            true,
        ));
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
    fn long_document_tabs_keep_the_extension_and_fit_the_limit() {
        assert_eq!(
            compact_tab_title("calculus_and_domains.scas", 18),
            "calculus_and….scas"
        );
        assert_eq!(
            compact_tab_title("calculus_and_domains.scas •", 18),
            "calculus_a….scas •"
        );
        assert_eq!(compact_tab_title("short.scas", 18), "short.scas");
        assert_eq!(
            compact_tab_title("数学文件名称很长.scas", 10),
            "数学文件….scas"
        );
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
            severity: DiagnosticSeverity::Error,
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
    fn problems_navigation_selects_the_utf8_source_position() {
        let context = egui::Context::default();
        let mut app = ScasApp::new(&context);
        "let π = ;".clone_into(&mut app.source);
        let byte_start = app.source.find(';').unwrap();
        let diagnostic = EditorDiagnostic {
            byte_start,
            byte_end: byte_start + 1,
            line: 1,
            column: 9,
            message: "Expected expression".to_owned(),
            severity: DiagnosticSeverity::Error,
        };
        app.jump_to_diagnostic(&context, &diagnostic);
        let editor_id = egui::Id::new(("source-editor", app.buffer_generation));
        let state = TextEdit::load_state(&context, editor_id).unwrap();
        assert_eq!(state.cursor.char_range().unwrap().primary.index, 8);
        assert_eq!(app.current_line, 0);
    }

    #[test]
    fn unicode_document_diagnostics_keep_layout_ranges_on_char_boundaries() {
        let source = concat!(
            "/// A live, typeset result\n",
            "let angle = π / 6;\n",
            "\\[ sin(angle) => \\]\n",
            "print(sin(angle));\n\n",
            "\\[γ(1/2) => \\]",
        );
        let mut diagnostics = source_diagnostics(source);
        let document = scas::document::build_with_limits(source, live_preview_limits()).unwrap();
        diagnostics.extend(document.diagnostics.into_iter().map(|diagnostic| {
            EditorDiagnostic {
                byte_start: diagnostic.span.byte_start.min(source.len()),
                byte_end: diagnostic
                    .span
                    .byte_end
                    .max(diagnostic.span.byte_start + 1)
                    .min(source.len()),
                line: diagnostic.span.line,
                column: diagnostic.span.column,
                message: diagnostic.message,
                severity: diagnostic.severity,
            }
        }));
        diagnostics.push(EditorDiagnostic {
            byte_start: 90,
            byte_end: 91,
            line: 6,
            column: 3,
            message: "stale diagnostic from the previous buffer revision".to_owned(),
            severity: DiagnosticSeverity::Error,
        });
        let job = highlight(
            source,
            &Theme::bundled_themes()[0],
            FontId::monospace(15.0),
            &diagnostics,
        );
        assert!(job.sections.iter().all(|section| {
            job.text.is_char_boundary(section.byte_range.start)
                && job.text.is_char_boundary(section.byte_range.end)
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
    fn editor_gutter_fits_line_numbers_without_a_wide_empty_strip() {
        let single = editor_gutter_width(9, 27.0, LineNumberMode::Absolute);
        let triple = editor_gutter_width(999, 27.0, LineNumberMode::Absolute);
        assert!(single < 50.0);
        assert!(triple < 80.0);
        assert!(triple > single);
        assert_eq!(editor_gutter_width(999, 27.0, LineNumberMode::Off), 20.0);
    }

    #[test]
    fn preferences_round_trip_all_editor_choices() {
        let directory =
            std::env::temp_dir().join(format!("scas-preferences-test-{}", std::process::id()));
        let path = directory.join("preferences.toml");
        let expected = GuiPreferences {
            theme: "Nord".to_owned(),
            custom_theme_path: Some(PathBuf::from("/tmp/custom-theme.toml")),
            editor_font_size: 18.5,
            custom_font_path: Some(PathBuf::from("/tmp/custom-font.ttf")),
            line_numbers: LineNumberMode::Hybrid.key().to_owned(),
            auto_greek: false,
        };
        save_preferences(&path, &expected).unwrap();
        let (actual, warning) = load_preferences(Some(&path));
        assert_eq!(actual, expected);
        assert!(warning.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn preferences_use_each_platforms_config_directory() {
        let home = Path::new("/users/ada");
        let xdg = Path::new("/settings");
        let app_data = Path::new("C:/Users/Ada/AppData/Roaming");
        assert_eq!(
            preferences_path_for("linux", Some(xdg), None, Some(home)),
            Some(PathBuf::from("/settings/scas/preferences.toml"))
        );
        assert_eq!(
            preferences_path_for("macos", None, None, Some(home)),
            Some(PathBuf::from(
                "/users/ada/Library/Application Support/SCAS/preferences.toml"
            ))
        );
        assert_eq!(
            preferences_path_for("windows", None, Some(app_data), None),
            Some(app_data.join("SCAS/preferences.toml"))
        );
    }

    #[test]
    fn system_latex_produces_a_real_computer_modern_preview_when_available() {
        if Command::new("latex").arg("--version").output().is_err()
            || Command::new("dvisvgm").arg("--version").output().is_err()
            || Command::new("rsvg-convert")
                .arg("--version")
                .output()
                .is_err()
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
        let rendered = render_latex_png(&document.body(), [20, 20, 20], 1.0).unwrap();
        assert!(rendered.size[0] > 20);
        assert!(rendered.size[1] > 20);
        assert_eq!(rendered.rgba.len(), rendered.size[0] * rendered.size[1] * 4);

        // Preserve the preview package's page bounds so display math stays
        // centered even when short prose establishes the left edge above it.
        for zoom in [0.5, 1.0, 2.0] {
            let mixed =
                render_latex_png("a\n\n\\[\\sin(x) = \\frac{1}{2}\\]", [20, 20, 20], zoom).unwrap();
            let [width, height] = mixed.size;
            let mut formula_min_x = width;
            let mut formula_max_x = 0;
            for y in height / 2..height {
                for x in 0..width {
                    if mixed.rgba[(y * width + x) * 4 + 3] != 0 {
                        formula_min_x = formula_min_x.min(x);
                        formula_max_x = formula_max_x.max(x);
                    }
                }
            }
            assert!(formula_min_x < formula_max_x);
            let formula_center = (formula_min_x + formula_max_x) as f32 / 2.0;
            let page_center = width as f32 / 2.0;
            assert!((formula_center - page_center).abs() <= width as f32 * 0.05);
        }

        // REPL snippets are rendered in math mode via the same system LaTeX
        // pipeline, so fractions and symbols use actual Computer Modern glyphs.
        let repl = render_repl_math_png(r"\frac{1}{2} + \sqrt{2}", [20, 20, 20]).unwrap();
        assert!(repl.size[0] > 20);
        assert!(repl.size[1] > 10);

        let plot_document = scas::document::build(r"\[ plot(sin(x), x, -pi, pi) \]").unwrap();
        let plot = render_latex_png(&plot_document.body(), [20, 20, 20], 1.0).unwrap();
        assert!(plot.size[0] > 100);
        assert!(plot.size[1] > 50);

        let gallery = scas::document::build_with_limits(
            include_str!("../../tests/render_all_builtins.scas"),
            live_preview_limits(),
        )
        .unwrap();
        let pages = render_latex_document_pngs(&gallery.body(), [20, 20, 20], 1.0).unwrap();
        assert!(pages.len() > 2);
        assert!(pages
            .iter()
            .all(|page| page.size[0] > 20 && page.size[1] > 20));

        // The complete all-steps gallery may intentionally exceed the bounded
        // texture cache. Verify caching with a normal-sized page instead of
        // requiring an unusually large document to remain resident in full.
        let cached_page = render_latex_png_cached(r"\[x^2+1\]", [20, 20, 20], 1.0).unwrap();
        let cached_again = render_latex_png_cached(r"\[x^2+1\]", [20, 20, 20], 1.0).unwrap();
        assert!(Arc::ptr_eq(&cached_page, &cached_again));
        let zoomed_page = render_latex_png_cached(r"\[x^2+1\]", [20, 20, 20], 2.0).unwrap();
        assert!(zoomed_page.size[0] > cached_page.size[0]);
        assert!(zoomed_page.size[1] > cached_page.size[1]);
    }
}
