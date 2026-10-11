//! Browser entry point for a real egui SCAS workspace. The native IDE remains
//! separate while platform services are being extracted from its binary.

use crate::ast::SourceSpan;
use crate::document::{self, Document, DocumentFragment, FragmentKind};
use crate::lexer::{Scanner, TokenType};
use crate::plot::{self, PlotBounds};
use crate::web_runtime::{MIT_EXAMPLE, STARTER};
use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat},
    Color32, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = scasDownload)]
    fn download_file(name: &str, content: &str, mime: &str);
    #[wasm_bindgen(js_namespace = window, js_name = scasImport)]
    fn import_file();
    #[wasm_bindgen(js_namespace = window, js_name = scasPromptInput)]
    fn prompt_input(message: &str) -> Option<String>;
    #[wasm_bindgen(catch, js_namespace = window, js_name = scasCreateWorker)]
    fn create_worker() -> Result<Worker, JsValue>;
    #[wasm_bindgen(js_namespace = window, js_name = scasShowPreview)]
    fn show_preview(
        fragments: &str,
        tex: &str,
        pending: bool,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixels_per_point: f32,
    );
    #[wasm_bindgen(js_namespace = window, js_name = scasHidePreview)]
    fn hide_preview();
}

#[wasm_bindgen]
pub fn worker_evaluate(request: &str) -> String {
    crate::web_runtime::process_request(request)
}

#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    pub async fn start(&self, canvas_id: &str) -> Result<(), JsValue> {
        self.runner
            .start(
                canvas_id,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    configure_context(&cc.egui_ctx);
                    Ok(Box::new(WebApp::new(cc.egui_ctx.clone())))
                }),
            )
            .await
    }

    pub fn load_source(&self, source: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.load_source(source);
        }
    }

    pub fn import_data_file(&self, name: String, content: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.import_data_file(name, content);
        }
    }

    pub fn close_preview(&self) -> bool {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.preview_open = false;
            app.context.request_repaint();
            true
        } else {
            false
        }
    }
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn configure_context(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrains Mono".to_owned(),
        FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains Mono".to_owned());
    fonts.font_data.insert(
        "VS Code Codicons".to_owned(),
        FontData::from_static(include_bytes!("../assets/icons/codicon.ttf")),
    );
    fonts.families.insert(
        FontFamily::Name("VS Code Codicons".into()),
        vec!["VS Code Codicons".to_owned()],
    );
    context.set_fonts(fonts);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(37, 37, 38);
    visuals.window_fill = Color32::from_rgb(37, 37, 38);
    visuals.extreme_bg_color = Color32::from_rgb(31, 31, 31);
    visuals.faint_bg_color = Color32::from_rgb(31, 31, 31);
    visuals.override_text_color = Some(Color32::from_rgb(204, 204, 204));
    visuals.selection.bg_fill = Color32::from_rgb(38, 79, 120);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 122, 204);
    context.set_visuals(visuals);
    let mut style = (*context.style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(12.0));
    context.set_style(style);
}

fn highlight_source(source: &str, font: FontId) -> LayoutJob {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut spans = tokens
        .iter()
        .filter_map(|token| {
            use TokenType::*;
            let color = match token.token_type {
                Eof => return None,
                Let | Fn | If | Else | For | While | Break | Continue | Return | Print | And
                | Or | Xor | True | False | Nil | Exit | Help | Clear => {
                    Color32::from_rgb(197, 134, 192)
                }
                Number => Color32::from_rgb(181, 206, 168),
                Str => Color32::from_rgb(206, 145, 120),
                DocComment => Color32::from_rgb(96, 139, 78),
                Identifier => Color32::from_rgb(156, 220, 254),
                RenderLeftBrace | RenderRightBrace | RenderLeftBracket | RenderRightBracket => {
                    Color32::from_rgb(78, 201, 176)
                }
                LeftParen | RightParen | LeftBracket | RightBracket | LeftBrace | RightBrace
                | Comma | Dot | Colon | Semicolon => Color32::from_rgb(255, 215, 0),
                _ => Color32::from_rgb(212, 212, 212),
            };
            Some((token.byte_start, token.byte_end, color))
        })
        .collect::<Vec<_>>();
    for (start, line) in source.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    }) {
        for (offset, _) in line.match_indices("//") {
            let at = start + offset;
            if !spans.iter().any(|(from, to, _)| *from <= at && at < *to) {
                spans.push((
                    at,
                    start + line.trim_end_matches('\n').len(),
                    Color32::from_rgb(106, 153, 85),
                ));
                break;
            }
        }
    }
    spans.sort_by_key(|(start, end, _)| (*start, *end));
    let mut job = LayoutJob::default();
    let mut cursor = 0;
    let mut append = |text: &str, color| {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        )
    };
    for (start, end, color) in spans {
        if start < cursor || start >= end || end > source.len() {
            continue;
        }
        append(&source[cursor..start], Color32::from_rgb(212, 212, 212));
        append(&source[start..end], color);
        cursor = end;
    }
    append(&source[cursor..], Color32::from_rgb(212, 212, 212));
    job
}

