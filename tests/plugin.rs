mod harness;

use harness::{Harness, TOOL};

#[test]
fn tool_registers() {
    let h = Harness::new(vec![]);
    assert!(h.reg.has(TOOL), "multi_review should register");
}

#[test]
fn empty_reviewer_list_explains_how_to_configure() {
    let h = Harness::new(vec![]);
    let err = h.review("one file changed").unwrap_err();
    assert!(err.contains("multireview.reviewers"), "got: {err}");
}

#[test]
fn unusable_reviewers_report_a_configuration_error() {
    for (entries, expected) in [
        ("\"not a list\"", "multireview.reviewers"),
        ("42", "multireview.reviewers"),
        ("{ { name = 7 } }", "Reviewer configuration is wrong"),
        ("{ { model_name = 7 } }", "Reviewer configuration is wrong"),
        (
            "{ { name = \"ok\", model_name = true } }",
            "Reviewer configuration is wrong",
        ),
    ] {
        let h = Harness::new(vec![]);
        h.set_reviewers(entries);
        let err = h
            .review("one file changed")
            .expect_err(&format!("{entries} should be unusable"));
        assert!(err.contains(expected), "{entries}: {err}");
    }
}

#[test]
fn the_old_model_key_is_rejected_with_the_new_name() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model = \"claude/claude-opus-5\" } }");

    let err = h.review("ctx").unwrap_err();
    assert!(err.contains("model_name"), "got: {err}");
    assert_eq!(h.task_count(), 0, "it must not run with the wrong model");
}

