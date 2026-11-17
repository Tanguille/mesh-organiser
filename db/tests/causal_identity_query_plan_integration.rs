//! Local-only event volume must not expand portable identity-claim lookups.
mod common;

#[tokio::test]
async fn portable_identity_claim_indexes_exclude_hot_local_history() {
    let (_dir, db) = common::test_db().await;
    for name in ["causal_event_global_claim", "causal_event_birth_claim"] {
        let index: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='index' AND name=?")
                .bind(name)
                .fetch_one(&db)
                .await
                .unwrap();
        let normalized = index.to_lowercase();
        assert!(
            normalized.contains("where")
                && normalized.contains("encoding")
                && normalized.contains("portable_v2"),
            "{name} must exclude local-only journal rows: {index}"
        );
    }
    // The actual claim queries must include the partial-index predicate, or SQLite
    // cannot use an index that excludes local-only history.
    let source = include_str!("../src/causal_store.rs");
    assert!(
        source.matches("encoding = 'portable_v2'").count() >= 3,
        "every received global, birth and unknown claim lookup must restrict the encoding"
    );
    for (query, index, argument_count) in [
        (
            "EXPLAIN QUERY PLAN SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id,writer_id,counter) WHERE user_id=? AND encoding = 'portable_v2' AND json_extract(envelope,'$.entity.kind')=? AND json_extract(envelope,'$.entity.global_id')=? AND json_extract(envelope,'$.entity.birth_id') IS NOT NULL AND json_extract(envelope,'$.entity.birth_id') != ?",
            "causal_event_global_claim",
            4,
        ),
        (
            "EXPLAIN QUERY PLAN SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id,writer_id,counter) WHERE user_id=? AND encoding = 'portable_v2' AND json_extract(envelope,'$.entity.kind')=? AND json_extract(envelope,'$.entity.birth_id')=? AND json_extract(envelope,'$.entity.birth_id') IS NOT NULL AND json_extract(envelope,'$.entity.global_id') != ?",
            "causal_event_birth_claim",
            4,
        ),
        (
            "EXPLAIN QUERY PLAN SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id,writer_id,counter) WHERE user_id=? AND encoding = 'portable_v2' AND json_extract(envelope,'$.entity.kind')=? AND json_extract(envelope,'$.entity.global_id')=? AND json_extract(envelope,'$.entity.birth_id') IS NULL",
            "causal_event_global_claim",
            3,
        ),
    ] {
        let mut statement = sqlx::query_as(query)
            .bind(1_i64)
            .bind("group")
            .bind("identity");
        if argument_count == 4 {
            statement = statement.bind("other");
        }
        let plan: Vec<(i64, i64, i64, String)> = statement.fetch_all(&db).await.unwrap();
        assert!(
            plan.iter().any(|row| row.3.contains(index)),
            "claim lookup should use {index}: {plan:?}"
        );
    }
}
