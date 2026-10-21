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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
use transport::Frame;

/// What the server starts with; the client's `initializationOptions`
/// may name another host directory (`{"host": "<dir>"}`).
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The directory of the host's declaration files.
    pub host: Option<PathBuf>,
}

/// Where the server is in the protocol's lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Before `initialize`: every request but it is refused.
    Waiting,
    Running,
    /// After `shutdown`: every request is refused; `exit` ends cleanly.
    ShutDown,
}

/// The server's state between messages.
struct Server {
    host: Host,
    host_dir: Option<PathBuf>,
    encoding: Encoding,
    phase: Phase,
    /// Open documents' texts, by URI.
    documents: BTreeMap<String, String>,
    /// Each open document's analysis, kept until a document or a file
    /// on disk changes: requests between changes share it.
    analyses: RefCell<BTreeMap<String, Rc<Analysis>>>,
}

/// Serve the protocol over `input` and `output` until `exit` or the end
/// of the input; whether the client shut the server down first.
///
/// # Errors
/// A transport failure, or a header without a length: the framing is
/// lost.
pub fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    options: &Options,
) -> io::Result<bool> {
    let mut server = Server {
        host: loader::host(options.host.as_deref()),
        host_dir: options.host.clone(),
        encoding: Encoding::Utf16,
        phase: Phase::Waiting,
        documents: BTreeMap::new(),
        analyses: RefCell::new(BTreeMap::new()),
    };
    while let Some(frame) = transport::frame(input)? {
        let message = match frame {
            Frame::Message(message) => message,
            Frame::TooLong(length) => {
                let text = format!("a message of {length} bytes is too long");
                transport::write(output, &error(&Value::Null, -32700, &text))?;
                continue;
            }
            Frame::Unparsable(reason) => {
                let text = format!("parse error: {reason}");
                transport::write(output, &error(&Value::Null, -32700, &text))?;
                continue;
            }
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response from the client, or no message at all.
            if message.get("result").is_none() && message.get("error").is_none()
            {
                let id = id.unwrap_or(Value::Null);
                let text = "invalid request: no method";
                transport::write(output, &error(&id, -32600, text))?;
            }
            continue;
        };
        if method == "exit" {
            return Ok(server.phase == Phase::ShutDown);
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match id {
            Some(id) => {
                let reply = match server.request(method, params) {
                    Ok(result) => {
                        json!({"jsonrpc": "2.0", "id": id, "result": result})
                    }
                    Err((code, text)) => error(&id, code, &text),
                };
                transport::write(output, &reply)?;
            }
            None if server.phase == Phase::Running => {
                for published in server.notification(method, &params) {
                    transport::write(output, &published)?;
                }
            }
            None => {}
        }
    }
    Ok(server.phase == Phase::ShutDown)
}

/// An error response.
fn error(id: &Value, code: i64, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": text},
    })
}

type Answer = Result<Value, (i64, String)>;

fn invalid(error: &serde_json::Error) -> (i64, String) {
    (-32602, format!("invalid params: {error}"))
}

impl Server {
    fn request(&mut self, method: &str, params: Value) -> Answer {
        match (self.phase, method) {
            (Phase::Waiting, "initialize") => {
                self.phase = Phase::Running;
                return Ok(self.initialize(&params));
            }
            (Phase::Waiting, _) => {
                return Err((
                    -32002,
                    String::from("the server is not initialized"),
                ));
            }
            (Phase::ShutDown, _) => {
                return Err((-32600, String::from("the server is shut down")));
            }
            (Phase::Running, "initialize") => {
                return Err((
                    -32600,
                    String::from("the server is already initialized"),
                ));
            }
            (Phase::Running, "shutdown") => {
                self.phase = Phase::ShutDown;
                return Ok(Value::Null);
            }
            (Phase::Running, _) => {}
        }
        match method {
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
                "textDocumentSync": 2,
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
        if method == "workspace/didChangeWatchedFiles" {
            // A file changed on disk: every analysis may read it.
            self.analyses.borrow_mut().clear();
            let open: Vec<String> = self.documents.keys().cloned().collect();
            return open.iter().filter_map(|uri| self.diagnose(uri)).collect();
        }
        let uri = params["textDocument"]["uri"].as_str().map(String::from);
        let Some(uri) = uri else { return Vec::new() };
        match method {
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"].as_str();
                self.documents
                    .insert(uri.clone(), String::from(text.unwrap_or("")));
            }
            "textDocument/didChange" => {
                let Some(text) = self.documents.get(&uri) else {
                    return Vec::new();
                };
                let mut text = text.clone();
                for change in
                    params["contentChanges"].as_array().into_iter().flatten()
                {
                    self.apply(&mut text, change);
                }
                self.documents.insert(uri.clone(), text);
            }
            "textDocument/didClose" => {
                self.documents.remove(&uri);
            }
            _ => return Vec::new(),
        }
        // Any analysis may read the changed document as a file module.
        self.analyses.borrow_mut().clear();
        let mut published = Vec::new();
        if method == "textDocument/didClose" {
            published.extend(publish(&uri, Vec::new()));
        } else {
            published.extend(self.diagnose(&uri));
        }
        // The other open documents that use the changed one.
        let changed = uri.parse::<Uri>().ok().and_then(|uri| uri::path(&uri));
        if let Some(changed) = changed {
            let others: Vec<String> = self
                .documents
                .keys()
                .filter(|other| **other != uri)
                .cloned()
                .collect();
            for other in others {
                let uses = self.analyse(&other).is_some_and(|analysis| {
                    analysis.files.iter().any(|file| file.path == changed)
                });
                if uses {
                    published.extend(self.diagnose(&other));
                }
            }
        }
        published
    }

