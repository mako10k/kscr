#[test]
fn scaffold_parser_accepts_binding() {
    let m = crate::parser::parse_module("x = 1").unwrap();
    assert_eq!(m.items.len(), 1);
}
