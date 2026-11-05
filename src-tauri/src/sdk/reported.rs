//! Conditions Locus detects and reports on purpose: a stale snapshot, a busy
//! checkout, a plan that is not ready, a blocking Unity dialog. They reach the
//! SDK as JSON-RPC `error.data = {reported, kind, hint}` so the Python tool can
//! show the message and its next step without the SDK's internal frames.
//! Everything else stays unclassified and keeps its complete traceback; add a
//! message here only when it is an expected outcome with a known next step.
use serde_json::{json, Value};

const REPLAN: &str = "Nothing was written. The checkout changed after this job's snapshot; check that the change is expected, then `job = await job.replan()` prepares a fresh snapshot from the same source commits and keeps the decisions for files whose destination is unchanged (`job.replan_report` lists what was dropped). Preview the new plan and apply with its plan_hash.";
const AFTER_APPLY: &str = "The destination changed after this job was applied (by the Editor, a user or another tool). Inspect the named path with git status/diff. To redo the integration on the current state, `await plan.abort()` (restores only this job's own writes and keeps later edits), then `job = await job.replan()`.";
const DURING_APPLY: &str = "A planned file changed while applying. `await plan.abort()` restores only this job's own writes and keeps later edits; then `job = await job.replan()`.";
const DESTINATION_MOVED: &str = "The job is bound to the previous branch or checkout assignment and cannot be replanned; prepare a new job for the current destination.";
const DURING_PREPARE: &str = "The checkout changed while the job was being prepared; prepare again once other writers are idle.";
const PLAN_HASH: &str = "The plan changed after the preview that produced this hash; run `preview = await plan.preview()` and apply with `preview.plan_hash`.";
const PLAN_DURING_VALIDATION: &str = "Decisions changed during validation; preview and validate the plan again.";
const ASSET_REVISION: &str = "Read the asset again with `plan.assets.read(path)` and retry with its current revision.";
const NOT_READY: &str = "Inspect `(await plan.preview()).issues`, change the plan to resolve them, preview again, then apply with the new plan_hash.";
const BUSY: &str = "Another operation holds this checkout and nothing was changed; retry after it finishes.";

pub(super) fn classify(method: &str, message: &str) -> Option<Value> {
    let (kind, hint) = if crate::unity_bridge::dialog::is_unity_modal_dialog_blocked_error(message) {
        // The message already names the dialog, its choices and how to continue.
        ("unity_modal_dialog_blocked", None)
    } else if let Some(action) = method.strip_prefix("merges.") {
        merge_condition(action, message)?
    } else if method.starts_with("assets.") && busy(message) {
        ("busy", Some(BUSY))
    } else {
        return None;
    };
    let mut data = json!({"reported": true, "kind": kind});
    if let Some(hint) = hint {
        data["hint"] = json!(hint);
    }
    Some(data)
}

fn busy(message: &str) -> bool {
    message.starts_with("Destination is busy")
        || message.starts_with("Destination Git filesystem operation is busy")
        || message.starts_with("Another merge operation is active")
}

fn merge_condition(action: &str, message: &str) -> Option<(&'static str, Option<&'static str>)> {
    let starts = |prefix: &str| message.starts_with(prefix);
    Some(if busy(message) {
        ("busy", Some(BUSY))
    } else if starts("stale: preview hash does not match") {
        ("plan_changed", Some(PLAN_HASH))
    } else if starts("stale: plan changed while validating") || starts("stale: merge plan changed during Unity validation") {
        ("plan_changed", Some(PLAN_DURING_VALIDATION))
    } else if starts("stale_revision:") {
        ("stale_revision", Some(ASSET_REVISION))
    } else if starts("stale during apply:") {
        ("stale", Some(DURING_APPLY))
    } else if starts("stale: destination branch changed")
        || starts("stale: destination assignment changed")
        || starts("stale: destination pool assignment changed")
    {
        ("stale", Some(DESTINATION_MOVED))
    } else if starts("stale:") {
        let hint = match action {
            "prepare" => DURING_PREPARE,
            "apply" => REPLAN,
            _ => AFTER_APPLY,
        };
        ("stale", Some(hint))
    } else if starts("Unresolved merge dependencies/conflicts:") {
        ("not_ready", Some(NOT_READY))
    } else if (starts("Merge job ") && message.contains("no longer exists"))
        || starts("This merge job was created by an earlier Locus version")
    {
        // The message names why the job is gone and that a new prepare follows.
        ("job_removed", None)
    } else if starts("Cannot replan:")
        || starts("Apply the plan before")
        || (starts("Job is ") && message.contains("before changing decisions"))
    {
        // These messages already state the required next step.
        ("job_state", None)
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn stale_apply_is_reported_with_the_replan_step() {
        let data = classify(
            "merges.apply",
            "stale: Locus/workspace-trees/default.json changed after the snapshot; replan before writing",
        )
        .expect("reported");
        assert_eq!(data["reported"], true);
        assert_eq!(data["kind"], "stale");
        assert!(data["hint"].as_str().unwrap().contains("job.replan()"));
    }

    #[test]
    fn the_hint_follows_the_action_and_message() {
        let hint = |method: &str, message: &str| classify(method, message).unwrap()["hint"].as_str().unwrap().to_string();
        assert!(hint("merges.prepare", "stale: a.txt changed after the snapshot; replan before writing").contains("prepare again"));
        assert!(hint("merges.commit", "stale: HEAD changed before commit").contains("plan.abort()"));
        assert!(hint("merges.apply", "stale: destination branch changed").contains("cannot be replanned"));
        assert!(hint("merges.apply", "stale: preview hash does not match the current plan").contains("plan.preview()"));
        assert!(hint("assets.apply", "Destination is busy; retry the merge operation").contains("retry"));
    }

    #[test]
    fn unexpected_failures_stay_unclassified() {
        for (method, message) in [
            ("merges.apply", "Could not read LFS object abc: Access is denied"),
            ("merges.prepare", "Invalid tree entry"),
            ("tools.call", "stale: something else entirely"),
            ("assets.apply", "Could not atomically replace a.asset: denied"),
        ] {
            assert!(classify(method, message).is_none(), "{method}: {message}");
        }
    }

    #[test]
    fn a_removed_job_is_reported_without_internal_frames() {
        let data = classify("merges.preview", "Merge job 0b1a4c8d-24f2-4527-9b33-5a5084796f9a no longer exists; a job is removed once it is committed or aborted, or after its destination moved on. Prepare a new job").unwrap();
        assert_eq!(data["kind"], "job_removed");
        assert_eq!(classify("merges.get", "This merge job was created by an earlier Locus version; prepare a new job").unwrap()["kind"], "job_removed");
    }

    #[test]
    fn self_describing_conditions_carry_no_extra_hint() {
        let data = classify("merges.include", "Job is applied; abort it or prepare a new job before changing decisions").unwrap();
        assert_eq!(data["kind"], "job_state");
        assert!(data.get("hint").is_none());
        let dialog = classify("unity.editor.ensure", "code=unity_modal_dialog_blocked\nUnity 主线程已被模态弹窗阻塞。").unwrap();
        assert_eq!(dialog["kind"], "unity_modal_dialog_blocked");
    }
}
