use fipe_repl_core::{
    source_continuation, ContinuationReason, SourceContinuation,
};

#[test]
fn observed_1_01_complete() {
    assert_eq!(source_continuation("1 + 1"), SourceContinuation::Complete);
}

#[test]
fn observed_1_02_open_list() {
    assert_eq!(
        source_continuation("[1,\n"),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
}

#[test]
fn observed_1_03_repaired_list() {
    assert_eq!(source_continuation("[1,\n2]"), SourceContinuation::Complete);
}

#[test]
fn observed_1_04_open_block() {
    assert_eq!(
        source_continuation("{\nx = 1\n"),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
}

#[test]
fn observed_1_05_repaired_block() {
    assert_eq!(
        source_continuation("{\nx = 1\nx\n}"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_06_open_doc() {
    assert_eq!(
        source_continuation("## Value.\n"),
        SourceContinuation::Open(ContinuationReason::Documentation)
    );
}

#[test]
fn observed_1_07_repaired_doc() {
    assert_eq!(
        source_continuation("## Value.\nx = 7"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_08_open_raw() {
    assert_eq!(
        source_continuation("'first\n"),
        SourceContinuation::Open(ContinuationReason::Quote)
    );
}

#[test]
fn observed_1_09_repaired_raw() {
    assert_eq!(
        source_continuation("'first\nlast'"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_10_crossed_closers() {
    assert_eq!(source_continuation("([)"), SourceContinuation::Complete);
}

#[test]
fn observed_1_11_invalid_open_prefix() {
    assert_eq!(source_continuation("(1 + *"), SourceContinuation::Complete);
}

#[test]
fn observed_1_12_incomplete_ordinary_operator() {
    assert_eq!(source_continuation("1 +"), SourceContinuation::Complete);
}

#[test]
fn observed_1_13_open_parameter_default() {
    assert_eq!(
        source_continuation("f = (copy ="),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
}

#[test]
fn observed_1_14_double_open_ordinary() {
    assert_eq!(source_continuation("\"first"), SourceContinuation::Complete);
}

#[test]
fn observed_1_15_double_escaped_lf() {
    assert_eq!(
        source_continuation("\"first \\\n"),
        SourceContinuation::Open(ContinuationReason::Quote)
    );
}

#[test]
fn observed_1_16_double_escaped_repaired() {
    assert_eq!(
        source_continuation("\"first \\\nlast\""),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_17_join_eof() {
    assert_eq!(
        source_continuation("1 \\"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_18_join_lf() {
    assert_eq!(
        source_continuation("1 \\\n"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_19_join_blank() {
    assert_eq!(
        source_continuation("1 \\\n\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_20_join_spaced_crlf() {
    assert_eq!(
        source_continuation("1 \\ \t\r\n"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_21_pipe_eof() {
    assert_eq!(
        source_continuation("1 |"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_22_pipe_lf() {
    assert_eq!(
        source_continuation("1 |\n"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_23_pipe_blank() {
    assert_eq!(source_continuation("1 |\n\n"), SourceContinuation::Complete);
}

#[test]
fn observed_1_24_pipe_comment_eof() {
    assert_eq!(
        source_continuation("1 | # tail"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_25_pipe_comment_lf() {
    assert_eq!(
        source_continuation("1 | # tail\n"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_26_pipe_comment_blank() {
    assert_eq!(
        source_continuation("1 | # tail\n\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_27_join_comment_eof() {
    assert_eq!(
        source_continuation("1 \\\n# tail"),
        SourceContinuation::Open(ContinuationReason::LineContinuation)
    );
}

#[test]
fn observed_1_28_join_comment_lf() {
    assert_eq!(
        source_continuation("1 \\\n# tail\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_29_join_comment_blank() {
    assert_eq!(
        source_continuation("1 \\\n# tail\n\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_30_comment_lookalikes() {
    assert_eq!(
        source_continuation("# ([ \\ \"\n1"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_31_string_lookalikes() {
    assert_eq!(
        source_continuation("'([ \\ # text'"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_1_32_unicode_open_list() {
    assert_eq!(
        source_continuation("[\"한\",\n"),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
}

#[test]
fn observed_2_01_double_bare_escape_eof() {
    assert_eq!(
        source_continuation("\"first\\"),
        SourceContinuation::Open(ContinuationReason::Quote)
    );
}

#[test]
fn observed_2_02_double_bare_escape_repaired() {
    assert_eq!(
        source_continuation("\"first\\\nlast\""),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_03_double_escaped_backslash_eof() {
    assert_eq!(
        source_continuation("\"first\\\\"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_04_double_joined_indent() {
    assert_eq!(
        source_continuation("\"first\\\n   "),
        SourceContinuation::Open(ContinuationReason::Quote)
    );
}

#[test]
fn observed_2_05_double_joined_nonempty() {
    assert_eq!(
        source_continuation("\"first\\\nlast"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_06_open_default_group() {
    assert_eq!(
        source_continuation("f = (copy =\n"),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
}

#[test]
fn observed_2_07_default_group_repaired() {
    assert_eq!(
        source_continuation("f = (copy =\n7) => copy"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_08_pipe_doc_lf() {
    assert_eq!(
        source_continuation("1 | ## note\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_09_double_bad_escape() {
    assert_eq!(
        source_continuation("\"first\\q"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_10_double_ordinary_lf() {
    assert_eq!(
        source_continuation("\"first\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn observed_2_11_double_joined_indent_repaired() {
    assert_eq!(
        source_continuation("\"first\\\n   last\""),
        SourceContinuation::Complete
    );
}

#[test]
fn a_rejected_default_repair_does_not_imply_open_promises_a_valid_suffix() {
    let prefix = "f = (copy =\n";
    let rejected = "f = (copy =\n7) => copy";
    assert_eq!(
        source_continuation(prefix),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
    assert!(fipe::parse::parse(fipe::lex::lex(rejected)).is_err());
    assert_eq!(source_continuation(rejected), SourceContinuation::Complete);
}

#[test]
fn an_earlier_invalid_escape_wins_even_when_a_later_lf_is_consumed() {
    assert_eq!(
        source_continuation("\"bad\\q\\\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn hard_invalid_raw_quote_is_complete_without_changing_repl_recovery() {
    assert_eq!(
        source_continuation("'bad\u{200b}"),
        SourceContinuation::Complete
    );
}

#[test]
fn all_five_pipes_continue_through_a_comment_until_a_blank_line() {
    for pipe in ["|", "|:", "|!", "||", "&&"] {
        let open = format!("1 {pipe} # comment\n");
        assert_eq!(
            source_continuation(&open),
            SourceContinuation::Open(ContinuationReason::LineContinuation),
            "{pipe}"
        );
        assert_eq!(
            source_continuation(&format!("{open}\n")),
            SourceContinuation::Complete,
            "{pipe}"
        );
    }
}

#[test]
fn joins_use_lexer_spacing_and_survive_only_in_uncovered_trivia() {
    for source in ["1 \\\u{a0}\n", "1\n \\"] {
        assert_eq!(
            source_continuation(source),
            SourceContinuation::Open(ContinuationReason::LineContinuation)
        );
    }
    assert_eq!(
        source_continuation("1 \\\n+ 2"),
        SourceContinuation::Complete
    );
    assert_eq!(
        source_continuation("1 # \\\n"),
        SourceContinuation::Complete
    );
}

#[test]
fn standalone_docs_and_nested_open_blocks_keep_distinct_reasons() {
    assert_eq!(
        source_continuation("## Value.\n"),
        SourceContinuation::Open(ContinuationReason::Documentation)
    );
    assert_eq!(
        source_continuation("{\n## Value.\n"),
        SourceContinuation::Open(ContinuationReason::Delimiter)
    );
    assert_eq!(
        source_continuation("1 ## Value.\n"),
        SourceContinuation::Complete
    );
    assert_eq!(source_continuation(""), SourceContinuation::Complete);
}