fn paint_line_numbers(ui: &egui::Ui, output: &egui::text_edit::TextEditOutput, gutter: f32) {
    let left = output.galley_pos.x - gutter;
    let clip = egui::Rect::from_min_max(
        egui::pos2(left, output.response.rect.top()),
        output.response.rect.right_bottom(),
    )
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let separator = output.galley_pos.x - 10.0;
    painter.line_segment(
        [
            egui::pos2(separator, clip.top()),
            egui::pos2(separator, clip.bottom()),
        ],
        egui::Stroke::new(1.0, Color32::from_rgb(53, 53, 53)),
    );
    let mut line = 1;
    let mut first_row = true;
    for row in &output.galley.rows {
        if first_row {
            painter.text(
                egui::pos2(separator - 10.0, output.galley_pos.y + row.rect.center().y),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                FontId::monospace(if gutter < 60.0 { 13.0 } else { 18.0 }),
                Color32::from_rgb(0, 122, 204),
            );
        }
        first_row = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
}

fn paint_web_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 27.0), egui::Sense::hover());
    let center = rect.center();
    let stroke = egui::Stroke::new(2.4, Color32::from_rgb(0, 122, 204));
    for (a, b) in [
        (egui::vec2(-8.0, -7.0), egui::vec2(7.0, -7.0)),
        (egui::vec2(-8.0, -7.0), egui::vec2(-1.0, 0.0)),
        (egui::vec2(-1.0, 0.0), egui::vec2(-8.0, 7.0)),
        (egui::vec2(-8.0, 7.0), egui::vec2(7.0, 7.0)),
    ] {
        ui.painter().line_segment([center + a, center + b], stroke);
    }
    ui.painter()
        .circle_filled(center + egui::vec2(8.0, -7.0), 1.8, stroke.color);
    ui.painter()
        .circle_filled(center + egui::vec2(8.0, 7.0), 1.8, stroke.color);
}

fn web_icon_button(ui: &mut egui::Ui, glyph: char, tooltip: &str) -> egui::Response {
    ui.add_sized(
        [27.0, 27.0],
        egui::Button::new(RichText::new(glyph).font(FontId::new(
            16.0,
            FontFamily::Name("VS Code Codicons".into()),
        )))
        .frame(false),
    )
    .on_hover_text(tooltip)
}

fn web_activity_button(
    ui: &mut egui::Ui,
    glyph: char,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 38.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            0.0,
            Color32::from_rgb(0, 122, 204),
        );
    }
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(2.0), 2.0, Color32::from_gray(70));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(23.0, FontFamily::Name("VS Code Codicons".into())),
        if selected {
            Color32::from_rgb(204, 204, 204)
        } else {
            Color32::from_rgb(157, 157, 157)
        },
    );
    response.on_hover_text(tooltip)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Document,
    Repl,
    Plot,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Output,
    Problems,
}

struct WebSeries {
    label: String,
    samples: Vec<(f64, Option<f64>)>,
}

struct WebProblem {
    line: usize,
    column: usize,
    message: String,
}

struct WebReplEntry {
    input: String,
    output: String,
    error: Option<String>,
}

struct WebApp {
    context: egui::Context,
    source: String,
    output: String,
    error: Option<String>,
    problems: Vec<WebProblem>,
    document: Option<Document>,
    tex: String,
    tab: Tab,
    plot_expression: String,
    plot_series: Vec<WebSeries>,
    plot_bounds: [f64; 4],
    plot_svg: String,
    plot_custom_view: bool,
    plot_fit_y_pending: bool,
    plot_due: Option<f64>,
    repl_input: String,
    repl_entries: Vec<WebReplEntry>,
    repl_history_index: Option<usize>,
    last_repl_submission: String,
    queued_inputs: Vec<String>,
    generation: u64,
    pending: bool,
    due: Option<f64>,
    worker: Option<Worker>,
    _onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    responses: Receiver<String>,
    status: String,
    source_name: String,
    preview_json: String,
    explorer_open: bool,
    preview_open: bool,
    output_open: bool,
    bottom_tab: BottomTab,
    highlight_source: String,
    highlight_job: LayoutJob,
    first_frame: bool,
    quick_open: bool,
    quick_open_query: String,
    explorer_root_open: bool,
    explorer_expanded: HashSet<String>,
}

