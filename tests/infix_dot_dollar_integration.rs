use kscr::parser_impl::parse_module;
use std::process::Command;

#[test]
fn infix_dot_dollar_execution() {
    let status = Command::new("cargo")
        .args(&[
            "run",
            "--bin",
            "kscr",
            "--",
            "run",
            "tests/infix_dot_dollar.ks",
        ])
        .status()
        .expect("failed to run kscr");

    assert!(
        status.success(),
        "infix_dot_dollar.ks should execute successfully"
    );
}

#[test]
fn parse_dot_composition() {
    // Test that we can parse function composition with spaces
    let src = "module TestDotCompose where\n  f = g . h\n";
    let result = parse_module(src);
    assert!(
        result.is_ok(),
        "should parse dot composition: {:?}",
        result.err()
    );
}

#[test]
fn parse_dollar_application() {
    // Test that we can parse $ application
    let src = "module TestDollar where\n  x = f $ g $ h 1\n";
    let result = parse_module(src);
    assert!(
        result.is_ok(),
        "should parse $ application: {:?}",
        result.err()
    );
}

#[test]
fn parse_dot_sections() {
    // Test that we can parse dot sections
    let src = "module TestDotSections where\n  f = (.)\n  g = (.h)\n  i = (k.)\n";
    let result = parse_module(src);
    assert!(
        result.is_ok(),
        "should parse dot sections: {:?}",
        result.err()
    );
}

#[test]
fn parse_traditional_qualification() {
    // Test that traditional module qualification still works
    let src =
        "module TestQualification where\n  import Prelude\n  x = Prelude.map\n  y = A.B.C.func\n";
    let result = parse_module(src);
    assert!(
        result.is_ok(),
        "should parse traditional qualification: {:?}",
        result.err()
    );
}

#[test]
fn parse_traditional_field_access() {
    // Test that field access still works (no-op test since field syntax uses Dot token)
    // The lexer test already confirms a.b without spaces produces TokenKind::Dot
    let src = "module TestFieldAccess where\n  x = rec.field1\n  y = obj.field2\n";
    let result = parse_module(src);
    // This will parse successfully - the semantic check for field existence comes later
    assert!(
        result.is_ok(),
        "should parse field access: {:?}",
        result.err()
    );
}
