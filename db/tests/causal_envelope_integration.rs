//! Wire envelopes are typed, bounded and canonical before they are persisted or exchanged.
use serde_json::{Value, json};

use db::causal_envelope::{
    MAX_CONTEXT_ITEMS, MAX_NAME_BYTES, canonical_causal_envelope, parse_causal_envelope,
};

const WRITER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const WRITER_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const GROUP: &str = "gggggggggggggggggggggggggggggggg";
const MODEL: &str = "mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
const GROUP_BIRTH: &str = "cccccccccccccccccccccccccccccccc";
const MODEL_BIRTH: &str = "dddddddddddddddddddddddddddddddd";

fn dot(writer: &str, counter: &str) -> Value {
    json!({"writer_id": writer, "counter": counter})
}
fn entity(kind: &str, global: &str, birth: Option<&str>) -> Value {
    json!({"kind": kind, "global_id": global, "birth_id": birth})
}
fn group() -> Value {
    entity("group", GROUP, Some(GROUP_BIRTH))
}
fn model() -> Value {
    entity("model", MODEL, Some(MODEL_BIRTH))
}
#[allow(clippy::needless_pass_by_value)]
fn dep(entity: Value, event: Value) -> Value {
    json!({"entity": entity, "event": event})
}
#[allow(clippy::needless_pass_by_value)]
fn envelope(entity: Value, action: Value) -> Value {
    json!({"version":2, "entity":entity, "event":dot(WRITER,"10"), "parents":[], "dependencies":[], "action":action})
}
fn parse(value: &Value) -> bool {
    parse_causal_envelope(&value.to_string()).is_ok()
}

#[test]
fn supported_actions_round_trip_without_losing_typed_references() {
    let deletion = dot(WRITER, "3");
    let membership = dot(WRITER, "4");
    let mut restore_group = envelope(
        group(),
        json!({"type":"group_restored", "deletion": deletion}),
    );
    restore_group["dependencies"] = json!([dep(group(), deletion.clone())]);
    let mut add = envelope(
        group(),
        json!({"type":"group_member_added", "model":model(), "membership_event": membership}),
    );
    add["dependencies"] = json!([dep(model(), membership)]);
    let mut detach = envelope(
        model(),
        json!({"type":"model_detached", "group":group(), "deletion":deletion}),
    );
    detach["dependencies"] = json!([dep(group(), deletion.clone())]);
    let mut restore_model = envelope(
        model(),
        json!({"type":"model_restored", "group":group(), "deletion":deletion, "restoration":dot(WRITER,"5")}),
    );
    restore_model["dependencies"] = json!([dep(group(), deletion), dep(group(), dot(WRITER, "5"))]);
    let mut move_model = envelope(
        model(),
        json!({"type":"model_moved","from":null,"to":group(),"target_lifecycle":dot(WRITER,"6")}),
    );
    move_model["dependencies"] = json!([dep(group(), dot(WRITER, "6"))]);
    let cases = [
        envelope(group(), json!({"type":"group_created","name":"Cabinet"})),
        envelope(model(), json!({"type":"model_created"})),
        envelope(group(), json!({"type":"group_renamed","name":"Shelf"})),
        envelope(
            group(),
            json!({"type":"group_deleted","deletion_operation_id":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}),
        ),
        restore_group,
        add,
        move_model,
        detach,
        restore_model,
        envelope(
            entity("group", GROUP, None),
            json!({"type":"legacy_observed","observation_digest":"legacy-digest"}),
        ),
    ];
    for input in cases {
        let parsed = parse_causal_envelope(&input.to_string()).unwrap();
        let canonical = canonical_causal_envelope(&parsed).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&canonical).unwrap(), input);
        assert_eq!(
            parse_causal_envelope(&canonical).unwrap(),
            parsed,
            "{input}"
        );
    }
}

