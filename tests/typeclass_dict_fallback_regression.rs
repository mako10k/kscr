use std::path::Path;

use kscr::types::{self, DictFallbackDecision};

#[test]
fn repro_return_in_letrec_fail_typechecks_and_records_fallback_trace() {
    let tm = types::typecheck_file(Path::new("tests/repro_return_in_letrec_fail.ks"))
        .expect("repro_return_in_letrec_fail.ks should typecheck");

    assert!(
        tm.dict_fallback_trace.iter().any(|e| {
            e.method_name == "return"
                && matches!(
                    e.decision,
                    DictFallbackDecision::SelectedFromInferredApplicationType
                        | DictFallbackDecision::SelectedFromEnclosingBindingReturnType
                )
        }),
        "missing return fallback decision trace: {:?}",
        tm.dict_fallback_trace
    );
}
