#[cfg(target_arch = "wasm32")]
use crate::document::{Document, FragmentKind};
use eframe::egui::{self, Align, Color32, FontFamily, FontId, Layout, RichText, Stroke};
#[cfg(not(target_arch = "wasm32"))]
use scas::document::{Document, FragmentKind};

#[derive(Debug, Clone)]
enum Math {
    Text(String),
    Row(Vec<Math>),
    Fraction(Box<Math>, Box<Math>),
    Root(Box<Math>),
    Superscript(Box<Math>),
    Subscript(Box<Math>),
    Matrix(Vec<Vec<Math>>),
}

pub fn show_document(
    ui: &mut egui::Ui,
    document: &Document,
    accent: Color32,
    muted: Color32,
    scale: f32,
    scroll_target: Option<usize>,
) -> Option<usize> {
    let mut selected = None;
    for (index, fragment) in document.fragments.iter().enumerate() {
        let response = ui
            .scope(|ui| match fragment.kind {
                FragmentKind::Prose => {
                    ui.add_space(8.0);
                    show_prose(ui, &fragment.latex, accent, scale);
                    ui.add_space(8.0);
                }
                FragmentKind::DisplayMath => {
                    let body = fragment
                        .latex
                        .strip_prefix(r"\[")
                        .and_then(|text| text.strip_suffix(r"\]"))
                        .unwrap_or(&fragment.latex);
                    let body = body
                        .strip_prefix(r"\begin{aligned}")
                        .and_then(|text| text.strip_suffix(r"\end{aligned}"))
                        .unwrap_or(body);
                    let rows = body.split("\\\\\n").collect::<Vec<_>>();
                    formula_card(ui, &rows, accent, muted, scale);
                }
                FragmentKind::AlignedMath => {
                    let body = fragment
                        .latex
                        .strip_prefix(r"\begin{align*}")
                        .and_then(|text| text.strip_suffix(r"\end{align*}"))
                        .unwrap_or(&fragment.latex);
                    let rows = body.split("\\\\\n").collect::<Vec<_>>();
                    formula_card(ui, &rows, accent, muted, scale);
                }
            })
            .response;
        let response = ui
            .interact(
                response.rect,
                ui.id().with(("preview-fragment", index)),
                egui::Sense::click(),
            )
            .on_hover_text(format!("Go to source line {}", fragment.span.line));
        if scroll_target == Some(index) {
            ui.scroll_to_rect(response.rect, Some(Align::Center));
        }
        if response.clicked() {
            selected = Some(index);
        }
    }
    selected
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn show_expression(ui: &mut egui::Ui, latex: &str, color: Color32, size: f32) {
    // Retain expressions used in consecutive frames; unused entries expire.
    // Arc avoids cloning the parsed tree on each hover, scroll or caret redraw.
    type MathCache = egui::util::cache::FrameCache<std::sync::Arc<Math>, MathParser>;
    let math = ui
        .ctx()
        .memory_mut(|memory| memory.caches.cache::<MathCache>().get(latex));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 1.5;
        render(ui, &math, size, color);
    });
}

#[derive(Default)]
struct MathParser;

impl egui::util::cache::ComputerMut<&str, std::sync::Arc<Math>> for MathParser {
    fn compute(&mut self, source: &str) -> std::sync::Arc<Math> {
        std::sync::Arc::new(parse(source))
    }
}

fn show_prose(ui: &mut egui::Ui, prose: &str, color: Color32, scale: f32) {
    for line in prose.lines() {
        if let Some(title) = latex_heading(line, r"\section*{") {
            ui.heading(
                RichText::new(title)
                    .size(25.0 * scale)
                    .strong()
                    .color(color),
            );
            continue;
        }
        if let Some(title) = latex_heading(line, r"\subsection*{") {
            ui.label(
                RichText::new(title)
                    .size(20.0 * scale)
                    .strong()
                    .color(color),
            );
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (index, part) in line.split('$').enumerate() {
                if index % 2 == 0 {
                    if !part.is_empty() {
                        ui.label(
                            RichText::new(part)
                                .size(16.0 * scale)
                                .line_height(Some(23.0 * scale)),
                        );
                    }
                } else {
                    render(ui, &parse(part), 17.0 * scale, color);
                }
            }
        });
    }
}