#[test]
fn reordered_context_has_one_canonical_representation() {
    let mut input = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
    input["parents"] = json!([dot(WRITER_B, "9"), dot(WRITER, "2")]);
    input["dependencies"] = json!([
        dep(model(), dot(WRITER_B, "7")),
        dep(model(), dot(WRITER, "3"))
    ]);
    let mut reordered = input.clone();
    reordered["parents"].as_array_mut().unwrap().reverse();
    reordered["dependencies"].as_array_mut().unwrap().reverse();
    let first = parse_causal_envelope(&input.to_string()).unwrap();
    let second = parse_causal_envelope(&reordered.to_string()).unwrap();
    assert_eq!(
        canonical_causal_envelope(&first).unwrap(),
        canonical_causal_envelope(&second).unwrap()
    );
    let direct_first: db::causal_envelope::CausalEnvelope = serde_json::from_value(input).unwrap();
    let direct_second: db::causal_envelope::CausalEnvelope =
        serde_json::from_value(reordered.clone()).unwrap();
    assert_eq!(
        canonical_causal_envelope(&direct_first).unwrap(),
        canonical_causal_envelope(&direct_second).unwrap()
    );
    reordered["action"]["name"] = json!("Different");
    assert_ne!(
        canonical_causal_envelope(&first).unwrap(),
        canonical_causal_envelope(&parse_causal_envelope(&reordered.to_string()).unwrap()).unwrap()
    );
}

#[test]
fn counters_are_decimal_strings_and_do_not_lose_precision() {
    for counter in ["1", "9007199254740993", "9223372036854775807"] {
        let mut input = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
        input["event"]["counter"] = json!(counter);
        let canonical =
            canonical_causal_envelope(&parse_causal_envelope(&input.to_string()).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&canonical).unwrap()["event"]["counter"],
            counter
        );
    }
    for counter in [
        json!(0),
        json!(1),
        json!(1.0),
        json!("0"),
        json!("-1"),
        json!("+1"),
        json!("01"),
        json!(" 1"),
        json!("9223372036854775808"),
        json!(""),
    ] {
        let mut input = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
        input["event"]["counter"] = counter;
        assert!(!parse(&input), "{input}");
    }
}

#[test]
fn unknown_fields_kinds_and_tags_are_rejected_at_every_level() {
    let base = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
    let mut cases = Vec::new();
    let mut value = base.clone();
    value["extra"] = json!(1);
    cases.push(value);
    let mut value = base.clone();
    value["entity"]["extra"] = json!(1);
    cases.push(value);
    let mut value = base.clone();
    value["event"]["extra"] = json!(1);
    cases.push(value);
    let mut value = base.clone();
    value["action"]["extra"] = json!(1);
    cases.push(value);
    let mut value = base.clone();
    value["action"]["type"] = json!("unknown");
    cases.push(value);
    let mut value = base.clone();
    value["entity"]["kind"] = json!("unknown");
    cases.push(value);
    let mut value = base.clone();
    value["version"] = json!(3);
    cases.push(value);
    let mut value = base.clone();
    value["dependencies"] = json!([{ "entity":model(), "event":dot(WRITER,"3"), "extra":true }]);
    cases.push(value);
    let mut value = base;
    value.as_object_mut().unwrap().remove("event");
    cases.push(value);
    for value in cases {
        assert!(!parse(&value), "{value}");
    }
}

#[test]
fn rejects_invalid_entity_action_reference_and_legacy_birth_combinations() {
    let mut cases = vec![
        envelope(
            model(),
            json!({"type":"group_renamed","name":"wrong entity"}),
        ),
        envelope(
            group(),
            json!({"type":"model_moved","from":null,"to":group(),"target_lifecycle":dot(WRITER,"2")}),
        ),
        envelope(
            entity("group", GROUP, None),
            json!({"type":"group_renamed","name":"unknown birth"}),
        ),
        envelope(
            model(),
            json!({"type":"model_moved","from":group(),"to":group(),"target_lifecycle":dot(WRITER,"2")}),
        ),
        envelope(
            model(),
            json!({"type":"model_moved","from":null,"to":model(),"target_lifecycle":dot(WRITER,"2")}),
        ),
        envelope(
            group(),
            json!({"type":"group_member_added","model":group(),"membership_event":dot(WRITER,"2")}),
        ),
        envelope(
            group(),
            json!({"type":"group_restored","deletion":dot(WRITER,"2")}),
        ),
        envelope(
            model(),
            json!({"type":"model_detached","group":group(),"deletion":dot(WRITER,"2")}),
        ),
    ];
    for (field, bad) in [
        ("global_id", ""),
        ("global_id", "short"),
        ("birth_id", "not-hex"),
        ("birth_id", "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
    ] {
        let mut input = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
        input["entity"][field] = json!(bad);
        cases.push(input);
    }
    let mut bad_writer = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
    bad_writer["event"]["writer_id"] = json!("BAD");
    cases.push(bad_writer);
    for input in cases {
        assert!(!parse(&input), "{input}");
    }
}

#[test]
fn rejects_duplicate_self_and_impossible_same_writer_dependencies() {
    let base = envelope(group(), json!({"type":"group_renamed","name":"Shelf"}));
    let mut cases = Vec::new();
    let mut input = base.clone();
    input["parents"] = json!([dot(WRITER, "2"), dot(WRITER, "2")]);
    cases.push(input);
    let mut input = base.clone();
    input["parents"] = json!([dot(WRITER, "10")]);
    cases.push(input);
    let mut input = base.clone();
    input["parents"] = json!([dot(WRITER, "11")]);
    cases.push(input);
    let mut input = base.clone();
    input["dependencies"] = json!([
        dep(model(), dot(WRITER, "2")),
        dep(model(), dot(WRITER, "2"))
    ]);
    cases.push(input);
    let mut input = base.clone();
    input["dependencies"] = json!([dep(group(), dot(WRITER, "10"))]);
    cases.push(input);
    let mut input = base;
    input["parents"] = json!([dot(WRITER, "2")]);
    input["dependencies"] = json!([dep(model(), dot(WRITER, "2"))]);
    cases.push(input);
    for input in cases {
        assert!(!parse(&input), "{input}");
    }
}

#[test]
fn group_restore_can_reference_its_deletion_as_both_parent_and_dependency() {
    let deletion = dot(WRITER, "3");
    let mut input = envelope(
        group(),
        json!({"type":"group_restored","deletion":deletion}),
    );
    input["parents"] = json!([deletion]);
    input["dependencies"] = json!([dep(group(), deletion)]);
    let parsed = parse_causal_envelope(&input.to_string()).unwrap();
    let canonical = canonical_causal_envelope(&parsed).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&canonical).unwrap(), input);
}

