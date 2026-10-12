//! An in-worker LSP endpoint for the browser IDE. Messages use JSON-RPC 2.0
//! shapes; the outer worker transport is intentionally separate from execution.

use crate::intelligence::{evaluate_hover, SourceIndex};
use crate::interpreter::value::Builtin;
use crate::lexer::Scanner;
use crate::parser::Parser;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static DOCUMENTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

fn position(source: &str, byte: usize) -> Value {
    let prefix = &source[..byte.min(source.len())];
    json!({
        "line": prefix.bytes().filter(|byte| *byte == b'\n').count(),
        "character": prefix.rsplit('\n').next().unwrap_or("").encode_utf16().count(),
    })
}

fn offset(source: &str, position: &Value) -> usize {
    let line = position["line"].as_u64().unwrap_or(0) as usize;
    let character = position["character"].as_u64().unwrap_or(0) as usize;
    let start = source
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    let mut utf16 = 0;
    for (index, ch) in source[start..].char_indices() {
        if ch == '\n' || utf16 + ch.len_utf16() > character {
            return start + index;
        }
        utf16 += ch.len_utf16();
    }
    source.len()
}

fn range(source: &str, start: usize, end: usize) -> Value {
    json!({"start": position(source, start), "end": position(source, end)})
}

fn location(uri: &str, source: &str, start: usize, end: usize) -> Value {
    json!({"uri": uri, "range": range(source, start, end)})
}

fn diagnostic_range(source: &str, line: usize, column: usize, length: usize) -> Value {
    let line_start = source
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    let start = line_start
        + source[line_start..]
            .chars()
            .take(column.saturating_sub(1))
            .take_while(|ch| *ch != '\n')
            .map(char::len_utf8)
            .sum::<usize>();
    let end = source[start..]
        .char_indices()
        .nth(length.max(1))
        .map_or(source.len(), |(index, _)| start + index);
    range(source, start, end)
}

fn diagnostics(uri: &str, source: &str, version: &Value) -> Value {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut items = scanner
        .errors()
        .iter()
        .map(|error| {
            json!({
                "range": diagnostic_range(source, error.line, error.column, error.length),
                "severity": 1, "source": "scas", "message": error.message,
            })
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        items = Parser::new(tokens)
            .parse_recovering()
            .errors
            .into_iter()
            .map(|error| {
                json!({
                    "range": diagnostic_range(source, error.line, error.column, error.length),
                    "severity": 1, "source": "scas", "message": error.message,
                })
            })
            .collect();
    }
    items.truncate(100);
    json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
        "uri": uri, "version": version, "diagnostics": items,
    }})
}