fn latex_heading<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.trim().strip_prefix(prefix)?.strip_suffix('}')
}

fn formula_card(ui: &mut egui::Ui, rows: &[&str], accent: Color32, muted: Color32, scale: f32) {
    egui::Frame::none()
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(1.0_f32, muted.gamma_multiply(0.35)))
        .rounding(8.0 * scale)
        .inner_margin(egui::Margin::symmetric(18.0 * scale, 14.0 * scale))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for row in rows {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    render(
                        ui,
                        &parse(row.replace('&', "").trim()),
                        23.0 * scale,
                        accent,
                    );
                });
                ui.add_space(5.0);
            }
        });
    ui.add_space(10.0);
}

fn render(ui: &mut egui::Ui, math: &Math, size: f32, color: Color32) {
    match math {
        Math::Text(text) => {
            let rich = RichText::new(text)
                .font(FontId::new(size, FontFamily::Proportional))
                .color(color);
            ui.label(rich);
        }
        Math::Row(parts) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.5;
                for part in parts {
                    render(ui, part, size, color);
                }
            });
        }
        Math::Fraction(numerator, denominator) => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    render(ui, numerator, size * 0.78, color)
                });
                let width = ui.min_rect().width().max(22.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 1.5), egui::Sense::hover());
                ui.painter().line_segment(
                    [rect.left_center(), rect.right_center()],
                    Stroke::new(1.25_f32, color),
                );
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    render(ui, denominator, size * 0.78, color)
                });
            });
        }
        Math::Root(value) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.label(RichText::new("√").size(size * 1.2).color(color));
                egui::Frame::none()
                    .stroke(Stroke { width: 0.0, color })
                    .show(ui, |ui| render(ui, value, size, color));
            });
        }
        Math::Superscript(value) => {
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                render(ui, value, size * 0.62, color)
            });
        }
        Math::Subscript(value) => {
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                render(ui, value, size * 0.62, color)
            });
        }
        Math::Matrix(rows) => {
            ui.label(RichText::new("(").size(size * 1.25).color(color));
            egui::Grid::new(ui.next_auto_id())
                .spacing(egui::vec2(12.0, 5.0))
                .show(ui, |ui| {
                    for row in rows {
                        for value in row {
                            render(ui, value, size * 0.86, color);
                        }
                        ui.end_row();
                    }
                });
            ui.label(RichText::new(")").size(size * 1.25).color(color));
        }
    }
}

fn parse(source: &str) -> Math {
    let mut parser = Parser { source, cursor: 0 };
    Math::Row(parser.sequence(None))
}

struct Parser<'a> {
    source: &'a str,
    cursor: usize,
}

