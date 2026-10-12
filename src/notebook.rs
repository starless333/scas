//! Text-first notebook cells, execution sessions, typed output, and manifests.
//!
//! A notebook is always ordinary `.scas` source. This module owns the shared
//! semantics used by native clients, the CLI, and the browser worker.

use crate::ast::{Expr, ExprKind, Statement, StatementKind};
use crate::interpreter::Interpreter;
use crate::latex::LatexRenderer;
use crate::lexer::literal::Literal;
use crate::lexer::{Scanner, SymbolTable, TokenType};
use crate::parser::Parser;
use crate::runtime_io::{self, RuntimeIo};
use crate::{ExecutionOptions, Limits, RuntimeError};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookHost {
    Native,
    Browser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookTrust {
    Local,
    Untrusted,
    Trusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Code,
    Prose,
    Math,
    Verification,
    Mixed,
}

impl CellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Prose => "prose",
            Self::Math => "math",
            Self::Verification => "verification",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    NotRun,
    Queued,
    Running,
    Current,
    Stale,
    Failed,
    Blocked,
    Cancelled,
}

impl CellState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookSpan {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line: usize,
    pub column: usize,
}

impl NotebookSpan {
    fn json(&self) -> JsonValue {
        json!({
            "byte_start": self.byte_start,
            "byte_end": self.byte_end,
            "line": self.line,
            "column": self.column,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCell {
    /// Stable while the cell keeps its position in this document revision.
    /// Persistent cross-revision IDs deliberately remain a future sidecar
    /// feature; no generated identifier is written into `.scas` source.
    pub id: String,
    pub ordinal: usize,
    pub label: Option<String>,
    pub kind: CellKind,
    pub span: NotebookSpan,
}

impl NotebookCell {
    pub fn json(&self) -> JsonValue {
        json!({
            "id": self.id,
            "ordinal": self.ordinal,
            "label": self.label,
            "kind": self.kind.as_str(),
            "span": self.span.json(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    Text,
    Stdout,
    Value,
    Math,
    Verification,
    Diagnostic,
    Input,
}

impl OutputKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Stdout => "stdout",
            Self::Value => "value",
            Self::Math => "math",
            Self::Verification => "verification",
            Self::Diagnostic => "diagnostic",
            Self::Input => "input",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputEvent {
    pub cell_id: String,
    pub span: NotebookSpan,
    pub generation: u64,
    pub sequence: u64,
    pub kind: OutputKind,
    pub state: CellState,
    /// Required accessible/CLI representation of every event.
    pub text: String,
    /// Optional renderer-ready payload. Frontends must not reevaluate source
    /// to produce rich output.
    pub data: JsonValue,
    pub duration_micros: Option<u64>,
    pub resource_usage: Option<JsonValue>,
}

impl OutputEvent {
    pub fn json(&self) -> JsonValue {
        json!({
            "type": "output",
            "cell_id": self.cell_id,
            "span": self.span.json(),
            "generation": self.generation,
            "sequence": self.sequence,
            "kind": self.kind.as_str(),
            "state": self.state.as_str(),
            "text": self.text,
            "data": self.data,
            "duration_micros": self.duration_micros,
            "resource_usage": self.resource_usage,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationSummary {
    pub passed: usize,
    pub failed: usize,
    pub unproved: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedModuleHash {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproducibilityManifest {
    pub schema: u32,
    pub scas_version: String,
    pub target: String,
    pub source_sha256: String,
    pub imported_modules: Vec<ImportedModuleHash>,
    pub decimal_precision: u64,
    pub limits: Limits,
    pub random_seed: Option<u64>,
    pub input_data: Vec<ImportedModuleHash>,
    pub verification: VerificationSummary,
}

impl ReproducibilityManifest {
    pub fn json(&self) -> JsonValue {
        json!({
            "schema": self.schema,
            "scas_version": self.scas_version,
            "target": self.target,
            "source_sha256": self.source_sha256,
            "imported_modules": self.imported_modules.iter().map(|module| json!({
                "path": module.path, "sha256": module.sha256
            })).collect::<Vec<_>>(),
            "settings": {
                "decimal_precision": self.decimal_precision,
                "limits": limits_json(self.limits),
            },
            "random_seed": self.random_seed,
            "input_data": self.input_data.iter().map(|data| json!({
                "name": data.path, "sha256": data.sha256
            })).collect::<Vec<_>>(),
            "verification": {
                "passed": self.verification.passed,
                "failed": self.verification.failed,
                "unproved": self.verification.unproved,
            },
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotebookCapabilities {
    pub imports: Vec<String>,
    pub reads_input: bool,
    pub reads_files: bool,
    pub writes_files: bool,
}

impl NotebookCapabilities {
    pub fn json(&self, host: NotebookHost) -> JsonValue {
        json!({
            "imports": self.imports,
            "reads_input": self.reads_input,
            "reads_files": self.reads_files,
            "writes_files": self.writes_files,
            "file_access": match host {
                NotebookHost::Native => "native",
                NotebookHost::Browser => "virtual_only",
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotebookRun {
    pub generation: u64,
    pub events: Vec<OutputEvent>,
    pub states: Vec<CellState>,
    pub clean: bool,
    pub manifest: Option<ReproducibilityManifest>,
}

impl NotebookRun {
    pub fn succeeded(&self) -> bool {
        !self.states.iter().any(|state| {
            matches!(
                state,
                CellState::Failed | CellState::Blocked | CellState::Cancelled
            )
        })
    }
}

#[derive(Clone)]
pub struct CancellationHandle(Arc<AtomicBool>);

impl CancellationHandle {
    pub fn interrupt(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub struct NotebookSession {
    source: String,
    cells: Vec<NotebookCell>,
    states: Vec<CellState>,
    symbols: SymbolTable,
    interpreter: Interpreter<Vec<u8>>,
    io: Rc<dyn RuntimeIo>,
    options: ExecutionOptions,
    host: NotebookHost,
    trust: NotebookTrust,
    generation: u64,
    cancellation: Arc<AtomicBool>,
}

impl NotebookSession {
    pub fn new(source: impl Into<String>) -> Self {
        Self::with_io(
            source,
            ExecutionOptions::default(),
            runtime_io::default_io(),
            if cfg!(target_arch = "wasm32") {
                NotebookHost::Browser
            } else {
                NotebookHost::Native
            },
            NotebookTrust::Local,
        )
    }

    pub fn with_io(
        source: impl Into<String>,
        options: ExecutionOptions,
        io: Rc<dyn RuntimeIo>,
        host: NotebookHost,
        trust: NotebookTrust,
    ) -> Self {
        let source = source.into();
        let cells = parse_cells(&source);
        let symbols = SymbolTable::new();
        let cancellation = Arc::new(AtomicBool::new(false));
        let mut interpreter = Interpreter::with_limits_and_io(
            symbols.clone(),
            Vec::new(),
            options.limits,
            Rc::clone(&io),
        );
        let _ = interpreter.set_decimal_precision(options.decimal_precision);
        interpreter.set_cancellation_flag(Arc::clone(&cancellation));
        Self {
            states: vec![CellState::NotRun; cells.len()],
            source,
            cells,
            symbols,
            interpreter,
            io,
            options,
            host,
            trust,
            generation: 0,
            cancellation,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn cells(&self) -> &[NotebookCell] {
        &self.cells
    }

    pub fn states(&self) -> &[CellState] {
        &self.states
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn accepts_generation(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub fn trust(&self) -> NotebookTrust {
        self.trust
    }

    pub fn set_trust(&mut self, trust: NotebookTrust) {
        self.trust = trust;
    }

    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle(Arc::clone(&self.cancellation))
    }

    pub fn interrupt(&self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }

    pub fn capabilities(&self) -> NotebookCapabilities {
        capabilities(&self.source)
    }

    /// Replaces source without executing it. The first changed cell and every
    /// following previously-run cell become stale (a deliberately conservative
    /// dependency policy).
    pub fn update_source(&mut self, source: impl Into<String>) {
        let source = source.into();
        let next_cells = parse_cells(&source);
        let first_changed = first_changed_cell(&self.source, &self.cells, &source, &next_cells);
        let mut next_states = vec![CellState::NotRun; next_cells.len()];
        for index in 0..next_cells.len().min(self.states.len()) {
            next_states[index] = if first_changed.is_some_and(|changed| index >= changed) {
                if self.states[index] == CellState::NotRun {
                    CellState::NotRun
                } else {
                    CellState::Stale
                }
            } else {
                self.states[index]
            };
        }
        self.source = source;
        self.cells = next_cells;
        self.states = next_states;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn restart(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.reset_kernel();
        for state in &mut self.states {
            if *state != CellState::NotRun {
                *state = CellState::Stale;
            }
        }
    }

    pub fn run_cell(&mut self, index: usize) -> NotebookRun {
        self.begin_generation();
        let mut events = Vec::new();
        let mut verification = VerificationSummary::default();
        if index >= self.cells.len() {
            events.push(self.synthetic_diagnostic(index, "cell index is out of range"));
            return self.finish(events, false, None);
        }
        self.states[index] = CellState::Queued;
        self.evaluate_cell(index, &mut events, &mut verification);
        for state in self.states.iter_mut().skip(index + 1) {
            if *state != CellState::NotRun {
                *state = CellState::Stale;
            }
        }
        self.finish(events, false, None)
    }

    pub fn run_above(&mut self, index: usize) -> NotebookRun {
        self.begin_generation();
        let mut events = Vec::new();
        let mut verification = VerificationSummary::default();
        if index >= self.cells.len() {
            events.push(self.synthetic_diagnostic(index, "cell index is out of range"));
            return self.finish(events, false, None);
        }
        self.reset_kernel();
        let previous_states = self.states.clone();
        self.states.fill(CellState::NotRun);
        self.run_ordered(0..=index, &mut events, &mut verification);
        for (state, previous) in self.states.iter_mut().zip(previous_states).skip(index + 1) {
            if previous != CellState::NotRun {
                *state = CellState::Stale;
            }
        }
        self.finish(events, false, None)
    }

    pub fn run_all(&mut self) -> NotebookRun {
        self.begin_generation();
        self.reset_kernel();
        self.states.fill(CellState::NotRun);
        let mut events = Vec::new();
        let mut verification = VerificationSummary::default();
        self.run_ordered(0..self.cells.len(), &mut events, &mut verification);
        let clean = self.states.iter().all(|state| *state == CellState::Current);
        let manifest = clean.then(|| self.manifest(verification));
        self.finish(events, clean, manifest)
    }

    fn begin_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cancellation.store(false, Ordering::Relaxed);
    }

    fn reset_kernel(&mut self) {
        self.symbols = SymbolTable::new();
        self.interpreter = Interpreter::with_limits_and_io(
            self.symbols.clone(),
            Vec::new(),
            self.options.limits,
            Rc::clone(&self.io),
        );
        let _ = self
            .interpreter
            .set_decimal_precision(self.options.decimal_precision);
        self.interpreter
            .set_cancellation_flag(Arc::clone(&self.cancellation));
    }

    fn run_ordered<I: IntoIterator<Item = usize>>(
        &mut self,
        indices: I,
        events: &mut Vec<OutputEvent>,
        verification: &mut VerificationSummary,
    ) {
        let mut failed = false;
        for index in indices {
            if failed {
                self.states[index] = CellState::Blocked;
                continue;
            }
            self.states[index] = CellState::Queued;
            failed = !self.evaluate_cell(index, events, verification);
        }
    }

    fn evaluate_cell(
        &mut self,
        index: usize,
        events: &mut Vec<OutputEvent>,
        verification: &mut VerificationSummary,
    ) -> bool {
        self.states[index] = CellState::Running;
        let first_event = events.len();
        let cell = self.cells[index].clone();
        let cell_source = self.source[cell.span.byte_start..cell.span.byte_end].to_owned();
        let result = self.parse_submission(&cell_source).and_then(|program| {
            for statement in &program {
                self.execute_notebook_statement(&cell, statement, events, verification)?;
                // Imports can grow the interpreter's symbol table while they
                // execute. Keep subsequent independently parsed cells on the
                // same symbol IDs as the persistent environment.
                self.symbols = self.interpreter.symbols().clone();
                self.flush_stdout(&cell, events);
            }
            Ok(())
        });
        self.flush_stdout(&cell, events);
        match result {
            Ok(()) => {
                self.states[index] = CellState::Current;
                for event in &mut events[first_event..] {
                    event.state = CellState::Current;
                }
                true
            }
            Err(error) => {
                let cancelled = error.message == "execution cancelled";
                let input = error
                    .message
                    .strip_prefix("input required: ")
                    .map(str::to_owned);
                let kind = if input.is_some() {
                    OutputKind::Input
                } else {
                    OutputKind::Diagnostic
                };
                let text = input.clone().unwrap_or_else(|| error.message.clone());
                self.push_event(
                    events,
                    &cell,
                    kind,
                    text,
                    if let Some(prompt) = input {
                        json!({"prompt": prompt})
                    } else {
                        json!({"severity": "error", "limit": error.is_limit})
                    },
                );
                self.states[index] = if cancelled {
                    CellState::Cancelled
                } else if kind == OutputKind::Input {
                    CellState::Blocked
                } else {
                    CellState::Failed
                };
                for event in &mut events[first_event..] {
                    event.state = self.states[index];
                }
                false
            }
        }
    }

    fn parse_submission(&mut self, source: &str) -> Result<Vec<Statement>, RuntimeError> {
        if source.chars().count() > self.options.limits.max_source_chars {
            return Err(RuntimeError::limit(
                "cell exceeds the source character limit",
                None,
            ));
        }
        let mut scanner = Scanner::with_symbols(source, self.symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        if let Some(error) = scanner.errors().first() {
            return Err(RuntimeError::new(error.message.clone(), None));
        }
        if tokens.len() > self.options.limits.max_tokens {
            return Err(RuntimeError::limit("cell exceeds the token limit", None));
        }
        let mut nesting = 0usize;
        for token in &tokens {
            match token.token_type {
                TokenType::LeftBrace | TokenType::LeftBracket | TokenType::LeftParen => {
                    nesting += 1;
                    if nesting > self.options.limits.max_nesting {
                        return Err(RuntimeError::limit("cell exceeds the nesting limit", None));
                    }
                }
                TokenType::RightBrace | TokenType::RightBracket | TokenType::RightParen => {
                    nesting = nesting.saturating_sub(1)
                }
                _ => {}
            }
        }
        let symbols = scanner.into_symbols();
        let program = Parser::new(tokens)
            .parse()
            .map_err(|error| RuntimeError::new(error.message, None))?;
        self.symbols = symbols.clone();
        self.interpreter.set_symbols(symbols);
        Ok(program)
    }

    fn execute_notebook_statement(
        &mut self,
        cell: &NotebookCell,
        statement: &Statement,
        events: &mut Vec<OutputEvent>,
        verification: &mut VerificationSummary,
    ) -> Result<(), RuntimeError> {
        match &statement.kind {
            StatementKind::DocComment(text) => {
                self.push_event(
                    events,
                    cell,
                    OutputKind::Text,
                    text.clone(),
                    json!({"format": "scas_doc"}),
                );
                Ok(())
            }
            StatementKind::RenderLine { expression, .. } | StatementKind::Show(expression) => {
                let value = self
                    .interpreter
                    .evaluate_documented_expression(expression)?;
                let text = self.interpreter.display_value(&value);
                let latex = LatexRenderer::new(self.interpreter.symbols()).value(&value);
                self.push_event(
                    events,
                    cell,
                    OutputKind::Math,
                    text,
                    json!({"latex": latex}),
                );
                Ok(())
            }
            StatementKind::RenderBlock(statements) | StatementKind::Chain(statements) => {
                for nested in statements {
                    self.execute_notebook_statement(cell, nested, events, verification)?;
                }
                Ok(())
            }
            StatementKind::Expr(expression)
                if verification_name(expression, &self.symbols).is_some() =>
            {
                let name = verification_name(expression, &self.symbols).unwrap_or("verification");
                match self.interpreter.evaluate_expression(expression) {
                    Ok(value) => {
                        verification.passed += 1;
                        self.push_event(
                            events,
                            cell,
                            OutputKind::Verification,
                            format!("{name}: passed"),
                            json!({"status": "passed", "operation": name, "value": self.interpreter.display_value(&value)}),
                        );
                        Ok(())
                    }
                    Err(error) => {
                        verification.failed += 1;
                        self.push_event(
                            events,
                            cell,
                            OutputKind::Verification,
                            format!("{name}: failed"),
                            json!({"status": "failed", "operation": name, "message": error.message.clone()}),
                        );
                        Err(error)
                    }
                }
            }
            StatementKind::Expr(expression) => {
                let value = self.interpreter.evaluate_expression(expression)?;
                let text = self.interpreter.display_value(&value);
                let latex = LatexRenderer::new(self.interpreter.symbols()).value(&value);
                self.push_event(
                    events,
                    cell,
                    OutputKind::Value,
                    text,
                    json!({"latex": latex}),
                );
                Ok(())
            }
            _ => self.interpreter.execute_statement(statement),
        }
    }

    fn flush_stdout(&mut self, cell: &NotebookCell, events: &mut Vec<OutputEvent>) {
        let bytes = std::mem::take(self.interpreter.output_mut());
        if !bytes.is_empty() {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            self.push_event(events, cell, OutputKind::Stdout, text, json!({}));
        }
    }

    fn push_event(
        &self,
        events: &mut Vec<OutputEvent>,
        cell: &NotebookCell,
        kind: OutputKind,
        text: String,
        data: JsonValue,
    ) {
        events.push(OutputEvent {
            cell_id: cell.id.clone(),
            span: cell.span.clone(),
            generation: self.generation,
            sequence: events.len() as u64,
            kind,
            state: CellState::Running,
            text,
            data,
            duration_micros: None,
            resource_usage: None,
        });
    }

    fn synthetic_diagnostic(&self, index: usize, message: &str) -> OutputEvent {
        let span = self
            .cells
            .get(index)
            .map(|cell| cell.span.clone())
            .unwrap_or(NotebookSpan {
                byte_start: self.source.len(),
                byte_end: self.source.len(),
                line: self.source.lines().count().max(1),
                column: 1,
            });
        OutputEvent {
            cell_id: self
                .cells
                .get(index)
                .map(|cell| cell.id.clone())
                .unwrap_or_default(),
            span,
            generation: self.generation,
            sequence: 0,
            kind: OutputKind::Diagnostic,
            state: CellState::Failed,
            text: message.to_owned(),
            data: json!({"severity": "error"}),
            duration_micros: None,
            resource_usage: None,
        }
    }

    fn finish(
        &self,
        events: Vec<OutputEvent>,
        clean: bool,
        manifest: Option<ReproducibilityManifest>,
    ) -> NotebookRun {
        NotebookRun {
            generation: self.generation,
            events,
            states: self.states.clone(),
            clean,
            manifest,
        }
    }

    fn manifest(&self, verification: VerificationSummary) -> ReproducibilityManifest {
        let imports = import_paths(&self.source)
            .into_iter()
            .filter_map(|path| {
                self.io
                    .read_text(&path)
                    .ok()
                    .map(|content| ImportedModuleHash {
                        path,
                        sha256: sha256_hex(content.as_bytes()),
                    })
            })
            .collect();
        let input_data = data_paths(&self.source)
            .into_iter()
            .filter_map(|path| {
                self.io
                    .read_text(&path)
                    .ok()
                    .map(|content| ImportedModuleHash {
                        path,
                        sha256: sha256_hex(content.as_bytes()),
                    })
            })
            .collect();
        ReproducibilityManifest {
            schema: 1,
            scas_version: env!("CARGO_PKG_VERSION").to_owned(),
            target: match self.host {
                NotebookHost::Native => std::env::consts::ARCH.to_owned(),
                NotebookHost::Browser => "wasm32-browser".to_owned(),
            },
            source_sha256: sha256_hex(self.source.as_bytes()),
            imported_modules: imports,
            decimal_precision: self.options.decimal_precision.get(),
            limits: self.options.limits,
            random_seed: None,
            input_data,
            verification,
        }
    }
}

pub fn parse_cells(source: &str) -> Vec<NotebookCell> {
    if source.is_empty() {
        return Vec::new();
    }
    let markers = marker_lines(source);
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let report = Parser::new(tokens).parse_recovering();
    let mut starts = BTreeSet::from([0usize]);
    starts.extend(markers.iter().map(|(offset, _)| *offset));
    for statement in &report.statements {
        let start = statement.span.byte_start;
        let region_start = markers
            .iter()
            .rev()
            .find(|(offset, _)| *offset <= start)
            .map_or(0, |(offset, _)| *offset);
        // The first semantic statement after a marker stays with that marker.
        let has_statement_in_region = report.statements.iter().any(|previous| {
            previous.span.byte_start >= region_start && previous.span.byte_start < start
        });
        if has_statement_in_region
            || region_start == 0
                && start > 0
                && report
                    .statements
                    .iter()
                    .any(|previous| previous.span.byte_start < start)
        {
            starts.insert(start);
        }
    }
    let starts = starts
        .into_iter()
        .filter(|start| *start < source.len())
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(ordinal, start)| {
            let end = starts.get(ordinal + 1).copied().unwrap_or(source.len());
            let label = markers
                .iter()
                .find(|(offset, _)| offset == start)
                .map(|(_, label)| label.clone());
            let (line, column) = line_column(source, *start);
            let fragment = &source[*start..end];
            NotebookCell {
                id: format!("cell-{}", ordinal + 1),
                ordinal,
                label,
                kind: classify_cell(fragment),
                span: NotebookSpan {
                    byte_start: *start,
                    byte_end: end,
                    line,
                    column,
                },
            }
        })
        .collect()
}

fn marker_lines(source: &str) -> Vec<(usize, String)> {
    let mut result = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(label) = trimmed.strip_prefix("// %%") {
            result.push((offset, label.trim().to_owned()));
        }
        offset += line.len();
    }
    result
}

fn classify_cell(source: &str) -> CellKind {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let Ok(program) = Parser::new(tokens).parse() else {
        return CellKind::Code;
    };
    let prose = program
        .iter()
        .any(|statement| matches!(statement.kind, StatementKind::DocComment(_)));
    let math = program.iter().any(|statement| {
        matches!(
            statement.kind,
            StatementKind::RenderLine { .. }
                | StatementKind::RenderBlock(_)
                | StatementKind::Show(_)
        )
    });
    let verification = program.iter().any(|statement| match &statement.kind {
        StatementKind::Expr(expression) => {
            verification_name(expression, scanner.symbols()).is_some()
        }
        _ => false,
    });
    let code = program.iter().any(|statement| match &statement.kind {
        StatementKind::DocComment(_)
        | StatementKind::RenderLine { .. }
        | StatementKind::RenderBlock(_)
        | StatementKind::Show(_) => false,
        StatementKind::Expr(expression) => {
            verification_name(expression, scanner.symbols()).is_none()
        }
        _ => true,
    });
    let count = [prose, math, verification, code]
        .into_iter()
        .filter(|value| *value)
        .count();
    if count > 1 {
        CellKind::Mixed
    } else if prose {
        CellKind::Prose
    } else if math {
        CellKind::Math
    } else if verification {
        CellKind::Verification
    } else {
        CellKind::Code
    }
}

fn verification_name<'a>(expression: &Expr, symbols: &'a SymbolTable) -> Option<&'a str> {
    let ExprKind::Call { callee, .. } = &expression.kind else {
        return None;
    };
    let ExprKind::Variable(symbol) = callee.kind else {
        return None;
    };
    let name = symbols.resolve(symbol);
    matches!(name, "assert" | "verify" | "verify_chain").then_some(name)
}

fn first_changed_cell(
    old_source: &str,
    old: &[NotebookCell],
    new_source: &str,
    new: &[NotebookCell],
) -> Option<usize> {
    let shared = old.len().min(new.len());
    for index in 0..shared {
        let old_text = &old_source[old[index].span.byte_start..old[index].span.byte_end];
        let new_text = &new_source[new[index].span.byte_start..new[index].span.byte_end];
        if old_text != new_text || old[index].label != new[index].label {
            return Some(index);
        }
    }
    (old.len() != new.len()).then_some(shared)
}

fn line_column(source: &str, byte: usize) -> (usize, usize) {
    let prefix = &source[..byte];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

fn capabilities(source: &str) -> NotebookCapabilities {
    let imports = import_paths(source);
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let names = scanner.symbols();
    let has_name = |name: &str| {
        tokens.iter().any(|token| {
            token.token_type == TokenType::Identifier && names.resolve(token.symbol) == name
        })
    };
    NotebookCapabilities {
        imports,
        reads_input: has_name("input"),
        reads_files: has_name("read_text") || has_name("read_csv") || has_name("read_json"),
        writes_files: has_name("write_text"),
    }
}

fn data_paths(source: &str) -> Vec<String> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let symbols = scanner.symbols();
    let mut paths = BTreeSet::new();
    for tokens in tokens.windows(3) {
        if tokens[0].token_type != TokenType::Identifier
            || tokens[1].token_type != TokenType::LeftParen
            || tokens[2].token_type != TokenType::Str
        {
            continue;
        }
        if !matches!(
            symbols.resolve(tokens[0].symbol),
            "read_text" | "read_csv" | "read_json"
        ) {
            continue;
        }
        if let Some(Literal::Str(path)) = &tokens[2].literal {
            paths.insert(path.clone());
        }
    }
    paths.into_iter().collect()
}

fn import_paths(source: &str) -> Vec<String> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    Parser::new(tokens)
        .parse_recovering()
        .statements
        .into_iter()
        .filter_map(|statement| match statement.kind {
            StatementKind::Import(path) => Some(path),
            _ => None,
        })
        .collect()
}

fn limits_json(limits: Limits) -> JsonValue {
    json!({
        "max_source_chars": limits.max_source_chars,
        "max_tokens": limits.max_tokens,
        "max_nesting": limits.max_nesting,
        "max_steps": limits.max_steps,
        "max_call_depth": limits.max_call_depth,
        "max_exponent": limits.max_exponent,
        "max_shift": limits.max_shift,
        "max_decimal_precision": limits.max_decimal_precision,
        "max_transcendental_precision": limits.max_transcendental_precision,
        "max_decimal_scale": limits.max_decimal_scale,
        "max_number_bits": limits.max_number_bits,
        "max_matrix_elements": limits.max_matrix_elements,
    })
}

// Small dependency-free SHA-256 implementation keeps native and Wasm
// manifests identical and avoids relying on platform hashers.
fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut message = input.to_vec();
    let bit_len = (message.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_io::MemoryIo;

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn cells_preserve_source_and_boundaries() {
        let source = "/// Intro\n// %% values\nlet x = 2;\nprint(x);\n";
        let cells = parse_cells(source);
        assert!(cells.len() >= 3);
        assert_eq!(
            cells
                .iter()
                .map(|cell| &source[cell.span.byte_start..cell.span.byte_end])
                .collect::<String>(),
            source
        );
        assert_eq!(cells[1].label.as_deref(), Some("values"));
    }

    #[test]
    fn run_all_is_clean_and_cell_edits_stale_downstream() {
        let io = Rc::new(MemoryIo::new());
        let mut session = NotebookSession::with_io(
            "// %% a\nlet x = 2;\n// %% b\nprint(x + 1);\n",
            ExecutionOptions::default(),
            io,
            NotebookHost::Browser,
            NotebookTrust::Untrusted,
        );
        let run = session.run_all();
        assert!(run.clean);
        assert!(run
            .events
            .iter()
            .any(|event| event.kind == OutputKind::Stdout && event.text == "3\n"));
        assert!(run.manifest.is_some());
        session.update_source("// %% a\nlet x = 4;\n// %% b\nprint(x + 1);\n");
        assert_eq!(session.states(), &[CellState::Stale, CellState::Stale]);
    }

    #[test]
    fn failure_blocks_later_cells() {
        let mut session =
            NotebookSession::new("// %% bad\nassert(false);\n// %% later\nprint(1);\n");
        let run = session.run_all();
        assert!(!run.clean);
        assert_eq!(run.states, vec![CellState::Failed, CellState::Blocked]);
    }

    #[test]
    fn imports_keep_symbol_ids_stable_across_cells() {
        let io = Rc::new(MemoryIo::new());
        io.insert_file("module.scas", "let imported = 40;".to_owned())
            .unwrap();
        let mut session = NotebookSession::with_io(
            "// %% import\nimport \"module.scas\";\n// %% use\nprint(imported + 2);\n",
            ExecutionOptions::default(),
            io,
            NotebookHost::Browser,
            NotebookTrust::Trusted,
        );
        let run = session.run_all();
        assert!(run.clean, "{:?}", run.events);
        assert!(run
            .events
            .iter()
            .any(|event| event.kind == OutputKind::Stdout && event.text == "42\n"));
        assert_eq!(run.manifest.unwrap().imported_modules.len(), 1);
    }
}
