mod harness;

use harness::Harness;

#[test]
fn a_pinned_model_without_allow_model_is_an_error() {
    let h = Harness::new_with_global_task(vec![Ok("f".into())], false);
    h.answer_available_models(None);
    h.set_reviewers("{ { name = \"deep\", model_name = \"claude/claude-opus-5\" } }");

    let err = h.review("ctx").unwrap_err();
    assert!(err.contains("allow_model"), "got: {err}");
    assert!(err.contains("deep"), "the error names the reviewer: {err}");
    assert_eq!(h.task_count(), 0, "it must not run on the wrong model");
}

#[test]
fn a_pinned_model_with_allow_model_runs() {
    let h = Harness::new_with_global_task(vec![Ok("findings".into())], true);
    h.answer_available_models(None);
    h.set_reviewers("{ { name = \"deep\", model_name = \"claude/claude-opus-5\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("findings"), "got: {out}");
    assert_eq!(
        h.model_args()[0].as_deref(),
        Some("claude/claude-opus-5"),
        "the model must reach the task call"
    );
}

#[test]
fn a_tier_only_reviewer_runs_without_the_model_field() {
    let h = Harness::new_with_global_task(vec![Ok("findings".into())], false);
    h.answer_available_models(None);
    h.set_reviewers("{ { name = \"cheap\", model_tier = \"weak\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("findings"), "got: {out}");
    assert_eq!(h.task_input(0)["model_tier"], "weak");
}

#[test]
fn a_reviewer_without_a_model_is_unaffected() {
    let h = Harness::new_with_global_task(vec![Ok("findings".into())], false);
    h.answer_available_models(None);
    h.set_reviewers("{ { name = \"plain\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("findings"), "got: {out}");
}
