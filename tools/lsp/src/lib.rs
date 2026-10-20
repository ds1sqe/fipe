//! A language server for Fipe over the `fipe` crate: diagnostics from
//! every stage, go to definition, references and hover. The server
//! holds no parser, resolver or checker of its own; it asks the crate
//! and presents the answers.
//!
//! One thread reads requests from a byte stream and answers each in
//! turn, analysing a whole document again on every change.

pub mod analysis;
pub mod loader;
pub mod position;
pub mod transport;
pub mod uri;
pub mod words;

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use lsp_types::{
    Diagnostic, DiagnosticSeverity, GotoDefinitionParams, Hover, HoverContents,
    HoverParams, Location, MarkupContent, MarkupKind, PublishDiagnosticsParams,
    ReferenceParams, TextDocumentPositionParams, Uri,
};
use serde_json::{json, Value};

use analysis::{Analysis, Located, Source};
use loader::Host;
use position::Encoding;

/// What the server starts with; the client's `initializationOptions`
/// may name another host directory (`{"host": "<dir>"}`).
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The directory of the host's declaration files.
    pub host: Option<PathBuf>,
}

/// The server's state between messages.
struct Server {
    host: Host,
    host_dir: Option<PathBuf>,
    encoding: Encoding,
    /// Open documents' texts, by URI.
    documents: BTreeMap<String, String>,
}

/// Serve the protocol over `input` and `output` until `exit` or the end
/// of the input.
///
/// # Errors
/// A transport failure.
pub fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    options: &Options,
) -> io::Result<()> {
    let mut server = Server {
        host: loader::host(options.host.as_deref()),
        host_dir: options.host.clone(),
        encoding: Encoding::Utf16,
        documents: BTreeMap::new(),
    };
    while let Some(message) = transport::read(input)? {
        let method = message["method"].as_str().unwrap_or_default();
        if method == "exit" {
            break;
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match message.get("id").cloned() {
            Some(id) => {
                let reply = match server.request(method, params) {
                    Ok(result) => {
                        json!({"jsonrpc": "2.0", "id": id, "result": result})
                    }
                    Err((code, text)) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": code, "message": text},
                    }),
                };
                transport::write(output, &reply)?;
            }
            None => {
                for published in server.notification(method, &params) {
                    transport::write(output, &published)?;
                }
            }
        }
    }
    Ok(())
}

type Answer = Result<Value, (i64, String)>;

fn invalid(error: &serde_json::Error) -> (i64, String) {
    (-32602, format!("invalid params: {error}"))
}