impl WebApp {
    fn new(context: egui::Context) -> Self {
        let (sender, responses) = mpsc::channel();
        let (worker, onmessage, status) = match create_worker() {
            Ok(worker) => {
                let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(result) = event.data().as_string() {
                        let _ = sender.send(result);
                    }
                }) as Box<dyn FnMut(_)>);
                worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                (Some(worker), Some(onmessage), "Ready".to_owned())
            }
            Err(_) => (
                None,
                None,
                "Web Worker unavailable; execution disabled".to_owned(),
            ),
        };
        let stored = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item("scas.draft").ok().flatten());
        let shared = web_sys::window()
            .and_then(|window| window.location().hash().ok())
            .and_then(|hash| hash.strip_prefix("#source=").map(str::to_owned))
            .and_then(|encoded| js_sys::decode_uri_component(&encoded).ok())
            .map(String::from);
        let source_name = if shared.is_some() {
            "shared.scas"
        } else if stored.is_some() {
            "untitled.scas"
        } else {
            "Untitled"
        };
        let source = shared.or(stored).unwrap_or_else(|| STARTER.to_owned());
        let initial_preview = if source_name == "Untitled" {
            document::build_live_with_limits(&source, crate::web_runtime::browser_limits()).ok()
        } else {
            None
        };
        let initial_tex = initial_preview
            .as_ref()
            .map_or_else(String::new, Document::latex);
        let initial_preview_json = initial_preview.as_ref().map_or_else(|| "[]".to_owned(), |document| {
            serde_json::to_string(&document.fragments.iter().map(|fragment| json!({
                "kind": match fragment.kind { FragmentKind::Prose => "prose", FragmentKind::DisplayMath => "display", FragmentKind::AlignedMath => "aligned" },
                "latex": fragment.latex,
                "span": { "line": fragment.span.line }
            })).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".to_owned())
        });
        let mut app = Self {
            context,
            source,
            output: String::new(),
            error: None,
            problems: Vec::new(),
            document: initial_preview,
            tex: initial_tex,
            tab: Tab::Document,
            plot_expression: "sin(x)".to_owned(),
            plot_series: Vec::new(),
            plot_bounds: [-10.0, 10.0, -1.0, 1.0],
            plot_svg: String::new(),
            plot_custom_view: false,
            plot_fit_y_pending: false,
            plot_due: None,
            repl_input: String::new(),
            repl_entries: Vec::new(),
            repl_history_index: None,
            last_repl_submission: String::new(),
            queued_inputs: Vec::new(),
            generation: 0,
            pending: false,
            due: None,
            worker,
            _onmessage: onmessage,
            responses,
            status,
            source_name: source_name.to_owned(),
            preview_json: initial_preview_json,
            explorer_open: true,
            preview_open: true,
            output_open: false,
            bottom_tab: BottomTab::Output,
            highlight_source: String::new(),
            highlight_job: LayoutJob::default(),
            first_frame: true,
            quick_open: false,
            quick_open_query: String::new(),
            explorer_root_open: true,
            explorer_expanded: HashSet::new(),
        };
        app.request("run");
        app
    }

    fn load_source(&mut self, source: String) {
        self.source = source;
        "imported.scas".clone_into(&mut self.source_name);
        self.save_draft();
        self.request("run");
    }

    fn import_data_file(&mut self, name: String, content: String) {
        let name = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_owned();
        if let Some(worker) = &self.worker {
            let payload =
                json!({"kind": "file_import", "generation": 0, "name": name, "content": content});
            if worker
                .post_message(&JsValue::from_str(&payload.to_string()))
                .is_ok()
            {
                self.status = format!("Imported {name}");
                self.context.request_repaint();
            }
        }
    }

    fn save_draft(&self) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("scas.draft", &self.source);
        }
    }

    fn request(&mut self, kind: &str) {
        self.generation = self.generation.wrapping_add(1);
        let request = json!({
            "generation": self.generation,
            "kind": kind,
            "source": self.source,
            "expression": self.plot_expression,
            "input": self.repl_input,
            "inputs": self.queued_inputs,
            "view": if kind == "plot" && self.plot_custom_view { Some(self.plot_bounds) } else { None },
            "fit_y": kind == "plot" && self.plot_fit_y_pending,
        });
        if let Some(worker) = &self.worker {
            if worker
                .post_message(&JsValue::from_str(&request.to_string()))
                .is_ok()
            {
                self.pending = true;
                (if kind == "plot" {
                    "Sampling plot…"
                } else {
                    "Evaluating…"
                })
                .clone_into(&mut self.status);
            } else {
                "Could not send work to the browser worker".clone_into(&mut self.status);
            }
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        while let Ok(response) = self.responses.try_recv() {
            let Ok(value) = serde_json::from_str::<Value>(&response) else {
                continue;
            };
            if value["generation"].as_u64() != Some(self.generation) {
                continue;
            }
            self.pending = false;
            if let Some(prompt) = value["input_required"].as_str() {
                if let Some(answer) = prompt_input(prompt) {
                    self.queued_inputs.push(answer);
                    if value["kind"].as_str() == Some("repl") {
                        self.repl_input = self.last_repl_submission.clone();
                        self.request("repl");
                        self.repl_input.clear();
                    } else {
                        self.request("run");
                    }
                    continue;
                }
            } else {
                self.queued_inputs.clear();
            }
            if value["kind"].as_str() == Some("repl") {
                let entry = WebReplEntry {
                    input: value["input"].as_str().unwrap_or("").to_owned(),
                    output: value["output"].as_str().unwrap_or("").to_owned(),
                    error: value["error"].as_str().map(str::to_owned),
                };
                if entry.input.trim() == ":reset" && entry.error.is_none() {
                    self.repl_entries.clear();
                }
                self.status = if entry.error.is_some() {
                    "Needs attention"
                } else {
                    "Ready"
                }
                .to_owned();
                self.repl_entries.push(entry);
                context.request_repaint();
                continue;
            }
            self.error = value["error"].as_str().map(str::to_owned);
            self.problems = value["diagnostics"]
                .as_array()
                .map(|diagnostics| {
                    diagnostics
                        .iter()
                        .map(|diagnostic| WebProblem {
                            line: diagnostic["line"].as_u64().unwrap_or(1) as usize,
                            column: diagnostic["column"].as_u64().unwrap_or(1) as usize,
                            message: diagnostic["message"].as_str().unwrap_or("").to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(output) = value["output"].as_str() {
                output.clone_into(&mut self.output);
            }
            if let Some(tex) = value["tex"].as_str() {
                tex.clone_into(&mut self.tex);
            }
            if let Some(fragments) = value["fragments"].as_array() {
                let mut document = Document::default();
                for fragment in fragments {
                    let kind = match fragment["kind"].as_str() {
                        Some("prose") => FragmentKind::Prose,
                        Some("aligned") => FragmentKind::AlignedMath,
                        _ => FragmentKind::DisplayMath,
                    };
                    let span = &fragment["span"];
                    document.fragments.push(DocumentFragment {
                        kind,
                        latex: fragment["latex"].as_str().unwrap_or("").to_owned(),
                        span: SourceSpan {
                            line: span["line"].as_u64().unwrap_or(1) as usize,
                            column: span["column"].as_u64().unwrap_or(1) as usize,
                            length: span["length"].as_u64().unwrap_or(1) as usize,
                            byte_start: span["byte_start"].as_u64().unwrap_or(0) as usize,
                            byte_end: span["byte_end"].as_u64().unwrap_or(0) as usize,
                        },
                    });
                }
                if self.error.is_none() || self.document.is_none() {
                    self.preview_json =
                        serde_json::to_string(fragments).unwrap_or_else(|_| "[]".to_owned());
                    self.document = Some(document);
                }
            }
            if let Some(bounds) = value["bounds"].as_array() {
                if bounds.len() == 4 {
                    for (index, bound) in bounds.iter().enumerate() {
                        if !self.plot_custom_view || (self.plot_fit_y_pending && index >= 2) {
                            self.plot_bounds[index] =
                                bound.as_f64().unwrap_or(self.plot_bounds[index]);
                        }
                    }
                    self.plot_fit_y_pending = false;
                }
            }
            if let Some(series) = value["series"].as_array() {
                self.plot_series = series
                    .iter()
                    .map(|series| WebSeries {
                        label: series["label"].as_str().unwrap_or("curve").to_owned(),
                        samples: series["samples"]
                            .as_array()
                            .map(|samples| {
                                samples
                                    .iter()
                                    .filter_map(|point| {
                                        let pair = point.as_array()?;
                                        Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect();
            }
            if let Some(svg) = value["svg"].as_str() {
                svg.clone_into(&mut self.plot_svg);
            }
            (if self.error.is_some() {
                "Needs attention"
            } else {
                "Ready"
            })
            .clone_into(&mut self.status);
            context.request_repaint();
        }
        if self.pending {
            context.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn plot_view(&self) -> PlotBounds {
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        PlotBounds::new(x_min, x_max, y_min, y_max)
    }

    fn apply_plot_view(&mut self, view: PlotBounds, now: f64, context: &egui::Context) {
        self.plot_bounds = [view.x_min, view.x_max, view.y_min, view.y_max];
        self.plot_custom_view = true;
        self.plot_fit_y_pending = false;
        self.plot_svg.clear();
        self.plot_due = Some(now + 0.14);
        context.request_repaint_after(Duration::from_millis(150));
    }

    fn zoom_plot(
        &mut self,
        factor: f64,
        x_ratio: f64,
        y_ratio: f64,
        now: f64,
        context: &egui::Context,
    ) {
        if let Some(view) = self.plot_view().zoomed_at(factor, x_ratio, y_ratio) {
            self.apply_plot_view(view, now, context);
        }
    }

    fn plot(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let now = ui.input(|input| input.time);
        let drag = if response.dragged() {
            ui.input(|input| input.pointer.delta())
        } else {
            egui::Vec2::ZERO
        };
        if drag != egui::Vec2::ZERO {
            if let Some(view) =
                self.plot_view()
                    .panned_by_pixels(drag.x, drag.y, rect.width(), rect.height())
            {
                self.apply_plot_view(view, now, ui.ctx());
            }
        }
        let (scroll, pointer) = ui.input(|input| {
            (
                input.raw_scroll_delta.y,
                input.pointer.hover_pos().unwrap_or(rect.center()),
            )
        });
        if response.hovered() && scroll.abs() > 0.0 {
            let factor = f64::from((-scroll * 0.0015).exp()).clamp(0.5, 2.0);
            let x_ratio = f64::from((pointer.x - rect.left()) / rect.width().max(1.0));
            let y_ratio = f64::from((pointer.y - rect.top()) / rect.height().max(1.0));
            self.zoom_plot(factor, x_ratio, y_ratio, now, ui.ctx());
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(31, 31, 31));
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        let map = |x: f64, y: f64| {
            egui::pos2(
                rect.left() + ((x - x_min) / (x_max - x_min)) as f32 * rect.width(),
                rect.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * rect.height(),
            )
        };
        let x_step = plot::nice_step(x_max - x_min, 10.0);
        let y_step = plot::nice_step(y_max - y_min, 8.0);
        for (vertical, minimum, maximum, step) in
            [(true, x_min, x_max, x_step), (false, y_min, y_max, y_step)]
        {
            if !step.is_finite() || step <= 0.0 {
                continue;
            }
            let mut value = (minimum / step).ceil() * step;
            for _ in 0..80 {
                if value > maximum {
                    break;
                }
                let zero = value.abs() < step * 1e-6;
                let color = if zero {
                    Color32::from_gray(90)
                } else {
                    Color32::from_gray(46)
                };
                let stroke = egui::Stroke::new(if zero { 1.4 } else { 1.0 }, color);
                if vertical {
                    let x = map(value, 0.0).x;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        stroke,
                    );
                    painter.text(
                        egui::pos2(x + 3.0, rect.bottom() - 4.0),
                        egui::Align2::LEFT_BOTTOM,
                        format_plot_tick(value, step),
                        FontId::monospace(10.0),
                        Color32::from_gray(135),
                    );
                } else {
                    let y = map(0.0, value).y;
                    painter.line_segment(
                        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                        stroke,
                    );
                    painter.text(
                        egui::pos2(rect.left() + 5.0, y - 3.0),
                        egui::Align2::LEFT_BOTTOM,
                        format_plot_tick(value, step),
                        FontId::monospace(10.0),
                        Color32::from_gray(135),
                    );
                }
                value += step;
            }
        }
        let colors = [
            Color32::from_rgb(79, 156, 249),
            Color32::from_rgb(249, 119, 127),
            Color32::from_rgb(83, 196, 137),
        ];
        for (index, series) in self.plot_series.iter().enumerate() {
            let mut previous = None;
            for &(x, y) in &series.samples {
                let point = y.filter(|y| y.is_finite()).map(|y| map(x, y));
                if let (Some(a), Some(b)) = (previous, point) {
                    painter
                        .line_segment([a, b], egui::Stroke::new(2.0, colors[index % colors.len()]));
                }
                previous = point;
            }
        }
        if self.plot_series.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Enter an expression above, then select Plot.",
                FontId::proportional(13.0),
                Color32::from_gray(157),
            );
        }
        painter.text(
            egui::pos2(rect.right() - 10.0, rect.bottom() - 10.0),
            egui::Align2::RIGHT_BOTTOM,
            "Drag to pan · Scroll to zoom",
            FontId::proportional(11.0),
            Color32::from_gray(157),
        );
    }
}

fn format_plot_tick(value: f64, step: f64) -> String {
    let digits = if step < 0.001 {
        6
    } else if step < 1.0 {
        3
    } else if step < 100.0 {
        1
    } else {
        0
    };
    let text = format!("{value:.digits$}");
    if digits == 0 {
        text
    } else {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

impl WebApp {
    fn submit_repl(&mut self) {
        if self.pending || self.repl_input.trim().is_empty() {
            return;
        }
        self.repl_input = self.repl_input.trim().to_owned();
        self.last_repl_submission = self.repl_input.clone();
        self.request("repl");
        self.repl_input.clear();
        self.repl_history_index = None;
    }

    fn show_repl(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(Color32::from_rgb(37, 37, 38))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    ui.label(RichText::new("REPL").strong().size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Reset session").clicked() && !self.pending {
                            self.repl_input = ":reset".to_owned();
                            self.submit_repl();
                        }
                    });
                });
            });
        let muted = Color32::from_rgb(157, 157, 157);
        let history_height = (ui.available_height() - 104.0).max(60.0);
        egui::ScrollArea::vertical()
            .id_source("web-repl-history")
            .max_height(history_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.repl_entries.is_empty() {
                    ui.add_space(26.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Start a calculation").strong().size(18.0));
                        ui.label(RichText::new("Enter an expression below. Definitions stay available for this browser session.").size(12.0).color(muted));
                        ui.monospace("Try: f(x) = x^2 - 2;   then   f(3)");
                    });
                }
                for (index, entry) in self.repl_entries.iter().enumerate() {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 9.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", index + 1)).monospace().size(10.5).color(muted));
                            ui.label(RichText::new(&entry.input).monospace().size(14.0));
                        });
                        if !entry.output.is_empty() {
                            ui.label(RichText::new(&entry.output).monospace().size(13.0));
                        }
                        if let Some(error) = &entry.error {
                            ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                        }
                    });
                    ui.separator();
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">")
                    .monospace()
                    .size(17.0)
                    .color(Color32::from_rgb(0, 122, 204)),
            );
            let response = ui.add_sized(
                [ui.available_width() - 75.0, 62.0],
                TextEdit::multiline(&mut self.repl_input)
                    .id(egui::Id::new("web-repl-input"))
                    .font(FontId::monospace(14.0))
                    .desired_rows(2)
                    .hint_text(
                        "SCAS expression or statement · Enter to run, Shift+Enter for a new line",
                    ),
            );
            if response.has_focus() {
                let (up, down, enter, shift) = ui.input(|input| {
                    (
                        input.key_pressed(egui::Key::ArrowUp),
                        input.key_pressed(egui::Key::ArrowDown),
                        input.key_pressed(egui::Key::Enter),
                        input.modifiers.shift,
                    )
                });
                if up && self.repl_input.trim().is_empty() {
                    let index = self.repl_entries.len().saturating_sub(1);
                    if let Some(entry) = self.repl_entries.get(index) {
                        self.repl_input.clone_from(&entry.input);
                        self.repl_history_index = Some(index);
                    }
                } else if down {
                    if let Some(index) = self.repl_history_index {
                        if let Some(next) = self.repl_entries.get(index + 1) {
                            self.repl_input.clone_from(&next.input);
                            self.repl_history_index = Some(index + 1);
                        } else {
                            self.repl_input.clear();
                            self.repl_history_index = None;
                        }
                    }
                }
                if enter && !shift {
                    self.submit_repl();
                }
            }
            if ui
                .add_enabled(
                    !self.pending && !self.repl_input.trim().is_empty(),
                    egui::Button::new("Run"),
                )
                .clicked()
            {
                self.submit_repl();
            }
        });
    }

    fn preview_panel(&self, ui: &mut egui::Ui, context: &egui::Context) {
        let rect = ui.max_rect();
        show_preview(
            &self.preview_json,
            &self.tex,
            self.pending,
            rect.left(),
            rect.top(),
            rect.width(),
            rect.height(),
            context.pixels_per_point(),
        );
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            ui.menu_button("Examples", |ui| {
                if ui.button("Starter document").clicked() {
                    self.load_source(STARTER.to_owned());
                    "starter.scas".clone_into(&mut self.source_name);
                    ui.close_menu();
                }
                if ui.button("MIT 18.06 · Diagonalization").clicked() {
                    self.load_source(MIT_EXAMPLE.to_owned());
                    "mit-18-06-session-22.scas".clone_into(&mut self.source_name);
                    ui.close_menu();
                }
            });
            ui.separator();
            if ui.button("Import .scas…").clicked() {
                import_file();
                ui.close_menu();
            }
            if ui.button("Download .scas").clicked() {
                download_file(&self.source_name, &self.source, "text/plain");
                ui.close_menu();
            }
            if ui.button("Download .tex").clicked() {
                download_file("document.tex", &self.tex, "text/x-tex");
                ui.close_menu();
            }
            if ui.button("Share link").clicked() {
                if let Some(window) = web_sys::window() {
                    let hash = format!("source={}", js_sys::encode_uri_component(&self.source));
                    let _ = window.location().set_hash(&hash);
                    if let Ok(url) = window.location().href() {
                        ui.output_mut(|output| output.copied_text = url);
                    }
                    "Share link updated and copied".clone_into(&mut self.status);
                }
                ui.close_menu();
            }
        });
    }
}

