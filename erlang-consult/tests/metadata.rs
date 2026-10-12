// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 The Gleam contributors

use erlang_consult::Term;
use pretty_assertions::assert_eq;

fn binary(value: &str) -> Term {
    Term::Binary(value.as_bytes().to_vec())
}

fn list(values: impl IntoIterator<Item = Term>) -> Term {
    Term::List(values.into_iter().collect())
}

fn pair(key: Term, value: Term) -> Term {
    Term::Tuple(vec![key, value])
}

#[test]
fn gleam_export_metadata() {
    let expected = vec![
        pair(binary("name"), binary("wibble")),
        pair(binary("app"), binary("wibble")),
        pair(binary("version"), binary("1.0.0")),
        pair(binary("description"), binary("Splits on  by default, café")),
        pair(binary("licenses"), list([binary("Apache-2.0")])),
        pair(binary("build_tools"), list([binary("gleam")])),
        pair(
            binary("links"),
            list([
                pair(binary("Website"), binary("https://gleam.run/")),
                pair(
                    binary("Repository"),
                    binary("https://github.com/gleam-lang/wibble"),
                ),
            ]),
        ),
        pair(
            binary("requirements"),
            list([pair(
                binary("gleam_stdlib"),
                list([
                    pair(binary("app"), binary("gleam_stdlib")),
                    pair(binary("optional"), Term::Atom("false".to_string())),
                    pair(binary("requirement"), binary(">= 1.0.0 and < 2.0.0")),
                ]),
            )]),
        ),
        pair(
            binary("files"),
            list(
                [
                    "README.md",
                    "gleam.toml",
                    "src/wibble.app.src",
                    "src/wibble.erl",
                    "src/wibble.gleam",
                ]
                .into_iter()
                .map(binary),
            ),
        ),
    ];
    assert_eq!(
        erlang_consult::read_all(include_bytes!("gleam_export.config")),
        Ok(expected)
    );
}

#[test]
fn mix_extra_metadata() {
    let expected = vec![
        pair(
            binary("extra"),
            list([
                pair(binary("min_otp"), Term::Integer(26.into())),
                pair(binary("offset"), Term::Integer((-1).into())),
                pair(
                    binary("tag"),
                    list([Term::Integer(111.into()), Term::Integer(107.into())]),
                ),
            ]),
        ),
        pair(binary("links"), list([])),
        pair(binary("name"), binary("wibble")),
        pair(binary("version"), binary("1.0.0")),
        pair(
            binary("description"),
            binary("Fixture for integer metadata"),
        ),
        pair(binary("elixir"), binary("~> 1.15")),
        pair(
            binary("files"),
            list(
                [
                    "lib",
                    "lib/wibble.ex",
                    ".formatter.exs",
                    "mix.exs",
                    "README.md",
                ]
                .into_iter()
                .map(binary),
            ),
        ),
        pair(binary("app"), binary("wibble")),
        pair(binary("licenses"), list([binary("Apache-2.0")])),
        pair(binary("requirements"), list([])),
        pair(binary("build_tools"), list([binary("mix")])),
    ];
    assert_eq!(
        erlang_consult::read_all(include_bytes!("mix_extra.config")),
        Ok(expected)
    );
}