impl Server {
    fn request(&mut self, method: &str, params: Value) -> Answer {
        match method {
            "initialize" => Ok(self.initialize(&params)),
            "shutdown" => Ok(Value::Null),
            "textDocument/definition" => {
                let params: GotoDefinitionParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                Ok(self.definition(&params.text_document_position_params))
            }
            "textDocument/references" => {
                let params: ReferenceParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                Ok(self.references(
                    &params.text_document_position,
                    params.context.include_declaration,
                ))
            }
            "textDocument/hover" => {
                let params: HoverParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                Ok(self.hover(&params.text_document_position_params))
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        if let Some(dir) = params["initializationOptions"]["host"].as_str() {
            self.host_dir = Some(PathBuf::from(dir));
            self.host = loader::host(self.host_dir.as_deref());
        }
        let offered = &params["capabilities"]["general"]["positionEncodings"];
        let utf8 = offered
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "utf-8"));
        self.encoding = if utf8 {
            Encoding::Utf8
        } else {
            Encoding::Utf16
        };
        json!({
            "capabilities": {
                "positionEncoding": if utf8 { "utf-8" } else { "utf-16" },
                "textDocumentSync": 1,
                "definitionProvider": true,
                "referencesProvider": true,
                "hoverProvider": true,
            },
            "serverInfo": {
                "name": "fipe-lsp",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })
    }

    /// Handle a notification; the messages it publishes.
    fn notification(&mut self, method: &str, params: &Value) -> Vec<Value> {
        let uri = params["textDocument"]["uri"].as_str().map(String::from);
        let Some(uri) = uri else { return Vec::new() };
        match method {
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"].as_str();
                self.documents
                    .insert(uri.clone(), String::from(text.unwrap_or("")));
            }
            "textDocument/didChange" => {
                // Full synchronization: the last change is the text.
                let changes = params["contentChanges"].as_array();
                let text = changes
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str());
                if let Some(text) = text {
                    self.documents.insert(uri.clone(), String::from(text));
                }
            }
            "textDocument/didClose" => {
                self.documents.remove(&uri);
                return vec![publish(&uri, Vec::new())];
            }
            _ => return Vec::new(),
        }
        let Some(analysis) = self.analyse(&uri) else {
            return Vec::new();
        };
        let diagnostics = analysis
            .diagnostics
            .iter()
            .map(|(span, message)| Diagnostic {
                range: position::range(&analysis.text, *span, self.encoding),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(String::from("fipe")),
                message: message.clone(),
                ..Diagnostic::default()
            })
            .collect();
        vec![publish(&uri, diagnostics)]
    }

    /// The analysis of an open document.
    fn analyse(&self, uri: &str) -> Option<Analysis> {
        let text = self.documents.get(uri)?;
        let path = uri.parse::<Uri>().ok().and_then(|uri| uri::path(&uri));
        let open =
            |path: &Path| self.documents.get(uri::of(path).as_str()).cloned();
        Some(Analysis::new(text, path.as_deref(), &self.host, &open))
    }

    /// The analysis and byte offset of a position.
    fn at(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Option<(Analysis, usize)> {
        let analysis = self.analyse(position.text_document.uri.as_str())?;
        let offset =
            position::offset(&analysis.text, position.position, self.encoding);
        Some((analysis, offset))
    }

    /// A located span as an LSP location.
    fn location(
        &self,
        analysis: &Analysis,
        document: &Uri,
        (source, span): &Located,
    ) -> Option<Location> {
        let (uri, text) = match source {
            Source::This => (document.clone(), analysis.text.as_str()),
            Source::File(index) => {
                let file = analysis.files.get(*index)?;
                (uri::of(&file.path), file.text.as_str())
            }
            Source::Host(name) => {
                let (path, text) = self.host.source(name)?;
                let path = std::fs::canonicalize(path).ok()?;
                (uri::of(&path), text)
            }
            Source::Prelude => return None,
        };
        Some(Location::new(
            uri,
            position::range(text, *span, self.encoding),
        ))
    }

    fn definition(&self, position: &TextDocumentPositionParams) -> Value {
        let Some((analysis, offset)) = self.at(position) else {
            return Value::Null;
        };
        let Some((_, target)) = analysis.target_at(offset) else {
            return Value::Null;
        };
        let document = &position.text_document.uri;
        let locations: Vec<Location> = analysis
            .definition(&self.host, &target)
            .iter()
            .filter_map(|located| self.location(&analysis, document, located))
            .collect();
        serde_json::to_value(locations).unwrap_or(Value::Null)
    }

    fn references(
        &self,
        position: &TextDocumentPositionParams,
        declaration: bool,
    ) -> Value {
        let Some((analysis, offset)) = self.at(position) else {
            return Value::Null;
        };
        let Some((_, target)) = analysis.target_at(offset) else {
            return json!([]);
        };
        let document = &position.text_document.uri;
        let locations: Vec<Location> = analysis
            .references(&self.host, &target, declaration)
            .iter()
            .filter_map(|located| self.location(&analysis, document, located))
            .collect();
        serde_json::to_value(locations).unwrap_or(Value::Null)
    }

    fn hover(&self, position: &TextDocumentPositionParams) -> Value {
        let Some((analysis, offset)) = self.at(position) else {
            return Value::Null;
        };
        let Some(text) = analysis.hover(&self.host, offset) else {
            return Value::Null;
        };
        let hover = Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```fipe\n{text}\n```"),
            }),
            range: None,
        };
        serde_json::to_value(hover).unwrap_or(Value::Null)
    }
}

/// A `textDocument/publishDiagnostics` notification.
fn publish(uri: &str, diagnostics: Vec<Diagnostic>) -> Value {
    let Ok(uri) = uri.parse::<Uri>() else {
        return Value::Null;
    };
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": params,
    })
}