/// Process one LSP message and return its response and any notifications.
/// Notifications have no JSON-RPC response, as required by the protocol.
pub fn process(message: &Value) -> Value {
    let method = message["method"].as_str().unwrap_or("");
    let params = &message["params"];
    let id = &message["id"];
    let mut notifications = Vec::new();
    let result = match method {
        "initialize" => json!({"capabilities": {
            "textDocumentSync": 1,
            "completionProvider": {}, "hoverProvider": true,
            "definitionProvider": true, "referencesProvider": true,
            "renameProvider": true, "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
        }, "serverInfo": {"name": "scas-web-lsp", "version": env!("CARGO_PKG_VERSION")}}),
        "shutdown" => Value::Null,
        "textDocument/didOpen" => {
            let doc = &params["textDocument"];
            if let (Some(uri), Some(source)) = (doc["uri"].as_str(), doc["text"].as_str()) {
                if source.chars().count() <= crate::web_runtime::browser_limits().max_source_chars {
                    DOCUMENTS
                        .with(|docs| docs.borrow_mut().insert(uri.to_owned(), source.to_owned()));
                    notifications.push(diagnostics(uri, source, &doc["version"]));
                }
            }
            Value::Null
        }
        "textDocument/didChange" => {
            let doc = &params["textDocument"];
            if let (Some(uri), Some(source)) = (
                doc["uri"].as_str(),
                params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str()),
            ) {
                if source.chars().count() <= crate::web_runtime::browser_limits().max_source_chars {
                    DOCUMENTS
                        .with(|docs| docs.borrow_mut().insert(uri.to_owned(), source.to_owned()));
                    notifications.push(diagnostics(uri, source, &doc["version"]));
                }
            }
            Value::Null
        }
        "textDocument/didClose" => {
            if let Some(uri) = params["textDocument"]["uri"].as_str() {
                DOCUMENTS.with(|docs| docs.borrow_mut().remove(uri));
                notifications.push(json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": []}}));
            }
            Value::Null
        }
        _ => {
            let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
            DOCUMENTS.with(|docs| {
                let docs = docs.borrow();
                let Some(source) = docs.get(uri) else { return Value::Null; };
                let byte = offset(source, &params["position"]);
                let index = SourceIndex::new(source);
                let occurrence = index.at(byte);
                match method {
                    "textDocument/completion" => {
                        let prefix = source[..byte].chars().rev().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect::<String>().chars().rev().collect::<String>();
                        let mut items = Builtin::ALL.iter().filter(|(name, _)| name.starts_with(&prefix)).map(|(name, _)| json!({
                            "label": name, "kind": 3, "insertText": format!("{name}($0)"), "insertTextFormat": 2,
                        })).collect::<Vec<_>>();
                        items.extend(index.user_functions().filter(|function| function.name.starts_with(&prefix)).map(|function| json!({
                            "label": function.name, "kind": 3,
                            "detail": format!("fn {}({})", function.name, function.parameters.join(", ")),
                        })));
                        json!(items)
                    }
                    "textDocument/hover" => {
                        let evaluated = evaluate_hover(source, byte);
                        let function = occurrence.and_then(|item| index.functions.iter().find(|function| function.name == item.name));
                        let contents = if let Some(function) = function {
                            format!("fn {}({})", function.name, function.parameters.join(", "))
                        } else if let Some((_, value)) = &evaluated {
                            format!("= {value}")
                        } else if let Some(item) = occurrence {
                            item.name.clone()
                        } else { return Value::Null; };
                        let span = evaluated.map(|(span, _)| span).or_else(|| occurrence.map(|item| item.span.clone())).unwrap();
                        json!({"contents": contents, "range": range(source, span.start, span.end)})
                    }
                    "textDocument/definition" => occurrence.and_then(|item| index.definition(&item.name))
                        .map(|item| location(uri, source, item.span.start, item.span.end)).unwrap_or(Value::Null),
                    "textDocument/references" => occurrence.map(|item| index.references(&item.name).into_iter()
                        .filter(|item| params["context"]["includeDeclaration"].as_bool().unwrap_or(false) || !item.is_definition)
                        .map(|item| location(uri, source, item.span.start, item.span.end)).collect::<Vec<_>>())
                        .map_or(Value::Null, |items| json!(items)),
                    "textDocument/rename" => occurrence.and_then(|item| index.rename(&item.name, params["newName"].as_str().unwrap_or("")))
                        .map(|spans| json!({"changes": {uri: spans.into_iter().map(|span| json!({
                            "range": range(source, span.start, span.end), "newText": params["newName"],
                        })).collect::<Vec<_>>()}})).unwrap_or(Value::Null),
                    "textDocument/signatureHelp" => index.call_at(source, byte).map(|call| json!({
                        "signatures": [{"label": format!("{}({})", call.name, call.parameters.join(", ")),
                            "parameters": call.parameters.iter().map(|name| json!({"label": name})).collect::<Vec<_>>() }],
                        "activeSignature": 0, "activeParameter": call.active_parameter,
                    })).unwrap_or(Value::Null),
                    _ => Value::Null,
                }
            })
        }
    };
    let response = if id.is_null() {
        Value::Null
    } else {
        json!({"jsonrpc": "2.0", "id": id, "result": result})
    };
    json!({"kind": "lsp", "response": response, "notifications": notifications})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_lifecycle_and_unicode_positions() {
        assert_eq!(offset("a🧮b", &json!({"line": 0, "character": 3})), 5);
        assert_eq!(position("a🧮b", 5), json!({"line": 0, "character": 3}));
        let uri = "file:///unicode.scas";
        let source = "// 🧮\nlet value = 3;\nprint(value);";
        let opened = process(
            &json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
                "textDocument": {"uri": uri, "languageId": "scas", "version": 1, "text": source}
            }}),
        );
        assert!(opened["response"].is_null());
        assert!(opened["notifications"][0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty());

        let definition = process(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "textDocument/definition", "params": {
                "textDocument": {"uri": uri}, "position": {"line": 2, "character": 8}
            }}),
        );
        assert_eq!(
            definition["response"]["result"]["range"]["start"],
            json!({"line": 1, "character": 4})
        );

        let rename = process(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/rename", "params": {
                "textDocument": {"uri": uri}, "position": {"line": 2, "character": 8}, "newName": "result"
            }}),
        );
        assert_eq!(
            rename["response"]["result"]["changes"][uri]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let changed = process(
            &json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
                "textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "let x = ;"}]
            }}),
        );
        assert_eq!(changed["notifications"][0]["params"]["version"], 2);
        assert!(!changed["notifications"][0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty());

        let closed = process(
            &json!({"jsonrpc": "2.0", "method": "textDocument/didClose", "params": {
                "textDocument": {"uri": uri}
            }}),
        );
        assert!(closed["notifications"][0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty());
        let after_close = process(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/definition", "params": {
                "textDocument": {"uri": uri}, "position": {"line": 0, "character": 4}
            }}),
        );
        assert!(after_close["response"]["result"].is_null());
    }
}
