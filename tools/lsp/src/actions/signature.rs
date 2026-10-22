//! Signature help: the labeled spots of the command the cursor is
//! inside, with the active one.
use fipe::lex::lex;
use fipe::token::{Kind as Token, Symbol};
use lsp_types::{
    Documentation, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

use super::complete::{command, spot_details};
use crate::analysis::Analysis;
use crate::loader::Host;

/// The signature of the command written before byte `offset`.
///
/// When the command has labeled spots: one signature, `name (label:
/// type)…`, each spot a parameter with its doc line. The active spot is
/// the first the checker left open at that call, or else the count of
/// the operands written after the name.
#[must_use]
pub fn signature_help(
    analysis: &Analysis,
    host: &Host,
    offset: usize,
) -> Option<SignatureHelp> {
    let before = analysis.text.get(..offset)?;
    // A command is read on its own line, as completion reads it.
    let before = &before[before.rfind('\n').map_or(0, |at| at + 1)..];
    let command = command(before)?;
    let spots = spot_details(analysis, host, &command);
    if spots.is_empty() {
        return None;
    }
    let name = command.join(".");
    let parameters: Vec<ParameterInformation> = spots
        .iter()
        .map(|spot| ParameterInformation {
            label: ParameterLabel::Simple(format!(
                "({}: {})",
                spot.label, spot.ty
            )),
            documentation: spot.doc.clone().map(Documentation::String),
        })
        .collect();
    let label = std::iter::once(name)
        .chain(parameters.iter().map(|parameter| match &parameter.label {
            ParameterLabel::Simple(label) => label.clone(),
            ParameterLabel::LabelOffsets(_) => String::new(),
        }))
        .collect::<Vec<_>>()
        .join(" ");
    let active = bound_open(analysis, offset)
        .unwrap_or_else(|| written(before, command.len()))
        .min(spots.len().saturating_sub(1));
    let active = u32::try_from(active).unwrap_or(u32::MAX);
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: None,
            parameters: Some(parameters),
            active_parameter: Some(active),
        }],
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}

/// The first spot the checker left open at the innermost bound call
/// around `offset`, or the count it filled.
fn bound_open(analysis: &Analysis, offset: usize) -> Option<usize> {
    let at = offset.saturating_sub(1);
    analysis
        .typed
        .bound
        .iter()
        .filter(|((start, end), _)| *start <= at && at < *end)
        .min_by_key(|((start, end), _)| end - start)
        .map(|(_, bound)| {
            bound.open.first().copied().unwrap_or(bound.supplies.len())
        })
}

/// How many operands follow the command's name in `before`, at its
/// own bracket depth; the name itself is `segments` long.
fn written(before: &str, segments: usize) -> usize {
    let tokens: Vec<Token> = lex(before)
        .tokens
        .into_iter()
        .map(|token| token.kind)
        .filter(|kind| !matches!(kind, Token::Newline | Token::Eof))
        .collect();
    let _ = segments;
    // The command starts after the last opener or pipe at depth 0, as
    // `command` found it; walk forward from there.
    let mut start = 0;
    let mut depth = 0usize;
    for (index, kind) in tokens.iter().enumerate().rev() {
        match kind {
            Token::Symbol(
                Symbol::RParen | Symbol::RBrace | Symbol::RBracket,
            ) => depth += 1,
            Token::Symbol(
                Symbol::LParen | Symbol::LBrace | Symbol::LBracket,
            ) => {
                if depth == 0 {
                    start = index + 1;
                    break;
                }
                depth -= 1;
            }
            Token::Symbol(
                Symbol::Pipe
                | Symbol::PipeGood
                | Symbol::PipeBad
                | Symbol::Or
                | Symbol::And
                | Symbol::Assign
                | Symbol::FatArrow
                | Symbol::Comma,
            ) if depth == 0 => {
                start = index + 1;
                break;
            }
            _ => {}
        }
    }
    let mut count = 0;
    let mut depth = 0usize;
    for kind in tokens.iter().skip(start + 1) {
        match kind {
            Token::Symbol(
                Symbol::LParen | Symbol::LBrace | Symbol::LBracket,
            ) => {
                if depth == 0 {
                    count += 1;
                }
                depth += 1;
            }
            Token::Symbol(
                Symbol::RParen | Symbol::RBrace | Symbol::RBracket,
            ) => depth = depth.saturating_sub(1),
            _ if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}