impl eframe::App for WebApp {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(context);
        if self.first_frame {
            if context.screen_rect().width() < 1100.0 {
                self.explorer_open = false;
            }
            if context.screen_rect().width() < 850.0 {
                self.preview_open = false;
            }
            self.first_frame = false;
        }
        let editor_font = FontId::monospace(if context.screen_rect().width() < 600.0 {
            18.0
        } else {
            27.0
        });
        if self.highlight_source != self.source {
            self.highlight_job = highlight_source(&self.source, editor_font.clone());
            self.highlight_source.clone_from(&self.source);
        }
        let now = context.input(|input| input.time);
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.request("run");
        }
        if self.plot_due.is_some_and(|due| now >= due) {
            self.plot_due = None;
            self.request("plot");
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.explorer_open = !self.explorer_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::J)) {
            self.output_open = !self.output_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            self.quick_open = true;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            if self.tab == Tab::Repl {
                self.submit_repl();
            } else if self.tab == Tab::Document {
                self.request("run");
                self.output_open = true;
            } else {
                self.request("plot");
            }
        }
        let dark = Color32::from_rgb(31, 31, 31);
        let panel = Color32::from_rgb(37, 37, 38);
        let muted = Color32::from_rgb(157, 157, 157);
        let border = egui::Stroke::new(1.0, Color32::from_rgb(58, 58, 59));
        let mut jump_to = None;
        let mut preview_visible = false;
        let narrow = context.screen_rect().width() < 850.0;
        if narrow
            && self.preview_open
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.preview_open = false;
        }

        egui::TopBottomPanel::top("web-title-bar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(60, 60, 60))
                    .inner_margin(egui::Margin::symmetric(7.0, 4.0)),
            )
            .show(context, |ui| {
                let available = ui.available_width();
                let compact = available < 700.0;
                let left_width = if compact {
                    160.0
                } else {
                    (available * 0.32).clamp(410.0, 690.0)
                };
                let center_width = if compact {
                    (available - left_width - 90.0).max(80.0)
                } else {
                    (available * 0.38).clamp(350.0, 970.0)
                };
                ui.horizontal_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(left_width);
                            paint_web_logo(ui);
                            ui.add_space(9.0);
                            let button_frame = ui.visuals().button_frame;
                            ui.visuals_mut().button_frame = false;
                            self.file_menu(ui);
                            ui.menu_button("Edit", |ui| {
                                if ui.button("Copy source").clicked() {
                                    ui.output_mut(|output| {
                                        output.copied_text = self.source.clone()
                                    });
                                    "Source copied".clone_into(&mut self.status);
                                    ui.close_menu();
                                }
                            });
                            ui.menu_button("View", |ui| {
                                ui.checkbox(&mut self.explorer_open, "Explorer  Ctrl+B");
                                ui.checkbox(&mut self.preview_open, "Document Preview");
                                ui.checkbox(&mut self.output_open, "Output Panel  Ctrl+J");
                                if ui.button("Problems").clicked() {
                                    self.bottom_tab = BottomTab::Problems;
                                    self.output_open = true;
                                    ui.close_menu();
                                }
                                if ui.button("Interactive REPL").clicked() {
                                    self.tab = Tab::Repl;
                                    ui.close_menu();
                                }
                            });
                            if !compact {
                                ui.menu_button("Go", |ui| {
                                    if ui.button("Quick Open…  Ctrl+P").clicked() {
                                        self.quick_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Editor").clicked() {
                                        self.tab = Tab::Document;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.menu_button("Run", |ui| {
                                if ui.button("Run Active  Ctrl+Enter").clicked() {
                                    match self.tab {
                                        Tab::Document => {
                                            self.request("run");
                                            self.output_open = true;
                                        }
                                        Tab::Repl => self.submit_repl(),
                                        Tab::Plot => self.request("plot"),
                                    }
                                    ui.close_menu();
                                }
                                if ui.button("Graphing Calculator").clicked() {
                                    self.tab = Tab::Plot;
                                    ui.close_menu();
                                }
                            });
                            if !compact {
                                ui.menu_button("Terminal", |ui| {
                                    if ui.button("Interactive REPL").clicked() {
                                        self.tab = Tab::Repl;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.visuals_mut().button_frame = button_frame;
                        },
                    );
                    ui.add_space(5.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(center_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(center_width);
                            if ui
                                .add_sized(
                                    [center_width - 2.0, 26.0],
                                    egui::Button::new(RichText::new("scas").size(12.0))
                                        .fill(Color32::from_rgb(54, 54, 54))
                                        .stroke(border),
                                )
                                .on_hover_text("Quick Open · Ctrl+P")
                                .clicked()
                            {
                                self.quick_open = true;
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if web_icon_button(ui, '\u{ebf4}', "Toggle preview").clicked() {
                            self.preview_open = !self.preview_open;
                        }
                        if web_icon_button(ui, '\u{ebf2}', "Toggle output panel").clicked() {
                            self.output_open = !self.output_open;
                        }
                        if web_icon_button(ui, '\u{ebf3}', "Toggle Explorer").clicked() {
                            self.explorer_open = !self.explorer_open;
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("web-status")
            .exact_height(26.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0, 122, 204))
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button(format!("× {}", self.problems.len())).clicked() {
                        self.bottom_tab = BottomTab::Problems;
                        self.output_open = true;
                    }
                    ui.label(RichText::new(&self.status).color(Color32::WHITE).size(11.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("UTF-8    { } SCAS")
                                .color(Color32::WHITE)
                                .size(11.0),
                        );
                    });
                });
            });
        egui::SidePanel::left("web-activity")
            .exact_width(46.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(51, 51, 51))
                    .stroke(border)
                    .inner_margin(egui::Margin::symmetric(3.0, 6.0)),
            )
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    if web_activity_button(ui, '\u{eaf0}', self.explorer_open, "Explorer · Ctrl+B")
                        .clicked()
                    {
                        self.explorer_open = !self.explorer_open;
                    }
                    if web_activity_button(ui, '\u{ea6d}', self.quick_open, "Quick Open · Ctrl+P")
                        .clicked()
                    {
                        self.quick_open = true;
                    }
                    if web_activity_button(
                        ui,
                        '\u{ea85}',
                        self.tab == Tab::Repl,
                        "Interactive REPL",
                    )
                    .clicked()
                    {
                        self.tab = Tab::Repl;
                    }
                    if web_activity_button(ui, '\u{ebe2}', self.tab == Tab::Plot, "Graphing")
                        .clicked()
                    {
                        self.tab = Tab::Plot;
                    }
                });
            });
        if self.explorer_open {
            egui::SidePanel::left("web-explorer")
                .default_width(300.0)
                .width_range(200.0..=420.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(panel)
                        .stroke(border)
                        .inner_margin(egui::Margin::symmetric(12.0, 3.0)),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Explorer").strong().size(13.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if web_icon_button(ui, '\u{ea7c}', "Explorer actions").clicked() {
                                self.quick_open = true;
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(if self.explorer_root_open {
                                "⌄"
                            } else {
                                "›"
                            })
                            .clicked()
                        {
                            self.explorer_root_open = !self.explorer_root_open;
                        }
                        if ui
                            .selectable_label(false, RichText::new("scas").strong())
                            .clicked()
                        {
                            self.explorer_root_open = !self.explorer_root_open;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if web_icon_button(ui, '\u{eb1c}', "Collapse folders").clicked() {
                                self.explorer_expanded.clear();
                            }
                            if web_icon_button(ui, '\u{eb37}', "Refresh workspace").clicked() {
                                self.request("run");
                            }
                            if web_icon_button(ui, '\u{ea7b}', "New document").clicked() {
                                self.load_source(STARTER.to_owned());
                                self.source_name = "Untitled".to_owned();
                                self.tab = Tab::Document;
                            }
                        });
                    });
                    if self.explorer_root_open {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        for folder in [
                            "assets", "benches", "docs", "examples", "scripts", "src", "stdlib",
                            "tests", "themes", "web",
                        ] {
                            let expanded = self.explorer_expanded.contains(folder);
                            ui.horizontal(|ui| {
                                ui.add_space(7.0);
                                if ui.small_button(if expanded { "⌄" } else { "›" }).clicked() {
                                    if expanded {
                                        self.explorer_expanded.remove(folder);
                                    } else {
                                        self.explorer_expanded.insert(folder.to_owned());
                                    }
                                }
                                ui.label(
                                    RichText::new('\u{ea83}')
                                        .font(FontId::new(
                                            15.0,
                                            FontFamily::Name("VS Code Codicons".into()),
                                        ))
                                        .color(muted),
                                );
                                if ui.selectable_label(false, folder).clicked() {
                                    if expanded {
                                        self.explorer_expanded.remove(folder);
                                    } else {
                                        self.explorer_expanded.insert(folder.to_owned());
                                    }
                                }
                            });
                            if expanded && folder == "examples" {
                                ui.indent("web-examples", |ui| {
                                    if ui.button("Starter.scas").clicked() {
                                        self.load_source(STARTER.to_owned());
                                        self.source_name = "Starter.scas".to_owned();
                                        self.tab = Tab::Document;
                                    }
                                    if ui.button("MIT 18.06.scas").clicked() {
                                        self.load_source(MIT_EXAMPLE.to_owned());
                                        self.source_name = "MIT 18.06.scas".to_owned();
                                        self.tab = Tab::Document;
                                    }
                                });
                            }
                        }
                    }
                    ui.add_space(7.0);
                    ui.separator();
                    ui.collapsing("Outline", |ui| {
                        ui.label("SCAS document symbols");
                    });
                    ui.collapsing("Timeline", |ui| {
                        ui.label("No timeline entries");
                    });
                });
        }
        if self.tab == Tab::Document {
            if self.output_open {
                egui::TopBottomPanel::bottom("web-output")
                    .resizable(true)
                    .default_height(230.0)
                    .height_range(120.0..=500.0)
                    .frame(
                        egui::Frame::none()
                            .fill(dark)
                            .stroke(border)
                            .inner_margin(egui::Margin::symmetric(10.0, 6.0)),
                    )
                    .show(context, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.bottom_tab == BottomTab::Output, "OUTPUT")
                                .clicked()
                            {
                                self.bottom_tab = BottomTab::Output;
                            }
                            if ui
                                .selectable_label(
                                    self.bottom_tab == BottomTab::Problems,
                                    format!("PROBLEMS  {}", self.problems.len()),
                                )
                                .clicked()
                            {
                                self.bottom_tab = BottomTab::Problems;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("×").on_hover_text("Close panel").clicked()
                                    {
                                        self.output_open = false;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| match self.bottom_tab {
                            BottomTab::Output => {
                                if !self.output.is_empty() {
                                    ui.monospace(&self.output);
                                } else {
                                    ui.label(RichText::new("No output yet.").color(muted));
                                }
                                if let Some(error) = &self.error {
                                    ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                                }
                            }
                            BottomTab::Problems => {
                                if self.problems.is_empty() {
                                    ui.label(RichText::new("No problems detected.").color(muted));
                                }
                                for problem in &self.problems {
                                    let label = format!(
                                        "{}:{}:{}  {}",
                                        self.source_name,
                                        problem.line,
                                        problem.column,
                                        problem.message
                                    );
                                    if ui
                                        .selectable_label(
                                            false,
                                            RichText::new(label)
                                                .color(Color32::from_rgb(249, 119, 127)),
                                        )
                                        .clicked()
                                    {
                                        jump_to = Some((problem.line, problem.column));
                                    }
                                }
                            }
                        });
                    });
            }
            if self.preview_open {
                preview_visible = true;
                if !narrow {
                    egui::SidePanel::right("web-preview")
                        .resizable(true)
                        .default_width(220.0)
                        .width_range(220.0..=750.0)
                        .frame(
                            egui::Frame::none()
                                .fill(panel)
                                .stroke(border)
                                .inner_margin(egui::Margin::symmetric(10.0, 7.0)),
                        )
                        .show(context, |ui| self.preview_panel(ui, context));
                }
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| {
                    egui::Frame::none().fill(panel).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            egui::Frame::none()
                                .fill(dark)
                                .inner_margin(egui::Margin::symmetric(10.0, 2.0))
                                .show(ui, |ui| {
                                    ui.horizontal_centered(|ui| {
                                        ui.set_min_height(30.0);
                                        ui.label(
                                            RichText::new("SC")
                                                .strong()
                                                .size(12.0)
                                                .color(Color32::from_rgb(0, 122, 204)),
                                        );
                                        ui.label(RichText::new(&self.source_name).size(12.0));
                                        if web_icon_button(
                                            ui,
                                            '\u{ea76}',
                                            "Reset to a new document",
                                        )
                                        .clicked()
                                        {
                                            self.load_source(STARTER.to_owned());
                                            "untitled.scas".clone_into(&mut self.source_name);
                                        }
                                    });
                                });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if web_icon_button(ui, '\u{ea7c}', "Editor actions").clicked() {
                                        self.quick_open = true;
                                    }
                                    if web_icon_button(
                                        ui,
                                        '\u{ebf4}',
                                        "Toggle mathematical preview",
                                    )
                                    .clicked()
                                    {
                                        self.preview_open = !self.preview_open;
                                    }
                                    if web_icon_button(
                                        ui,
                                        '\u{eb2c}',
                                        "Run active workspace · Ctrl+Enter",
                                    )
                                    .clicked()
                                    {
                                        self.request("run");
                                        self.output_open = true;
                                    }
                                },
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.source_name).size(11.0).color(muted));
                        ui.label(RichText::new("›").size(11.0).color(muted));
                        ui.label(RichText::new("SCAS").size(11.0).color(muted));
                    });
                    ui.add_space(4.0);
                    let available_rows =
                        (ui.available_height() / if narrow { 19.0 } else { 28.0 }) as usize;
                    let cached_source = &self.highlight_source;
                    let cached_job = &self.highlight_job;
                    let mut layouter = |ui: &egui::Ui, source: &str, width: f32| {
                        let mut job = if source == cached_source {
                            cached_job.clone()
                        } else {
                            highlight_source(source, editor_font.clone())
                        };
                        job.wrap.max_width = width;
                        ui.fonts(|fonts| fonts.layout_job(job))
                    };
                    let gutter = 48.0;
                    let response = egui::ScrollArea::both()
                        .show(ui, |ui| {
                            let output = TextEdit::multiline(&mut self.source)
                                .id(egui::Id::new("web-source"))
                                .font(editor_font.clone())
                                .code_editor()
                                .frame(false)
                                .margin(egui::Margin {
                                    left: gutter,
                                    right: 6.0,
                                    top: 4.0,
                                    bottom: 4.0,
                                })
                                .desired_width(ui.available_width())
                                .layouter(&mut layouter)
                                .desired_rows(available_rows.max(12))
                                .show(ui);
                            paint_line_numbers(ui, &output, gutter);
                            output.response
                        })
                        .inner;
                    if response.changed() {
                        self.save_draft();
                        self.due = Some(now + 0.4);
                        context.request_repaint_after(Duration::from_millis(420));
                    }
                });
            if let Some((line, column)) = jump_to {
                let index = self
                    .source
                    .lines()
                    .take(line.saturating_sub(1))
                    .map(|line| line.chars().count() + 1)
                    .sum::<usize>()
                    + column.saturating_sub(1);
                let editor_id = egui::Id::new("web-source");
                let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(index.min(self.source.chars().count())),
                    )));
                TextEdit::store_state(context, editor_id, state);
                context.memory_mut(|memory| memory.request_focus(editor_id));
            }
        } else if self.tab == Tab::Repl {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| self.show_repl(ui));
        } else {
            egui::TopBottomPanel::top("plot-controls")
                .frame(egui::Frame::none().fill(panel))
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("y =");
                        let expression = ui.text_edit_singleline(&mut self.plot_expression);
                        if ui.button("Plot").clicked()
                            || (expression.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        {
                            self.plot_custom_view = false;
                            self.plot_fit_y_pending = false;
                            self.plot_due = None;
                            self.request("plot");
                        }
                        ui.separator();
                        if ui.button("+").on_hover_text("Zoom in").clicked() {
                            self.zoom_plot(0.75, 0.5, 0.5, now, context);
                        }
                        if ui.button("−").on_hover_text("Zoom out").clicked() {
                            self.zoom_plot(1.0 / 0.75, 0.5, 0.5, now, context);
                        }
                        if ui
                            .button("Fit y")
                            .on_hover_text("Fit the vertical range to the visible samples")
                            .clicked()
                        {
                            self.plot_custom_view = true;
                            self.plot_fit_y_pending = true;
                            self.request("plot");
                        }
                        if ui.button("Reset view").clicked() {
                            self.plot_custom_view = false;
                            self.plot_fit_y_pending = false;
                            self.request("plot");
                        }
                        if ui
                            .add_enabled(
                                !self.plot_svg.is_empty(),
                                egui::Button::new("Download SVG"),
                            )
                            .clicked()
                        {
                            download_file("scas-plot.svg", &self.plot_svg, "image/svg+xml");
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                    }
                    for series in &self.plot_series {
                        ui.label(RichText::new(&series.label).size(11.0));
                    }
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| self.plot(ui));
        }
        if narrow && self.tab == Tab::Document && self.preview_open {
            let screen = context.screen_rect();
            show_preview(
                &self.preview_json,
                &self.tex,
                self.pending,
                46.0,
                38.0,
                screen.width() - 46.0,
                screen.height() - 64.0,
                context.pixels_per_point(),
            );
        }
        if !preview_visible {
            hide_preview();
        }
        if self.quick_open {
            let mut open = true;
            let mut selected = None;
            egui::Window::new("Quick Open")
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 48.0))
                .fixed_size(egui::vec2(
                    440.0_f32.min(context.screen_rect().width() - 36.0),
                    130.0,
                ))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(context, |ui| {
                    let response = ui.add_sized(
                        [ui.available_width(), 26.0],
                        TextEdit::singleline(&mut self.quick_open_query)
                            .hint_text("Search files and views"),
                    );
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        selected = Some(0);
                    }
                    let query = self.quick_open_query.to_lowercase();
                    for (index, label) in [
                        "mit-18-06-session-22.scas  ·  examples",
                        "starter.scas  ·  examples",
                        "Graphing calculator",
                        "Interactive REPL",
                    ]
                    .iter()
                    .enumerate()
                    {
                        if label.to_lowercase().contains(&query)
                            && ui.selectable_label(false, *label).clicked()
                        {
                            selected = Some(index);
                        }
                    }
                });
            if context.input(|input| input.key_pressed(egui::Key::Escape)) {
                open = false;
            }
            self.quick_open = open;
            if let Some(index) = selected {
                match index {
                    0 => {
                        self.load_source(MIT_EXAMPLE.to_owned());
                        "mit-18-06-session-22.scas".clone_into(&mut self.source_name);
                        self.tab = Tab::Document;
                    }
                    1 => {
                        self.load_source(STARTER.to_owned());
                        "starter.scas".clone_into(&mut self.source_name);
                        self.tab = Tab::Document;
                    }
                    2 => self.tab = Tab::Plot,
                    _ => self.tab = Tab::Repl,
                }
                self.quick_open = false;
                self.quick_open_query.clear();
            }
        }
    }
}