    /// Apply one content change to `text`: a range replaced, or the
    /// whole text when the change has no range.
    fn apply(&self, text: &mut String, change: &Value) {
        let Some(new) = change["text"].as_str() else {
            return;
        };
        let range =
            serde_json::from_value::<lsp_types::Range>(change["range"].clone());
        match range {
            Ok(range) => {
                let start = position::offset(text, range.start, self.encoding);
                let end = position::offset(text, range.end, self.encoding);
                if start <= end {
                    text.replace_range(start..end, new);
                }
            }
            Err(_) => *text = String::from(new),
        }
    }

    /// The diagnostics of open document `uri`, as a notification.
    fn diagnose(&self, uri: &str) -> Option<Value> {
        let analysis = self.analyse(uri)?;
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
        publish(uri, diagnostics)
    }

    /// The analysis of an open document, made once per change.
    fn analyse(&self, uri: &str) -> Option<Rc<Analysis>> {
        if let Some(kept) = self.analyses.borrow().get(uri) {
            return Some(Rc::clone(kept));
        }
        let text = self.documents.get(uri)?;
        let path = uri.parse::<Uri>().ok().and_then(|uri| uri::path(&uri));
        let open =
            |path: &Path| self.documents.get(uri::of(path).as_str()).cloned();
        let analysis =
            Rc::new(Analysis::new(text, path.as_deref(), &self.host, &open));
        self.analyses
            .borrow_mut()
            .insert(String::from(uri), Rc::clone(&analysis));
        Some(analysis)
    }

    /// The analysis and byte offset of a position, for a request about
    /// the name there: a position just after a name is on it, as an
    /// editor's cursor at a word's end is.
    fn at(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Option<(Rc<Analysis>, usize)> {
        let analysis = self.analyse(position.text_document.uri.as_str())?;
        let offset =
            position::offset(&analysis.text, position.position, self.encoding);
        let offset = analysis.on_name(offset);
        Some((analysis, offset))
    }

    /// The analysis and byte offset of a position, unadjusted.
    fn at_exactly(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Option<(Rc<Analysis>, usize)> {
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
            Source::Prelude => {
                let path = prelude_file()?;
                Some((uri::of(&path), fipe::decl::prelude_source()))
            }
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
        let text = actions::complete::spot_hover(&analysis, &self.host, offset)
            .or_else(|| analysis.hover(&self.host, offset));
        let Some(text) = text else {
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
        let Some((analysis, offset)) = self.at_exactly(position) else {
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
        // At a member chain, only its head's segment renames its head.
        let on_head = analysis.target_at(offset).is_none_or(|(node, _)| {
            let start = Analysis::span(&analysis.module, node).map(|s| s.0);
            analysis::name_at(&analysis.text, offset).map(|s| s.0) == start
        });
        let target = rename::renamed(&target, on_head).map_err(failed)?;
        let old = rename::old_name(&analysis.text, &target)
            .ok_or_else(|| failed("nothing here can be renamed"))?;
        let new = params.new_name.as_str();
        if !rename::valid(&old, new) {
            return Err((
                -32602,
                format!("`{new}` is not a name of the same case as `{old}`"),
            ));
        }
        // A name already written in the file might be captured or
        // shadow what the renamed name meant.
        if new != old && analysis.mentions(new) {
            return Err((
                -32602,
                format!("`{new}` is already used in this file"),
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

/// A file holding the prelude's source, which has none of its own on
/// disk, so a client can open a prelude declaration: written once per
/// server version under the system's temporary directory.
fn prelude_file() -> Option<PathBuf> {
    let dir = std::env::temp_dir()
        .join(format!("fipe-lsp-{}", env!("CARGO_PKG_VERSION")));
    let path = dir.join("prelude.fipe");
    let source = fipe::decl::prelude_source();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(source) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, source).ok()?;
    }
    std::fs::canonicalize(path).ok()
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

/// A `textDocument/publishDiagnostics` notification; none for a URI
/// that does not parse.
fn publish(uri: &str, diagnostics: Vec<Diagnostic>) -> Option<Value> {
    let uri = uri.parse::<Uri>().ok()?;
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    Some(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": params,
    }))
}
