use idealis_sat_eval::dispatch;
use serde_json::json;

#[test]
fn sat_unsat_empty_and_concrete_models_are_distinct() {
    let sat = dispatch(
        &serde_json::to_vec(
            &json!({"variables":2,"clauses":[[1,2],[-1]],"rounds":[{"add":[],"assumptions":[]}]}),
        )
        .unwrap(),
    );
    assert_eq!(sat["kind"], "results");
    assert_eq!(sat["results"][0]["status"], "sat");
    assert_eq!(sat["results"][0]["model"], json!([false, true]));
    let empty =
        dispatch(br#"{"variables":0,"clauses":[[]],"rounds":[{"add":[],"assumptions":[]}]}"#);
    assert_eq!(empty["results"][0]["status"], "unsat");
}

#[test]
fn assumptions_are_replaced_and_new_clauses_remain_incremental() {
    let result = dispatch(br#"{"variables":2,"clauses":[[1,2]],"rounds":[{"add":[],"assumptions":[-1]},{"add":[],"assumptions":[-1,-2]},{"add":[],"assumptions":[]},{"add":[[-1],[-2]],"assumptions":[]}]}"#);
    assert_eq!(
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|answer| answer["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["sat", "unsat", "sat", "unsat"]
    );
}

#[test]
fn unsupported_work_limits_and_invalid_literals_never_succeed() {
    let unsupported = dispatch(
        br#"{"variables":1,"clauses":[],"rounds":[{"add":[],"assumptions":[]}],"maxWork":0}"#,
    );
    assert_eq!(unsupported["kind"], "unsupported");
    for input in [
        br#"{"variables":1,"clauses":[[0]],"rounds":[{"add":[],"assumptions":[]}]}"#.as_slice(),
        br#"{"variables":1,"clauses":[[2]],"rounds":[{"add":[],"assumptions":[]}]}"#,
    ] {
        assert_eq!(dispatch(input)["kind"], "error");
    }
}
