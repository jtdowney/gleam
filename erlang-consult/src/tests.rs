// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 The Gleam contributors

use std::collections::BTreeMap;

use num_bigint::BigInt;

use crate::{Error, ErrorKind, Term};
use pretty_assertions::assert_eq;

fn error(kind: ErrorKind, offset: usize) -> Result<Vec<Term>, Error> {
    Err(Error { kind, offset })
}

fn utf8(text: &str) -> Term {
    Term::Binary(text.as_bytes().to_vec())
}

fn charlist(text: &str) -> Term {
    Term::List(
        text.chars()
            .map(|character| Term::Integer(BigInt::from(u32::from(character))))
            .collect(),
    )
}

#[test]
fn empty_input() {
    assert_eq!(crate::read_all(b""), Ok(vec![]));
}

#[test]
fn arbitrary_atoms_and_form_boundaries() {
    assert_eq!(
        crate::read_all(b"wibble@Wobble_42.true.nil."),
        Ok(vec![
            Term::Atom("wibble@Wobble_42".into()),
            Term::Atom("true".into()),
            Term::Atom("nil".into()),
        ])
    );
}

#[test]
fn quoted_atoms() {
    assert_eq!(
        crate::read_all(r#"'true'. 'two words'. ''. 'don\'t\\stop'. 'été "quoted"'."#.as_bytes()),
        Ok(vec![
            Term::Atom("true".into()),
            Term::Atom("two words".into()),
            Term::Atom("".into()),
            Term::Atom("don't\\stop".into()),
            Term::Atom("été \"quoted\"".into()),
        ])
    );
}

#[test]
fn unterminated_quoted_atom_reports_eof() {
    assert_eq!(crate::read_all(b"'ok"), error(ErrorKind::UnexpectedEnd, 3));
    assert_eq!(
        crate::read_all(b"'ok\\"),
        error(ErrorKind::UnexpectedEnd, 4)
    );
}

#[test]
fn required_terminator_offsets() {
    assert_eq!(crate::read_all(b"ok"), error(ErrorKind::UnexpectedEnd, 2));
    assert_eq!(
        crate::read_all(b"ok next."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn duplicate_form_terminator() {
    assert_eq!(
        crate::read_all(b"ok.."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn ascii_control_whitespace() {
    assert_eq!(
        crate::read_all(b"\0\t\r\n ok \x1f . "),
        Ok(vec![Term::Atom("ok".into())])
    );
}

#[test]
fn non_ascii_whitespace_is_not_ignored() {
    assert_eq!(
        crate::read_all(b"ok.\xc2\xa0"),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn comments_between_forms_and_at_eof() {
    assert_eq!(
        crate::read_all(b"% header\r\nfirst. % note\r wibble.\n second. % tail"),
        Ok(vec![
            Term::Atom("first".into()),
            Term::Atom("second".into()),
        ])
    );
    assert_eq!(crate::read_all(b"% only comment"), Ok(vec![]));
}

#[test]
fn comments_between_nested_syntax() {
    assert_eq!(
        crate::read_all(
            br#"#{% key
a% arrow
=>[% list
<<1,% next byte
2>>,<<"100%">>]% map end
}."#
        ),
        Ok(vec![Term::Map(BTreeMap::from([(
            Term::Atom("a".into()),
            Term::List(vec![
                Term::Binary(vec![1, 2]),
                Term::Binary(b"100%".to_vec()),
            ]),
        )]))])
    );
}

#[test]
fn comment_cannot_replace_form_terminator() {
    let input = b"ok % no dot";
    assert_eq!(
        crate::read_all(input),
        error(ErrorKind::UnexpectedEnd, input.len())
    );
}

#[test]
fn unsupported_term_starts_are_rejected() {
    let inputs: [&[u8]; 3] = [b".", b"Upper.", b"_atom."];
    assert_eq!(
        inputs.map(crate::read_all),
        inputs.map(|_| error(ErrorKind::UnexpectedSyntax, 0))
    );
}

#[test]
fn long_atom_preserves_full_payload() {
    let atom = "a".repeat(100_000);
    let input = format!("{atom}.");

    assert_eq!(
        crate::read_all(input.as_bytes()),
        Ok(vec![Term::Atom(atom)])
    );
}

#[test]
fn integers() {
    assert_eq!(
        crate::read_all(b"[0,26,007,-1,- % comment\n 42]."),
        Ok(vec![Term::List(vec![
            Term::Integer(0.into()),
            Term::Integer(26.into()),
            Term::Integer(7.into()),
            Term::Integer((-1).into()),
            Term::Integer((-42).into()),
        ])])
    );
}

#[test]
fn signed_and_parenthesised_integers() {
    assert_eq!(
        crate::read_all(b"[+1,+ 2,-(3),+(4),- ( (5) ) ]."),
        Ok(vec![Term::List(vec![
            Term::Integer(1.into()),
            Term::Integer(2.into()),
            Term::Integer((-3).into()),
            Term::Integer(4.into()),
            Term::Integer((-5).into()),
        ])])
    );
}

#[test]
fn integers_beyond_64_bits() {
    let digits = "9".repeat(100);
    let input = format!("{digits}.-{digits}.");
    let value: BigInt = digits.parse().unwrap();

    assert_eq!(
        crate::read_all(input.as_bytes()),
        Ok(vec![Term::Integer(value.clone()), Term::Integer(-value)])
    );
}

#[test]
fn minus_requires_digits() {
    assert_eq!(
        crate::read_all(b"-ok."),
        error(ErrorKind::UnexpectedSyntax, 1)
    );
    assert_eq!(crate::read_all(b"-"), error(ErrorKind::UnexpectedEnd, 1));
}

#[test]
fn parenthesised_integers_reject_bad_input() {
    assert_eq!(
        crate::read_all(b"-(1."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
    assert_eq!(
        crate::read_all(b"-((1)."),
        error(ErrorKind::UnexpectedSyntax, 5)
    );
    assert_eq!(
        crate::read_all(b"-(-1)."),
        error(ErrorKind::UnexpectedSyntax, 2)
    );
    assert_eq!(
        crate::read_all(b"- -1."),
        error(ErrorKind::UnexpectedSyntax, 2)
    );
    assert_eq!(
        crate::read_all(b"(1)."),
        error(ErrorKind::UnexpectedSyntax, 0)
    );
}

#[test]
fn charlists_are_lists_of_character_codes() {
    assert_eq!(
        crate::read_all("[\"ok\", \"\", \"a\\n\" \"é\", \"π\"].".as_bytes()),
        Ok(vec![Term::List(vec![
            charlist("ok"),
            Term::List(vec![]),
            charlist("a\né"),
            charlist("π"),
        ])])
    );
}

#[test]
fn charlists_equal_lists_of_integers() {
    assert_eq!(
        crate::read_all(br#""ab". [97, 98]. "". []."#),
        Ok(vec![
            charlist("ab"),
            charlist("ab"),
            Term::List(vec![]),
            Term::List(vec![])
        ])
    );
}

#[test]
fn integer_lists_stay_lists() {
    assert_eq!(
        crate::read_all(b"[111,107]."),
        Ok(vec![Term::List(vec![
            Term::Integer(111.into()),
            Term::Integer(107.into()),
        ])])
    );
}

#[test]
fn unterminated_charlist_reports_eof() {
    assert_eq!(crate::read_all(b"\"ok"), error(ErrorKind::UnexpectedEnd, 3));
}

#[test]
fn named_escapes_decode_to_controls() {
    assert_eq!(
        crate::read_all(br#"<<"\n\r\t\v\b\f\e\s\d">>."#),
        Ok(vec![Term::Binary(vec![
            b'\n', b'\r', b'\t', 0x0B, 0x08, 0x0C, 0x1B, b' ', 0x7F,
        ])])
    );
}

#[test]
fn three_digit_octal_escapes_pack_to_eight_bits() {
    assert_eq!(
        crate::read_all(br#"<<"\101\777">>."#),
        Ok(vec![Term::Binary(vec![b'A', 0xFF])])
    );
}

#[test]
fn control_escapes_mask_character_codes() {
    assert_eq!(
        crate::read_all(br#"<<"\^A\^?">>."#),
        Ok(vec![Term::Binary(vec![0x01, 0x7F])])
    );
}

#[test]
fn control_escapes_consume_the_next_character() {
    assert_eq!(
        crate::read_all(br#"<<"\^\">>."#),
        Ok(vec![Term::Binary(vec![0x1C])])
    );
    assert_eq!(
        crate::read_all(br#"<<"a\^">>."#),
        error(ErrorKind::UnexpectedEnd, 10)
    );
}

#[test]
fn escaped_whitespace_is_kept() {
    assert_eq!(
        crate::read_all(br#"<<"\  ">>."#),
        Ok(vec![Term::Binary(vec![b' ', b' '])])
    );
    assert_eq!(
        crate::read_all(b"<<\"\\\n \">>."),
        Ok(vec![Term::Binary(vec![b'\n', b' '])])
    );
}

#[test]
fn escaped_quote_and_backslash() {
    assert_eq!(
        crate::read_all(br#"<<"\"\\">>."#),
        Ok(vec![Term::Binary(vec![b'"', b'\\'])])
    );
}

#[test]
fn raw_carriage_returns_are_preserved() {
    assert_eq!(
        crate::read_all(b"<<\"raw\rcrlf\r\nend\">>."),
        Ok(vec![Term::Binary(b"raw\rcrlf\r\nend".to_vec())])
    );
}

#[test]
fn unterminated_escape_reports_eof() {
    assert_eq!(
        crate::read_all(br#"<<"\"#),
        error(ErrorKind::UnexpectedEnd, 4)
    );
    assert_eq!(
        crate::read_all(br#"<<"\^"#),
        error(ErrorKind::UnexpectedEnd, 5)
    );
}

#[test]
fn quoted_binary() {
    assert_eq!(
        crate::read_all(br#"<<"wibble">>."#),
        Ok(vec![Term::Binary(b"wibble".to_vec())])
    );
}

#[test]
fn empty_quoted_binary_is_empty_bytes() {
    assert_eq!(
        crate::read_all(br#"<<"">>."#),
        Ok(vec![Term::Binary(vec![])])
    );
}

#[test]
fn plain_binary_packs_unicode_to_eight_bits() {
    assert_eq!(
        crate::read_all(r#"<<"é✨">>."#.as_bytes()),
        Ok(vec![Term::Binary(vec![0xE9, 0x28])])
    );
}

#[test]
fn utf8_binary_modifier_encodes_unicode_as_utf8() {
    assert_eq!(
        crate::read_all(r#"<<"é✨"/utf8>>."#.as_bytes()),
        Ok(vec![utf8("é✨")])
    );
}

#[test]
fn utf8_segments_concatenate() {
    assert_eq!(
        crate::read_all(br#"<<"a"/utf8, "b"/utf8>>."#),
        Ok(vec![utf8("ab")])
    );
}

#[test]
fn utf8_modifier_does_not_change_ascii_binaries() {
    assert_eq!(
        crate::read_all(br#"<<"ab">>. <<"ab"/utf8>>. <<97, 98>>."#),
        Ok(vec![utf8("ab"), utf8("ab"), utf8("ab")])
    );
}

#[test]
fn adjacent_binary_segments_apply_modifier_per_segment() {
    assert_eq!(
        crate::read_all(r#"<< "a" , "é" / utf8 >>."#.as_bytes()),
        Ok(vec![Term::Binary(vec![b'a', 0xC3, 0xA9])])
    );
}

#[test]
fn adjacent_strings_concatenate_before_modifier() {
    assert_eq!(
        crate::read_all(r#"<<"é" "é" /utf8, "é">>."#.as_bytes()),
        Ok(vec![Term::Binary(vec![0xC3, 0xA9, 0xC3, 0xA9, 0xE9])])
    );
}

#[test]
fn text_modifier_must_match_exact_utf8_atom() {
    assert_eq!(
        crate::read_all(br#"<<"a"/utf8extra>>."#),
        error(ErrorKind::UnexpectedSyntax, 6)
    );
}

#[test]
fn numeric_byte_binary() {
    assert_eq!(
        crate::read_all(b"<<0,127,255>>."),
        Ok(vec![Term::Binary(vec![0, 127, 255])])
    );
}

#[test]
fn empty_binary_is_empty_bytes() {
    assert_eq!(crate::read_all(b"<<>>."), Ok(vec![Term::Binary(vec![])]));
}

#[test]
fn numeric_byte_accepts_leading_zeroes() {
    assert_eq!(
        crate::read_all(b"<<000255>>."),
        Ok(vec![Term::Binary(vec![255])])
    );
}

#[test]
fn numeric_byte_rejects_out_of_range() {
    assert_eq!(
        crate::read_all(b"<<256>>."),
        error(ErrorKind::ByteOutOfRange, 2)
    );
}

#[test]
fn numeric_binary_rejects_negative_segment() {
    assert_eq!(
        crate::read_all(b"<<-1>>."),
        error(ErrorKind::UnexpectedSyntax, 2)
    );
}

#[test]
fn numeric_binary_rejects_float_segment() {
    assert_eq!(
        crate::read_all(b"<<1.0>>."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn numeric_binary_rejects_trailing_comma() {
    assert_eq!(
        crate::read_all(b"<<1,>>."),
        error(ErrorKind::UnexpectedSyntax, 4)
    );
}

#[test]
fn numeric_binary_rejects_utf8_modifier() {
    assert_eq!(
        crate::read_all(b"<<1/utf8>>."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn mixed_binary_segments() {
    assert_eq!(
        crate::read_all(br#"<<"a",1>>. <<1,"a">>."#),
        Ok(vec![
            Term::Binary(vec![b'a', 1]),
            Term::Binary(vec![1, b'a']),
        ])
    );
}

#[test]
fn mixed_binary_segments_preserve_text_encoding() {
    assert_eq!(
        crate::read_all(r#"<<0,"é" "✨"/utf8,255,"é",1>>."#.as_bytes()),
        Ok(vec![Term::Binary(vec![
            0, 0xC3, 0xA9, 0xE2, 0x9C, 0xA8, 255, 0xE9, 1,
        ])])
    );
}

#[test]
fn mixed_binary_segments_reject_out_of_range_byte() {
    assert_eq!(
        crate::read_all(br#"<<"a",256>>."#),
        error(ErrorKind::ByteOutOfRange, 6)
    );
}

#[test]
fn truncated_empty_binary_reports_eof() {
    assert_eq!(crate::read_all(b"<<"), error(ErrorKind::UnexpectedEnd, 2));
}

#[test]
fn truncated_numeric_binary_reports_eof() {
    assert_eq!(crate::read_all(b"<<1,"), error(ErrorKind::UnexpectedEnd, 4));
}

#[test]
fn truncated_quoted_binary_reports_eof() {
    assert_eq!(
        crate::read_all(br#"<<"a","#),
        error(ErrorKind::UnexpectedEnd, 6)
    );
}

#[test]
fn truncated_binary_modifier_reports_eof() {
    assert_eq!(
        crate::read_all(br#"<<"a"/"#),
        error(ErrorKind::UnexpectedEnd, 6)
    );
}

#[test]
fn tuple_values() {
    assert_eq!(
        crate::read_all(b"{ok,{}}."),
        Ok(vec![Term::Tuple(vec![
            Term::Atom("ok".into()),
            Term::Tuple(vec![]),
        ])])
    );
}

#[test]
fn lists() {
    assert_eq!(
        crate::read_all(b"[{},[ok],[]]."),
        Ok(vec![Term::List(vec![
            Term::Tuple(vec![]),
            Term::List(vec![Term::Atom("ok".into())]),
            Term::List(vec![]),
        ])])
    );
}

#[test]
fn list_rejects_trailing_comma() {
    assert_eq!(
        crate::read_all(b"[ok,]."),
        error(ErrorKind::UnexpectedSyntax, 4)
    );
}

#[test]
fn tuple_rejects_mismatched_closer() {
    assert_eq!(
        crate::read_all(b"{ok]."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn tuple_requires_element_separator() {
    assert_eq!(
        crate::read_all(b"{ok other}."),
        error(ErrorKind::UnexpectedSyntax, 4)
    );
}

#[test]
fn unterminated_list_reports_eof() {
    assert_eq!(crate::read_all(b"[ok"), error(ErrorKind::UnexpectedEnd, 3));
}

#[test]
fn map_duplicate_keys_keep_last_value() {
    assert_eq!(
        crate::read_all(b"#{a=>old,a=>new}."),
        Ok(vec![Term::Map(BTreeMap::from([(
            Term::Atom("a".into()),
            Term::Atom("new".into()),
        )]))])
    );
}

#[test]
fn map_accepts_every_term_kind_as_a_key() {
    assert_eq!(
        crate::read_all(
            br#"#{a=>atom,{}=>tuple,[]=>list,<<65>>=>bytes,<<"text">>=>text,#{}=>map}."#
        ),
        Ok(vec![Term::Map(BTreeMap::from([
            (Term::Atom("a".into()), Term::Atom("atom".into())),
            (Term::Tuple(Vec::new()), Term::Atom("tuple".into())),
            (Term::List(Vec::new()), Term::Atom("list".into())),
            (Term::Binary(vec![65]), Term::Atom("bytes".into())),
            (Term::Binary(b"text".to_vec()), Term::Atom("text".into())),
            (Term::Map(BTreeMap::new()), Term::Atom("map".into())),
        ]))])
    );
}

#[test]
fn maps_with_different_insertion_order_are_equal_keys() {
    assert_eq!(
        crate::read_all(b"#{#{a=>b,c=>d}=>old,#{c=>d,a=>b}=>new}."),
        Ok(vec![Term::Map(BTreeMap::from([(
            Term::Map(BTreeMap::from([
                (Term::Atom("a".into()), Term::Atom("b".into())),
                (Term::Atom("c".into()), Term::Atom("d".into())),
            ])),
            Term::Atom("new".into()),
        )]))])
    );
}

#[test]
fn binary_map_keys_compare_by_value() {
    assert_eq!(
        crate::read_all(r#" #{<<"é">>=>old,<<"é"/utf8>>=>utf8,<<233>>=>bytes}. "#.as_bytes()),
        Ok(vec![Term::Map(BTreeMap::from([
            (utf8("é"), Term::Atom("utf8".into())),
            (Term::Binary(vec![0xE9]), Term::Atom("bytes".into())),
        ]))])
    );
}

#[test]
fn charlist_and_binary_map_keys_are_distinct() {
    assert_eq!(
        crate::read_all(br#"#{"a" => list, <<"a"/utf8>> => binary}."#),
        Ok(vec![Term::Map(BTreeMap::from([
            (charlist("a"), Term::Atom("list".into())),
            (utf8("a"), Term::Atom("binary".into())),
        ]))])
    );
}

#[test]
fn map_hash_and_brace_can_be_separated() {
    assert_eq!(
        crate::read_all(b"# % note\n {a => b}."),
        Ok(vec![Term::Map(BTreeMap::from([(
            Term::Atom("a".into()),
            Term::Atom("b".into()),
        )]))])
    );
}

#[test]
fn hash_without_brace_is_rejected() {
    assert_eq!(
        crate::read_all(b"# a."),
        error(ErrorKind::UnexpectedSyntax, 2)
    );
}

#[test]
fn map_rejects_colon_equals_separator() {
    assert_eq!(
        crate::read_all(b"#{a:=b}."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn map_requires_value_after_arrow() {
    assert_eq!(
        crate::read_all(b"#{a=>}."),
        error(ErrorKind::UnexpectedSyntax, 5)
    );
}

#[test]
fn map_rejects_trailing_comma() {
    assert_eq!(
        crate::read_all(b"#{a=>b,}."),
        error(ErrorKind::UnexpectedSyntax, 7)
    );
}

#[test]
fn characters_and_floats_are_rejected() {
    assert_eq!(
        crate::read_all(b"$c."),
        error(ErrorKind::UnexpectedSyntax, 0)
    );
    assert_eq!(
        crate::read_all(b"1.0."),
        error(ErrorKind::UnexpectedSyntax, 1)
    );
}

#[test]
fn based_and_separated_integers_are_rejected() {
    assert_eq!(
        crate::read_all(b"16#ff."),
        error(ErrorKind::UnexpectedSyntax, 2)
    );
    assert_eq!(
        crate::read_all(b"1_000."),
        error(ErrorKind::UnexpectedSyntax, 1)
    );
}

#[test]
fn non_ascii_unquoted_atoms_are_rejected() {
    assert_eq!(
        crate::read_all("été.".as_bytes()),
        error(ErrorKind::UnexpectedSyntax, 0)
    );
}

#[test]
fn list_rejects_improper_tail() {
    assert_eq!(
        crate::read_all(b"[ok|nil]."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn binary_segment_sizes_are_rejected() {
    assert_eq!(
        crate::read_all(b"<<1:8>>."),
        error(ErrorKind::UnexpectedSyntax, 3)
    );
}

#[test]
fn quoted_binary_rejects_other_modifier() {
    assert_eq!(
        crate::read_all(br#"<<"a"/binary>>."#),
        error(ErrorKind::UnexpectedSyntax, 6)
    );
}

#[test]
fn fun_references_are_rejected() {
    assert_eq!(
        crate::read_all(b"fun lists:map/2."),
        error(ErrorKind::UnexpectedSyntax, 4)
    );
}

#[test]
fn octal_escapes_read_up_to_three_digits() {
    assert_eq!(
        crate::read_all(br#"<<"\0\12\1011">>."#),
        Ok(vec![Term::Binary(vec![0x00, b'\n', b'A', b'1'])])
    );
}

#[test]
fn unknown_escapes_drop_backslash() {
    assert_eq!(
        crate::read_all(br#"<<"\q">>."#),
        Ok(vec![Term::Binary(b"q".to_vec())])
    );
}

#[test]
fn hex_escapes() {
    assert_eq!(
        crate::read_all(br#"<<"\x41\x{4a}\x{3c0}"/utf8>>."#),
        Ok(vec![utf8("AJ\u{3c0}")])
    );
}

#[test]
fn malformed_hex_escapes_are_a_literal_x() {
    assert_eq!(
        crate::read_all(br#"<<"\x4">>. <<"\x{}">>. <<"\x{110000}">>. <<"src\xml">>."#),
        Ok(vec![
            Term::Binary(b"x4".to_vec()),
            Term::Binary(b"x{}".to_vec()),
            Term::Binary(b"x{110000}".to_vec()),
            Term::Binary(b"srcxml".to_vec()),
        ])
    );
}

#[test]
fn sigils_are_rejected() {
    assert_eq!(
        crate::read_all(br#"~"wibble"."#),
        error(ErrorKind::UnexpectedSyntax, 0)
    );
}

#[test]
fn invalid_late_byte_selects_latin1_for_everything() {
    assert_eq!(
        crate::read_all(b"<<\"\xc3\xa9\">>.<<\"\xff\">>."),
        Ok(vec![
            Term::Binary(vec![0xC3, 0xA9]),
            Term::Binary(vec![0xFF])
        ])
    );
}

#[test]
fn incomplete_utf8_uses_latin1_fallback() {
    assert_eq!(
        crate::read_all(b"<<\"\xc3\">>."),
        Ok(vec![Term::Binary(vec![0xC3])])
    );
}

#[test]
fn utf8_modifier_under_latin1_fallback_encodes_utf8() {
    assert_eq!(crate::read_all(b"<<\"\xe9\"/utf8>>."), Ok(vec![utf8("é")]));
}

#[test]
fn original_byte_offsets_in_both_encodings() {
    assert_eq!(
        crate::read_all(r#"<<"é">> ?"#.as_bytes()),
        error(ErrorKind::UnexpectedSyntax, 9)
    );
    assert_eq!(
        crate::read_all(b"<<\"\xe9\">> ?"),
        error(ErrorKind::UnexpectedSyntax, 8)
    );
}

#[test]
fn list_depth_boundary() {
    let source = format!("{}ok{}.", "[".repeat(64), "]".repeat(64));
    let expected = (0..64).fold(Term::Atom("ok".into()), |term, _| Term::List(vec![term]));
    assert_eq!(crate::read_all(source.as_bytes()), Ok(vec![expected]));

    let source = format!("{}ok{}.", "[".repeat(65), "]".repeat(65));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64)
    );
}

#[test]
fn tuple_depth_boundary() {
    let source = format!("{}ok{}.", "{".repeat(64), "}".repeat(64));
    let expected = (0..64).fold(Term::Atom("ok".into()), |term, _| Term::Tuple(vec![term]));
    assert_eq!(crate::read_all(source.as_bytes()), Ok(vec![expected]));

    let source = format!("{}ok{}.", "{".repeat(65), "}".repeat(65));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64)
    );
}

#[test]
fn map_value_depth_boundary() {
    let source = format!("{}ok{}.", "#{a=>".repeat(64), "}".repeat(64));
    let expected = (0..64).fold(Term::Atom("ok".into()), |value, _| {
        Term::Map(BTreeMap::from([(Term::Atom("a".into()), value)]))
    });
    assert_eq!(crate::read_all(source.as_bytes()), Ok(vec![expected]));

    let source = format!("{}ok{}.", "#{a=>".repeat(65), "}".repeat(65));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64 * 5)
    );
}

#[test]
fn map_key_depth_boundary() {
    let source = format!("{}a{}.", "#{".repeat(64), "=>leaf}".repeat(64));
    let expected = (0..64).fold(Term::Atom("a".into()), |key, _| {
        Term::Map(BTreeMap::from([(key, Term::Atom("leaf".into()))]))
    });
    assert_eq!(crate::read_all(source.as_bytes()), Ok(vec![expected]));

    let source = format!("{}a{}.", "#{".repeat(65), "=>leaf}".repeat(65));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 2 * 64)
    );
}

#[test]
fn empty_list_counts_toward_depth_limit() {
    let source = format!("{}[]{}.", "[".repeat(64), "]".repeat(64));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64)
    );
}

#[test]
fn empty_tuple_counts_toward_depth_limit() {
    let source = format!("{}{}{}.", "{".repeat(64), "{}", "}".repeat(64));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64)
    );
}

#[test]
fn empty_map_counts_toward_depth_limit() {
    let source = format!("{}#{{}}{}.", "#{a=>".repeat(64), "}".repeat(64));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::NestingLimit, 64 * 5)
    );
}

#[test]
fn binary_does_not_count_toward_depth_limit() {
    let source = format!("{}<<65>>{}.", "[".repeat(64), "]".repeat(64));
    let expected = (0..64).fold(Term::Binary(vec![65]), |term, _| Term::List(vec![term]));
    assert_eq!(crate::read_all(source.as_bytes()), Ok(vec![expected]));
}

#[test]
fn deep_eof_and_wrong_delimiter_keep_original_offsets() {
    let eof = format!("{}ok", "[".repeat(64));
    assert_eq!(
        crate::read_all(eof.as_bytes()),
        error(ErrorKind::UnexpectedEnd, 66)
    );

    let wrong_delimiter = format!("{}ok}}{}.", "[".repeat(64), "]".repeat(63));
    assert_eq!(
        crate::read_all(wrong_delimiter.as_bytes()),
        error(ErrorKind::UnexpectedSyntax, 66)
    );
}

#[test]
fn long_text_run_with_late_escape() {
    let text = "a".repeat(100_000);
    let input = format!(r#"<<"{text}\n">>."#);
    let expected = format!("{text}\n").into_bytes();

    assert_eq!(
        crate::read_all(input.as_bytes()),
        Ok(vec![Term::Binary(expected)])
    );
}

#[test]
fn numeric_byte_rejects_long_decimal() {
    let source = format!("<<{}>>.", "9".repeat(100_000));
    assert_eq!(
        crate::read_all(source.as_bytes()),
        error(ErrorKind::ByteOutOfRange, 2)
    );
}
