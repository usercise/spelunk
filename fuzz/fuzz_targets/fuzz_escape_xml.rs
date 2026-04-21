#![no_main]

use libfuzzer_sys::fuzz_target;

/// Fuzz the XML-escaping transformation used in `spelunk ask` prompt building.
///
/// Run with:
///   cargo +nightly fuzz run fuzz_escape_xml -- -max_total_time=600
///
/// `escape_xml` in src/cli/cmd/ask.rs is a private function, so the equivalent
/// transformation is replicated inline.  Goal: confirm no panic on arbitrary
/// UTF-8 input.
fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    // Mirrors the private `escape_xml` in src/cli/cmd/ask.rs exactly.
    let _ = s.replace('<', "&lt;").replace('>', "&gt;");
});
