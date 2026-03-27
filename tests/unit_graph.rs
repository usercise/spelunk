//! Unit tests for graph EdgeKind parsing and Display.

use spelunk::indexer::graph::EdgeKind;

#[test]
fn parse_known_kinds() {
    assert_eq!(EdgeKind::parse("calls"), EdgeKind::Calls);
    assert_eq!(EdgeKind::parse("extends"), EdgeKind::Extends);
    assert_eq!(EdgeKind::parse("implements"), EdgeKind::Implements);
}

#[test]
fn parse_unknown_falls_back_to_imports() {
    assert_eq!(EdgeKind::parse("imports"), EdgeKind::Imports);
    assert_eq!(EdgeKind::parse("bogus"), EdgeKind::Imports);
    assert_eq!(EdgeKind::parse(""), EdgeKind::Imports);
}

#[test]
fn display_roundtrips_through_parse() {
    for kind in [
        EdgeKind::Imports,
        EdgeKind::Calls,
        EdgeKind::Extends,
        EdgeKind::Implements,
    ] {
        assert_eq!(EdgeKind::parse(&kind.to_string()), kind);
    }
}