#[test]
fn canonical_serializer_also_rejects_programmatically_constructed_invalid_envelopes() {
    let input = envelope(
        model(),
        json!({"type":"group_renamed","name":"wrong entity"}),
    );
    let typed: db::causal_envelope::CausalEnvelope = serde_json::from_value(input).unwrap();
    assert!(canonical_causal_envelope(&typed).is_err());
}

#[test]
fn nullable_fields_must_be_present_and_nested_dots_are_validated() {
    let mut missing_birth = envelope(
        entity("group", GROUP, None),
        json!({"type":"legacy_observed","observation_digest":"legacy"}),
    );
    missing_birth["entity"]
        .as_object_mut()
        .unwrap()
        .remove("birth_id");
    assert!(!parse(&missing_birth));
    for field in ["from", "to"] {
        let mut input = envelope(
            model(),
            json!({"type":"model_moved","from":null,"to":group(),"target_lifecycle":dot(WRITER,"2")}),
        );
        input["action"].as_object_mut().unwrap().remove(field);
        assert!(!parse(&input));
    }
    let mut input = envelope(
        group(),
        json!({"type":"group_restored", "deletion": {"writer_id":WRITER,"counter":42}}),
    );
    input["dependencies"] = json!([dep(group(), dot(WRITER, "2"))]);
    assert!(!parse(&input));
    let mut input = envelope(group(), json!({"type":"group_renamed", "name":"x"}));
    input["dependencies"] = json!([dep(model(), json!({"writer_id":WRITER,"counter":"00"}))]);
    assert!(!parse(&input));
}

#[test]
fn duplicate_dot_across_different_dependency_entities_is_rejected() {
    let mut input = envelope(group(), json!({"type":"group_renamed", "name":"x"}));
    let other = entity(
        "model",
        "nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn",
        Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
    );
    input["dependencies"] = json!([dep(model(), dot(WRITER, "2")), dep(other, dot(WRITER, "2"))]);
    assert!(!parse(&input));
}

#[test]
fn name_context_and_total_wire_size_bounds_are_enforced() {
    let mut input = envelope(
        group(),
        json!({"type":"group_renamed", "name":"x".repeat(MAX_NAME_BYTES)}),
    );
    assert!(parse(&input));
    input["action"]["name"] = json!(format!("{}é", "x".repeat(MAX_NAME_BYTES - 1)));
    assert!(!parse(&input));
    let mut input = envelope(group(), json!({"type":"group_renamed", "name":"x"}));
    input["event"]["counter"] = json!((MAX_CONTEXT_ITEMS + 2).to_string());
    input["parents"] = Value::Array(
        (1..=MAX_CONTEXT_ITEMS)
            .map(|item| dot(WRITER, &item.to_string()))
            .collect(),
    );
    assert!(parse(&input));
    input["parents"]
        .as_array_mut()
        .unwrap()
        .push(dot(WRITER, &(MAX_CONTEXT_ITEMS + 1).to_string()));
    assert!(!parse(&input));
    let mut wire = envelope(group(), json!({"type":"group_renamed", "name":"x"})).to_string();
    wire.push_str(&" ".repeat(db::causal_envelope::MAX_ENVELOPE_BYTES));
    assert!(parse_causal_envelope(&wire).is_err());
}

