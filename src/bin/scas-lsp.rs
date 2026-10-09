//! SCAS language server. Start with `scas-lsp`; an LSP client communicates
//! over stdin/stdout using the standard JSON-RPC framing.

use scas::intelligence::{evaluate_hover, Occurrence, SourceIndex};
use scas::interpreter::value::Builtin;
use scas::lexer::Scanner;
use scas::parser::Parser;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, String>>,
}

impl Backend {
    async fn source(&self, uri: &Url) -> Option<String> {
        self.documents.read().await.get(uri).cloned()
    }

    async fn publish(&self, uri: Url, source: &str) {
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let mut diagnostics = Vec::new();
        for error in scanner.errors() {
            let start = byte_from_line_column(source, error.line, error.column);
            let end = source[start..]
                .char_indices()
                .nth(error.length.max(1))
                .map_or(source.len(), |(offset, _)| start + offset);
            diagnostics.push(Diagnostic {
                range: range_from_bytes(source, start, end),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("scas".to_owned()),
                message: error.message.clone(),
                ..Default::default()
            });
        }
        if diagnostics.is_empty() {
            for error in Parser::new(tokens).parse_recovering().errors {
                let start = byte_from_line_column(source, error.line, error.column);
                let end = source[start..]
                    .char_indices()
                    .nth(error.length.max(1))
                    .map_or(source.len(), |(offset, _)| start + offset);
                diagnostics.push(Diagnostic {
                    range: range_from_bytes(source, start, end),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("scas".to_owned()),
                    message: error.message,
                    ..Default::default()
                });
            }
        }
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions::default()),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                signature_help_provider: Some(SignatureHelpOptions::default()),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "scas-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "SCAS language server ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let source = params.text_document.text;
        self.documents
            .write()
            .await
            .insert(uri.clone(), source.clone());
        self.publish(uri, &source).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        self.documents
            .write()
            .await
            .insert(uri.clone(), change.text.clone());
        self.publish(uri, &change.text).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position.position);
        let prefix = source[..offset]
            .chars()
            .rev()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        let mut items = Vec::new();
        for &(name, _) in Builtin::ALL {
            if name.starts_with(&prefix) {
                items.push(CompletionItem {
                    label: name.to_owned(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    insert_text: Some(format!("{name}($0)")),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                });
            }
        }
        for function in SourceIndex::new(&source).user_functions() {
            if function.name.starts_with(&prefix) {
                items.push(CompletionItem {
                    label: function.name.clone(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(format!(
                        "fn {}({})",
                        function.name,
                        function.parameters.join(", ")
                    )),
                    ..Default::default()
                });
            }
        }
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position_params.position);
        let index = SourceIndex::new(&source);
        let occurrence = index.at(offset);
        let evaluated = evaluate_hover(&source, offset);
        let contents = if let Some(function) = occurrence.and_then(|item| {
            index
                .functions
                .iter()
                .find(|function| function.name == item.name)
        }) {
            format!("fn {}({})", function.name, function.parameters.join(", "))
        } else if let Some((_, value)) = &evaluated {
            format!("= {value}")
        } else if let Some(item) = occurrence {
            item.name.clone()
        } else {
            return Ok(None);
        };
        let span = evaluated
            .map(|(span, _)| span)
            .or_else(|| occurrence.map(|item| item.span.clone()))
            .unwrap();
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(contents)),
            range: Some(range_from_bytes(&source, span.start, span.end)),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position_params.position);
        let index = SourceIndex::new(&source);
        let definition = index
            .at(offset)
            .and_then(|item| index.definition(&item.name));
        Ok(definition.map(|item| GotoDefinitionResponse::Scalar(location(&uri, &source, item))))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position.position);
        let index = SourceIndex::new(&source);
        let Some(item) = index.at(offset) else {
            return Ok(None);
        };
        Ok(Some(
            index
                .references(&item.name)
                .into_iter()
                .filter(|reference| params.context.include_declaration || !reference.is_definition)
                .map(|reference| location(&uri, &source, reference))
                .collect(),
        ))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position.position);
        let index = SourceIndex::new(&source);
        let Some(item) = index.at(offset) else {
            return Ok(None);
        };
        let Some(spans) = index.rename(&item.name, &params.new_name) else {
            return Ok(None);
        };
        let edits = spans
            .into_iter()
            .map(|span| TextEdit {
                range: range_from_bytes(&source, span.start, span.end),
                new_text: params.new_name.clone(),
            })
            .collect();
        Ok(Some(WorkspaceEdit {
            changes: Some(HashMap::from([(uri, edits)])),
            ..Default::default()
        }))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(source) = self.source(&uri).await else {
            return Ok(None);
        };
        let offset = byte_from_position(&source, params.text_document_position_params.position);
        let Some(call) = SourceIndex::new(&source).call_at(&source, offset) else {
            return Ok(None);
        };
        let parameters = call
            .parameters
            .iter()
            .map(|name| ParameterInformation {
                label: ParameterLabel::Simple(name.clone()),
                documentation: None,
            })
            .collect();
        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: format!("{}({})", call.name, call.parameters.join(", ")),
                documentation: None,
                parameters: Some(parameters),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(call.active_parameter as u32),
        }))
    }
}

fn location(uri: &Url, source: &str, occurrence: &Occurrence) -> Location {
    Location {
        uri: uri.clone(),
        range: range_from_bytes(source, occurrence.span.start, occurrence.span.end),
    }
}

fn byte_from_line_column(source: &str, line: usize, column: usize) -> usize {
    let start = source
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    start
        + source[start..]
            .chars()
            .take(column.saturating_sub(1))
            .map(char::len_utf8)
            .sum::<usize>()
}

fn byte_from_position(source: &str, position: Position) -> usize {
    let start = source
        .split_inclusive('\n')
        .take(position.line as usize)
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    let mut utf16 = 0;
    for (offset, ch) in source[start..].char_indices() {
        if ch == '\n' || utf16 >= position.character {
            return start + offset;
        }
        utf16 += ch.len_utf16() as u32;
    }
    source.len()
}

fn position_from_byte(source: &str, byte: usize) -> Position {
    let prefix = &source[..byte.min(source.len())];
    Position {
        line: prefix.bytes().filter(|byte| *byte == b'\n').count() as u32,
        character: prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count() as u32,
    }
}

fn range_from_bytes(source: &str, start: usize, end: usize) -> Range {
    Range {
        start: position_from_byte(source, start),
        end: position_from_byte(source, end),
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: RwLock::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
