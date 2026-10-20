//! A language server for Fipe over the `fipe` crate: diagnostics from
//! every stage, go to definition, references, hover, completion,
//! rename and formatting. The server
//! holds no parser, resolver or checker of its own; it asks the crate
//! and presents the answers.
//!
//! One thread reads requests from a byte stream and answers each in
//! turn, analysing a whole document again on every change.

pub mod actions;
pub mod analysis;
pub mod format;
pub mod loader;
pub mod position;
pub mod transport;
pub mod uri;
pub mod words;

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use lsp_types::{
    CompletionParams, Diagnostic, DiagnosticSeverity, DocumentFormattingParams,
    GotoDefinitionParams, Hover, HoverContents, HoverParams, Location,
    MarkupContent, MarkupKind, PublishDiagnosticsParams, ReferenceParams,
    RenameParams, TextDocumentPositionParams, Uri,
};
use serde_json::{json, Value};

use actions::rename;
use analysis::{Analysis, Located, Source};
use fipe::resolve::{Origin, Target};
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
            "textDocument/completion" => {
                let params: CompletionParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                Ok(self.completion(&params.text_document_position))
            }
            "textDocument/rename" => {
                let params: RenameParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                self.rename(&params)
            }
            "textDocument/formatting" => {
                let params: DocumentFormattingParams =
                    serde_json::from_value(params).map_err(|e| invalid(&e))?;
                self.formatting(&params.text_document.uri)
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
                "completionProvider": {"triggerCharacters": [".", "-"]},
                "renameProvider": true,
                "documentFormattingProvider": true,
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

    /// The URI and text of a source the analysis of `document` names.
    fn place<'a>(
        &'a self,
        analysis: &'a Analysis,
        document: &Uri,
        source: &Source,
    ) -> Option<(Uri, &'a str)> {
        match source {
            Source::This => Some((document.clone(), analysis.text.as_str())),
            Source::File(index) => {
                let file = analysis.files.get(*index)?;
                Some((uri::of(&file.path), file.text.as_str()))
            }
            Source::Host(name) => {
                let (path, text) = self.host.source(name)?;
                let path = std::fs::canonicalize(path).ok()?;
                Some((uri::of(&path), text))
            }
            Source::Prelude => None,
        }
    }

    /// A located span as an LSP location.
    fn location(
        &self,
        analysis: &Analysis,
        document: &Uri,
        (source, span): &Located,
    ) -> Option<Location> {
        let (uri, text) = self.place(analysis, document, source)?;
        Some(Location::new(
            uri,
            position::range(text, *span, self.encoding),
        ))
    }

    fn definition(&self, position: &TextDocumentPositionParams) -> Value {
        let Some((analysis, offset)) = self.at(position) else {
            return Value::Null;
        };
        let targets = analysis.targets_at(offset);
        if targets.is_empty() {
            return Value::Null;
        }
        let document = &position.text_document.uri;
        let mut located: Vec<Located> = Vec::new();
        for target in &targets {
            for place in analysis.definition(&self.host, target) {
                if !located.contains(&place) {
                    located.push(place);
                }
            }
        }
        let locations: Vec<Location> = located
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
        let document = &position.text_document.uri;
        let mut located: Vec<Located> = Vec::new();
        let mut targets = analysis.targets_at(offset);
        if targets.is_empty() {
            targets.extend(analysis.declaration_at(offset));
        }
        let mut importers = Vec::new();
        for target in &targets {
            for place in analysis.references(&self.host, target, declaration) {
                if !located.contains(&place) {
                    located.push(place);
                }
            }
            // The files beside the document that import what it defines.
            let shared = shared(&analysis, target);
            if let (Some(Origin::This), Some(name)) =
                (origin(&shared), rename::old_name(&analysis.text, target))
            {
                importers.extend(self.importers(document, &shared, &name));
            }
        }
        let mut locations: Vec<Location> = located
            .iter()
            .filter_map(|located| self.location(&analysis, document, located))
            .collect();
        for (uri, text, span) in importers {
            let location =
                Location::new(uri, position::range(&text, span, self.encoding));
            if !locations.contains(&location) {
                locations.push(location);
            }
        }
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

impl Server {
    /// An open document's text, or the disk's.
    fn text_of(&self, path: &Path) -> Option<String> {
        self.documents
            .get(uri::of(path).as_str())
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    fn completion(&self, position: &TextDocumentPositionParams) -> Value {
        let Some((analysis, offset)) = self.at(position) else {
            return Value::Null;
        };
        let path = uri::path(&position.text_document.uri);
        let open =
            |path: &Path| self.documents.get(uri::of(path).as_str()).cloned();
        let reanalyse = |text: &str| {
            Analysis::new(text, path.as_deref(), &self.host, &open)
        };
        let dir = path.as_deref().and_then(Path::parent);
        let items = actions::complete::complete(
            &analysis, &self.host, dir, offset, &reanalyse,
        );
        serde_json::to_value(items).unwrap_or(Value::Null)
    }

    fn rename(&self, params: &RenameParams) -> Answer {
        let failed = |text: &str| (-32803, String::from(text));
        let position = &params.text_document_position;
        let (analysis, offset) = self
            .at(position)
            .ok_or_else(|| failed("the document is not open"))?;
        // A top-level definition's own name is the document's value or
        // type, which files beside it may import, not a local.
        let declared = analysis.declaration_at(offset);
        let target = declared
            .clone()
            .filter(|target| !matches!(target, Target::Local { .. }))
            .or_else(|| analysis.targets_at(offset).into_iter().next())
            .or(declared)
            .ok_or_else(|| failed("nothing here can be renamed"))?;
        let target = rename::renamed(&target).map_err(failed)?;
        let old = rename::old_name(&analysis.text, &target)
            .ok_or_else(|| failed("nothing here can be renamed"))?;
        let new = params.new_name.as_str();
        if !rename::valid(&old, new) {
            return Err((
                -32602,
                format!("`{new}` is not a name of the same case as `{old}`"),
            ));
        }
        let document = &position.text_document.uri;
        // Each edited document's URI, text and the spans to replace.
        let mut edits: BTreeMap<String, (String, Vec<(usize, usize)>)> =
            BTreeMap::new();
        let mut add = |uri: &Uri, text: &str, span: Option<(usize, usize)>| {
            let Some(span) = span else { return };
            let entry = edits
                .entry(uri.as_str().to_string())
                .or_insert_with(|| (String::from(text), Vec::new()));
            if !entry.1.contains(&span) {
                entry.1.push(span);
            }
        };
        for (source, span) in analysis.references(&self.host, &target, true) {
            if let Some((uri, text)) = self.place(&analysis, document, &source)
            {
                add(&uri, text, rename::name_span(text, span, &old));
            }
        }
        // The document's own `use` segments are among the references.
        let shared = shared(&analysis, &target);
        if matches!(origin(&shared), Some(Origin::This)) {
            for (uri, text, span) in self.importers(document, &shared, &old) {
                add(&uri, &text, Some(span));
            }
        }
        let changes: serde_json::Map<String, Value> = edits
            .into_iter()
            .map(|(uri, (text, mut spans))| {
                spans.sort_unstable();
                let edits: Vec<Value> = spans
                    .iter()
                    .map(|span| {
                        json!({
                            "range": position::range(&text, *span, self.encoding),
                            "newText": new,
                        })
                    })
                    .collect();
                (uri, Value::Array(edits))
            })
            .collect();
        Ok(json!({ "changes": changes }))
    }

    /// The places in the files beside `document` that import `target`,
    /// which `document` defines, by name: each file's URI, text and the
    /// span of the name.
    fn importers(
        &self,
        document: &Uri,
        target: &Target,
        old: &str,
    ) -> Vec<(Uri, String, (usize, usize))> {
        let Some(path) = uri::path(document) else {
            return Vec::new();
        };
        let (Some(stem), Some(dir)) =
            (path.file_stem().and_then(|s| s.to_str()), path.parent())
        else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut siblings: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|other| {
                other.extension().is_some_and(|ext| ext == "fipe")
                    && *other != path
            })
            .collect();
        siblings.sort();
        let open =
            |path: &Path| self.documents.get(uri::of(path).as_str()).cloned();
        let imported = analysis::with_origin(target, Origin::File(stem.into()));
        for sibling in siblings {
            let Some(text) = self.text_of(&sibling) else {
                continue;
            };
            let other = Analysis::new(&text, Some(&sibling), &self.host, &open);
            if !loader::uses(&other.module).any(|(used, _)| used == stem) {
                continue;
            }
            let uri = uri::of(&sibling);
            for node in other.resolved.uses(&imported) {
                let span = Analysis::span(&other.module, node)
                    .and_then(|span| rename::name_span(&other.text, span, old));
                if let Some(span) = span {
                    found.push((uri.clone(), other.text.clone(), span));
                }
            }
            for span in analysis::import_spans(&other.resolved, &imported) {
                found.push((uri.clone(), other.text.clone(), span));
            }
        }
        found
    }

    fn formatting(&self, document: &Uri) -> Answer {
        let failed = |text: &str| (-32803, String::from(text));
        let analysis = self
            .analyse(document.as_str())
            .ok_or_else(|| failed("the document is not open"))?;
        if !analysis.diagnostics.is_empty() {
            return Err(failed("a file with errors is not formatted"));
        }
        let text = &analysis.text;
        let formatted = format::format(text).map_err(failed)?;
        if formatted == *text {
            return Ok(json!([]));
        }
        let end = position::position(text, text.len(), self.encoding);
        Ok(json!([{
            "range": {"start": {"line": 0, "character": 0}, "end": end},
            "newText": formatted,
        }]))
    }
}

/// What other files know `target` as: a top-level binding is a local
/// of the document, and the value its export names for the files that
/// import it.
fn shared(analysis: &Analysis, target: &Target) -> Target {
    match target {
        Target::Local { site } => analysis
            .resolved
            .exports
            .iter()
            .find(|export| export.span == *site)
            .map_or_else(|| target.clone(), |export| export.target.clone()),
        _ => target.clone(),
    }
}

/// The origin of a declared target.
const fn origin(target: &Target) -> Option<&Origin> {
    match target {
        Target::Value { origin, .. }
        | Target::Constructor { origin, .. }
        | Target::Type { origin, .. } => Some(origin),
        _ => None,
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