#[test]
fn every_reviewer_gets_a_section() {
    let h = Harness::new(vec![Ok("first findings".into()), Ok("second findings".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    let out = h.review("two files changed").unwrap();
    assert_eq!(
        out,
        "## 1-one\n\nfirst findings\n\n---\n\n## 2-two\n\nsecond findings"
    );
}

#[test]
fn generated_names_use_the_last_path_segment() {
    let h = Harness::new(vec![Ok("x".into()), Ok("y".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/b/c\" }, { model_name = \"plain\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("## 1-c\n"), "got: {out}");
    assert!(out.contains("## 2-plain\n"), "got: {out}");
}

#[test]
fn name_reaches_the_prompt_and_the_header() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers(
        "{ { name = \"sql injection\", model_name = \"claude/claude-opus-5\" }, { model_name = \"a/b\" } }",
    );

    let out = h.review("ctx").unwrap();
    assert!(
        out.contains("## sql injection - claude/claude-opus-5\n"),
        "got: {out}"
    );
    assert!(out.contains("## 2-b\n"), "got: {out}");

    let prompts = h.task_prompts();
    assert!(
        prompts[0].contains("from a sql injection angle"),
        "got: {}",
        prompts[0]
    );
    assert!(
        prompts[1].contains("from a 2-b angle"),
        "got: {}",
        prompts[1]
    );
}

#[test]
fn the_review_context_reaches_every_prompt() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    h.review("src/parser.rs: added bounds check").unwrap();

    for prompt in h.task_prompts() {
        assert!(
            prompt.contains("src/parser.rs: added bounds check"),
            "got: {prompt}"
        );
    }
}

#[test]
fn a_blocked_model_is_skipped_with_a_reason() {
    let h = Harness::new(vec![Ok("survivor findings".into())]);
    h.answer_available_models(Some(vec!["a/one".to_owned()]));
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("Skipped reviewers:"), "got: {out}");
    assert!(out.contains("2-two"), "got: {out}");
    assert!(out.contains("b/two"), "got: {out}");
    assert!(out.contains("survivor findings"), "got: {out}");
    assert_eq!(h.task_count(), 1, "the blocked reviewer must not run");
}

#[test]
fn every_model_blocked_is_an_error() {
    let h = Harness::new(vec![]);
    h.answer_available_models(Some(vec!["a/one".to_owned()]));
    h.set_reviewers("{ { model_name = \"b/two\" }, { model_name = \"c/three\" } }");

    let err = h.review("ctx").unwrap_err();
    assert!(err.contains("Skipped reviewers:"), "got: {err}");
    assert!(err.contains("No reviewer could run"), "got: {err}");
    assert_eq!(h.task_count(), 0);
}

#[test]
fn a_failed_reviewer_is_reported_without_sinking_the_run() {
    let h = Harness::new(vec![
        Err("model unavailable".into()),
        Ok("healthy findings".into()),
    ]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("FAILED: model unavailable"), "got: {out}");
    assert!(out.contains("healthy findings"), "got: {out}");

    let flashes = h.flashes();
    assert_eq!(flashes.len(), 1, "got: {flashes:?}");
    assert!(flashes[0].contains("1-one"), "got: {:?}", flashes[0]);
    assert!(
        flashes[0].contains("model unavailable"),
        "got: {:?}",
        flashes[0]
    );
}

#[test]
fn an_empty_reply_is_reported_as_a_failure() {
    let h = Harness::new(vec![Ok(String::new()), Ok("real findings".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("FAILED: returned nothing"), "got: {out}");
    assert!(out.contains("real findings"), "got: {out}");
}

#[test]
fn a_partly_failed_run_does_not_blame_the_last_reviewer() {
    let h = Harness::new(vec![
        Ok("head findings".into()),
        Err("boom".into()),
        Ok("tail findings".into()),
    ]);
    h.answer_available_models(None);
    h.set_reviewers(
        "{ { model_name = \"a/one\" }, { model_name = \"b/two\" }, { model_name = \"c/three\" } }",
    );

    let out = h.review("ctx").unwrap();
    assert!(out.contains("FAILED: boom"), "got: {out}");
    assert!(out.contains("head findings"), "got: {out}");
    assert!(out.contains("tail findings"), "got: {out}");
    assert!(!out.contains("unknown error"), "got: {out}");
}

#[test]
fn every_reviewer_failing_is_an_error() {
    let h = Harness::new(vec![Err("first boom".into()), Err("second boom".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"a/one\" }, { model_name = \"b/two\" } }");

    let err = h.review("ctx").unwrap_err();
    assert!(err.contains("Every reviewer that ran failed"), "got: {err}");
    assert!(err.contains("second boom"), "got: {err}");
}

#[test]
fn the_configured_model_is_forwarded_to_the_subagent() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { model_name = \"claude/claude-opus-5\" }, {} }");

    h.review("ctx").unwrap();

    let models = h.model_args();
    assert_eq!(models[0].as_deref(), Some("claude/claude-opus-5"));
    assert_eq!(models[1], None, "an empty reviewer must not pin a model");
}

#[test]
fn depth_settings_are_forwarded_to_the_subagent() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers(
        "{ { name = \"deep\", model_name = \"a/one\", thinking = \"xhigh\" }, \
           { name = \"budgeted\", thinking = 8192 }, \
           { name = \"cheap\", model_tier = \"weak\" }, \
           {} }",
    );

    h.review("ctx").unwrap();

    assert_eq!(h.task_input(0)["thinking"], "xhigh");
    assert_eq!(h.task_input(1)["thinking"], 8192);
    assert_eq!(h.task_input(2)["model_tier"], "weak");
    assert!(
        h.task_input(3).get("thinking").is_none(),
        "an unset thinking must stay absent, not null: {:?}",
        h.task_input(3)
    );
    assert!(
        h.task_input(3).get("model_tier").is_none(),
        "an unset model_tier must stay absent: {:?}",
        h.task_input(3)
    );
}

#[test]
fn a_non_string_thinking_is_a_configuration_error() {
    for entries in [
        "{ { thinking = true } }",
        "{ { model_tier = 3 } }",
        "{ { thinking = {} } }",
    ] {
        let h = Harness::new(vec![]);
        h.set_reviewers(entries);
        let err = h
            .review("one file changed")
            .expect_err(&format!("{entries} should be unusable"));
        assert!(err.contains("Reviewer configuration is wrong"), "{entries}: {err}");
    }
}

#[test]
fn the_command_is_registered() {
    let h = Harness::new(vec![]);
    let names = h.command_names();
    assert!(
        names.iter().any(|n| n == "/multi-review"),
        "got: {names:?}"
    );
}

#[test]
fn the_command_prompts_the_agent() {
    let h = Harness::new(vec![]);
    h.run_command(harness::PLUGIN_NAME, "focus on error paths");

    let prompts = h.session_prompts();
    assert_eq!(prompts.len(), 1, "got: {prompts:?}");
    assert!(
        prompts[0].contains("focus on error paths"),
        "got: {}",
        prompts[0]
    );
    assert!(prompts[0].contains("multi_review"), "got: {}", prompts[0]);
}

#[test]
fn a_thinking_request_above_the_session_is_reported() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.answer_session_thinking("medium", None);
    h.set_reviewers(
        "{ { name = \"deep\", thinking = \"high\" }, { name = \"shallow\", thinking = \"low\" } }",
    );

    let out = h.review("ctx").unwrap();
    assert!(out.contains("capped at this session (medium)"), "got: {out}");
    assert!(out.contains("deep asked for high, ran at medium"), "got: {out}");
    assert!(
        !out.contains("shallow asked"),
        "a request below the session is left alone: {out}"
    );
}

#[test]
fn an_off_session_drops_every_thinking_request() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.answer_session_thinking("off", None);
    h.set_reviewers("{ { name = \"deep\", thinking = \"high\" } }");

    let out = h.review("ctx").unwrap();
    assert!(out.contains("deep asked for high, ran at off"), "got: {out}");
}

#[test]
fn an_adaptive_session_reports_nothing() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.answer_session_thinking("adaptive", None);
    h.set_reviewers("{ { name = \"deep\", thinking = \"high\" } }");

    let out = h.review("ctx").unwrap();
    assert!(!out.contains("capped at this session"), "got: {out}");
}

#[test]
fn an_unreadable_session_reports_no_downgrade() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.set_reviewers("{ { name = \"deep\", thinking = \"high\" } }");

    let out = h.review("ctx").unwrap();
    assert!(!out.contains("capped at this session"), "got: {out}");
    assert!(out.contains("f"), "the review still runs: {out}");
}

#[test]
fn reviewers_without_thinking_are_not_reported() {
    let h = Harness::new(vec![Ok("f".into())]);
    h.answer_available_models(None);
    h.answer_session_thinking("minimal", None);
    h.set_reviewers("{ { name = \"plain\" } }");

    let out = h.review("ctx").unwrap();
    assert!(!out.contains("capped at this session"), "got: {out}");
}