impl Parser<'_> {
    fn sequence(&mut self, stop: Option<char>) -> Vec<Math> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(character) = self.peek() {
            if Some(character) == stop {
                self.bump();
                break;
            }
            if character == '\\' {
                self.flush(&mut text, &mut nodes);
                self.bump();
                let command = self.take_while(char::is_alphabetic);
                match command.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let numerator = self.group();
                        let denominator = self.group();
                        nodes.push(Math::Fraction(Box::new(numerator), Box::new(denominator)));
                    }
                    "sqrt" => nodes.push(Math::Root(Box::new(self.group()))),
                    "left" | "right" => {}
                    "cdot" => nodes.push(Math::Text(" · ".to_owned())),
                    "times" => nodes.push(Math::Text(" × ".to_owned())),
                    "le" => nodes.push(Math::Text(" ≤ ".to_owned())),
                    "ge" => nodes.push(Math::Text(" ≥ ".to_owned())),
                    "ne" => nodes.push(Math::Text(" ≠ ".to_owned())),
                    "land" => nodes.push(Math::Text(" ∧ ".to_owned())),
                    "lor" => nodes.push(Math::Text(" ∨ ".to_owned())),
                    "infty" => nodes.push(Math::Text("∞".to_owned())),
                    "varnothing" => nodes.push(Math::Text("∅".to_owned())),
                    "quad" | "qquad" => nodes.push(Math::Text("  ".to_owned())),
                    "text" | "textbf" | "mathrm" | "mathbf" | "mathit" | "operatorname"
                    | "mathbin" | "mathord" => nodes.push(self.group()),
                    "begin" if self.group_text().as_deref() == Some("pmatrix") => {
                        let end = r"\end{pmatrix}";
                        let remaining = &self.source[self.cursor..];
                        if let Some(length) = remaining.find(end) {
                            let body = &remaining[..length];
                            self.cursor += length + end.len();
                            let rows = body
                                .split(r"\\")
                                .map(|row| row.split('&').map(|cell| parse(cell.trim())).collect())
                                .collect();
                            nodes.push(Math::Matrix(rows));
                        }
                    }
                    name => nodes.push(Math::Text(command_text(name))),
                }
            } else if character == '^' {
                self.flush(&mut text, &mut nodes);
                self.bump();
                nodes.push(Math::Superscript(Box::new(self.group())));
            } else if character == '_' {
                self.flush(&mut text, &mut nodes);
                self.bump();
                nodes.push(Math::Subscript(Box::new(self.group())));
            } else if character == '{' {
                self.flush(&mut text, &mut nodes);
                nodes.push(self.group());
            } else {
                text.push(character);
                self.bump();
            }
        }
        self.flush(&mut text, &mut nodes);
        nodes
    }

    fn group(&mut self) -> Math {
        if self.peek() == Some('{') {
            self.bump();
            Math::Row(self.sequence(Some('}')))
        } else {
            let text = self
                .peek()
                .map(|character| character.to_string())
                .unwrap_or_default();
            self.bump();
            Math::Text(text)
        }
    }

    fn group_text(&mut self) -> Option<String> {
        if self.peek()? != '{' {
            return None;
        }
        self.bump();
        let start = self.cursor;
        while self.peek()? != '}' {
            self.bump();
        }
        let value = self.source[start..self.cursor].to_owned();
        self.bump();
        Some(value)
    }

    fn flush(&self, text: &mut String, nodes: &mut Vec<Math>) {
        if !text.is_empty() {
            nodes.push(Math::Text(std::mem::take(text)));
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.cursor..].chars().next()
    }
    fn bump(&mut self) {
        if let Some(character) = self.peek() {
            self.cursor += character.len_utf8();
        }
    }
    fn take_while(&mut self, predicate: impl Fn(char) -> bool) -> String {
        let mut output = String::new();
        while self.peek().is_some_and(&predicate) {
            output.push(self.peek().unwrap());
            self.bump();
        }
        output
    }
}

fn command_text(command: &str) -> String {
    match command {
        "alpha" | "upalpha" => "α",
        "beta" | "upbeta" => "β",
        "gamma" | "upgamma" => "γ",
        "delta" | "updelta" => "δ",
        "epsilon" | "upepsilon" => "ε",
        "theta" | "uptheta" => "θ",
        "lambda" | "uplambda" => "λ",
        "mu" | "upmu" => "μ",
        "pi" | "uppi" => "π",
        "rho" | "uprho" => "ρ",
        "sigma" | "upsigma" => "σ",
        "phi" | "upphi" => "φ",
        "omega" | "upomega" => "ω",
        "Gamma" | "Upgamma" => "Γ",
        "Delta" | "Updelta" => "Δ",
        "Theta" | "Uptheta" => "Θ",
        "Lambda" | "Uplambda" => "Λ",
        "Sigma" | "Upsigma" => "Σ",
        "Phi" | "Upphi" => "Φ",
        "Omega" | "Upomega" => "Ω",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "ln" => "ln",
        "log" => "log",
        "exp" => "exp",
        "det" => "det",
        "int" => "∫",
        other => other,
    }
    .to_owned()
}