#[test]
fn model_restore_requires_exact_group_deletion_and_restoration_dependencies() {
    let deletion = dot(WRITER, "3");
    let restoration = dot(WRITER, "5");
    let mut input = envelope(
        model(),
        json!({"type":"model_restored", "group":group(), "deletion":deletion, "restoration":restoration}),
    );
    input["dependencies"] = json!([
        dep(group(), deletion.clone()),
        dep(group(), restoration.clone())
    ]);
    assert!(parse(&input));
    let mut missing_restoration = input.clone();
    missing_restoration["action"]
        .as_object_mut()
        .unwrap()
        .remove("restoration");
    assert!(!parse(&missing_restoration));
    let mut missing_dependency = input.clone();
    missing_dependency["dependencies"] = json!([dep(group(), deletion.clone())]);
    assert!(!parse(&missing_dependency));
    let mut missing_deletion = input.clone();
    missing_deletion["dependencies"] = json!([dep(group(), restoration.clone())]);
    assert!(!parse(&missing_deletion));
    let mut wrong_group = input.clone();
    wrong_group["dependencies"] = json!([
        dep(group(), deletion.clone()),
        dep(
            entity(
                "group",
                "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh",
                Some("ffffffffffffffffffffffffffffffff")
            ),
            restoration
        )
    ]);
    assert!(!parse(&wrong_group));
    let mut same_dot = input;
    same_dot["action"]["restoration"] = deletion.clone();
    same_dot["dependencies"] = json!([dep(group(), deletion)]);
    assert!(!parse(&same_dot));
}

#[test]
fn move_to_group_requires_target_lifecycle_dependency_and_ungroup_requires_null() {
    let target = dot(WRITER, "6");
    let mut into = envelope(
        model(),
        json!({"type":"model_moved","from":null,"to":group(),"target_lifecycle":target}),
    );
    into["dependencies"] = json!([dep(group(), target.clone())]);
    assert!(parse(&into));
    let mut absent = into.clone();
    absent["action"]
        .as_object_mut()
        .unwrap()
        .remove("target_lifecycle");
    assert!(!parse(&absent));
    let mut null = into.clone();
    null["action"]["target_lifecycle"] = Value::Null;
    assert!(!parse(&null));
    let mut missing_dependency = into.clone();
    missing_dependency["dependencies"] = json!([]);
    assert!(!parse(&missing_dependency));
    let mut wrong_entity = into;
    wrong_entity["dependencies"] = json!([dep(
        entity(
            "group",
            "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh",
            Some("ffffffffffffffffffffffffffffffff")
        ),
        target.clone()
    )]);
    assert!(!parse(&wrong_entity));
    let mut ungroup = envelope(
        model(),
        json!({"type":"model_moved","from":group(),"to":null,"target_lifecycle":null}),
    );
    assert!(parse(&ungroup));
    ungroup["action"]["target_lifecycle"] = target;
    assert!(!parse(&ungroup));
}

#[test]
fn move_may_reference_an_observed_legacy_target_after_birth_adoption() {
    let legacy_group = group();
    let observed = dot(WRITER, "6");
    let mut input = envelope(
        model(),
        json!({"type":"model_moved", "from":null, "to":legacy_group, "target_lifecycle":observed}),
    );
    input["dependencies"] = json!([dep(legacy_group, observed)]);
    let parsed = parse_causal_envelope(&input.to_string()).unwrap();
    let canonical: Value =
        serde_json::from_str(&canonical_causal_envelope(&parsed).unwrap()).unwrap();
    assert_eq!(canonical["action"]["to"]["birth_id"], GROUP_BIRTH);
    assert_eq!(
        canonical["dependencies"][0]["entity"]["birth_id"],
        GROUP_BIRTH
    );
    let mut unknown = input;
    unknown["action"]["to"]["birth_id"] = Value::Null;
    unknown["dependencies"][0]["entity"]["birth_id"] = Value::Null;
    assert!(!parse(&unknown));
}
